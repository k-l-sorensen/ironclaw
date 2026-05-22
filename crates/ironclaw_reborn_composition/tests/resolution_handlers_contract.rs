//! Production-handler contract tests (#3094 Slice 3 wiring).
//!
//! Exercises the end-to-end loop: `ProductInboundEnvelope` →
//! `TurnCoordinatorApprovalHandler` / `TurnCoordinatorAuthHandler` →
//! recorded `TurnCoordinator` resume/cancel call. Wires the workflow
//! handlers exactly as production composition will.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductWorkflow, ProtocolAuthEvidence,
    TrustedInboundContext,
};
use ironclaw_product_workflow::{
    DefaultProductWorkflow, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService,
};
use ironclaw_reborn_composition::{TurnCoordinatorApprovalHandler, TurnCoordinatorAuthHandler};
use ironclaw_turns::{
    CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest, ResumeTurnRequest,
    ResumeTurnResponse, SubmitTurnRequest, SubmitTurnResponse, TurnCoordinator, TurnError,
    TurnRunId, TurnRunState, TurnStatus,
};

#[tokio::test]
async fn approval_approve_once_routes_to_resume_turn() {
    let (workflow, coordinator) = build_workflow_with_handlers();
    let run_id = TurnRunId::new();
    let envelope = approval_envelope(
        run_id,
        "gate:approval-x",
        ApprovalDecision::ApproveOnce,
        "evt:1",
    );

    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("approve_once should accept");
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));

    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1, "exactly one resume_turn call");
    assert_eq!(resumes[0].run_id, run_id);
    assert_eq!(resumes[0].gate_resolution_ref.as_str(), "gate:approval-x");
    assert_eq!(coordinator.cancels().len(), 0);
}

#[tokio::test]
async fn approval_deny_routes_to_cancel_run() {
    let (workflow, coordinator) = build_workflow_with_handlers();
    let run_id = TurnRunId::new();
    let envelope = approval_envelope(run_id, "gate:approval-y", ApprovalDecision::Deny, "evt:2");

    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("deny should accept");
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));

    let cancels = coordinator.cancels();
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0].run_id, run_id);
    assert_eq!(coordinator.resumes().len(), 0);
}

#[tokio::test]
async fn approval_always_allow_is_refused_until_policy_port_lands() {
    let (workflow, coordinator) = build_workflow_with_handlers();
    let run_id = TurnRunId::new();
    let envelope = approval_envelope(
        run_id,
        "gate:approval-z",
        ApprovalDecision::AlwaysAllow,
        "evt:3",
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("always_allow without policy port should fail closed");
    assert!(!err.is_retryable());
    // No turn coordinator side effects when the request is refused upstream.
    assert_eq!(coordinator.resumes().len(), 0);
    assert_eq!(coordinator.cancels().len(), 0);
}

#[tokio::test]
async fn auth_credential_provided_routes_to_resume_turn() {
    let (workflow, coordinator) = build_workflow_with_handlers();
    let run_id = TurnRunId::new();
    let envelope = auth_envelope(
        run_id,
        "auth:flow-a",
        AuthResolutionResult::CredentialProvided {
            credential_ref: "cred-1".into(),
        },
        "evt:4",
    );

    let ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("credential_provided should accept");
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));

    let resumes = coordinator.resumes();
    assert_eq!(resumes.len(), 1);
    assert_eq!(resumes[0].run_id, run_id);
    assert_eq!(resumes[0].gate_resolution_ref.as_str(), "auth:flow-a");
    assert_eq!(coordinator.cancels().len(), 0);
}

#[tokio::test]
async fn auth_callback_completed_routes_to_resume_turn() {
    let (workflow, coordinator) = build_workflow_with_handlers();
    let run_id = TurnRunId::new();
    let envelope = auth_envelope(
        run_id,
        "auth:flow-b",
        AuthResolutionResult::CallbackCompleted {
            callback_ref: "cb-1".into(),
        },
        "evt:5",
    );

    workflow
        .accept_inbound(envelope)
        .await
        .expect("callback_completed should accept");
    assert_eq!(coordinator.resumes().len(), 1);
    assert_eq!(coordinator.cancels().len(), 0);
}

#[tokio::test]
async fn auth_denied_routes_to_cancel_run() {
    let (workflow, coordinator) = build_workflow_with_handlers();
    let run_id = TurnRunId::new();
    let envelope = auth_envelope(run_id, "auth:flow-c", AuthResolutionResult::Denied, "evt:6");

    workflow
        .accept_inbound(envelope)
        .await
        .expect("denied should accept");
    assert_eq!(coordinator.cancels().len(), 1);
    assert_eq!(coordinator.resumes().len(), 0);
}

#[tokio::test]
async fn turn_coordinator_error_propagates_as_terminal_rejection() {
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    coordinator.fail_next_resume_with(TurnError::Conflict {
        reason: "thread already running".into(),
    });
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::default()),
        Arc::new(FakeIdempotencyLedger::new()),
        binding.clone(),
    )
    .with_approval_resolution_handler(Arc::new(TurnCoordinatorApprovalHandler::new(
        binding.clone(),
        coordinator.clone(),
    )));

    let run_id = TurnRunId::new();
    let envelope = approval_envelope(run_id, "gate:err", ApprovalDecision::ApproveOnce, "evt:err");
    let err = workflow.accept_inbound(envelope).await.expect_err("error");
    assert!(!err.is_retryable());
}

// --- Recording coordinator + helpers ----------------------------------------

#[derive(Default)]
struct RecordingTurnCoordinator {
    resumes: Mutex<Vec<ResumeTurnRequest>>,
    cancels: Mutex<Vec<CancelRunRequest>>,
    fail_resume: Mutex<Option<TurnError>>,
}

impl RecordingTurnCoordinator {
    fn resumes(&self) -> Vec<ResumeTurnRequest> {
        self.resumes.lock().expect("lock").clone()
    }

    fn cancels(&self) -> Vec<CancelRunRequest> {
        self.cancels.lock().expect("lock").clone()
    }

    fn fail_next_resume_with(&self, error: TurnError) {
        *self.fail_resume.lock().expect("lock") = Some(error);
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn unused in resolution-handler tests");
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        if let Some(err) = self.fail_resume.lock().expect("lock").take() {
            return Err(err);
        }
        let run_id = request.run_id;
        self.resumes.lock().expect("lock").push(request);
        Ok(ResumeTurnResponse {
            run_id,
            status: TurnStatus::Running,
            event_cursor: EventCursor::default(),
        })
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        self.cancels.lock().expect("lock").push(request);
        Ok(CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor::default(),
            already_terminal: false,
        })
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state unused in resolution-handler tests");
    }
}

fn build_workflow_with_handlers() -> (DefaultProductWorkflow, Arc<RecordingTurnCoordinator>) {
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let binding: Arc<FakeConversationBindingService> =
        Arc::new(FakeConversationBindingService::new());
    let approval_handler = Arc::new(TurnCoordinatorApprovalHandler::new(
        binding.clone(),
        coordinator.clone(),
    ));
    let auth_handler = Arc::new(TurnCoordinatorAuthHandler::new(
        binding.clone(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::default()),
        Arc::new(FakeIdempotencyLedger::new()),
        binding,
    )
    .with_approval_resolution_handler(approval_handler)
    .with_auth_resolution_handler(auth_handler);
    (workflow, coordinator)
}

fn approval_envelope(
    run_id: TurnRunId,
    gate_ref: &str,
    decision: ApprovalDecision,
    event_suffix: &str,
) -> ProductInboundEnvelope {
    envelope_with_payload(
        event_suffix,
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(run_id, gate_ref, decision).expect("valid payload"),
        ),
    )
}

fn auth_envelope(
    run_id: TurnRunId,
    auth_ref: &str,
    result: AuthResolutionResult,
    event_suffix: &str,
) -> ProductInboundEnvelope {
    envelope_with_payload(
        event_suffix,
        ProductInboundPayload::AuthResolution(
            AuthResolutionPayload::new(run_id, auth_ref, result).expect("valid payload"),
        ),
    )
}

fn envelope_with_payload(
    event_suffix: &str,
    payload: ProductInboundPayload,
) -> ProductInboundEnvelope {
    let adapter_id = ProductAdapterId::new("test_adapter").expect("valid");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("valid");
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        installation_id.as_str(),
    );
    let context = TrustedInboundContext::from_verified_evidence(
        adapter_id,
        installation_id,
        Utc::now(),
        &evidence,
    )
    .expect("context");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conv"),
        payload,
    )
    .expect("parsed");
    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}
