use std::{borrow::Cow, sync::Arc};

use ironclaw_authorization::{GrantAuthorizer, TrustAwareCapabilityDispatchAuthorizer};
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityDescriptor, Decision, EffectKind,
    ExecutionContext, Principal, ResourceEstimate,
    runtime_policy::{ApprovalPolicy, EffectiveRuntimePolicy},
};
use ironclaw_trust::TrustDecision;

use crate::local_dev_capability_policy::LocalDevCapabilityPolicy;

struct LocalDevApprovalPolicyAuthorizer {
    inner: GrantAuthorizer,
    approval_policy: ApprovalPolicy,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
}

impl LocalDevApprovalPolicyAuthorizer {
    fn new(
        approval_policy: ApprovalPolicy,
        capability_policy: Arc<LocalDevCapabilityPolicy>,
    ) -> Self {
        Self {
            inner: GrantAuthorizer::new(),
            approval_policy,
            capability_policy,
        }
    }
}

#[async_trait::async_trait]
impl TrustAwareCapabilityDispatchAuthorizer for LocalDevApprovalPolicyAuthorizer {
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
        require_approval_for_local_dev_policy(
            decision,
            context,
            descriptor,
            estimate,
            LocalDevApprovalActionKind::Dispatch,
            self.approval_policy,
            &self.capability_policy,
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
        require_approval_for_local_dev_policy(
            decision,
            context,
            descriptor,
            estimate,
            LocalDevApprovalActionKind::SpawnCapability,
            self.approval_policy,
            &self.capability_policy,
        )
    }
}

#[derive(Clone, Copy)]
enum LocalDevApprovalActionKind {
    Dispatch,
    SpawnCapability,
}

pub(crate) fn local_dev_authorizer(
    runtime_policy: Option<&EffectiveRuntimePolicy>,
    capability_policy: Arc<LocalDevCapabilityPolicy>,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    let approval_policy = runtime_policy
        .map(|policy| policy.approval_policy)
        .unwrap_or(ApprovalPolicy::AskDestructive);
    match approval_policy {
        // Minimal ~ yolo: skip approval gates entirely and delegate to
        // the grant authorizer only. A misconfigured policy reaching
        // build_local_dev with Minimal will run every effectful capability
        // ungated — intentional, and exercised by the Minimal integration test.
        ApprovalPolicy::Minimal => Arc::new(GrantAuthorizer::new()),
        other => Arc::new(LocalDevApprovalPolicyAuthorizer::new(
            other,
            capability_policy,
        )),
    }
}

fn require_approval_for_local_dev_policy(
    decision: Decision,
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: LocalDevApprovalActionKind,
    approval_policy: ApprovalPolicy,
    capability_policy: &LocalDevCapabilityPolicy,
) -> Decision {
    // A spawn exercises SpawnProcess even when the capability's own descriptor
    // does not declare it: the underlying GrantAuthorizer authorizes spawns
    // against `spawn_descriptor`, which adds EffectKind::SpawnProcess. Evaluate
    // the approval gate against the same elevated effect set so a dispatch-only
    // builtin (e.g. builtin.echo) cannot be spawned as a live process without
    // an approval gate.
    let gate_effects = approval_gate_effects(action_kind, descriptor);
    match decision {
        Decision::Allow { .. }
            if capability_policy.effects_require_approval(approval_policy, &gate_effects)
                && !has_matching_one_shot_approval_grant(
                    context,
                    descriptor,
                    &gate_effects,
                    approval_policy,
                    capability_policy,
                ) =>
        {
            Decision::RequireApproval {
                request: approval_request(context, descriptor, estimate, action_kind),
            }
        }
        other => other,
    }
}

/// Effects the local-dev approval gate evaluates for `action_kind`.
///
/// Mirrors `ironclaw_authorization::spawn_descriptor`: a spawn always exercises
/// `SpawnProcess`, so it is added to the capability's declared effects when
/// gating a spawn. Dispatch evaluates the declared effects unchanged.
fn approval_gate_effects(
    action_kind: LocalDevApprovalActionKind,
    descriptor: &CapabilityDescriptor,
) -> Cow<'_, [EffectKind]> {
    match action_kind {
        LocalDevApprovalActionKind::Dispatch => Cow::Borrowed(descriptor.effects.as_slice()),
        LocalDevApprovalActionKind::SpawnCapability => {
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

fn has_matching_one_shot_approval_grant(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    gate_effects: &[EffectKind],
    approval_policy: ApprovalPolicy,
    capability_policy: &LocalDevCapabilityPolicy,
) -> bool {
    // Hoist the expected grantee/issuer principals so they are not
    // re-allocated on every iteration of the any() closure.
    let expected_grantee = Principal::Extension(context.extension_id.clone());
    context.grants.grants.iter().any(|grant| {
        grant.capability == descriptor.id
            && grant.constraints.max_invocations == Some(1)
            && grant.issued_by == Principal::HostRuntime
            && grant.grantee == expected_grantee
            // Match against the spawn-elevated effect set so a one-shot lease
            // that does not cover SpawnProcess cannot satisfy a spawn gate.
            && gate_effects
                .iter()
                .all(|effect| grant.constraints.allowed_effects.contains(effect))
            && capability_policy
                .effects_require_approval(approval_policy, &grant.constraints.allowed_effects)
    })
}

fn approval_request(
    context: &ExecutionContext,
    descriptor: &CapabilityDescriptor,
    estimate: &ResourceEstimate,
    action_kind: LocalDevApprovalActionKind,
) -> ApprovalRequest {
    let action = match action_kind {
        LocalDevApprovalActionKind::Dispatch => Action::Dispatch {
            capability: descriptor.id.clone(),
            estimated_resources: estimate.clone(),
        },
        LocalDevApprovalActionKind::SpawnCapability => Action::SpawnCapability {
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
        reason: "this action requires your approval".to_string(),
        reusable_scope: None,
    }
}
