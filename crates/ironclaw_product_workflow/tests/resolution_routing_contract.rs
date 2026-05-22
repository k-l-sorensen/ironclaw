//! Contract tests for the Slice 3 wiring (#3094): approval/auth resolution
//! payloads are routed through the `ApprovalResolutionHandler` /
//! `AuthResolutionHandler` traits when wired, and continue to surface as
//! `UnsupportedActionKind` when unwired.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, ParsedProductInbound, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductRejection, ProductRejectionKind,
    ProductWorkflow, ProtocolAuthEvidence, TrustedInboundContext,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, ApprovalResolutionHandler, AuthResolutionHandler, DefaultProductWorkflow,
    FakeConversationBindingService, FakeIdempotencyLedger, FakeInboundTurnService,
    ProductWorkflowError,
};
use ironclaw_turns::AcceptedMessageRef;

#[tokio::test]
async fn approval_resolution_routes_through_handler_when_wired() {
    let inbound = Arc::new(FakeInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::default());
    let binding = Arc::new(FakeConversationBindingService::default());
    let approval_handler = Arc::new(RecordingApprovalHandler::accepted());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_approval_resolution_handler(approval_handler.clone());

    let payload = ProductInboundPayload::ApprovalResolution(
        ApprovalResolutionPayload::new(
            ironclaw_turns::TurnRunId::new(),
            "gate:abc",
            ApprovalDecision::ApproveOnce,
        )
        .expect("valid"),
    );
    let envelope = envelope_with_payload("evt:appr-1", payload);
    let ack = workflow.accept_inbound(envelope).await.expect("accepted");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let calls = approval_handler.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].gate_ref, "gate:abc");
    assert_eq!(calls[0].decision, ApprovalDecision::ApproveOnce);

    // The ledger settled the action with an ApprovalResolution dispatch kind,
    // not a generic UserMessageTurn — downstream observers can distinguish.
    let settled = ledger.last_settled().expect("settled action");
    assert!(matches!(
        settled.dispatch_kind,
        Some(ActionDispatchKind::ApprovalResolution { .. })
    ));
}

#[tokio::test]
async fn approval_resolution_returns_unsupported_when_handler_unwired() {
    let inbound = Arc::new(FakeInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::default());
    let binding = Arc::new(FakeConversationBindingService::default());
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding);

    let payload = ProductInboundPayload::ApprovalResolution(
        ApprovalResolutionPayload::new(
            ironclaw_turns::TurnRunId::new(),
            "gate:abc",
            ApprovalDecision::ApproveOnce,
        )
        .expect("valid"),
    );
    let envelope = envelope_with_payload("evt:appr-noh", payload);
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("unsupported");
    // ProductWorkflowError::UnsupportedActionKind { kind: "approval_resolution" }
    // surfaces through ProductAdapterError as a non-retryable failure.
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn approval_resolution_propagates_handler_rejection_ack() {
    let inbound = Arc::new(FakeInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::default());
    let binding = Arc::new(FakeConversationBindingService::default());
    let approval_handler = Arc::new(RecordingApprovalHandler::rejected(
        ProductRejectionKind::PolicyDenied,
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger.clone(), binding)
        .with_approval_resolution_handler(approval_handler.clone());

    let payload = ProductInboundPayload::ApprovalResolution(
        ApprovalResolutionPayload::new(
            ironclaw_turns::TurnRunId::new(),
            "gate:abc",
            ApprovalDecision::AlwaysAllow,
        )
        .expect("valid"),
    );
    let envelope = envelope_with_payload("evt:appr-reject", payload);
    let ack = workflow.accept_inbound(envelope).await.expect("ack");
    assert!(matches!(ack, ProductInboundAck::Rejected(_)));
    assert_eq!(approval_handler.calls().len(), 1);
    // Rejection still settles the ledger so a replay of the same external
    // event id returns the same rejection (idempotent).
    assert!(ledger.last_settled().is_some());
}

#[tokio::test]
async fn auth_resolution_routes_through_handler_when_wired() {
    let inbound = Arc::new(FakeInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::default());
    let binding = Arc::new(FakeConversationBindingService::default());
    let auth_handler = Arc::new(RecordingAuthHandler::accepted());
    let workflow = DefaultProductWorkflow::new(inbound, ledger.clone(), binding)
        .with_auth_resolution_handler(auth_handler.clone());

    let payload = ProductInboundPayload::AuthResolution(
        AuthResolutionPayload::new(
            ironclaw_turns::TurnRunId::new(),
            "flow-xyz",
            AuthResolutionResult::CredentialProvided {
                credential_ref: "cred-1".into(),
            },
        )
        .expect("valid"),
    );
    let envelope = envelope_with_payload("evt:auth-1", payload);
    let ack = workflow.accept_inbound(envelope).await.expect("accepted");
    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    let calls = auth_handler.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].auth_request_ref, "flow-xyz");

    let settled = ledger.last_settled().expect("settled");
    assert!(matches!(
        settled.dispatch_kind,
        Some(ActionDispatchKind::AuthResolution { .. })
    ));
}

#[tokio::test]
async fn auth_resolution_returns_unsupported_when_handler_unwired() {
    let inbound = Arc::new(FakeInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::default());
    let binding = Arc::new(FakeConversationBindingService::default());
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding);

    let payload = ProductInboundPayload::AuthResolution(
        AuthResolutionPayload::new(
            ironclaw_turns::TurnRunId::new(),
            "flow-xyz",
            AuthResolutionResult::Denied,
        )
        .expect("valid"),
    );
    let envelope = envelope_with_payload("evt:auth-noh", payload);
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("unsupported");
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn handler_error_propagates_as_terminal_rejection() {
    let inbound = Arc::new(FakeInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::default());
    let binding = Arc::new(FakeConversationBindingService::default());
    let approval_handler = Arc::new(RecordingApprovalHandler::error_unknown());
    let workflow = DefaultProductWorkflow::new(inbound, ledger.clone(), binding)
        .with_approval_resolution_handler(approval_handler);

    let payload = ProductInboundPayload::ApprovalResolution(
        ApprovalResolutionPayload::new(
            ironclaw_turns::TurnRunId::new(),
            "gate:missing",
            ApprovalDecision::Deny,
        )
        .expect("valid"),
    );
    let envelope = envelope_with_payload("evt:appr-err", payload);
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("handler error");
    // The handler returned UnsupportedActionKind. The workflow translates
    // that to a terminal Rejected ack via `terminal_ack_for_error`, settles
    // the ledger (so replays are idempotent), and surfaces the error to the
    // caller as a non-retryable adapter error.
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);
}

// --- Recording handler fakes ------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct ApprovalCall {
    gate_ref: String,
    decision: ApprovalDecision,
}

#[derive(Debug, Clone, PartialEq)]
struct AuthCall {
    auth_request_ref: String,
    result: AuthResolutionResult,
}

enum ApprovalOutcome {
    Accepted,
    Rejected(ProductRejectionKind),
    Error,
}

struct RecordingApprovalHandler {
    outcome: ApprovalOutcome,
    calls: Mutex<Vec<ApprovalCall>>,
}

impl RecordingApprovalHandler {
    fn accepted() -> Self {
        Self {
            outcome: ApprovalOutcome::Accepted,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn rejected(kind: ProductRejectionKind) -> Self {
        Self {
            outcome: ApprovalOutcome::Rejected(kind),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn error_unknown() -> Self {
        Self {
            outcome: ApprovalOutcome::Error,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<ApprovalCall> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ApprovalResolutionHandler for RecordingApprovalHandler {
    async fn handle(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let ProductInboundPayload::ApprovalResolution(payload) = envelope.payload() else {
            panic!("approval handler invoked for non-approval payload");
        };
        self.calls.lock().expect("lock").push(ApprovalCall {
            gate_ref: payload.gate_ref.clone(),
            decision: payload.decision.clone(),
        });
        match &self.outcome {
            ApprovalOutcome::Accepted => Ok(accepted_ack()),
            ApprovalOutcome::Rejected(kind) => Ok(rejected_ack(kind.clone())),
            ApprovalOutcome::Error => Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "test_unknown_gate".into(),
            }),
        }
    }
}

enum AuthOutcome {
    Accepted,
}

struct RecordingAuthHandler {
    outcome: AuthOutcome,
    calls: Mutex<Vec<AuthCall>>,
}

impl RecordingAuthHandler {
    fn accepted() -> Self {
        Self {
            outcome: AuthOutcome::Accepted,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<AuthCall> {
        self.calls.lock().expect("lock").clone()
    }
}

#[async_trait]
impl AuthResolutionHandler for RecordingAuthHandler {
    async fn handle(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let ProductInboundPayload::AuthResolution(payload) = envelope.payload() else {
            panic!("auth handler invoked for non-auth payload");
        };
        self.calls.lock().expect("lock").push(AuthCall {
            auth_request_ref: payload.auth_request_ref.clone(),
            result: payload.result.clone(),
        });
        match self.outcome {
            AuthOutcome::Accepted => Ok(accepted_ack()),
        }
    }
}

fn accepted_ack() -> ProductInboundAck {
    ProductInboundAck::Accepted {
        accepted_message_ref: AcceptedMessageRef::new("test-msg-ref").expect("valid ref"),
        submitted_run_id: ironclaw_turns::TurnRunId::new(),
    }
}

fn rejected_ack(kind: ProductRejectionKind) -> ProductInboundAck {
    ProductInboundAck::Rejected(ProductRejection::permanent(kind, "test handler rejection"))
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

trait IdempotencyLedgerLastSettled {
    fn last_settled(&self) -> Option<ironclaw_product_workflow::ProductInboundAction>;
}

impl IdempotencyLedgerLastSettled for FakeIdempotencyLedger {
    fn last_settled(&self) -> Option<ironclaw_product_workflow::ProductInboundAction> {
        self.settled_actions().last().cloned()
    }
}
