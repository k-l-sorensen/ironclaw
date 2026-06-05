use std::{path::Path, sync::Arc};

use ironclaw_auth::{AuthProviderClient, UnavailableAuthProviderClient};
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
#[cfg(not(feature = "libsql"))]
use ironclaw_host_api::SYSTEM_RESERVED_ID;
#[cfg(not(feature = "libsql"))]
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use ironclaw_secrets::{FilesystemSecretStore, SecretStore};

use crate::auth::RebornAuthContinuationDispatcher;
use crate::factory::LocalDevRootFilesystem;
use crate::product_auth_providers::OAuthProviderComposition;
use crate::{RebornBuildError, RebornProductAuthServicePorts, RebornProductAuthServices};
use ironclaw_auth_storage::FilesystemAuthProductServices;

#[cfg(feature = "libsql")]
const LOCAL_DEV_SECRETS_MASTER_KEY_PATH: &str = ".reborn-local-dev-secrets-master-key";

pub(crate) struct LocalDevProductAuthSubstrate {
    pub(crate) secret_store: Arc<dyn SecretStore>,
    filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
}

pub(crate) fn build_local_dev_product_auth_substrate(
    root: &Path,
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<LocalDevProductAuthSubstrate, RebornBuildError> {
    let filesystem = local_dev_product_auth_scoped_filesystem(filesystem)?;
    let secret_store = build_local_dev_secret_store(root, Arc::clone(&filesystem))?;
    Ok(LocalDevProductAuthSubstrate {
        filesystem,
        secret_store,
    })
}

pub(crate) fn compose_local_dev_default_product_auth_services(
    substrate: LocalDevProductAuthSubstrate,
    dispatcher: Arc<dyn RebornAuthContinuationDispatcher>,
    provider_composition: OAuthProviderComposition,
) -> Arc<RebornProductAuthServices> {
    let durable_services = Arc::new(FilesystemAuthProductServices::new(
        substrate.filesystem,
        Arc::clone(&substrate.secret_store),
    ));
    let provider_client: Arc<dyn AuthProviderClient> = provider_composition
        .client
        .clone()
        .unwrap_or_else(|| Arc::new(UnavailableAuthProviderClient));
    let mut services = RebornProductAuthServicePorts::from_shared_ports_with_provider(
        Arc::clone(&durable_services),
        provider_client,
    )
    .into_services(dispatcher)
    .with_flow_record_source(durable_services);
    if let Some(registry) = provider_composition.dcr_registry {
        services = services.with_dcr_oauth_registry(registry);
    }
    if let Some(registry) = provider_composition.gate_registry {
        services = services.with_oauth_gate_registry(registry);
    }
    Arc::new(services)
}

pub(crate) fn build_local_dev_secret_store<F>(
    root: &Path,
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
) -> Result<Arc<FilesystemSecretStore<F>>, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let master_key = resolve_local_dev_secret_master_key(root)?;
    let crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(master_key)?);
    Ok(Arc::new(FilesystemSecretStore::new(
        scoped_filesystem,
        crypto,
    )))
}

fn resolve_local_dev_secret_master_key(
    root: &Path,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    #[cfg(not(feature = "libsql"))]
    {
        let _ = root;
        return Ok(ironclaw_secrets::SecretMaterial::from(
            ironclaw_secrets::keychain::generate_master_key_hex(),
        ));
    }

    #[cfg(feature = "libsql")]
    {
        let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
        match std::fs::read_to_string(&key_path) {
            Ok(existing) => {
                return Ok(ironclaw_secrets::SecretMaterial::from(
                    existing.trim().to_string(),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!("local-dev secrets master key could not be read: {error}"),
                });
            }
        }

        let key = std::env::var(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(ironclaw_secrets::keychain::generate_master_key_hex);
        write_local_dev_secret_master_key(&key_path, &key)?;
        Ok(ironclaw_secrets::SecretMaterial::from(key))
    }
}

#[cfg(feature = "libsql")]
fn write_local_dev_secret_master_key(path: &Path, key: &str) -> Result<(), RebornBuildError> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be created: {error}"),
            })?;
        file.write_all(key.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be written: {error}"),
            })
    }
    #[cfg(windows)]
    {
        use std::io::Write as _;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be created: {error}"),
            })?;
        let account = std::env::var("USERDOMAIN")
            .ok()
            .filter(|domain| !domain.trim().is_empty())
            .zip(
                std::env::var("USERNAME")
                    .ok()
                    .filter(|user| !user.trim().is_empty()),
            )
            .map(|(domain, user)| format!("{domain}\\{user}"))
            .or_else(|| std::env::var("USERNAME").ok())
            .ok_or_else(|| RebornBuildError::InvalidConfig {
                reason: "local-dev secrets master key could not be restricted: USERNAME is unset"
                    .to_string(),
            })?;
        let status = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{account}:F"))
            .status()
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev secrets master key permissions could not be set: {error}"
                ),
            })?;
        if !status.success() {
            let _ = std::fs::remove_file(path);
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev secrets master key permissions could not be set: icacls exited with {status}"
                ),
            });
        }
        file.write_all(key.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be written: {error}"),
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        let _ = key;
        Err(RebornBuildError::InvalidConfig {
            reason:
                "local-dev filesystem secret persistence requires Unix permissions or Windows ACLs"
                    .to_string(),
        })
    }
}

#[cfg(feature = "libsql")]
pub(crate) fn local_dev_product_auth_scoped_filesystem(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<ScopedFilesystem<LocalDevRootFilesystem>>, RebornBuildError> {
    Ok(crate::wrap_scoped(filesystem))
}

#[cfg(not(feature = "libsql"))]
pub(crate) fn local_dev_product_auth_scoped_filesystem(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<ScopedFilesystem<LocalDevRootFilesystem>>, RebornBuildError> {
    Ok(Arc::new(ScopedFilesystem::new(filesystem, |scope| {
        let tenant_id = local_dev_scope_path_segment(scope.tenant_id.as_str());
        let user_id = local_dev_scope_path_segment(scope.user_id.as_str());
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/secrets")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/users/{user_id}/secrets"))?,
            MountPermissions::read_write_list_delete(),
        )])
    })))
}

#[cfg(not(feature = "libsql"))]
fn local_dev_scope_path_segment(value: &str) -> &str {
    if value == SYSTEM_RESERVED_ID {
        "__system__"
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::{
        BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind,
        InMemoryBackend, IndexPolicy, MountDescriptor, StorageClass,
    };
    use ironclaw_host_api::{
        AgentId, InvocationId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
        VirtualPath,
    };
    use secrecy::ExposeSecret;

    #[tokio::test]
    async fn local_dev_product_auth_secret_store_isolates_tenants() {
        let dir = tempfile::tempdir().expect("tempdir");
        let filesystem = local_dev_test_root_filesystem();
        let secret_store = build_local_dev_secret_store(
            dir.path(),
            local_dev_product_auth_scoped_filesystem(filesystem)
                .expect("local-dev product-auth filesystem"),
        )
        .expect("local-dev secret store");
        let scope_a = local_dev_secret_scope("tenant-a", "alice");
        let scope_b = local_dev_secret_scope("tenant-b", "alice");
        let handle = SecretHandle::new("shared-runtime-token").expect("secret handle");

        secret_store
            .put(
                scope_a.clone(),
                handle.clone(),
                ironclaw_secrets::SecretMaterial::from("tenant-a-token"),
            )
            .await
            .expect("put tenant-a secret");
        secret_store
            .put(
                scope_b.clone(),
                handle.clone(),
                ironclaw_secrets::SecretMaterial::from("tenant-b-token"),
            )
            .await
            .expect("put tenant-b secret");

        let lease_a = secret_store
            .lease_once(&scope_a, &handle)
            .await
            .expect("tenant-a lease");
        let material_a = secret_store
            .consume(&scope_a, lease_a.id)
            .await
            .expect("tenant-a consume");
        let lease_b = secret_store
            .lease_once(&scope_b, &handle)
            .await
            .expect("tenant-b lease");
        let material_b = secret_store
            .consume(&scope_b, lease_b.id)
            .await
            .expect("tenant-b consume");

        assert_eq!(material_a.expose_secret(), "tenant-a-token");
        assert_eq!(material_b.expose_secret(), "tenant-b-token");
    }

    fn local_dev_test_root_filesystem() -> Arc<LocalDevRootFilesystem> {
        let backend = Arc::new(InMemoryBackend::new());
        let mut filesystem = CompositeRootFilesystem::new();
        filesystem
            .mount(
                MountDescriptor {
                    virtual_root: VirtualPath::new("/tenants").expect("virtual root"),
                    backend_id: BackendId::new("local-dev-test-tenants").expect("backend id"),
                    backend_kind: BackendKind::MemoryDocuments,
                    storage_class: StorageClass::StructuredRecords,
                    content_kind: ContentKind::StructuredRecord,
                    index_policy: IndexPolicy::NotIndexed,
                    capabilities: BackendCapabilities::bytes_only(),
                },
                backend,
            )
            .expect("tenant mount");
        Arc::new(filesystem)
    }

    fn local_dev_secret_scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).expect("tenant id"),
            user_id: UserId::new(user).expect("user id"),
            agent_id: Some(AgentId::new("agent").expect("agent id")),
            project_id: Some(ProjectId::new("project").expect("project id")),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }
}
