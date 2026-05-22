//! Approval/auth resolution handler traits.
//!
//! These are the workflow-level boundary the host wires when products
//! submit `ProductInboundPayload::ApprovalResolution` /
//! `ProductInboundPayload::AuthResolution`. The traits are deliberately
//! coarse (they take the inbound envelope, return an ack) so production
//! composition can compose them out of `ironclaw_interactions` services +
//! binding resolution + scope derivation without exposing those
//! internals to the workflow.
//!
//! Adapters (Web, Telegram, CLI, …) must never call `TurnCoordinator`,
//! approval stores, or auth-flow stores directly. They route through
//! `ProductWorkflow`, which routes through these handlers, which route
//! through the interaction services. See issue #3094 Slice 3.

use async_trait::async_trait;
use ironclaw_product_adapters::{ProductInboundAck, ProductInboundEnvelope};

use crate::error::ProductWorkflowError;

/// Workflow-level handler for `ProductInboundPayload::ApprovalResolution`.
///
/// Implementors typically:
///
/// 1. Resolve the envelope's adapter+installation+actor to a typed scope
///    via the host's binding service.
/// 2. Parse the wire-format `gate_ref` into an `ApprovalRequestId`.
/// 3. Call `ApprovalInteractionService::approve` / `::deny` from
///    `ironclaw_interactions`.
/// 4. Map the interaction outcome (or error) to a redacted ack.
///
/// Production wiring lives in composition; tests stub this trait directly.
#[async_trait]
pub trait ApprovalResolutionHandler: Send + Sync {
    async fn handle(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductWorkflowError>;
}

/// Workflow-level handler for `ProductInboundPayload::AuthResolution`.
///
/// Mirrors [`ApprovalResolutionHandler`] for the auth side: resolve
/// binding, parse `auth_request_ref` into an `AuthFlowRef`, route through
/// `AuthInteractionService::resume` / `::cancel`, return a redacted ack.
#[async_trait]
pub trait AuthResolutionHandler: Send + Sync {
    async fn handle(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<ProductInboundAck, ProductWorkflowError>;
}
