use ironclaw_extensions::{
    ExtensionManifestV2, MANIFEST_SCHEMA_VERSION, ManifestSource, ManifestV2Error,
};
use ironclaw_host_api::{HostPortCatalog, HostPortCatalogEntry, HostPortId};
use ironclaw_product_adapters::{AuthRequirement, ProductCapabilityFlag, ProductSurfaceKind};

fn catalog() -> HostPortCatalog {
    HostPortCatalog::new(vec![HostPortCatalogEntry::new(
        HostPortId::new("host.events.audit").unwrap(),
    )])
    .unwrap()
}

fn telegram_manifest(extra: &str) -> String {
    format!(
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
required_host_ports = ["host.events.audit"]

[product_adapter]
surface_kind = "external_channel"

[product_adapter.auth]
kind = "shared_secret_header"
header_name = "X-Telegram-Bot-Api-Secret-Token"

[product_adapter.capabilities]
flags = ["inbound_messages", "external_final_reply_push"]

[[product_adapter.required_credentials]]
handle = "telegram_bot_token"

[[product_adapter.egress]]
host = "api.telegram.org"
credential_handle = "telegram_bot_token"

{extra}
"#,
        schema = MANIFEST_SCHEMA_VERSION,
    )
}

#[test]
fn parses_product_adapter_block_inside_extension_manifest_v2() {
    let manifest = ExtensionManifestV2::parse(
        &telegram_manifest(""),
        ManifestSource::InstalledLocal,
        &catalog(),
    )
    .unwrap();

    let product_adapter = manifest.product_adapter.as_ref().unwrap();
    assert_eq!(product_adapter.adapter_id().as_str(), "telegram-v2");
    assert_eq!(
        product_adapter.surface_kind(),
        ProductSurfaceKind::ExternalChannel
    );
    assert!(matches!(
        product_adapter.auth_requirement(),
        AuthRequirement::SharedSecretHeader { header_name }
            if header_name == "X-Telegram-Bot-Api-Secret-Token"
    ));
    assert!(
        product_adapter
            .capabilities()
            .contains(ProductCapabilityFlag::InboundMessages)
    );
    assert_eq!(product_adapter.required_credentials().len(), 1);
    assert_eq!(product_adapter.declared_egress().len(), 1);
}

#[test]
fn rejects_egress_credential_not_declared_as_required() {
    let raw = telegram_manifest(
        r#"
[[product_adapter.egress]]
host = "api.example.com"
credential_handle = "undeclared_token"
"#,
    );

    let err =
        ExtensionManifestV2::parse(&raw, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(matches!(
        err,
        ManifestV2Error::UndeclaredProductAdapterEgressCredential { .. }
            | ManifestV2Error::DuplicateProductAdapterEgressTarget
    ));
}

#[test]
fn rejects_inline_secret_material_in_extension_manifest_v2() {
    let raw = telegram_manifest(
        r#"
[[product_adapter.required_credentials]]
handle = "other_token"
secret_value = "123456789:AABBccDDeeFFgg"
"#,
    );

    let err =
        ExtensionManifestV2::parse(&raw, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(matches!(err, ManifestV2Error::InlineSecretMaterial { .. }));
}

#[test]
fn rejects_auth_header_injection_shape() {
    let raw = telegram_manifest("").replace(
        "header_name = \"X-Telegram-Bot-Api-Secret-Token\"",
        "header_name = \"X-Foo\\r\\nInjected: x\"",
    );

    let err =
        ExtensionManifestV2::parse(&raw, ManifestSource::InstalledLocal, &catalog()).unwrap_err();
    assert!(matches!(
        err,
        ManifestV2Error::InvalidProductAdapterField { field, .. }
            if field == "product_adapter.auth.header_name"
    ));
}
