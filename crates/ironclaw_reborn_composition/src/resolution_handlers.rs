//! Production implementations of the workflow-level resolution handlers.
//!
//! [`TurnCoordinatorApprovalHandler`] and [`TurnCoordinatorAuthHandler`]
//! translate `ProductInboundEnvelope::{Approval,Auth}Resolution` payloads
//! into typed `TurnCoordinator` requests:
//!
//! - resolve the envelope's adapter/installation/actor through
//!   `ConversationBindingService::lookup_binding`;
//! - build a `TurnScope` + `TurnActor` from the binding;
//! - parse the wire `gate_ref` / `auth_request_ref` as `GateRef`;
//! - dispatch approve/resume to `TurnCoordinator::resume_turn` and
//!   deny/cancel to `TurnCoordinator::cancel_run`;
//! - map `TurnError` outcomes to a redacted `ProductInboundAck`.
//!
//! `ApprovalDecision::AlwaysAllow` is refused with a stable
//! `UnsupportedActionKind` error mirroring `RebornServicesApi::resolve_gate`
//! — the persistent-approval policy port is not yet defined and silently
//! downgrading to a one-shot approve would widen authority without a
//! decision record.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_adapters::{
    ApprovalDecision, AuthResolutionResult, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload,
};
use ironclaw_product_workflow::{
    ApprovalResolutionHandler, AuthResolutionHandler, ConversationBindingService,
    ProductConversationRouteKind, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, GateRef, IdempotencyKey, ReplyTargetBindingRef,
    ResumeTurnRequest, ResumeTurnResponse, SanitizedCancelReason, SourceBindingRef, TurnActor,
    TurnCoordinator, TurnError, TurnRunId, TurnScope,
};

/// Production `ApprovalResolutionHandler` that routes approve/deny
/// decisions through `TurnCoordinator`.
pub struct TurnCoordinatorApprovalHandler {
    binding_service: Arc<dyn ConversationBindingService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl TurnCoordinatorApprovalHandler {
    pub fn new(
        binding_service: Arc<dyn ConversationBindingService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            binding_service,
            turn_coordinator,
        }
    }
}

#[async_trait]
impl ApprovalResolutionHandler for TurnCoordinatorApprovalHandler {
    async fn handle(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let ProductInboundPayload::ApprovalResolution(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "approval_resolution".into(),
            });
        };

        // Persistent approvals need an approval-policy port that doesn't
        // exist yet. Mirror `RebornServicesApi::resolve_gate`'s refusal
        // rather than silently downgrading authority.
        if matches!(payload.decision, ApprovalDecision::AlwaysAllow) {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "approval_resolution.always_allow".into(),
            });
        }

        let binding = self
            .binding_service
            .lookup_binding(envelope_to_binding_request(envelope))
            .await?;
        let scope = scope_from_binding(&binding);
        let actor = TurnActor::new(binding.user_id.clone());
        let gate_ref = parse_gate_ref(&payload.gate_ref)?;
        let idempotency_key = idempotency_key_from_envelope(envelope)?;
        let binding_id = product_gate_binding_id(envelope, &payload.gate_ref);
        let source_binding_ref =
            bounded_source_ref("product-gate-src", &binding_id, "source_binding_ref")?;
        let reply_target_binding_ref = bounded_reply_target_ref(
            "product-gate-reply",
            &binding_id,
            "reply_target_binding_ref",
        )?;

        match payload.decision {
            ApprovalDecision::ApproveOnce => {
                let response = self
                    .turn_coordinator
                    .resume_turn(ResumeTurnRequest {
                        scope,
                        actor,
                        run_id: payload.run_id,
                        gate_resolution_ref: gate_ref,
                        source_binding_ref,
                        reply_target_binding_ref,
                        idempotency_key,
                    })
                    .await
                    .map_err(turn_error_to_workflow_error)?;
                Ok(accepted_ack_for_resume(payload.run_id, &response))
            }
            ApprovalDecision::Deny => {
                self.turn_coordinator
                    .cancel_run(CancelRunRequest {
                        scope,
                        actor,
                        run_id: payload.run_id,
                        reason: SanitizedCancelReason::UserRequested,
                        idempotency_key,
                    })
                    .await
                    .map_err(turn_error_to_workflow_error)?;
                Ok(accepted_ack_for_cancel(payload.run_id))
            }
            ApprovalDecision::AlwaysAllow => unreachable!("guarded above"),
        }
    }
}

/// Production `AuthResolutionHandler` that routes resume/cancel through
/// `TurnCoordinator`, matching the approval handler's shape.
///
/// `AuthResolutionResult::CredentialProvided` and `CallbackCompleted` both
/// map to `resume_turn` — the credential broker (see #3068) records the
/// supplied refs against the auth flow before resume reaches the
/// coordinator. `Denied` maps to `cancel_run`.
pub struct TurnCoordinatorAuthHandler {
    binding_service: Arc<dyn ConversationBindingService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
}

impl TurnCoordinatorAuthHandler {
    pub fn new(
        binding_service: Arc<dyn ConversationBindingService>,
        turn_coordinator: Arc<dyn TurnCoordinator>,
    ) -> Self {
        Self {
            binding_service,
            turn_coordinator,
        }
    }
}

#[async_trait]
impl AuthResolutionHandler for TurnCoordinatorAuthHandler {
    async fn handle(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        let ProductInboundPayload::AuthResolution(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "auth_resolution".into(),
            });
        };

        let binding = self
            .binding_service
            .lookup_binding(envelope_to_binding_request(envelope))
            .await?;
        let scope = scope_from_binding(&binding);
        let actor = TurnActor::new(binding.user_id.clone());
        let gate_ref = parse_gate_ref(&payload.auth_request_ref)?;
        let idempotency_key = idempotency_key_from_envelope(envelope)?;
        let binding_id = product_auth_binding_id(envelope, &payload.auth_request_ref);
        let source_binding_ref =
            bounded_source_ref("product-auth-src", &binding_id, "source_binding_ref")?;
        let reply_target_binding_ref = bounded_reply_target_ref(
            "product-auth-reply",
            &binding_id,
            "reply_target_binding_ref",
        )?;

        match payload.result {
            AuthResolutionResult::CredentialProvided { .. }
            | AuthResolutionResult::CallbackCompleted { .. } => {
                let response = self
                    .turn_coordinator
                    .resume_turn(ResumeTurnRequest {
                        scope,
                        actor,
                        run_id: payload.run_id,
                        gate_resolution_ref: gate_ref,
                        source_binding_ref,
                        reply_target_binding_ref,
                        idempotency_key,
                    })
                    .await
                    .map_err(turn_error_to_workflow_error)?;
                Ok(accepted_ack_for_resume(payload.run_id, &response))
            }
            AuthResolutionResult::Denied => {
                self.turn_coordinator
                    .cancel_run(CancelRunRequest {
                        scope,
                        actor,
                        run_id: payload.run_id,
                        reason: SanitizedCancelReason::UserRequested,
                        idempotency_key,
                    })
                    .await
                    .map_err(turn_error_to_workflow_error)?;
                Ok(accepted_ack_for_cancel(payload.run_id))
            }
        }
    }
}

// --- Shared helpers --------------------------------------------------------

fn envelope_to_binding_request(envelope: &ProductInboundEnvelope) -> ResolveBindingRequest {
    ResolveBindingRequest {
        adapter_id: envelope.adapter_id().clone(),
        installation_id: envelope.installation_id().clone(),
        external_actor_ref: envelope.external_actor_ref().clone(),
        external_conversation_ref: envelope.external_conversation_ref().clone(),
        external_event_id: envelope.external_event_id().clone(),
        // Gate resolution is a per-actor decision; the route kind only
        // matters for inbound user messages where shared-route admission
        // policy applies. Direct is the conservative default here.
        route_kind: ProductConversationRouteKind::Direct,
        auth_claim: envelope.auth_claim().clone(),
    }
}

fn scope_from_binding(binding: &ResolvedBinding) -> TurnScope {
    TurnScope::new(
        binding.tenant_id.clone(),
        binding.agent_id.clone(),
        binding.project_id.clone(),
        binding.thread_id.clone(),
    )
}

fn parse_gate_ref(raw: &str) -> Result<GateRef, ProductWorkflowError> {
    GateRef::new(raw.to_string()).map_err(|reason| ProductWorkflowError::TurnSubmissionRejected {
        reason: format!("invalid gate_ref: {reason}"),
    })
}

fn idempotency_key_from_envelope(
    envelope: &ProductInboundEnvelope,
) -> Result<IdempotencyKey, ProductWorkflowError> {
    IdempotencyKey::new(envelope.external_event_id().as_str().to_string()).map_err(|reason| {
        ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid idempotency_key from external_event_id: {reason}"),
        }
    })
}

/// Build a stable, scope-anchored binding id so resume/cancel retries from
/// the same envelope hit the same coordinator-side binding. The shape
/// mirrors the WebUI gate-resolution path; the prefix marks the product
/// adapter origin so audit can distinguish the two surfaces.
fn product_gate_binding_id(envelope: &ProductInboundEnvelope, gate_ref: &str) -> String {
    format!(
        "{}|{}|{}",
        envelope.external_event_id().as_str(),
        gate_ref,
        envelope.adapter_id().as_str(),
    )
}

fn product_auth_binding_id(envelope: &ProductInboundEnvelope, auth_request_ref: &str) -> String {
    format!(
        "{}|{}|{}",
        envelope.external_event_id().as_str(),
        auth_request_ref,
        envelope.adapter_id().as_str(),
    )
}

fn bounded_source_ref(
    prefix: &str,
    raw: &str,
    field: &'static str,
) -> Result<SourceBindingRef, ProductWorkflowError> {
    SourceBindingRef::new(format!("{prefix}:{raw}")).map_err(|reason| {
        ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid {field}: {reason}"),
        }
    })
}

fn bounded_reply_target_ref(
    prefix: &str,
    raw: &str,
    field: &'static str,
) -> Result<ReplyTargetBindingRef, ProductWorkflowError> {
    ReplyTargetBindingRef::new(format!("{prefix}:{raw}")).map_err(|reason| {
        ProductWorkflowError::TurnSubmissionRejected {
            reason: format!("invalid {field}: {reason}"),
        }
    })
}

fn accepted_ack_for_resume(run_id: TurnRunId, _response: &ResumeTurnResponse) -> ProductInboundAck {
    // `accepted_message_ref` is wire-stable; using the run id as its raw
    // form gives downstream surfaces a deterministic correlation back to
    // the resumed run without leaking inner coordinator handles.
    let accepted_message_ref = AcceptedMessageRef::new(format!("resume:{run_id}"))
        .expect("resume:<uuid> always satisfies bounded_ref shape");
    ProductInboundAck::Accepted {
        accepted_message_ref,
        submitted_run_id: run_id,
    }
}

fn accepted_ack_for_cancel(run_id: TurnRunId) -> ProductInboundAck {
    let accepted_message_ref = AcceptedMessageRef::new(format!("cancel:{run_id}"))
        .expect("cancel:<uuid> always satisfies bounded_ref shape");
    ProductInboundAck::Accepted {
        accepted_message_ref,
        submitted_run_id: run_id,
    }
}

fn turn_error_to_workflow_error(error: TurnError) -> ProductWorkflowError {
    ProductWorkflowError::TurnSubmissionFailed { error }
}
