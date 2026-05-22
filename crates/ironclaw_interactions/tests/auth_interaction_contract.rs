//! Contract tests for [`AuthInteractionService`] and the
//! [`AuthFlowManager`] boundary.
//!
//! Covers the acceptance criteria from issue #3094 Slice 2:
//! - list pending auth flows (scope-filtered)
//! - resume / cancel route through the manager
//! - missing flow, terminal flow, cross-scope denial
//! - redaction sentinel (no raw credential / callback / diagnostic leak)

use ironclaw_host_api::{CapabilityId, InvocationId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_interactions::auth::{
    AuthInteractionError, AuthInteractionService, PendingAuthSummary,
};
use ironclaw_interactions::auth_flow::{
    AuthCancelReason, AuthFlowError, AuthFlowManager, AuthFlowRef, AuthResumeEvidence,
    AuthResumeOutcome, InMemoryAuthFlowManager,
};

#[tokio::test]
async fn list_pending_returns_only_scope_owned_pending_flows() {
    let manager = InMemoryAuthFlowManager::new();
    let scope_a = scope("tenant1", "user1");
    let scope_b = scope("tenant1", "user2");

    let flow_a = AuthFlowRef::new("flow-a").unwrap();
    let flow_b = AuthFlowRef::new("flow-b").unwrap();
    let flow_resumed = AuthFlowRef::new("flow-resumed").unwrap();

    manager.register_pending(
        flow_a.clone(),
        scope_a.clone(),
        capability("notion.search_pages"),
    );
    manager.register_pending(
        flow_b.clone(),
        scope_b.clone(),
        capability("github.search_issues"),
    );
    manager.register_pending(
        flow_resumed.clone(),
        scope_a.clone(),
        capability("memory.read"),
    );
    manager
        .resume(
            &scope_a,
            &flow_resumed,
            AuthResumeEvidence::CredentialProvided {
                credential_ref: "cred-1".to_string(),
            },
        )
        .await
        .unwrap();

    let service = AuthInteractionService::new(&manager);
    let listed = service.list_pending(&scope_a).await.unwrap();
    assert_eq!(
        listed,
        vec![PendingAuthSummary {
            flow_ref: flow_a,
            capability: capability("notion.search_pages"),
        }]
    );
}

#[tokio::test]
async fn resume_routes_through_manager_and_returns_resumed_outcome() {
    let manager = InMemoryAuthFlowManager::new();
    let scope = scope("tenant1", "user1");
    let flow = AuthFlowRef::new("flow-x").unwrap();
    manager.register_pending(
        flow.clone(),
        scope.clone(),
        capability("notion.search_pages"),
    );

    let service = AuthInteractionService::new(&manager);
    let outcome = service
        .resume(
            &scope,
            &flow,
            AuthResumeEvidence::CallbackCompleted {
                callback_ref: "cb-1".to_string(),
            },
        )
        .await
        .unwrap();
    assert_eq!(outcome, AuthResumeOutcome::Resumed);

    // The flow is no longer pending after resume.
    assert!(service.list_pending(&scope).await.unwrap().is_empty());
}

#[tokio::test]
async fn cancel_routes_through_manager_and_returns_cancelled_outcome() {
    let manager = InMemoryAuthFlowManager::new();
    let scope = scope("tenant1", "user1");
    let flow = AuthFlowRef::new("flow-y").unwrap();
    manager.register_pending(
        flow.clone(),
        scope.clone(),
        capability("notion.search_pages"),
    );

    let service = AuthInteractionService::new(&manager);
    let outcome = service
        .cancel(&scope, &flow, AuthCancelReason::UserDenied)
        .await
        .unwrap();
    assert_eq!(outcome, AuthResumeOutcome::Cancelled);
    assert!(service.list_pending(&scope).await.unwrap().is_empty());
}

#[tokio::test]
async fn resume_returns_unknown_for_missing_flow() {
    let manager = InMemoryAuthFlowManager::new();
    let scope = scope("tenant1", "user1");
    let service = AuthInteractionService::new(&manager);
    let bogus = AuthFlowRef::new("nope").unwrap();

    let err = service
        .resume(
            &scope,
            &bogus,
            AuthResumeEvidence::CredentialProvided {
                credential_ref: "cred-1".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, AuthInteractionError::Unknown);
}

#[tokio::test]
async fn resume_returns_unknown_for_cross_scope_flow() {
    let manager = InMemoryAuthFlowManager::new();
    let scope_a = scope("tenant1", "user1");
    let scope_b = scope("tenant1", "user2");
    let flow = AuthFlowRef::new("flow-z").unwrap();
    manager.register_pending(
        flow.clone(),
        scope_a.clone(),
        capability("notion.search_pages"),
    );

    let service = AuthInteractionService::new(&manager);
    let err = service
        .resume(
            &scope_b,
            &flow,
            AuthResumeEvidence::CredentialProvided {
                credential_ref: "cred-1".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, AuthInteractionError::Unknown);

    // The flow stays pending for the owner scope; attacker did not move state.
    assert_eq!(service.list_pending(&scope_a).await.unwrap().len(), 1);
}

#[tokio::test]
async fn resume_returns_terminal_for_already_resolved_flow() {
    let manager = InMemoryAuthFlowManager::new();
    let scope = scope("tenant1", "user1");
    let flow = AuthFlowRef::new("flow-double").unwrap();
    manager.register_pending(
        flow.clone(),
        scope.clone(),
        capability("notion.search_pages"),
    );

    let service = AuthInteractionService::new(&manager);
    service
        .resume(
            &scope,
            &flow,
            AuthResumeEvidence::CredentialProvided {
                credential_ref: "cred-1".to_string(),
            },
        )
        .await
        .unwrap();
    let err = service
        .resume(
            &scope,
            &flow,
            AuthResumeEvidence::CredentialProvided {
                credential_ref: "cred-2".to_string(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err, AuthInteractionError::Terminal);
}

#[tokio::test]
async fn pending_auth_summary_does_not_leak_evidence_strings() {
    // PendingAuthSummary fields are `flow_ref` and `capability`. The
    // service must not surface credential refs, callback refs, or any
    // other evidence the manager handles. Register a flow with a
    // distinctive ref string and confirm the only string in the
    // summary's debug output is the ref itself (i.e. no extra fields).
    let manager = InMemoryAuthFlowManager::new();
    let scope = scope("tenant1", "user1");
    let flow = AuthFlowRef::new("flow-redact").unwrap();
    manager.register_pending(
        flow.clone(),
        scope.clone(),
        capability("notion.search_pages"),
    );

    let service = AuthInteractionService::new(&manager);
    let listed = service.list_pending(&scope).await.unwrap();
    let PendingAuthSummary {
        flow_ref,
        capability,
    } = &listed[0];
    assert_eq!(flow_ref.as_str(), "flow-redact");
    assert_eq!(*capability, self::capability("notion.search_pages"));

    let rendered = format!("{:?}", listed);
    assert!(!rendered.contains("credential"));
    assert!(!rendered.contains("callback"));
    assert!(!rendered.contains("secret"));
}

#[tokio::test]
async fn auth_flow_ref_rejects_invalid_strings() {
    assert_eq!(
        AuthFlowRef::new("").unwrap_err(),
        AuthFlowError::InvalidRef {
            reason: "must not be empty".to_string()
        }
    );
    assert!(matches!(
        AuthFlowRef::new("has space"),
        Err(AuthFlowError::InvalidRef { .. })
    ));
    assert!(matches!(
        AuthFlowRef::new("has\tcontrol"),
        Err(AuthFlowError::InvalidRef { .. })
    ));
    assert!(AuthFlowRef::new("flow-valid_123").is_ok());
}

// --- Fixture helpers --------------------------------------------------------

fn scope(tenant: &str, user: &str) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new(tenant).unwrap(),
        user_id: UserId::new(user).unwrap(),
        agent_id: None,
        project_id: Some(ProjectId::new("project1").unwrap()),
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

fn capability(id: &str) -> CapabilityId {
    CapabilityId::new(id).unwrap()
}
