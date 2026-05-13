//! Glue between extension-manifest-declared predicates and the dispatcher's
//! hook trait surface.
//!
//! The registry installer constructs a [`PredicateBackedBeforeCapabilityHook`]
//! for each `[[hooks]]` entry whose body is `HookManifestBody::Predicate`.
//! The hook holds an `Arc` to the shared [`PredicateEvaluator`] (so sliding-
//! window state is shared across all predicate-backed hooks in a run) plus
//! the spec it was constructed from.

use std::sync::Arc;

use async_trait::async_trait;

use crate::evaluator::{EvaluatorDecision, PredicateEvaluator};
use crate::identity::HookId;
use crate::points::BeforeCapabilityHookContext;
use crate::predicate::HookPredicateSpec;
use crate::sink::{RestrictedBeforeCapabilityHook, RestrictedGateSink};

/// A `before_capability` hook implementation backed by a declarative
/// predicate from an extension manifest. Always `Installed`-tier.
pub struct PredicateBackedBeforeCapabilityHook {
    hook_id: HookId,
    spec: HookPredicateSpec,
    evaluator: Arc<PredicateEvaluator>,
}

impl PredicateBackedBeforeCapabilityHook {
    pub fn new(
        hook_id: HookId,
        spec: HookPredicateSpec,
        evaluator: Arc<PredicateEvaluator>,
    ) -> Self {
        Self {
            hook_id,
            spec,
            evaluator,
        }
    }
}

#[async_trait]
impl RestrictedBeforeCapabilityHook for PredicateBackedBeforeCapabilityHook {
    async fn evaluate(&self, ctx: &BeforeCapabilityHookContext, sink: &mut dyn RestrictedGateSink) {
        // Sinks take `&'static str` reasons to keep adversarial format!-built
        // strings out of the seam. Predicate reasons come from the manifest
        // (author-controlled) and are dynamic, so the evaluator's reason
        // string is leaked as a closed vocabulary of static labels here.
        // Richer reasons surface in audit, not in the model-visible decision.
        match self.evaluator.evaluate(self.hook_id, &self.spec, ctx) {
            EvaluatorDecision::Allow => {
                // The predicate did not match — the hook has no opinion. The
                // dispatcher recognizes `pass()` as a no-opinion contribution
                // and continues composing without short-circuiting.
                sink.pass();
            }
            EvaluatorDecision::Deny { .. } => {
                sink.deny("hook_predicate_denied");
            }
            EvaluatorDecision::PauseApproval { .. } => {
                sink.pause_approval("hook_predicate_pause_requested");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{ExtensionId, HookLocalId, HookVersion};
    use crate::predicate::{CapabilityPredicate, HookPredicateSpec};
    use crate::sink::RecordingGateSink;
    use ironclaw_host_api::TenantId;

    fn hook_id() -> HookId {
        HookId::derive(
            &ExtensionId("ext".to_string()),
            "1.0",
            &HookLocalId("h".to_string()),
            HookVersion::ONE,
        )
    }

    #[tokio::test]
    async fn deny_predicate_routes_to_sink_deny() {
        let evaluator = Arc::new(PredicateEvaluator::new());
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: "ignored: routes to closed-vocab label".to_string(),
        };
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id(), spec, evaluator);
        let mut sink = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new(
            TenantId::new("alpha").expect("ok"),
            "shell.exec".to_string(),
            [0u8; 32],
        );

        hook.evaluate(&ctx, &mut sink as &mut dyn RestrictedGateSink)
            .await;
        let decision = sink.decision().expect("hook emitted a decision");
        assert!(!decision.permits());
    }

    #[tokio::test]
    async fn allow_predicate_routes_to_sink_pass() {
        use crate::sink::GateSinkState;

        let evaluator = Arc::new(PredicateEvaluator::new());
        // Spec only fires on `shell.exec`; context invokes a different
        // capability so the evaluator returns Allow.
        let spec = HookPredicateSpec::DenyCapability {
            when: CapabilityPredicate::NameEquals {
                name: "shell.exec".to_string(),
            },
            reason: "shell denied".to_string(),
        };
        let hook = PredicateBackedBeforeCapabilityHook::new(hook_id(), spec, evaluator);
        let mut sink = RecordingGateSink::new();
        let ctx = BeforeCapabilityHookContext::new(
            TenantId::new("alpha").expect("ok"),
            "memory.read".to_string(),
            [0u8; 32],
        );

        hook.evaluate(&ctx, &mut sink as &mut dyn RestrictedGateSink)
            .await;
        assert!(
            sink.decision().is_none(),
            "no-opinion path must not record a decision"
        );
        assert_eq!(sink.state, GateSinkState::Passed);
    }
}
