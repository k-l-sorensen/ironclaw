//! Bridge between extension-manifest `[[hooks]]` entries and a configured
//! [`HookDispatcher`].
//!
//! The [`HookRegistrar`] is the single seam that the registry installer (or
//! anything that ships an extension's hook block into a live dispatcher)
//! goes through. For each manifest entry it:
//!
//! 1. Validates the entry's well-formedness via
//!    [`crate::manifest::HookManifestEntry::validate`].
//! 2. Derives a content-addressed [`HookId`] from the extension identity +
//!    entry id + versions.
//! 3. Builds a [`HookBinding`] tagged `HookTrustClass::Installed` and
//!    inserts it into the registry (which re-checks phase × trust).
//! 4. Constructs the runtime impl from the manifest body and installs it
//!    against the same `HookId` in the dispatcher.
//!
//! Trust class is *not* settable here — registry-sourced hooks are always
//! `Installed`. Builtin and Trusted hooks bypass this path entirely.

use std::sync::Arc;

use crate::dispatch::HookDispatcher;
use crate::error::HookError;
use crate::evaluator::PredicateEvaluator;
use crate::identity::{ExtensionId, HookId, HookVersion};
use crate::installed_hook::PredicateBackedBeforeCapabilityHook;
use crate::manifest::{HookManifestBody, HookManifestEntry, HookManifestKind};

/// Converts validated [`HookManifestEntry`] values into installed bindings +
/// dispatcher impls. One registrar per run; the shared
/// [`PredicateEvaluator`] threads sliding-window state across every
/// predicate-backed hook the registrar produces.
pub struct HookRegistrar {
    evaluator: Arc<PredicateEvaluator>,
}

impl HookRegistrar {
    pub fn new(evaluator: Arc<PredicateEvaluator>) -> Self {
        Self { evaluator }
    }

    /// Install all entries against `dispatcher`. Returns the
    /// [`HookId`]s in the same order as `entries`. If any entry fails
    /// validation or impl construction, the registrar returns the error
    /// without rolling back earlier inserts — callers wanting all-or-nothing
    /// semantics should build into a scratch dispatcher first.
    pub fn install(
        &self,
        extension: ExtensionId,
        extension_version: String,
        entries: Vec<HookManifestEntry>,
        dispatcher: &mut HookDispatcher,
    ) -> Result<Vec<HookId>, HookError> {
        let mut installed = Vec::with_capacity(entries.len());
        for entry in entries {
            let hook_id = self.install_one(&extension, &extension_version, entry, dispatcher)?;
            installed.push(hook_id);
        }
        Ok(installed)
    }

    fn install_one(
        &self,
        extension: &ExtensionId,
        extension_version: &str,
        entry: HookManifestEntry,
        dispatcher: &mut HookDispatcher,
    ) -> Result<HookId, HookError> {
        entry.validate().map_err(|e| {
            HookError::RegistryConstruction(format!(
                "manifest entry `{}` failed validation: {}",
                entry.id, e
            ))
        })?;

        let hook_version = HookVersion::ONE;
        let hook_id = HookId::derive(extension, extension_version, &entry.id, hook_version);

        match entry.body {
            HookManifestBody::Predicate { spec } => match entry.kind {
                HookManifestKind::BeforeCapability => {
                    let hook = PredicateBackedBeforeCapabilityHook::new(
                        hook_id,
                        spec,
                        Arc::clone(&self.evaluator),
                    );
                    dispatcher.install_installed_before_capability(
                        hook_id,
                        entry.phase,
                        Box::new(hook),
                    )?;
                }
                other => {
                    return Err(HookError::RegistryConstruction(format!(
                        "predicate body is only supported for `before_capability` hooks; \
                         entry `{}` declared kind {:?}",
                        entry.id, other
                    )));
                }
            },
            HookManifestBody::Wasm { .. } => {
                return Err(HookError::RegistryConstruction(format!(
                    "WASM hook execution is not yet implemented; entry `{}` was \
                     rejected by the registrar",
                    entry.id
                )));
            }
        }

        Ok(hook_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::HookLocalId;
    use crate::manifest::{HookManifestBody, HookManifestKind, HookManifestScope, WasmBudget};
    use crate::ordering::{HookPhase, HookPriority};
    use crate::points::BeforeCapabilityHookContext;
    use crate::predicate::{CapabilityPredicate, HookPredicateSpec};
    use crate::registry::HookRegistry;

    fn extension() -> ExtensionId {
        ExtensionId("polymarket-trader".to_string())
    }

    fn predicate_entry(local: &str) -> HookManifestEntry {
        HookManifestEntry {
            id: HookLocalId(local.to_string()),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Predicate {
                spec: HookPredicateSpec::DenyCapability {
                    when: CapabilityPredicate::NameEquals {
                        name: "shell.exec".to_string(),
                    },
                    reason: "shell denied".to_string(),
                },
            },
        }
    }

    #[tokio::test]
    async fn install_predicate_entry_builds_binding_and_installs_hook() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let ids = registrar
            .install(
                extension(),
                "0.4.2".to_string(),
                vec![predicate_entry("deny-shell")],
                &mut dispatcher,
            )
            .expect("install ok");
        assert_eq!(ids.len(), 1);

        // Dispatch and confirm the registered predicate fires.
        let tenant = ironclaw_host_api::TenantId::new("alpha").expect("tenant");
        let ctx = BeforeCapabilityHookContext::new(tenant, "shell.exec".to_string(), [0u8; 32]);
        let outcome = dispatcher.dispatch_before_capability(&ctx).await;
        assert!(!outcome.decision.permits());
    }

    #[test]
    fn install_rejects_wasm_body_for_now() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let entry = HookManifestEntry {
            id: HookLocalId("wasm-hook".to_string()),
            kind: HookManifestKind::BeforeCapability,
            scope: HookManifestScope::OwnCapabilities,
            phase: HookPhase::Policy,
            priority: HookPriority::DEFAULT,
            description: None,
            requires_grant: None,
            body: HookManifestBody::Wasm {
                export: "evaluate".to_string(),
                budget: WasmBudget::default(),
            },
        };
        let err = registrar
            .install(
                extension(),
                "0.1.0".to_string(),
                vec![entry],
                &mut dispatcher,
            )
            .expect_err("wasm body must be rejected");
        match err {
            HookError::RegistryConstruction(msg) => {
                assert!(msg.contains("WASM"), "unexpected message: {msg}");
            }
            other => panic!("expected RegistryConstruction, got {other:?}"),
        }
    }

    #[test]
    fn install_rejects_invalid_phase_for_installed_tier() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let mut entry = predicate_entry("bad-phase");
        // Validation phase is Builtin-only — manifest validation rejects it
        // before the registry would.
        entry.phase = HookPhase::Validation;
        let err = registrar
            .install(
                extension(),
                "0.1.0".to_string(),
                vec![entry],
                &mut dispatcher,
            )
            .expect_err("validation phase must be rejected");
        assert!(matches!(err, HookError::RegistryConstruction(_)));
    }

    #[tokio::test]
    async fn install_returns_hook_ids_in_input_order() {
        let registrar = HookRegistrar::new(Arc::new(PredicateEvaluator::new()));
        let mut dispatcher = HookDispatcher::new(HookRegistry::new());
        let entries = vec![
            predicate_entry("first"),
            predicate_entry("second"),
            predicate_entry("third"),
        ];
        let expected: Vec<HookId> = entries
            .iter()
            .map(|e| HookId::derive(&extension(), "0.4.2", &e.id, HookVersion::ONE))
            .collect();

        let actual = registrar
            .install(extension(), "0.4.2".to_string(), entries, &mut dispatcher)
            .expect("install ok");
        assert_eq!(actual, expected);
    }
}
