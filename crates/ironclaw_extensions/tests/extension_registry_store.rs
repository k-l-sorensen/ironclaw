use chrono::Utc;
use ironclaw_extensions::{
    ExtensionActivationState, ExtensionCredentialBinding, ExtensionInstallation,
    ExtensionInstallationId, ExtensionManifestRecord, ExtensionManifestRef, ExtensionManifestV2,
    ExtensionRegistryError, ExtensionRegistryStore, InMemoryExtensionRegistryStore,
    MANIFEST_SCHEMA_VERSION, ManifestHash, ManifestSource,
};
use ironclaw_host_api::{ExtensionId, HostPortCatalog};
use ironclaw_product_adapters::EgressCredentialHandle;

fn manifest_hash(value: &str) -> ManifestHash {
    ManifestHash::new(value).unwrap()
}

fn extension_id() -> ExtensionId {
    ExtensionId::new("telegram-v2").unwrap()
}

fn installation_id() -> ExtensionInstallationId {
    ExtensionInstallationId::new("acme-telegram-prod").unwrap()
}

fn credential(value: &str) -> EgressCredentialHandle {
    EgressCredentialHandle::new(value).unwrap()
}

fn manifest(required_credential: &str) -> ExtensionManifestV2 {
    let raw = format!(
        r#"
schema_version = "{schema}"
id = "telegram-v2"
name = "Telegram"
version = "0.1.0"
description = "Telegram product adapter"
trust = "third_party"

[runtime]
kind = "wasm"
module = "adapters/telegram-v2.wasm"

[[capabilities]]
id = "telegram-v2.parse"
description = "Parses Telegram inbound payloads"
default_permission = "allow"
visibility = "api"
input_schema_ref = "schemas/telegram/input.json"
output_schema_ref = "schemas/telegram/output.json"

[product_adapter]
surface_kind = "external_channel"

[product_adapter.auth]
kind = "bearer_token"

[product_adapter.capabilities]
flags = ["inbound_messages"]

[[product_adapter.required_credentials]]
handle = "{required_credential}"
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    );
    ExtensionManifestV2::parse(
        &raw,
        ManifestSource::InstalledLocal,
        &HostPortCatalog::empty(),
    )
    .unwrap()
}

fn record(required_credential: &str, hash: &str) -> ExtensionManifestRecord {
    ExtensionManifestRecord::new(manifest(required_credential), Some(manifest_hash(hash)))
}

fn installation(state: ExtensionActivationState) -> ExtensionInstallation {
    ExtensionInstallation::new(
        installation_id(),
        extension_id(),
        state,
        ExtensionManifestRef::new(extension_id(), Some(manifest_hash("sha256:abc123"))),
        vec![ExtensionCredentialBinding::new(
            credential("telegram_bot_token"),
            ironclaw_host_api::SecretHandle::new("secret_telegram_bot_token").unwrap(),
        )],
        Utc::now(),
    )
    .unwrap()
}

#[tokio::test]
async fn default_registry_has_no_enabled_installations() {
    let store = InMemoryExtensionRegistryStore::default();

    assert!(store.list_manifests().await.unwrap().is_empty());
    assert!(store.list_enabled_installations().await.unwrap().is_empty());
}

#[tokio::test]
async fn explicit_activation_makes_installation_enabled() {
    let store = InMemoryExtensionRegistryStore::default();
    store
        .upsert_manifest(record("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    store
        .upsert_installation(installation(ExtensionActivationState::Installed))
        .await
        .unwrap();

    store
        .set_activation_state(&installation_id(), ExtensionActivationState::Enabled)
        .await
        .unwrap();

    let enabled = store.list_enabled_installations().await.unwrap();
    assert_eq!(enabled.len(), 1);
    assert_eq!(enabled[0].installation_id(), &installation_id());
}

#[tokio::test]
async fn credential_binding_must_reference_declared_manifest_handle() {
    let store = InMemoryExtensionRegistryStore::default();
    store
        .upsert_manifest(record("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    let invalid = ExtensionInstallation::new(
        installation_id(),
        extension_id(),
        ExtensionActivationState::Installed,
        ExtensionManifestRef::new(extension_id(), Some(manifest_hash("sha256:abc123"))),
        vec![ExtensionCredentialBinding::new(
            credential("slack_bot_token"),
            ironclaw_host_api::SecretHandle::new("secret_slack_bot_token").unwrap(),
        )],
        Utc::now(),
    )
    .unwrap();

    let err = store.upsert_installation(invalid).await.unwrap_err();
    assert!(matches!(
        err,
        ExtensionRegistryError::UndeclaredCredentialHandle { .. }
    ));
}

#[tokio::test]
async fn manifest_hash_mismatch_is_rejected() {
    let store = InMemoryExtensionRegistryStore::default();
    store
        .upsert_manifest(record("telegram_bot_token", "sha256:different"))
        .await
        .unwrap();

    let err = store
        .upsert_installation(installation(ExtensionActivationState::Installed))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ExtensionRegistryError::ManifestHashMismatch { .. }
    ));
}

#[tokio::test]
async fn upsert_manifest_rejects_when_existing_installation_binding_revoked() {
    let store = InMemoryExtensionRegistryStore::default();
    store
        .upsert_manifest(record("telegram_bot_token", "sha256:abc123"))
        .await
        .unwrap();
    store
        .upsert_installation(installation(ExtensionActivationState::Enabled))
        .await
        .unwrap();

    let err = store
        .upsert_manifest(record("other_token", "sha256:abc123"))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ExtensionRegistryError::UndeclaredCredentialHandle { .. }
    ));
}
