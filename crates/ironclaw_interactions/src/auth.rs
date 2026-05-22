//! Auth interaction surface.
//!
//! `AuthInteractionService` is the adapter/UI-safe boundary for listing
//! auth-required gates and resolving them. It composes the host-supplied
//! [`crate::auth_flow::AuthFlowManager`] so that:
//!
//! - product/UI surfaces receive only redacted [`PendingAuthSummary`] DTOs
//!   that omit raw credentials, secret handles, redirect URLs, callback
//!   payloads, and runtime diagnostics;
//! - resume/cancel decisions route through one typed boundary that the
//!   host owns;
//! - wrong-scope reads and resolutions look unknown.

use ironclaw_host_api::{CapabilityId, ResourceScope};
use thiserror::Error;

use crate::auth_flow::{
    AuthCancelReason, AuthFlowError, AuthFlowManager, AuthFlowRef, AuthResumeEvidence,
    AuthResumeOutcome,
};

/// Redacted summary of a pending auth-required gate.
///
/// Holds the opaque [`AuthFlowRef`] the product surface must pass back to
/// resume/cancel and the [`CapabilityId`] that originally required the
/// auth so the surface can render "Connect <integration> to use
/// <capability>?" — never any redirect URL, secret handle, or backend
/// diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAuthSummary {
    pub flow_ref: AuthFlowRef,
    pub capability: CapabilityId,
}

/// Caller-facing resume/cancel error taxonomy.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthInteractionError {
    /// No flow matches `(scope, flow_ref)`. Same error for genuinely
    /// missing flows and for cross-scope lookups.
    #[error("auth flow is unknown")]
    Unknown,
    /// Flow is no longer active (resumed, cancelled, or expired).
    #[error("auth flow is no longer active")]
    Terminal,
    /// Resume evidence cannot be applied to this flow.
    #[error("auth flow evidence is invalid")]
    InvalidEvidence,
    /// Caller supplied an unparseable [`AuthFlowRef`].
    #[error("invalid auth flow ref")]
    InvalidRef,
    /// Any other backend or manager failure.
    #[error("auth interaction failed")]
    Backend,
}

impl From<AuthFlowError> for AuthInteractionError {
    fn from(error: AuthFlowError) -> Self {
        match error {
            AuthFlowError::Unknown => Self::Unknown,
            AuthFlowError::Terminal => Self::Terminal,
            AuthFlowError::InvalidEvidence => Self::InvalidEvidence,
            AuthFlowError::InvalidRef { .. } => Self::InvalidRef,
            AuthFlowError::Backend => Self::Backend,
        }
    }
}

/// Adapter/UI-safe auth interaction surface.
pub struct AuthInteractionService<'a, M>
where
    M: AuthFlowManager + ?Sized,
{
    manager: &'a M,
}

impl<'a, M> AuthInteractionService<'a, M>
where
    M: AuthFlowManager + ?Sized,
{
    pub fn new(manager: &'a M) -> Self {
        Self { manager }
    }

    /// List pending auth flows visible to `scope`.
    pub async fn list_pending(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<PendingAuthSummary>, AuthInteractionError> {
        let records = self.manager.list_pending(scope).await?;
        Ok(records
            .into_iter()
            .map(|record| PendingAuthSummary {
                flow_ref: record.flow_ref,
                capability: record.capability_id,
            })
            .collect())
    }

    /// Resume a pending auth flow with product-supplied evidence.
    ///
    /// Validates the flow's scope/status via the manager before applying
    /// evidence. Wrong-scope flows and terminal flows surface as
    /// `Unknown` / `Terminal` respectively; backend failures surface as
    /// `Backend`. Raw credentials are never returned through this path.
    pub async fn resume(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
        evidence: AuthResumeEvidence,
    ) -> Result<AuthResumeOutcome, AuthInteractionError> {
        let outcome = self.manager.resume(scope, flow_ref, evidence).await?;
        Ok(outcome)
    }

    /// Cancel a pending auth flow.
    pub async fn cancel(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
        reason: AuthCancelReason,
    ) -> Result<AuthResumeOutcome, AuthInteractionError> {
        let outcome = self.manager.cancel(scope, flow_ref, reason).await?;
        Ok(outcome)
    }
}
