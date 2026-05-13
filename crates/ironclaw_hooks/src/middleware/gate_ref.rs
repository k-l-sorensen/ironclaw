//! `HookGateRefFactory` — middleware-facing seam that mints `LoopGateRef`
//! values for hook-emitted pause decisions.
//!
//! When a `before_capability` hook returns `PauseApproval` or `PauseAuth`,
//! the `HookedLoopCapabilityPort` middleware needs to produce a real
//! `LoopGateRef` so the resulting `CapabilityOutcome::ApprovalRequired` /
//! `AuthRequired` can be routed through the host's gate-resolution
//! machinery. The middleware does not know how to mint refs that scope
//! correctly to the current run / approval-router — that knowledge lives
//! in the Reborn host composition. This trait is the seam the middleware
//! depends on; production code wires a concrete factory that talks to the
//! host's gate-router.
//!
//! The foundation slice ships [`UuidHookGateRefFactory`] — a deterministic
//! local-only implementation that mints opaque, run-scope-agnostic refs
//! using `uuid::Uuid::new_v4()`. It is suitable for tests and for the
//! foundation-slice end-to-end wiring, but production deployments should
//! provide a factory that takes the `LoopRunContext` at construction time
//! and emits refs that the host's approval-router will recognize.
//!
//! Failures bubble up as `AgentLoopHostError` so the middleware can fail
//! closed (mapping the suspension back to `Denied`) rather than silently
//! producing an unresolvable gate ref.

use async_trait::async_trait;
use ironclaw_turns::LoopGateRef;
use ironclaw_turns::run_profile::{AgentLoopHostError, AgentLoopHostErrorKind};

/// Mints gate refs for hook-emitted suspension decisions.
///
/// The trait is split into approval and auth variants so a future
/// production impl can route them through different gate-router channels
/// without having to inspect the decision kind here. Both methods return
/// a fully validated [`LoopGateRef`] or an [`AgentLoopHostError`] if the
/// gate-router refused to mint one (the middleware fails closed in that
/// case).
#[async_trait]
pub trait HookGateRefFactory: Send + Sync {
    async fn mint_approval_ref(&self, reason: &str) -> Result<LoopGateRef, AgentLoopHostError>;
    async fn mint_auth_ref(&self, reason: &str) -> Result<LoopGateRef, AgentLoopHostError>;
}

/// Foundation-slice default. Mints opaque `gate:hook-approval-<uuid>` /
/// `gate:hook-auth-<uuid>` refs using `uuid::Uuid::new_v4()`. Refs are
/// locally unique but carry no scope information — production factories
/// should embed the run context so the host's approval-router can route
/// gate-resolution events back to the right run.
#[derive(Debug, Default, Clone, Copy)]
pub struct UuidHookGateRefFactory;

impl UuidHookGateRefFactory {
    pub fn new() -> Self {
        Self
    }

    fn mint(prefix: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        // Uuid hyphenated form is exclusively ASCII alphanumeric + `-`,
        // which matches LoopGateRef's opaque-id charset.
        let id = uuid::Uuid::new_v4();
        let value = format!("gate:{prefix}-{id}");
        LoopGateRef::new(value).map_err(|err| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("hook gate-ref factory failed: {err}"),
            )
        })
    }
}

#[async_trait]
impl HookGateRefFactory for UuidHookGateRefFactory {
    async fn mint_approval_ref(&self, _reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        Self::mint("hook-approval")
    }

    async fn mint_auth_ref(&self, _reason: &str) -> Result<LoopGateRef, AgentLoopHostError> {
        Self::mint("hook-auth")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approval_ref_has_valid_format() {
        let factory = UuidHookGateRefFactory;
        let r = factory
            .mint_approval_ref("needs approval")
            .await
            .expect("mints");
        assert!(r.as_str().starts_with("gate:hook-approval-"));
    }

    #[tokio::test]
    async fn auth_ref_has_valid_format() {
        let factory = UuidHookGateRefFactory;
        let r = factory.mint_auth_ref("needs auth").await.expect("mints");
        assert!(r.as_str().starts_with("gate:hook-auth-"));
    }

    #[tokio::test]
    async fn refs_are_unique_across_calls() {
        let factory = UuidHookGateRefFactory;
        let a = factory.mint_approval_ref("r").await.expect("mints");
        let b = factory.mint_approval_ref("r").await.expect("mints");
        assert_ne!(a.as_str(), b.as_str());
    }
}
