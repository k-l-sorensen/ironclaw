//! Contract-only resolver for memory profile extension bindings.
//!
//! This module validates the operator-facing binding shape for required memory
//! profiles. It intentionally does not dispatch calls or change the existing
//! memory runtime path.

use ironclaw_host_api::{
    CapabilityProfileId, ExtensionId, HostApiError,
    runtime_policy::{DeploymentMode, EffectiveRuntimePolicy, RuntimeProfile},
};
use std::collections::BTreeMap;
use thiserror::Error;

pub const MEMORY_NATIVE_EXTENSION_ID: &str = "ironclaw.memory.native";
pub const MEMORY_DISABLED_EXTENSION_ID: &str = "memory.disabled";

/// Required host-defined memory profile contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RequiredMemoryProfileId {
    ContextRetrieval,
    InteractionLog,
    DocumentStore,
    SemanticSearch,
}

impl RequiredMemoryProfileId {
    pub const fn all() -> [Self; 4] {
        [
            Self::ContextRetrieval,
            Self::InteractionLog,
            Self::DocumentStore,
            Self::SemanticSearch,
        ]
    }

    pub const fn default_required() -> [Self; 3] {
        [
            Self::ContextRetrieval,
            Self::InteractionLog,
            Self::DocumentStore,
        ]
    }

    pub fn new(profile_id: CapabilityProfileId) -> Result<Self, MemoryProfileBindingError> {
        match profile_id.as_str() {
            "memory.context_retrieval.v1" => Ok(Self::ContextRetrieval),
            "memory.interaction_log.v1" => Ok(Self::InteractionLog),
            "memory.document_store.v1" => Ok(Self::DocumentStore),
            "memory.semantic_search.v1" => Ok(Self::SemanticSearch),
            _ => Err(MemoryProfileBindingError::UnknownRequiredProfile { profile_id }),
        }
    }

    pub fn capability_profile_id(&self) -> CapabilityProfileId {
        CapabilityProfileId::new(self.as_str())
            .expect("required memory profile id must validate") // safety: fixed contract id, validated by integration tests
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::ContextRetrieval => "memory.context_retrieval.v1",
            Self::InteractionLog => "memory.interaction_log.v1",
            Self::DocumentStore => "memory.document_store.v1",
            Self::SemanticSearch => "memory.semantic_search.v1",
        }
    }
}

impl std::fmt::Display for RequiredMemoryProfileId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Extension target for a required memory profile binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProfileBindingTarget {
    extension_id: ExtensionId,
}

impl MemoryProfileBindingTarget {
    pub fn native() -> Self {
        Self::literal(MEMORY_NATIVE_EXTENSION_ID)
    }

    pub fn disabled() -> Self {
        Self::literal(MEMORY_DISABLED_EXTENSION_ID)
    }

    pub fn extension(value: impl Into<String>) -> Result<Self, HostApiError> {
        Ok(Self {
            extension_id: ExtensionId::new(value)?,
        })
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub fn is_native(&self) -> bool {
        self.extension_id.as_str() == MEMORY_NATIVE_EXTENSION_ID
    }

    pub fn is_disabled(&self) -> bool {
        self.extension_id.as_str() == MEMORY_DISABLED_EXTENSION_ID
    }

    fn literal(value: &'static str) -> Self {
        Self {
            extension_id: ExtensionId::new(value)
                .expect("memory binding extension id must validate"), // safety: host-owned sentinel id, validated by integration tests
        }
    }
}

/// Deployment key used by explicit production third-party overrides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryProfileBindingDeployment {
    pub deployment: DeploymentMode,
    pub runtime_profile: RuntimeProfile,
}

impl MemoryProfileBindingDeployment {
    pub fn from_policy(policy: &EffectiveRuntimePolicy) -> Self {
        Self {
            deployment: policy.deployment,
            runtime_profile: policy.resolved_profile,
        }
    }

    pub fn is_production(&self) -> bool {
        !matches!(self.deployment, DeploymentMode::LocalSingleUser)
    }
}

impl std::fmt::Display for MemoryProfileBindingDeployment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}:{}", self.deployment, self.runtime_profile)
    }
}

/// Exact third-party allowance for a required profile in a deployment/profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProfileBindingOverride {
    profile_id: RequiredMemoryProfileId,
    extension_id: ExtensionId,
    deployment: MemoryProfileBindingDeployment,
}

impl MemoryProfileBindingOverride {
    pub fn new(
        profile_id: RequiredMemoryProfileId,
        extension_id: ExtensionId,
        deployment: MemoryProfileBindingDeployment,
    ) -> Self {
        Self {
            profile_id,
            extension_id,
            deployment,
        }
    }

    fn matches(
        &self,
        profile_id: RequiredMemoryProfileId,
        extension_id: &ExtensionId,
        deployment: MemoryProfileBindingDeployment,
    ) -> bool {
        self.profile_id == profile_id
            && &self.extension_id == extension_id
            && self.deployment == deployment
    }
}

/// Declarative binding config for required memory profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryProfileBindingConfig {
    required_profiles: Vec<RequiredMemoryProfileId>,
    explicit_bindings: BTreeMap<RequiredMemoryProfileId, MemoryProfileBindingTarget>,
    third_party_overrides: Vec<MemoryProfileBindingOverride>,
}

impl MemoryProfileBindingConfig {
    pub fn new(required_profiles: impl Into<Vec<RequiredMemoryProfileId>>) -> Self {
        Self {
            required_profiles: required_profiles.into(),
            explicit_bindings: BTreeMap::new(),
            third_party_overrides: Vec::new(),
        }
    }

    pub fn default_required_profiles() -> Self {
        Self::new(RequiredMemoryProfileId::default_required())
    }

    pub fn with_binding(
        mut self,
        profile_id: RequiredMemoryProfileId,
        target: MemoryProfileBindingTarget,
    ) -> Self {
        self.explicit_bindings.insert(profile_id, target);
        self
    }

    pub fn with_third_party_override(
        mut self,
        override_entry: MemoryProfileBindingOverride,
    ) -> Self {
        self.third_party_overrides.push(override_entry);
        self
    }

    pub fn required_profiles(&self) -> &[RequiredMemoryProfileId] {
        &self.required_profiles
    }
}

/// Resolved extension IDs for required memory profiles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedMemoryProfileBindings {
    bindings: BTreeMap<RequiredMemoryProfileId, ExtensionId>,
}

impl ResolvedMemoryProfileBindings {
    pub fn extension_for(&self, profile_id: RequiredMemoryProfileId) -> Option<&ExtensionId> {
        self.bindings.get(&profile_id)
    }

    pub fn bindings(&self) -> &BTreeMap<RequiredMemoryProfileId, ExtensionId> {
        &self.bindings
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MemoryProfileBindingError {
    #[error("unknown required memory profile `{profile_id}`")]
    UnknownRequiredProfile { profile_id: CapabilityProfileId },

    #[error(
        "required memory profile `{profile_id}` cannot resolve because native memory is unavailable"
    )]
    NativeUnavailable { profile_id: RequiredMemoryProfileId },

    #[error(
        "required memory profile `{profile_id}` is bound to memory.disabled in production deployment `{deployment}`"
    )]
    DisabledInProduction {
        profile_id: RequiredMemoryProfileId,
        deployment: MemoryProfileBindingDeployment,
    },

    #[error(
        "required memory profile `{profile_id}` is bound to third-party extension `{extension_id}` in production deployment `{deployment}` without an exact override"
    )]
    ThirdPartyBindingRequiresOverride {
        profile_id: RequiredMemoryProfileId,
        extension_id: ExtensionId,
        deployment: MemoryProfileBindingDeployment,
    },
}

pub fn resolve_memory_profile_bindings(
    config: &MemoryProfileBindingConfig,
    runtime_policy: &EffectiveRuntimePolicy,
    native_available: bool,
) -> Result<ResolvedMemoryProfileBindings, MemoryProfileBindingError> {
    let deployment = MemoryProfileBindingDeployment::from_policy(runtime_policy);
    let mut resolved = BTreeMap::new();

    for profile_id in config.required_profiles() {
        let target = match config.explicit_bindings.get(profile_id) {
            Some(target) => target.clone(),
            None if native_available => MemoryProfileBindingTarget::native(),
            None => {
                return Err(MemoryProfileBindingError::NativeUnavailable {
                    profile_id: *profile_id,
                });
            }
        };

        if target.is_native() && !native_available {
            return Err(MemoryProfileBindingError::NativeUnavailable {
                profile_id: *profile_id,
            });
        }
        validate_target(config, *profile_id, &target, deployment)?;
        resolved.insert(*profile_id, target.extension_id().clone());
    }

    Ok(ResolvedMemoryProfileBindings { bindings: resolved })
}

fn validate_target(
    config: &MemoryProfileBindingConfig,
    profile_id: RequiredMemoryProfileId,
    target: &MemoryProfileBindingTarget,
    deployment: MemoryProfileBindingDeployment,
) -> Result<(), MemoryProfileBindingError> {
    if target.is_disabled() {
        if deployment.is_production() {
            return Err(MemoryProfileBindingError::DisabledInProduction {
                profile_id,
                deployment,
            });
        }
        return Ok(());
    }

    if target.is_native() || !deployment.is_production() {
        return Ok(());
    }

    if config
        .third_party_overrides
        .iter()
        .any(|override_entry| override_entry.matches(profile_id, target.extension_id(), deployment))
    {
        return Ok(());
    }

    Err(
        MemoryProfileBindingError::ThirdPartyBindingRequiresOverride {
            profile_id,
            extension_id: target.extension_id().clone(),
            deployment,
        },
    )
}
