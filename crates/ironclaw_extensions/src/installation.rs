//! Generic extension installation registry state.
//!
//! This module owns activation state, manifest revision pins, credential
//! bindings, and health snapshots for Extension Manifest v2 packages. It does
//! not load runtimes, route webhooks, perform egress, or resolve raw secrets.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{ExtensionId, HostApiError, SecretHandle};
use ironclaw_product_adapters::{EgressCredentialHandle, RedactedString};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use tokio::sync::RwLock;

use crate::v2::ExtensionManifestV2;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ExtensionInstallationId(String);

impl ExtensionInstallationId {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionRegistryError> {
        let value = value.into();
        validate_identifier("installation_id", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ExtensionInstallationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for ExtensionInstallationId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ManifestHash(String);

impl ManifestHash {
    pub fn new(value: impl Into<String>) -> Result<Self, ExtensionRegistryError> {
        let value = value.into();
        validate_identifier("manifest_hash", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ManifestHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionManifestRef {
    extension_id: ExtensionId,
    manifest_hash: Option<ManifestHash>,
}

impl ExtensionManifestRef {
    pub fn new(extension_id: ExtensionId, manifest_hash: Option<ManifestHash>) -> Self {
        Self {
            extension_id,
            manifest_hash,
        }
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub fn manifest_hash(&self) -> Option<&ManifestHash> {
        self.manifest_hash.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManifestRecord {
    manifest: ExtensionManifestV2,
    manifest_hash: Option<ManifestHash>,
}

impl ExtensionManifestRecord {
    pub fn new(manifest: ExtensionManifestV2, manifest_hash: Option<ManifestHash>) -> Self {
        Self {
            manifest,
            manifest_hash,
        }
    }

    pub fn manifest(&self) -> &ExtensionManifestV2 {
        &self.manifest
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.manifest.id
    }

    pub fn manifest_hash(&self) -> Option<&ManifestHash> {
        self.manifest_hash.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionActivationState {
    Installed,
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionHealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionHealthSnapshot {
    status: ExtensionHealthStatus,
    message: Option<RedactedString>,
    checked_at: DateTime<Utc>,
}

impl ExtensionHealthSnapshot {
    pub fn healthy() -> Self {
        Self {
            status: ExtensionHealthStatus::Healthy,
            message: None,
            checked_at: Utc::now(),
        }
    }

    pub fn new(
        status: ExtensionHealthStatus,
        message: Option<RedactedString>,
        checked_at: DateTime<Utc>,
    ) -> Self {
        Self {
            status,
            message,
            checked_at,
        }
    }

    pub fn status(&self) -> ExtensionHealthStatus {
        self.status
    }

    pub fn message(&self) -> Option<&RedactedString> {
        self.message.as_ref()
    }

    pub fn checked_at(&self) -> DateTime<Utc> {
        self.checked_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtensionCredentialBinding {
    credential_handle: EgressCredentialHandle,
    secret_handle: SecretHandle,
}

impl ExtensionCredentialBinding {
    pub fn new(credential_handle: EgressCredentialHandle, secret_handle: SecretHandle) -> Self {
        Self {
            credential_handle,
            secret_handle,
        }
    }

    pub fn credential_handle(&self) -> &EgressCredentialHandle {
        &self.credential_handle
    }

    pub fn secret_handle(&self) -> &SecretHandle {
        &self.secret_handle
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExtensionInstallation {
    installation_id: ExtensionInstallationId,
    extension_id: ExtensionId,
    activation_state: ExtensionActivationState,
    manifest_ref: ExtensionManifestRef,
    credential_bindings: Vec<ExtensionCredentialBinding>,
    health: ExtensionHealthSnapshot,
    updated_at: DateTime<Utc>,
}

impl ExtensionInstallation {
    pub fn new(
        installation_id: ExtensionInstallationId,
        extension_id: ExtensionId,
        activation_state: ExtensionActivationState,
        manifest_ref: ExtensionManifestRef,
        credential_bindings: Vec<ExtensionCredentialBinding>,
        updated_at: DateTime<Utc>,
    ) -> Result<Self, ExtensionRegistryError> {
        if manifest_ref.extension_id() != &extension_id {
            return Err(ExtensionRegistryError::ManifestExtensionMismatch {
                extension_id,
                manifest_extension_id: manifest_ref.extension_id().clone(),
            });
        }
        validate_bindings_unique(&credential_bindings)?;
        Ok(Self {
            installation_id,
            extension_id,
            activation_state,
            manifest_ref,
            credential_bindings,
            health: ExtensionHealthSnapshot::healthy(),
            updated_at,
        })
    }

    pub fn installation_id(&self) -> &ExtensionInstallationId {
        &self.installation_id
    }

    pub fn extension_id(&self) -> &ExtensionId {
        &self.extension_id
    }

    pub fn activation_state(&self) -> ExtensionActivationState {
        self.activation_state
    }

    pub fn manifest_ref(&self) -> &ExtensionManifestRef {
        &self.manifest_ref
    }

    pub fn credential_bindings(&self) -> &[ExtensionCredentialBinding] {
        &self.credential_bindings
    }

    pub fn health(&self) -> &ExtensionHealthSnapshot {
        &self.health
    }

    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    fn set_activation_state(&mut self, state: ExtensionActivationState) {
        self.activation_state = state;
        self.updated_at = Utc::now();
    }

    fn set_health(&mut self, health: ExtensionHealthSnapshot) {
        self.health = health;
        self.updated_at = Utc::now();
    }
}

impl<'de> Deserialize<'de> for ExtensionInstallation {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Wire {
            installation_id: ExtensionInstallationId,
            extension_id: ExtensionId,
            activation_state: ExtensionActivationState,
            manifest_ref: ExtensionManifestRef,
            credential_bindings: Vec<ExtensionCredentialBinding>,
            health: ExtensionHealthSnapshot,
            updated_at: DateTime<Utc>,
        }

        let wire = Wire::deserialize(deserializer)?;
        if wire.manifest_ref.extension_id() != &wire.extension_id {
            return Err(serde::de::Error::custom(
                ExtensionRegistryError::ManifestExtensionMismatch {
                    extension_id: wire.extension_id,
                    manifest_extension_id: wire.manifest_ref.extension_id().clone(),
                },
            ));
        }
        validate_bindings_unique(&wire.credential_bindings).map_err(serde::de::Error::custom)?;
        Ok(Self {
            installation_id: wire.installation_id,
            extension_id: wire.extension_id,
            activation_state: wire.activation_state,
            manifest_ref: wire.manifest_ref,
            credential_bindings: wire.credential_bindings,
            health: wire.health,
            updated_at: wire.updated_at,
        })
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ExtensionRegistryError {
    #[error(transparent)]
    Contract(#[from] HostApiError),
    #[error("invalid {field}: {reason}")]
    InvalidValue { field: &'static str, reason: String },
    #[error("duplicate credential binding {handle}")]
    DuplicateCredentialBinding { handle: EgressCredentialHandle },
    #[error("installation references unknown extension manifest {extension_id}")]
    UnknownManifest { extension_id: ExtensionId },
    #[error("installation binds undeclared credential handle {handle}")]
    UndeclaredCredentialHandle { handle: EgressCredentialHandle },
    #[error(
        "installation extension {extension_id} does not match manifest extension {manifest_extension_id}"
    )]
    ManifestExtensionMismatch {
        extension_id: ExtensionId,
        manifest_extension_id: ExtensionId,
    },
    #[error(
        "installation manifest hash does not match registered manifest hash for {extension_id}"
    )]
    ManifestHashMismatch { extension_id: ExtensionId },
    #[error("installation {installation_id} was not found")]
    InstallationNotFound {
        installation_id: ExtensionInstallationId,
    },
}

#[async_trait]
pub trait ExtensionRegistryStore: Send + Sync {
    async fn list_manifests(&self) -> Result<Vec<ExtensionManifestRecord>, ExtensionRegistryError>;

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionRegistryError>;

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionRegistryError>;

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionRegistryError>;

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionRegistryError>;

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionRegistryError>;

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionRegistryError>;

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionRegistryError>;

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionRegistryError>;
}

#[async_trait]
impl<T> ExtensionRegistryStore for Arc<T>
where
    T: ExtensionRegistryStore + ?Sized,
{
    async fn list_manifests(&self) -> Result<Vec<ExtensionManifestRecord>, ExtensionRegistryError> {
        (**self).list_manifests().await
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionRegistryError> {
        (**self).get_manifest(extension_id).await
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionRegistryError> {
        (**self).upsert_manifest(manifest).await
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionRegistryError> {
        (**self).list_installations().await
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionRegistryError> {
        (**self).list_enabled_installations().await
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionRegistryError> {
        (**self).get_installation(installation_id).await
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionRegistryError> {
        (**self).upsert_installation(installation).await
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionRegistryError> {
        (**self).set_activation_state(installation_id, state).await
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionRegistryError> {
        (**self).update_health(installation_id, health).await
    }
}

#[derive(Debug, Default, Clone)]
pub struct InMemoryExtensionRegistryStore {
    inner: Arc<RwLock<InMemoryRegistryState>>,
}

#[derive(Debug, Default)]
struct InMemoryRegistryState {
    manifests: HashMap<ExtensionId, ExtensionManifestRecord>,
    installations: HashMap<ExtensionInstallationId, ExtensionInstallation>,
}

#[async_trait]
impl ExtensionRegistryStore for InMemoryExtensionRegistryStore {
    async fn list_manifests(&self) -> Result<Vec<ExtensionManifestRecord>, ExtensionRegistryError> {
        let inner = self.inner.read().await;
        let mut manifests: Vec<_> = inner.manifests.values().cloned().collect();
        manifests.sort_by(|a, b| a.extension_id().cmp(b.extension_id()));
        Ok(manifests)
    }

    async fn get_manifest(
        &self,
        extension_id: &ExtensionId,
    ) -> Result<Option<ExtensionManifestRecord>, ExtensionRegistryError> {
        Ok(self.inner.read().await.manifests.get(extension_id).cloned())
    }

    async fn upsert_manifest(
        &self,
        manifest: ExtensionManifestRecord,
    ) -> Result<(), ExtensionRegistryError> {
        let mut inner = self.inner.write().await;
        for installation in inner.installations.values() {
            if installation.extension_id() == manifest.extension_id() {
                validate_installation_against_one_manifest(&manifest, installation)?;
            }
        }
        inner
            .manifests
            .insert(manifest.extension_id().clone(), manifest);
        Ok(())
    }

    async fn list_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionRegistryError> {
        let inner = self.inner.read().await;
        let mut installations: Vec<_> = inner.installations.values().cloned().collect();
        installations.sort_by(|a, b| a.installation_id().cmp(b.installation_id()));
        Ok(installations)
    }

    async fn list_enabled_installations(
        &self,
    ) -> Result<Vec<ExtensionInstallation>, ExtensionRegistryError> {
        Ok(self
            .list_installations()
            .await?
            .into_iter()
            .filter(|installation| {
                installation.activation_state() == ExtensionActivationState::Enabled
            })
            .collect())
    }

    async fn get_installation(
        &self,
        installation_id: &ExtensionInstallationId,
    ) -> Result<Option<ExtensionInstallation>, ExtensionRegistryError> {
        Ok(self
            .inner
            .read()
            .await
            .installations
            .get(installation_id)
            .cloned())
    }

    async fn upsert_installation(
        &self,
        installation: ExtensionInstallation,
    ) -> Result<(), ExtensionRegistryError> {
        validate_bindings_unique(installation.credential_bindings())?;
        let mut inner = self.inner.write().await;
        validate_installation_against_manifest(&inner.manifests, &installation)?;
        inner
            .installations
            .insert(installation.installation_id().clone(), installation);
        Ok(())
    }

    async fn set_activation_state(
        &self,
        installation_id: &ExtensionInstallationId,
        state: ExtensionActivationState,
    ) -> Result<(), ExtensionRegistryError> {
        let mut inner = self.inner.write().await;
        let manifests = &inner.manifests;
        let installation = inner.installations.get(installation_id).ok_or_else(|| {
            ExtensionRegistryError::InstallationNotFound {
                installation_id: installation_id.clone(),
            }
        })?;
        if installation.activation_state() == state {
            return Ok(());
        }
        if state == ExtensionActivationState::Enabled {
            validate_installation_against_manifest(manifests, installation)?;
        }
        let installation = inner
            .installations
            .get_mut(installation_id)
            .ok_or_else(|| ExtensionRegistryError::InstallationNotFound {
                installation_id: installation_id.clone(),
            })?;
        installation.set_activation_state(state);
        Ok(())
    }

    async fn update_health(
        &self,
        installation_id: &ExtensionInstallationId,
        health: ExtensionHealthSnapshot,
    ) -> Result<(), ExtensionRegistryError> {
        let mut inner = self.inner.write().await;
        let installation = inner
            .installations
            .get_mut(installation_id)
            .ok_or_else(|| ExtensionRegistryError::InstallationNotFound {
                installation_id: installation_id.clone(),
            })?;
        installation.set_health(health);
        Ok(())
    }
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ExtensionRegistryError> {
    if value.is_empty() {
        return Err(ExtensionRegistryError::InvalidValue {
            field,
            reason: "must not be empty".to_string(),
        });
    }
    if value.chars().any(|c| c == '\0' || c.is_control()) {
        return Err(ExtensionRegistryError::InvalidValue {
            field,
            reason: "must not contain control characters".to_string(),
        });
    }
    Ok(())
}

fn validate_bindings_unique(
    credential_bindings: &[ExtensionCredentialBinding],
) -> Result<(), ExtensionRegistryError> {
    let mut seen = BTreeSet::new();
    for binding in credential_bindings {
        if !seen.insert(binding.credential_handle.clone()) {
            return Err(ExtensionRegistryError::DuplicateCredentialBinding {
                handle: binding.credential_handle.clone(),
            });
        }
    }
    Ok(())
}

fn validate_installation_against_manifest(
    manifests: &HashMap<ExtensionId, ExtensionManifestRecord>,
    installation: &ExtensionInstallation,
) -> Result<(), ExtensionRegistryError> {
    let manifest = manifests.get(installation.extension_id()).ok_or_else(|| {
        ExtensionRegistryError::UnknownManifest {
            extension_id: installation.extension_id().clone(),
        }
    })?;
    validate_installation_against_one_manifest(manifest, installation)
}

fn validate_installation_against_one_manifest(
    manifest: &ExtensionManifestRecord,
    installation: &ExtensionInstallation,
) -> Result<(), ExtensionRegistryError> {
    if manifest.extension_id() != installation.manifest_ref().extension_id() {
        return Err(ExtensionRegistryError::ManifestExtensionMismatch {
            extension_id: installation.extension_id().clone(),
            manifest_extension_id: installation.manifest_ref().extension_id().clone(),
        });
    }

    match (
        manifest.manifest_hash(),
        installation.manifest_ref().manifest_hash(),
    ) {
        (Some(registered), Some(referenced)) if registered != referenced => {
            return Err(ExtensionRegistryError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            });
        }
        (Some(_), None) | (None, Some(_)) => {
            return Err(ExtensionRegistryError::ManifestHashMismatch {
                extension_id: installation.extension_id().clone(),
            });
        }
        _ => {}
    }

    let declared: BTreeSet<_> = manifest
        .manifest()
        .product_adapter
        .as_ref()
        .map(|decl| decl.required_credentials().iter().cloned().collect())
        .unwrap_or_default();
    for binding in installation.credential_bindings() {
        if !declared.contains(binding.credential_handle()) {
            return Err(ExtensionRegistryError::UndeclaredCredentialHandle {
                handle: binding.credential_handle().clone(),
            });
        }
    }
    Ok(())
}
