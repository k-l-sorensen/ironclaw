use std::{borrow::Cow, sync::Arc};

use async_trait::async_trait;
use ironclaw_approvals::{ToolPermissionOverride, persistent_approval_grant_issuer};
use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityDescriptor, CapabilityGrant,
    CapabilityId, Decision, DenyReason, EffectKind, ExecutionContext, Principal, ResourceEstimate,
    ResourceScope, Timestamp, runtime_policy::ApprovalPolicy,
};
use ironclaw_trust::TrustDecision;

pub(crate) trait ProfileApprovalGatePolicy: Send + Sync {
    fn capability_exempt_from_approval(&self, _capability: &CapabilityId) -> bool {
        false
    }

    fn effects_require_approval(
        &self,
        approval_policy: ApprovalPolicy,
        effects: &[EffectKind],
    ) -> bool;

    /// Hard floor (#4776/#4959): effects that ALWAYS require an explicit
    /// approval gate and can never be auto-approved or satisfied by a stored
    /// always-allow grant, regardless of `ApprovalPolicy` or the global
    /// auto-approve setting. The reborn equivalent of v1's
    /// `ApprovalRequirement::Always`, expressed per-call over the invocation's
    /// effects. Defaults to "no floor".
    fn effects_force_approval(&self, _effects: &[EffectKind]) -> bool {
        false
    }
}

/// Per-(tenant, user, capability) approval settings resolved live at dispatch
/// time so a change made in the WebUI takes effect without a process restart
/// (#4959).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ResolvedApprovalSettings {
    /// Explicit per-tool override the user set, if any.
    pub(crate) tool_override: Option<ToolPermissionOverride>,
    /// Whether the user's global "auto-approve eligible tools" toggle is on.
    pub(crate) global_auto_approve: bool,
}

/// Resolves [`ResolvedApprovalSettings`] for one dispatch. Implementations read
/// the durable per-user stores; the authorizer queries this on every gate
/// decision so settings apply per-turn without restart.
#[async_trait]
pub(crate) trait ApprovalSettingsProvider: Send + Sync {
    async fn resolve(
        &self,
        scope: &ResourceScope,
        capability_id: &CapabilityId,
    ) -> ResolvedApprovalSettings;
}

/// No stored overrides and global auto-approve off: the gate behaves exactly as
/// it did before #4959. Test-only — production wires
/// `StoreApprovalSettingsProvider`.
#[cfg(test)]
pub(crate) struct EmptyApprovalSettingsProvider;

#[cfg(test)]
#[async_trait]
impl ApprovalSettingsProvider for EmptyApprovalSettingsProvider {
    async fn resolve(
        &self,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
    ) -> ResolvedApprovalSettings {
        ResolvedApprovalSettings::default()
    }
}

pub(crate) fn profile_approval_authorizer(
    approval_policy: ApprovalPolicy,
    gate_policy: Arc<dyn ProfileApprovalGatePolicy>,
    settings: Arc<dyn ApprovalSettingsProvider>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    Arc::new(ProfileApprovalPolicyAuthorizer::new(
        approval_policy,
        gate_policy,
        settings,
    ))
}

struct ProfileApprovalPolicyAuthorizer {
    inner: GrantAuthorizer,
    approval_policy: ApprovalPolicy,
    gate_policy: Arc<dyn ProfileApprovalGatePolicy>,
    settings: Arc<dyn ApprovalSettingsProvider>,
}

impl ProfileApprovalPolicyAuthorizer {
    fn new(
        approval_policy: ApprovalPolicy,
        gate_policy: Arc<dyn ProfileApprovalGatePolicy>,
        settings: Arc<dyn ApprovalSettingsProvider>,
    ) -> Self {
        Self {
            inner: GrantAuthorizer::new(),
            approval_policy,
            gate_policy,
            settings,
        }
    }
}

#[async_trait::async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for ProfileApprovalPolicyAuthorizer {
    async fn authorize_dispatch_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        let decision = self
            .inner
            .authorize_dispatch_with_trust(context, descriptor, estimate, trust_decision)
            .await;
        let settings = self
            .settings
            .resolve(&context.resource_scope, &descriptor.id)
            .await;
        require_approval_for_profile_policy(
            decision,
            context,
            descriptor,
            estimate,
            ProfileApprovalActionKind::Dispatch,
            self.approval_policy,
            self.gate_policy.as_ref(),
            settings,
        )
    }

    async fn authorize_spawn_with_trust(
        &self,
        context: &ExecutionContext,
        descriptor: &CapabilityDescriptor,
        estimate: &ResourceEstimate,
        trust_decision: &TrustDecision,
    ) -> Decision {
        let decision = self
            .inner
            .authorize_spawn_with_trust(context, descriptor, estimate, trust_decision)
            .await;
        let settings = self
            .settings
            .resolve(&context.resource_scope, &descriptor.id)
            .await;
        require_approval_for_profile_policy(
            decision,
            context,
            descriptor,
            estimate,
            ProfileApprovalActionKind::SpawnCapability,
            self.approval_policy,
            self.gate_policy.as_ref(),
            settings,
        )
    }
}

#[derive(Clone, Copy, Debug)]
enum ProfileApprovalActionKind {
    Dispatch,
    SpawnCapability,
}

#[allow(clippy::too_many_arguments)]
// arch-exempt: too_many_args, gate decision needs context+descriptor+estimate+policy+gate+settings, plan #4776
fn require_approval_for_profile_policy(
    decision: Decision,
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: ProfileApprovalActionKind,
    approval_policy: ApprovalPolicy,
    gate_policy: &dyn ProfileApprovalGatePolicy,
    settings: ResolvedApprovalSettings,
) -> Decision {
    // The profile approval gate only ever upgrades an underlying `Allow`; a
    // `Deny` / `RequireApproval` from the grant authorizer passes through
    // unchanged.
    let Decision::Allow { .. } = &decision else {
        return decision;
    };

    // A spawn exercises SpawnProcess even when the capability's own descriptor
    // does not declare it: the underlying GrantAuthorizer authorizes spawns
    // against `spawn_descriptor`, which adds EffectKind::SpawnProcess. Evaluate
    // the approval gate against the same elevated effect set so a dispatch-only
    // capability cannot be spawned as a live process without an approval gate.
    let gate_effects = approval_gate_effects(action_kind, descriptor);

    let require_approval = || Decision::RequireApproval {
        request: approval_request(context, descriptor, estimate, action_kind),
    };

    // Decision precedence (high → low), #4776:
    // 1. Explicit per-tool `disabled` → deny outright (strongest user intent).
    if matches!(
        settings.tool_override,
        Some(ToolPermissionOverride::Disabled)
    ) {
        return Decision::Deny {
            reason: DenyReason::PolicyDenied,
        };
    }
    // 2. Hard floor: never auto-approve / never satisfiable by a stored grant.
    if gate_policy.effects_force_approval(&gate_effects) {
        return require_approval();
    }
    // 3. Explicit per-tool `ask_each_time` → always gate, ignoring the global
    //    auto-approve setting and any stored always-allow grant.
    if matches!(
        settings.tool_override,
        Some(ToolPermissionOverride::AskEachTime)
    ) {
        return require_approval();
    }
    // 4. Capability deliberately exempt from the gate (in-turn consent).
    if gate_policy.capability_exempt_from_approval(&descriptor.id) {
        return decision;
    }
    // 5. Policy does not require a gate for this effect set.
    if !gate_policy.effects_require_approval(approval_policy, &gate_effects) {
        return decision;
    }
    // 6. Global auto-approve bypasses an otherwise-gated eligible tool.
    if settings.global_auto_approve {
        return decision;
    }
    // 7. A matching one-shot lease or persistent always-allow grant satisfies
    //    the gate.
    if has_matching_approval_grant(
        context,
        descriptor,
        &gate_effects,
        approval_policy,
        gate_policy,
    ) {
        return decision;
    }
    require_approval()
}

/// Effects the profile approval gate evaluates for `action_kind`.
///
/// Mirrors `ironclaw_authorization::spawn_descriptor`: a spawn always exercises
/// `SpawnProcess`, so it is added to the capability's declared effects when
/// gating a spawn. Dispatch evaluates the declared effects unchanged.
fn approval_gate_effects(
    action_kind: ProfileApprovalActionKind,
    descriptor: &CapabilityDescriptor,
) -> Cow<'_, [EffectKind]> {
    match action_kind {
        ProfileApprovalActionKind::Dispatch => Cow::Borrowed(descriptor.effects.as_slice()),
        ProfileApprovalActionKind::SpawnCapability => {
            if descriptor.effects.contains(&EffectKind::SpawnProcess) {
                Cow::Borrowed(descriptor.effects.as_slice())
            } else {
                let mut effects = descriptor.effects.clone();
                effects.push(EffectKind::SpawnProcess);
                Cow::Owned(effects)
            }
        }
    }
}

fn has_matching_approval_grant(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    gate_effects: &[EffectKind],
    approval_policy: ApprovalPolicy,
    gate_policy: &dyn ProfileApprovalGatePolicy,
) -> bool {
    let expected_grantee = Principal::Extension(context.extension_id.clone());
    let expected_user_approver = Principal::User(context.user_id.clone());
    let persistent_approval_issuer = persistent_approval_grant_issuer();
    let now = chrono::Utc::now();
    context.grants.grants.iter().any(|grant| {
        let grant_unexpired = grant_is_unexpired(grant, &now);
        let one_shot_approval_grant = grant.constraints.max_invocations == Some(1)
            && (grant.issued_by == Principal::HostRuntime
                || grant.issued_by == expected_user_approver)
            && grant_unexpired;
        let persistent_approval_grant = grant.constraints.max_invocations.is_none()
            && grant.issued_by == persistent_approval_issuer
            && grant_unexpired;
        grant.capability == descriptor.id
            && (one_shot_approval_grant || persistent_approval_grant)
            && grant.grantee == expected_grantee
            // Match against the spawn-elevated effect set so a one-shot lease
            // that does not cover SpawnProcess cannot satisfy a spawn gate.
            && gate_effects
                .iter()
                .all(|effect| grant.constraints.allowed_effects.contains(effect))
            && gate_policy
                .effects_require_approval(approval_policy, &grant.constraints.allowed_effects)
    })
}

fn grant_is_unexpired(grant: &CapabilityGrant, now: &Timestamp) -> bool {
    grant
        .constraints
        .expires_at
        .as_ref()
        .is_none_or(|expires_at| expires_at > now)
}

fn approval_request(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: ProfileApprovalActionKind,
) -> ApprovalRequest {
    let action = match action_kind {
        ProfileApprovalActionKind::Dispatch => Action::Dispatch {
            capability: descriptor.id.clone(),
            estimated_resources: estimate.clone(),
        },
        ProfileApprovalActionKind::SpawnCapability => Action::SpawnCapability {
            capability: descriptor.id.clone(),
            estimated_resources: estimate.clone(),
        },
    };
    ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: context.correlation_id,
        requested_by: Principal::Extension(context.extension_id.clone()),
        action: Box::new(action),
        invocation_fingerprint: None,
        reason: format!(
            "approval required for {:?} of {}",
            action_kind,
            descriptor.id.as_str()
        ),
        reusable_scope: None,
    }
}

#[cfg(test)]
mod tests;
