//! Resolver trait for converting `CapabilityInputRef` handles into sanitized
//! JSON the hook framework can hand to predicate evaluation.
//!
//! The hook crate intentionally does not know how to dereference a
//! [`CapabilityInputRef`] — that knowledge belongs to the production host
//! (which has workspace / store access). The middleware accepts an
//! `Arc<dyn CapabilityInputResolver>` and consults it before each invocation;
//! when no resolver is configured, the bundled
//! [`NullCapabilityInputResolver`] returns `None`, causing
//! [`crate::points::SanitizedArguments::unresolved`] to be threaded through.
//!
//! Predicate evaluation that requires argument contents (currently
//! `ValueOrRateBound::NumericSum`) is responsible for failing closed in the
//! unresolved case.

use async_trait::async_trait;
use ironclaw_turns::run_profile::CapabilityInvocation;

/// Resolves a [`CapabilityInvocation`]'s input ref to a sanitized JSON view.
///
/// Implementations should return:
///
/// - `Some(value)` when the ref was resolved and the JSON-shaped payload is
///   safe to hand to hook predicates (already free of secrets / handle
///   pointers / etc. — the framework will further bound size and depth).
/// - `None` when resolution is unavailable, fails, or the result is
///   unsafe to expose. The hook framework treats `None` as
///   "unresolved" — predicate evaluators that depend on argument
///   contents must fail closed in this case.
#[async_trait]
pub trait CapabilityInputResolver: Send + Sync {
    async fn resolve(&self, invocation: &CapabilityInvocation) -> Option<serde_json::Value>;
}

/// Default resolver that never resolves arguments. Used when middleware
/// composers haven't wired in a production resolver yet.
pub struct NullCapabilityInputResolver;

#[async_trait]
impl CapabilityInputResolver for NullCapabilityInputResolver {
    async fn resolve(&self, _invocation: &CapabilityInvocation) -> Option<serde_json::Value> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::CapabilityId;
    use ironclaw_turns::run_profile::{CapabilityInputRef, CapabilitySurfaceVersion};

    #[tokio::test]
    async fn null_resolver_returns_none() {
        let resolver = NullCapabilityInputResolver;
        let invocation = CapabilityInvocation {
            surface_version: CapabilitySurfaceVersion::new("v1").expect("ok"),
            capability_id: CapabilityId::new("cap.x").expect("ok"),
            input_ref: CapabilityInputRef::new("input:x").expect("ok"),
        };
        assert!(resolver.resolve(&invocation).await.is_none());
    }
}
