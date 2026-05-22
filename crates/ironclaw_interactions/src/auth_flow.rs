//! Typed auth-flow manager boundary.
//!
//! The interaction layer routes resume/cancel decisions through this trait
//! so [`crate::auth::AuthInteractionService`] never holds raw credentials,
//! never spawns OAuth machinery, and never reaches into runtime/process
//! state. The host composition implements [`AuthFlowManager`] against
//! whatever credential plumbing actually owns the lease (currently a
//! placeholder; the production wiring lands with the #3068 credential
//! brokering work).
//!
//! Tests use [`InMemoryAuthFlowManager`] which records resume/cancel
//! signals against an in-memory map of pending flows.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::{CapabilityId, ResourceScope};
use thiserror::Error;

/// Opaque host-issued reference to a pending auth flow.
///
/// Product surfaces hold this ref and pass it back to resolve/cancel the
/// flow. The underlying string is host-controlled — products may not
/// fabricate one. Validation matches the wire-format token rules used by
/// `ironclaw_product_adapters::AuthResolutionPayload`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AuthFlowRef(String);

impl AuthFlowRef {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthFlowError> {
        let value = value.into();
        validate_flow_ref(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_flow_ref(value: &str) -> Result<(), AuthFlowError> {
    if value.is_empty() {
        return Err(AuthFlowError::InvalidRef {
            reason: "must not be empty".to_string(),
        });
    }
    if value.len() > 256 {
        return Err(AuthFlowError::InvalidRef {
            reason: format!("exceeds 256 bytes ({})", value.len()),
        });
    }
    if value
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err(AuthFlowError::InvalidRef {
            reason: "must not contain control or whitespace characters".to_string(),
        });
    }
    Ok(())
}

/// Live record for a pending auth flow as observed by the manager.
///
/// Carries no raw credentials, secret handles, or runtime diagnostics —
/// the manager owns those internally and never crosses them through the
/// interaction surface. `capability_id` is included so products can
/// render "Connect <integration> to use <capability>?"
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthFlowRecord {
    pub flow_ref: AuthFlowRef,
    pub scope: ResourceScope,
    pub capability_id: CapabilityId,
}

/// Typed resume evidence supplied by a product surface.
///
/// Matches the public wire shape of
/// `ironclaw_product_adapters::AuthResolutionResult` so adapters can
/// translate inbound payloads directly without inventing new vocabulary.
/// The `credential_ref` / `callback_ref` fields are opaque host-issued
/// strings — products may not fabricate them and must not store raw
/// credential material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResumeEvidence {
    CredentialProvided { credential_ref: String },
    CallbackCompleted { callback_ref: String },
}

/// Reason a product surface cancels a pending auth flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthCancelReason {
    /// User actively denied the flow.
    UserDenied,
    /// User dismissed the flow without an explicit decision.
    UserCancelled,
}

/// Result of a resume or cancel decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthResumeOutcome {
    /// The flow was successfully resumed and is no longer pending.
    Resumed,
    /// The flow was cancelled and is no longer pending.
    Cancelled,
}

/// Stable, redacted error taxonomy.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthFlowError {
    /// No auth flow matches `(scope, flow_ref)`. Returned both for
    /// genuinely missing flows and for wrong-scope lookups.
    #[error("auth flow is unknown")]
    Unknown,
    /// The flow is no longer active (already resumed, cancelled,
    /// expired). Stable irrespective of which terminal status applies.
    #[error("auth flow is no longer active")]
    Terminal,
    /// Resume evidence cannot be applied to this flow (wrong evidence
    /// shape, missing context). Distinct from `Terminal` because the
    /// caller can retry with corrected evidence.
    #[error("auth flow evidence is invalid")]
    InvalidEvidence,
    /// The supplied [`AuthFlowRef`] failed validation.
    #[error("invalid auth flow ref: {reason}")]
    InvalidRef { reason: String },
    /// Any other backend failure. Stable category; inner detail is not
    /// part of the user-visible surface.
    #[error("auth flow backend failed")]
    Backend,
}

/// Host-owned manager for pending auth flows.
///
/// Production wires this to whatever credential broker owns the flow
/// (see #3068). The interaction service composes it via
/// [`crate::auth::AuthInteractionService`] — the trait is the only
/// boundary product surfaces ever cross to influence an auth flow.
#[async_trait]
pub trait AuthFlowManager: Send + Sync {
    /// List flows pending for `scope`. Wrong-scope flows must not appear.
    async fn list_pending(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<AuthFlowRecord>, AuthFlowError>;

    /// Look up one pending flow. Wrong-scope lookups return `Ok(None)`,
    /// not an error, so callers cannot probe other scopes by error
    /// classification.
    async fn get(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
    ) -> Result<Option<AuthFlowRecord>, AuthFlowError>;

    /// Resume a pending flow with evidence supplied by the product
    /// surface. Returns [`AuthResumeOutcome::Resumed`] on success.
    async fn resume(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
        evidence: AuthResumeEvidence,
    ) -> Result<AuthResumeOutcome, AuthFlowError>;

    /// Cancel a pending flow. Returns [`AuthResumeOutcome::Cancelled`]
    /// so callers receive a unified outcome shape across resume/cancel
    /// without having to inspect the trait method.
    async fn cancel(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
        reason: AuthCancelReason,
    ) -> Result<AuthResumeOutcome, AuthFlowError>;
}

/// Internal flow status for the in-memory manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlowStatus {
    Pending,
    Resumed,
    Cancelled,
}

#[derive(Debug, Clone)]
struct StoredFlow {
    record: AuthFlowRecord,
    status: FlowStatus,
}

/// In-memory implementation used for tests and early host wiring.
///
/// Production composition replaces this with a manager backed by the
/// credential broker. The in-memory variant exposes `register_pending`
/// for tests/wiring to populate flows directly.
#[derive(Default)]
pub struct InMemoryAuthFlowManager {
    flows: Mutex<HashMap<AuthFlowRef, StoredFlow>>,
}

impl InMemoryAuthFlowManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pending flow.
    ///
    /// Available to tests and host wiring that owns the flow lifecycle —
    /// product surfaces never construct flows; they only resolve them.
    pub fn register_pending(
        &self,
        flow_ref: AuthFlowRef,
        scope: ResourceScope,
        capability_id: CapabilityId,
    ) {
        let mut flows = self.flows.lock().unwrap_or_else(|p| p.into_inner());
        flows.insert(
            flow_ref.clone(),
            StoredFlow {
                record: AuthFlowRecord {
                    flow_ref,
                    scope,
                    capability_id,
                },
                status: FlowStatus::Pending,
            },
        );
    }
}

#[async_trait]
impl AuthFlowManager for InMemoryAuthFlowManager {
    async fn list_pending(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<AuthFlowRecord>, AuthFlowError> {
        let flows = self.flows.lock().unwrap_or_else(|p| p.into_inner());
        let records = flows
            .values()
            .filter(|stored| stored.status == FlowStatus::Pending)
            .filter(|stored| same_scope_owner(&stored.record.scope, scope))
            .map(|stored| stored.record.clone())
            .collect();
        Ok(records)
    }

    async fn get(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
    ) -> Result<Option<AuthFlowRecord>, AuthFlowError> {
        let flows = self.flows.lock().unwrap_or_else(|p| p.into_inner());
        Ok(flows
            .get(flow_ref)
            .filter(|stored| same_scope_owner(&stored.record.scope, scope))
            .filter(|stored| stored.status == FlowStatus::Pending)
            .map(|stored| stored.record.clone()))
    }

    async fn resume(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
        _evidence: AuthResumeEvidence,
    ) -> Result<AuthResumeOutcome, AuthFlowError> {
        let mut flows = self.flows.lock().unwrap_or_else(|p| p.into_inner());
        let stored = flows.get_mut(flow_ref).ok_or(AuthFlowError::Unknown)?;
        if !same_scope_owner(&stored.record.scope, scope) {
            return Err(AuthFlowError::Unknown);
        }
        if stored.status != FlowStatus::Pending {
            return Err(AuthFlowError::Terminal);
        }
        stored.status = FlowStatus::Resumed;
        Ok(AuthResumeOutcome::Resumed)
    }

    async fn cancel(
        &self,
        scope: &ResourceScope,
        flow_ref: &AuthFlowRef,
        _reason: AuthCancelReason,
    ) -> Result<AuthResumeOutcome, AuthFlowError> {
        let mut flows = self.flows.lock().unwrap_or_else(|p| p.into_inner());
        let stored = flows.get_mut(flow_ref).ok_or(AuthFlowError::Unknown)?;
        if !same_scope_owner(&stored.record.scope, scope) {
            return Err(AuthFlowError::Unknown);
        }
        if stored.status != FlowStatus::Pending {
            return Err(AuthFlowError::Terminal);
        }
        stored.status = FlowStatus::Cancelled;
        Ok(AuthResumeOutcome::Cancelled)
    }
}

fn same_scope_owner(left: &ResourceScope, right: &ResourceScope) -> bool {
    left.tenant_id == right.tenant_id
        && left.user_id == right.user_id
        && left.agent_id == right.agent_id
        && left.project_id == right.project_id
        && left.mission_id == right.mission_id
        && left.thread_id == right.thread_id
}
