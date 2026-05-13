//! Capability-port middleware that runs `dispatch_before_capability` ahead of
//! every invocation and translates hook decisions into the existing
//! `CapabilityOutcome` vocabulary.
//!
//! Translation:
//!
//! - `GateDecisionInner::Allow` → forward to inner port unchanged.
//! - `GateDecisionInner::Deny` → return `CapabilityOutcome::Denied` with
//!   `CapabilityDeniedReasonKind::Unknown("hook_denied")` and the sanitized
//!   reason as `safe_summary`.
//! - `GateDecisionInner::PauseApproval` / `PauseAuth` → return the
//!   corresponding suspension outcome. The middleware itself does not
//!   generate gate refs; Phase 2 (#3524 roadmap) wires those into the host's
//!   approval/auth gate machinery. For now, suspension hook decisions surface
//!   as `Denied` so the loop fails closed rather than silently allowing.
//!
//! Failure cases from the dispatcher (panic, timeout, missing impl) also map
//! to `Denied` per the [`crate::failure_policy`] rules.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::TenantId;
use ironclaw_turns::run_profile::{
    AgentLoopHostError, CapabilityBatchInvocation, CapabilityBatchOutcome, CapabilityDenied,
    CapabilityDeniedReasonKind, CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort,
    VisibleCapabilityRequest, VisibleCapabilitySurface,
};

use crate::dispatch::{BeforeCapabilityDispatchOutcome, HookDispatcher};
use crate::kinds::gate::GateDecisionInner;
use crate::middleware::resolver::{CapabilityInputResolver, NullCapabilityInputResolver};
use crate::points::{BeforeCapabilityHookContext, SanitizedArguments};

/// Wraps an inner `LoopCapabilityPort`, fires `before_capability` hooks ahead
/// of each invocation, and translates the dispatcher's composed decision into
/// the `CapabilityOutcome` vocabulary the loop driver already speaks.
pub struct HookedLoopCapabilityPort {
    inner: Arc<dyn LoopCapabilityPort>,
    dispatcher: Arc<HookDispatcher>,
    tenant_id: TenantId,
    resolver: Arc<dyn CapabilityInputResolver>,
}

impl HookedLoopCapabilityPort {
    /// Construct a middleware with the bundled
    /// [`NullCapabilityInputResolver`]. Predicate evaluators that depend on
    /// argument contents (e.g., `ValueOrRateBound::NumericSum`) will fail
    /// closed; use [`Self::with_resolver`] to wire in a production resolver.
    pub fn new(
        inner: Arc<dyn LoopCapabilityPort>,
        dispatcher: Arc<HookDispatcher>,
        tenant_id: TenantId,
    ) -> Self {
        Self {
            inner,
            dispatcher,
            tenant_id,
            resolver: Arc::new(NullCapabilityInputResolver),
        }
    }

    /// Override the resolver used to surface sanitized arguments to hook
    /// predicates. Returns `self` so callers can chain after `new`.
    #[must_use]
    pub fn with_resolver(mut self, resolver: Arc<dyn CapabilityInputResolver>) -> Self {
        self.resolver = resolver;
        self
    }

    async fn hook_context(&self, invocation: &CapabilityInvocation) -> BeforeCapabilityHookContext {
        let arguments = match self.resolver.resolve(invocation).await {
            Some(value) => SanitizedArguments::from_json(value),
            None => SanitizedArguments::unresolved(),
        };
        BeforeCapabilityHookContext::new(
            self.tenant_id.clone(),
            invocation.capability_id.to_string(),
            invocation_arguments_digest(invocation),
            arguments,
        )
    }

    async fn run_dispatch(
        &self,
        invocation: &CapabilityInvocation,
    ) -> BeforeCapabilityDispatchOutcome {
        let ctx = self.hook_context(invocation).await;
        self.dispatcher.dispatch_before_capability(&ctx).await
    }
}

#[async_trait]
impl LoopCapabilityPort for HookedLoopCapabilityPort {
    async fn visible_capabilities(
        &self,
        request: VisibleCapabilityRequest,
    ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
        // Visible-surface queries don't go through hooks (the surface itself
        // is owned by profile-scoped filtering; hooks gate invocation, not
        // listing).
        self.inner.visible_capabilities(request).await
    }

    async fn invoke_capability(
        &self,
        request: CapabilityInvocation,
    ) -> Result<CapabilityOutcome, AgentLoopHostError> {
        let outcome = self.run_dispatch(&request).await;
        match decision_to_outcome(&outcome) {
            Some(translated) => Ok(translated),
            None => self.inner.invoke_capability(request).await,
        }
    }

    async fn invoke_capability_batch(
        &self,
        request: CapabilityBatchInvocation,
    ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
        // Each invocation runs its own hook pre-flight. Hooks can deny one
        // call in a batch without affecting others — the inner port still
        // executes the non-denied calls.
        let CapabilityBatchInvocation {
            invocations,
            stop_on_first_suspension,
        } = request;
        let mut outcomes = Vec::with_capacity(invocations.len());
        let mut stopped_on_suspension = false;
        for invocation in invocations {
            if stopped_on_suspension {
                break;
            }
            let dispatch = self.run_dispatch(&invocation).await;
            let outcome = match decision_to_outcome(&dispatch) {
                Some(translated) => translated,
                None => self.inner.invoke_capability(invocation).await?,
            };
            if outcome.is_suspension() && stop_on_first_suspension {
                stopped_on_suspension = true;
            }
            outcomes.push(outcome);
        }
        Ok(CapabilityBatchOutcome {
            outcomes,
            stopped_on_suspension,
        })
    }
}

/// Returns `Some(outcome)` if the hook decision is restrictive (deny / pause
/// / failure-closed), or `None` if the hooks said allow and the inner port
/// should be consulted.
fn decision_to_outcome(dispatched: &BeforeCapabilityDispatchOutcome) -> Option<CapabilityOutcome> {
    match dispatched.decision.inner() {
        GateDecisionInner::Allow => None,
        GateDecisionInner::Deny { reason } => Some(CapabilityOutcome::Denied(CapabilityDenied {
            reason_kind: CapabilityDeniedReasonKind::unknown("hook_denied")
                .expect("hook_denied is a valid loop-safe identifier"),
            safe_summary: reason.as_str().to_string(),
        })),
        GateDecisionInner::PauseApproval { reason } | GateDecisionInner::PauseAuth { reason } => {
            // For the foundation slice, pause-class decisions fail closed at
            // the middleware boundary: the gate-ref plumbing belongs in the
            // approval-router wiring of the next slice. Returning Denied
            // keeps the host's existing approval flow untouched while
            // surfacing the hook's intent.
            Some(CapabilityOutcome::Denied(CapabilityDenied {
                reason_kind: CapabilityDeniedReasonKind::unknown("hook_paused")
                    .expect("hook_paused is a valid loop-safe identifier"),
                safe_summary: reason.as_str().to_string(),
            }))
        }
    }
}

/// Stable digest of capability arguments for hook context. The middleware
/// hashes the input-ref's underlying value so two invocations with identical
/// arguments produce the same digest, enabling repetition / rate-cap logic
/// without exposing raw arguments to hook code.
fn invocation_arguments_digest(invocation: &CapabilityInvocation) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    let cap = invocation.capability_id.to_string();
    hasher.update(&(cap.len() as u64).to_le_bytes());
    hasher.update(cap.as_bytes());
    let input = format!("{:?}", invocation.input_ref);
    hasher.update(&(input.len() as u64).to_le_bytes());
    hasher.update(input.as_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dispatch::BeforeCapabilityHookImpl;
    use crate::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
    use crate::ordering::HookPhase;
    use crate::registry::{HookBinding, HookPointSpec, HookRegistry};
    use crate::sink::{RestrictedBeforeCapabilityHook, RestrictedGateSink};
    use crate::trust::HookTrustClass;
    use async_trait::async_trait;
    use ironclaw_host_api::{CapabilityId, RuntimeKind};
    use ironclaw_turns::LoopResultRef;
    use ironclaw_turns::run_profile::{
        CapabilityDescriptorView, CapabilityInputRef, CapabilityResultMessage,
        CapabilitySurfaceVersion,
    };
    use std::sync::Mutex;

    fn tenant() -> TenantId {
        TenantId::new("alpha").expect("ok")
    }

    struct AlwaysCompletedPort {
        calls: Mutex<Vec<CapabilityId>>,
    }

    impl AlwaysCompletedPort {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<CapabilityId> {
            self.calls.lock().expect("not poisoned").clone()
        }
    }

    #[async_trait]
    impl LoopCapabilityPort for AlwaysCompletedPort {
        async fn visible_capabilities(
            &self,
            _request: VisibleCapabilityRequest,
        ) -> Result<VisibleCapabilitySurface, AgentLoopHostError> {
            Ok(VisibleCapabilitySurface {
                version: CapabilitySurfaceVersion::new("v1").expect("ok"),
                descriptors: vec![CapabilityDescriptorView {
                    capability_id: CapabilityId::new("cap.x").expect("ok"),
                    provider: None,
                    runtime: RuntimeKind::Wasm,
                    safe_name: "cap.x".to_string(),
                    safe_description: "test capability".to_string(),
                }],
            })
        }

        async fn invoke_capability(
            &self,
            request: CapabilityInvocation,
        ) -> Result<CapabilityOutcome, AgentLoopHostError> {
            self.calls
                .lock()
                .expect("not poisoned")
                .push(request.capability_id.clone());
            Ok(CapabilityOutcome::Completed(CapabilityResultMessage {
                result_ref: LoopResultRef::new(format!("result:{}", request.capability_id))
                    .expect("ok"),
                safe_summary: format!("ran {}", request.capability_id),
            }))
        }

        async fn invoke_capability_batch(
            &self,
            request: CapabilityBatchInvocation,
        ) -> Result<CapabilityBatchOutcome, AgentLoopHostError> {
            let mut outcomes = Vec::with_capacity(request.invocations.len());
            for invocation in request.invocations {
                outcomes.push(self.invoke_capability(invocation).await?);
            }
            Ok(CapabilityBatchOutcome {
                outcomes,
                stopped_on_suspension: false,
            })
        }
    }

    struct DenyingHook;
    #[async_trait]
    impl RestrictedBeforeCapabilityHook for DenyingHook {
        async fn evaluate(
            &self,
            _ctx: &BeforeCapabilityHookContext,
            sink: &mut dyn RestrictedGateSink,
        ) {
            sink.deny("blocked by extension policy");
        }
    }

    fn invocation(capability: &str) -> CapabilityInvocation {
        CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("v1").expect("ok"),
            capability_id: CapabilityId::new(capability).expect("ok"),
            input_ref: CapabilityInputRef::new(format!("input:{capability}")).expect("ok"),
        }
    }

    fn dispatcher_with_deny_hook() -> (Arc<HookDispatcher>, HookId) {
        let hook_id = HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("deny".to_string()),
            HookVersion::ONE,
        );
        let binding = HookBinding {
            hook_id,
            hook_version: HookVersion::ONE,
            trust_class: HookTrustClass::Installed,
            phase: HookPhase::Policy,
            point: HookPointSpec::BeforeCapability,
            poisoned: false,
        };
        let mut registry = HookRegistry::new();
        registry.insert(binding).expect("ok");
        let mut dispatcher = HookDispatcher::new(registry);
        dispatcher.install_before_capability(
            hook_id,
            BeforeCapabilityHookImpl::Restricted(Box::new(DenyingHook)),
        );
        (Arc::new(dispatcher), hook_id)
    }

    #[tokio::test]
    async fn deny_hook_short_circuits_invocation() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) = dispatcher_with_deny_hook();
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert!(matches!(outcome, CapabilityOutcome::Denied(_)));
        assert!(
            inner.calls().is_empty(),
            "inner port must not be invoked when a hook denies"
        );
    }

    #[tokio::test]
    async fn no_hooks_passes_through_to_inner() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let outcome = wrapped
            .invoke_capability(invocation("cap.x"))
            .await
            .expect("ok");

        assert!(matches!(outcome, CapabilityOutcome::Completed(_)));
        assert_eq!(inner.calls().len(), 1);
    }

    #[tokio::test]
    async fn batch_fires_dispatch_per_invocation() {
        // With the always-deny hook installed, every invocation in the batch
        // gets denied by hook dispatch and the inner port is never reached.
        // This verifies the wrapper's per-invocation dispatch loop, not just
        // the single-invocation path.
        let inner = Arc::new(AlwaysCompletedPort::new());
        let (dispatcher, _) = dispatcher_with_deny_hook();
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
        assert_eq!(outcome.outcomes.len(), 2);
        assert!(inner.calls().is_empty(), "inner must not be invoked");
        for entry in &outcome.outcomes {
            assert!(matches!(entry, CapabilityOutcome::Denied(_)));
        }
    }

    #[tokio::test]
    async fn batch_passes_through_when_no_hooks() {
        let inner = Arc::new(AlwaysCompletedPort::new());
        let dispatcher = Arc::new(HookDispatcher::new(HookRegistry::new()));
        let wrapped = HookedLoopCapabilityPort::new(inner.clone(), dispatcher, tenant());

        let batch = CapabilityBatchInvocation {
            invocations: vec![invocation("cap.alpha"), invocation("cap.beta")],
            stop_on_first_suspension: false,
        };
        let outcome = wrapped.invoke_capability_batch(batch).await.expect("ok");
        assert_eq!(outcome.outcomes.len(), 2);
        assert_eq!(inner.calls().len(), 2);
        for entry in &outcome.outcomes {
            assert!(matches!(entry, CapabilityOutcome::Completed(_)));
        }
    }
}
