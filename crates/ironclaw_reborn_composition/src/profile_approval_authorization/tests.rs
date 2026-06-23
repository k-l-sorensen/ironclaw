use ironclaw_host_api::{
    CapabilityDescriptor, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet,
    EffectKind, ExecutionContext, ExtensionId, GrantConstraints, MountView, NetworkPolicy,
    PermissionMode, Principal, ResourceEstimate, RuntimeKind, TrustClass,
};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use serde_json::json;

use super::*;

#[derive(Debug)]
struct TestGatePolicy;

impl ProfileApprovalGatePolicy for TestGatePolicy {
    fn effects_require_approval(
        &self,
        approval_policy: ApprovalPolicy,
        effects: &[EffectKind],
    ) -> bool {
        match approval_policy {
            ApprovalPolicy::Minimal => false,
            ApprovalPolicy::AskAlways => !effects.is_empty(),
            ApprovalPolicy::AskWrites | ApprovalPolicy::AskDestructive => {
                effects.contains(&EffectKind::SpawnProcess)
            }
            ApprovalPolicy::OrgPolicy => !effects.is_empty(),
            _ => !effects.is_empty(),
        }
    }

    fn effects_force_approval(&self, effects: &[EffectKind]) -> bool {
        effects.contains(&EffectKind::Financial)
    }
}

/// Returns fixed settings so the gate's per-turn resolution can be driven
/// deterministically (#4959).
struct StubSettingsProvider {
    tool_override: Option<ToolPermissionOverride>,
    global_auto_approve: bool,
}

#[async_trait]
impl ApprovalSettingsProvider for StubSettingsProvider {
    async fn resolve(
        &self,
        _scope: &ResourceScope,
        _capability_id: &CapabilityId,
    ) -> ResolvedApprovalSettings {
        ResolvedApprovalSettings {
            tool_override: self.tool_override,
            global_auto_approve: self.global_auto_approve,
        }
    }
}

/// Dispatch a `builtin.shell` capability carrying `effects`, with a granting
/// lease and trust ceiling that make the underlying decision `Allow`, under
/// the given approval policy + resolved settings.
async fn dispatch_decision(
    approval_policy: ApprovalPolicy,
    effects: Vec<EffectKind>,
    settings: StubSettingsProvider,
) -> Decision {
    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), effects.clone());
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: effects.clone(),
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let trust = TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: effects,
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: chrono::Utc::now(),
    };
    profile_approval_authorizer(
        approval_policy,
        Arc::new(TestGatePolicy),
        Arc::new(settings),
    )
    .authorize_dispatch_with_trust(&ctx, &descriptor, &ResourceEstimate::default(), &trust)
    .await
}

#[tokio::test]
async fn global_auto_approve_skips_gate_for_eligible_tool() {
    let decision = dispatch_decision(
        ApprovalPolicy::AskDestructive,
        vec![EffectKind::SpawnProcess],
        StubSettingsProvider {
            tool_override: None,
            global_auto_approve: true,
        },
    )
    .await;
    assert!(
        matches!(decision, Decision::Allow { .. }),
        "global auto-approve should skip the gate for an eligible tool, got {decision:?}"
    );
}

#[tokio::test]
async fn explicit_ask_each_time_overrides_global_auto_approve() {
    let decision = dispatch_decision(
        ApprovalPolicy::AskDestructive,
        vec![EffectKind::SpawnProcess],
        StubSettingsProvider {
            tool_override: Some(ToolPermissionOverride::AskEachTime),
            global_auto_approve: true,
        },
    )
    .await;
    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "explicit ask_each_time must gate even with global auto-approve on, got {decision:?}"
    );
}

#[tokio::test]
async fn explicit_disabled_denies_dispatch() {
    let decision = dispatch_decision(
        ApprovalPolicy::AskDestructive,
        vec![EffectKind::SpawnProcess],
        StubSettingsProvider {
            tool_override: Some(ToolPermissionOverride::Disabled),
            global_auto_approve: true,
        },
    )
    .await;
    assert!(
        matches!(
            decision,
            Decision::Deny {
                reason: DenyReason::PolicyDenied
            }
        ),
        "explicit disabled must deny, got {decision:?}"
    );
}

#[tokio::test]
async fn hard_floor_requires_approval_even_with_global_auto_approve() {
    let decision = dispatch_decision(
        ApprovalPolicy::AskDestructive,
        vec![EffectKind::Financial],
        StubSettingsProvider {
            tool_override: None,
            global_auto_approve: true,
        },
    )
    .await;
    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "hard-floor (Financial) must gate even with global auto-approve on, got {decision:?}"
    );
}

fn test_descriptor(effects: Vec<EffectKind>) -> CapabilityDescriptor {
    test_descriptor_with_id(CapabilityId::new("builtin.shell").unwrap(), effects)
}

fn test_descriptor_with_id(id: CapabilityId, effects: Vec<EffectKind>) -> CapabilityDescriptor {
    CapabilityDescriptor {
        id,
        provider: ExtensionId::new("builtin").unwrap(),
        runtime: RuntimeKind::FirstParty,
        trust_ceiling: TrustClass::UserTrusted,
        description: "test".to_string(),
        parameters_schema: json!({}),
        effects,
        default_permission: PermissionMode::Allow,
        runtime_credentials: Vec::new(),
        resource_profile: None,
    }
}

fn test_context(grants: CapabilitySet) -> ExecutionContext {
    let ctx = ExecutionContext::local_default(
        ironclaw_host_api::UserId::new("test-user").unwrap(),
        ExtensionId::new("builtin").unwrap(),
        RuntimeKind::FirstParty,
        TrustClass::UserTrusted,
        grants,
        MountView::default(),
    )
    .unwrap();
    ctx.validate().unwrap();
    ctx
}

fn test_trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::SpawnProcess, EffectKind::DispatchCapability],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: chrono::Utc::now(),
    }
}

fn test_authorizer(
    approval_policy: ApprovalPolicy,
) -> Arc<dyn TrustAwareCapabilityDispatchAuthorizer> {
    profile_approval_authorizer(
        approval_policy,
        Arc::new(TestGatePolicy),
        Arc::new(EmptyApprovalSettingsProvider),
    )
}

#[tokio::test]
async fn dispatch_with_destructive_effect_requires_approval() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "destructive dispatch should require approval, got {decision:?}"
    );
}

#[tokio::test]
async fn spawn_with_dispatch_only_capability_requires_approval() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let echo_id = CapabilityId::new("builtin.echo").unwrap();
    let descriptor = test_descriptor_with_id(echo_id.clone(), vec![EffectKind::DispatchCapability]);
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: echo_id,
            grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability, EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let decision = authorizer
        .authorize_spawn_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "spawn of dispatch-only capability should require approval via SpawnProcess elevation, got {decision:?}"
    );
}

#[tokio::test]
async fn minimal_policy_skips_approval_gate() {
    let authorizer = test_authorizer(ApprovalPolicy::Minimal);

    let descriptor = test_descriptor(vec![EffectKind::SpawnProcess]);
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new("builtin.shell").unwrap(),
            grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::Allow { .. }),
        "Minimal policy should delegate to GrantAuthorizer and Allow, got {decision:?}"
    );
}

#[tokio::test]
async fn user_issued_one_shot_approval_grant_allows_resume() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
    let base_ctx = test_context(CapabilitySet { grants: vec![] });
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(base_ctx.extension_id.clone()),
            issued_by: Principal::User(base_ctx.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        }],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::Allow { .. }),
        "same-user one-shot approval lease should satisfy the local-dev gate, got {decision:?}"
    );
}

#[tokio::test]
async fn persistent_approval_grant_allows_reuse() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
    let base_ctx = test_context(CapabilitySet { grants: vec![] });
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(base_ctx.extension_id.clone()),
            issued_by: persistent_approval_grant_issuer(),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::Allow { .. }),
        "persistent approval grant should satisfy the local-dev gate, got {decision:?}"
    );
}

#[tokio::test]
async fn hard_floor_requires_approval_even_with_persistent_grant() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::Financial]);
    let base_ctx = test_context(CapabilitySet { grants: vec![] });
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(base_ctx.extension_id.clone()),
            issued_by: persistent_approval_grant_issuer(),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::Financial],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let trust = TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: vec![EffectKind::Financial],
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::AdminConfig,
        evaluated_at: chrono::Utc::now(),
    };
    let decision = authorizer
        .authorize_dispatch_with_trust(&ctx, &descriptor, &ResourceEstimate::default(), &trust)
        .await;

    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "hard-floor effects must require fresh approval even when a persistent grant matches, got {decision:?}"
    );
}

#[tokio::test]
async fn user_issued_persistent_like_grant_does_not_allow_reuse() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
    let base_ctx = test_context(CapabilitySet { grants: vec![] });
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(base_ctx.extension_id.clone()),
            issued_by: Principal::User(base_ctx.user_id.clone()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "standing user grant must not impersonate persistent approval replay, got {decision:?}"
    );
}

#[tokio::test]
async fn other_user_issued_persistent_like_grant_does_not_allow_reuse() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
            issued_by: Principal::User(ironclaw_host_api::UserId::new("other-user").unwrap()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "different-user standing grant must not impersonate persistent approval replay, got {decision:?}"
    );
}

#[tokio::test]
async fn expired_persistent_approval_grant_does_not_allow_reuse() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
    let base_ctx = test_context(CapabilitySet { grants: vec![] });
    let ctx = test_context(CapabilitySet {
        grants: vec![
            CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id.clone(),
                grantee: Principal::Extension(base_ctx.extension_id.clone()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            },
            CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: shell_id,
                grantee: Principal::Extension(base_ctx.extension_id.clone()),
                issued_by: persistent_approval_grant_issuer(),
                constraints: GrantConstraints {
                    allowed_effects: vec![EffectKind::SpawnProcess],
                    mounts: MountView::default(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: Some(chrono::Utc::now() - chrono::Duration::seconds(1)),
                    max_invocations: None,
                },
            },
        ],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "expired persistent approval grant must not satisfy the local-dev gate, got {decision:?}"
    );
}

#[tokio::test]
async fn other_user_issued_approval_grant_does_not_allow_resume() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let shell_id = CapabilityId::new("builtin.shell").unwrap();
    let descriptor = test_descriptor_with_id(shell_id.clone(), vec![EffectKind::SpawnProcess]);
    let ctx = test_context(CapabilitySet {
        grants: vec![CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: shell_id,
            grantee: Principal::Extension(ExtensionId::new("builtin").unwrap()),
            issued_by: Principal::User(ironclaw_host_api::UserId::new("other-user").unwrap()),
            constraints: GrantConstraints {
                allowed_effects: vec![EffectKind::SpawnProcess],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        }],
    });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::RequireApproval { .. }),
        "different-user approval lease must not satisfy the local-dev gate, got {decision:?}"
    );
}

#[tokio::test]
async fn deny_decision_passes_through_unchanged() {
    let authorizer = test_authorizer(ApprovalPolicy::AskDestructive);

    let descriptor = test_descriptor(vec![EffectKind::DispatchCapability]);
    let ctx = test_context(CapabilitySet { grants: vec![] });
    let decision = authorizer
        .authorize_dispatch_with_trust(
            &ctx,
            &descriptor,
            &ResourceEstimate::default(),
            &test_trust_decision(),
        )
        .await;

    assert!(
        matches!(decision, Decision::Deny { .. }),
        "ungranted capability should return Deny unchanged, got {decision:?}"
    );
}

#[test]
fn approval_request_reason_includes_capability_id() {
    let descriptor = test_descriptor(vec![EffectKind::SpawnProcess]);
    let ctx = test_context(CapabilitySet { grants: vec![] });
    let req = approval_request(
        &ctx,
        &descriptor,
        &ResourceEstimate::default(),
        ProfileApprovalActionKind::Dispatch,
    );

    assert!(
        req.reason.contains("builtin.shell"),
        "reason should contain capability id, got: {:?}",
        req.reason
    );
    assert!(
        req.reason.contains("Dispatch"),
        "reason should contain action kind, got: {:?}",
        req.reason
    );
}
