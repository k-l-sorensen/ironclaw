//! Filesystem-backed durable secret storage.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_filesystem::{
    CasExpectation, Entry, FileType, FilesystemError, RecordKind, RootFilesystem, ScopedFilesystem,
    VersionedEntry,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ScopedPath, VirtualPath,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::crypto::{secret_record_aad, secret_store_key_check_aad};
use crate::legacy_store::{DecryptedSecret, Secret, SecretRef};
use crate::{CreateSecretParams, SecretConsumeResult, SecretError, SecretsCrypto, SecretsStore};

const SECRET_RECORD_KIND: &str = "secret_record";
const SECRET_KEY_CHECK_KIND: &str = "secret_store_key_check";
const SECRET_STORE_KEY_CHECK_ID: &str = "active";
const SECRET_STORE_KEY_CHECK_PLAINTEXT: &str = "reborn-secret-store-key-check-v1";

/// Durable [`SecretsStore`] implementation over the unified Reborn filesystem surface.
#[derive(Debug)]
pub struct FilesystemSecretsStore<F> {
    filesystem: ScopedFilesystem<F>,
    crypto: Arc<SecretsCrypto>,
}

impl<F> FilesystemSecretsStore<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: ScopedFilesystem<F>, crypto: Arc<SecretsCrypto>) -> Self {
        Self { filesystem, crypto }
    }

    pub fn over_root(root: Arc<F>, crypto: Arc<SecretsCrypto>) -> Result<Self, SecretError> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/secrets").map_err(secret_filesystem_error)?,
            VirtualPath::new("/secrets").map_err(secret_filesystem_error)?,
            MountPermissions {
                read: true,
                write: true,
                delete: true,
                list: true,
                execute: false,
            },
        )])
        .map_err(secret_filesystem_error)?;
        Ok(Self::new(ScopedFilesystem::new(root, mounts), crypto))
    }

    pub async fn verify_can_decrypt_existing_secrets(&self) -> Result<(), SecretError> {
        if let Some(check) = self.get_optional_entry(&key_check_path()?).await? {
            let record: StoredKeyCheck = parse_entry(check)?;
            return verify_secret_store_key_check(
                &self.crypto,
                &record.encrypted_value,
                &record.key_salt,
            );
        }

        self.verify_all_secret_payloads().await?;
        let record = build_key_check_record(&self.crypto)?;
        let entry = record_entry(SECRET_KEY_CHECK_KIND, &record)?;
        match self
            .filesystem
            .put(&key_check_path()?, entry, CasExpectation::Absent)
            .await
        {
            Ok(_) => {}
            Err(FilesystemError::VersionMismatch { .. }) => {}
            Err(error) => return Err(secret_filesystem_error(error)),
        }
        let Some(check) = self.get_optional_entry(&key_check_path()?).await? else {
            return Err(SecretError::Database(
                "secret store key check missing after bootstrap".to_string(),
            ));
        };
        let record: StoredKeyCheck = parse_entry(check)?;
        verify_secret_store_key_check(&self.crypto, &record.encrypted_value, &record.key_salt)
    }

    async fn get_optional_entry(
        &self,
        path: &ScopedPath,
    ) -> Result<Option<VersionedEntry>, SecretError> {
        match self.filesystem.get(path).await {
            Ok(entry) => Ok(entry),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(secret_filesystem_error(error)),
        }
    }

    async fn get_secret_entry(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<Option<VersionedEntry>, SecretError> {
        self.get_optional_entry(&record_path(user_id, name)?).await
    }

    async fn get_secret_record(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let name = normalize_secret_name(name);
        let Some(entry) = self.get_secret_entry(user_id, &name).await? else {
            return Err(SecretError::NotFound(name));
        };
        let secret: Secret = parse_stored_secret(entry)?;
        ensure_secret_not_expired(&secret)?;
        Ok(secret)
    }

    async fn verify_all_secret_payloads(&self) -> Result<(), SecretError> {
        for user_dir in self.list_dir_or_empty(&records_root_path()?).await? {
            if user_dir.file_type != FileType::Directory {
                continue;
            }
            let user_records_path = scoped_from_virtual(&user_dir.path)?;
            for record_entry in self.list_dir_or_empty(&user_records_path).await? {
                if record_entry.file_type != FileType::File {
                    continue;
                }
                let Some(versioned) = self
                    .get_optional_entry(&scoped_from_virtual(&record_entry.path)?)
                    .await?
                else {
                    continue;
                };
                let secret: Secret = parse_stored_secret(versioned)?;
                let aad = secret_record_aad(&secret.user_id, &secret.name);
                self.crypto
                    .decrypt(&secret.encrypted_value, &secret.key_salt, &aad)?;
            }
        }
        Ok(())
    }

    async fn list_dir_or_empty(
        &self,
        path: &ScopedPath,
    ) -> Result<Vec<ironclaw_filesystem::DirEntry>, SecretError> {
        match self.filesystem.list_dir(path).await {
            Ok(entries) => Ok(entries),
            Err(FilesystemError::NotFound { .. }) => Ok(Vec::new()),
            Err(error) => Err(secret_filesystem_error(error)),
        }
    }
}

#[async_trait]
impl<F> SecretsStore for FilesystemSecretsStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let secret = build_encrypted_secret(user_id, params, &self.crypto)?;
        let entry = record_entry(SECRET_RECORD_KIND, &StoredSecret::from(secret.clone()))?;
        self.filesystem
            .put(
                &record_path(user_id, &secret.name)?,
                entry,
                CasExpectation::Any,
            )
            .await
            .map_err(secret_filesystem_error)?;
        Ok(secret)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        self.get_secret_record(user_id, name).await
    }

    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        let aad = secret_record_aad(user_id, &secret.name);
        self.crypto
            .decrypt(&secret.encrypted_value, &secret.key_salt, &aad)
    }

    async fn consume_if_matches(
        &self,
        user_id: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<SecretConsumeResult, SecretError> {
        let name = normalize_secret_name(name);
        let Some(entry) = self.get_secret_entry(user_id, &name).await? else {
            return Ok(SecretConsumeResult::NotFound);
        };
        let secret: Secret = parse_stored_secret(entry)?;
        ensure_secret_not_expired(&secret)?;
        let aad = secret_record_aad(user_id, &secret.name);
        let decrypted = self
            .crypto
            .decrypt(&secret.encrypted_value, &secret.key_salt, &aad)?;
        if decrypted.expose() != expected_value {
            return Ok(SecretConsumeResult::Mismatched);
        }
        self.filesystem
            .delete(&record_path(user_id, &name)?)
            .await
            .map_err(secret_filesystem_error)?;
        Ok(SecretConsumeResult::Matched)
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        Ok(self
            .get_secret_entry(user_id, &normalize_secret_name(name))
            .await?
            .is_some())
    }

    async fn any_exist(&self) -> Result<bool, SecretError> {
        Ok(!self
            .list_dir_or_empty(&records_root_path()?)
            .await?
            .is_empty())
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let mut refs = Vec::new();
        for entry in self
            .list_dir_or_empty(&user_records_path(user_id)?)
            .await?
            .into_iter()
            .filter(|entry| entry.file_type == FileType::File)
        {
            let Some(versioned) = self
                .get_optional_entry(&scoped_from_virtual(&entry.path)?)
                .await?
            else {
                continue;
            };
            let secret: Secret = parse_stored_secret(versioned)?;
            ensure_secret_not_expired(&secret)?;
            refs.push(SecretRef {
                name: secret.name,
                provider: secret.provider,
            });
        }
        refs.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(refs)
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        match self.filesystem.delete(&record_path(user_id, name)?).await {
            Ok(()) => Ok(true),
            Err(FilesystemError::NotFound { .. }) => Ok(false),
            Err(error) => Err(secret_filesystem_error(error)),
        }
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        for user_dir in self.list_dir_or_empty(&records_root_path()?).await? {
            if user_dir.file_type != FileType::Directory {
                continue;
            }
            let user_records_path = scoped_from_virtual(&user_dir.path)?;
            for dir_entry in self.list_dir_or_empty(&user_records_path).await? {
                if dir_entry.file_type != FileType::File {
                    continue;
                }
                let record_path = scoped_from_virtual(&dir_entry.path)?;
                let Some(versioned) = self.get_optional_entry(&record_path).await? else {
                    continue;
                };
                let mut secret: Secret = parse_stored_secret(versioned.clone())?;
                if secret.id != secret_id {
                    continue;
                }
                secret.last_used_at = Some(Utc::now());
                secret.usage_count += 1;
                secret.updated_at = Utc::now();
                let entry = record_entry(SECRET_RECORD_KIND, &StoredSecret::from(secret))?;
                self.filesystem
                    .put(
                        &record_path,
                        entry,
                        CasExpectation::Version(versioned.version),
                    )
                    .await
                    .map_err(secret_filesystem_error)?;
                return Ok(());
            }
        }
        Err(SecretError::NotFound(secret_id.to_string()))
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        let secret_name_lower = normalize_secret_name(secret_name);
        if !self.exists(user_id, &secret_name_lower).await? {
            return Ok(false);
        }
        for pattern in allowed_secrets {
            let pattern_lower = pattern.to_lowercase();
            if pattern_lower == secret_name_lower {
                return Ok(true);
            }
            if let Some(prefix) = pattern_lower.strip_suffix('*')
                && secret_name_lower.starts_with(prefix)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSecret {
    id: Uuid,
    user_id: String,
    name: String,
    encrypted_value: Vec<u8>,
    key_salt: Vec<u8>,
    provider: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
    last_used_at: Option<chrono::DateTime<Utc>>,
    usage_count: i64,
    created_at: chrono::DateTime<Utc>,
    updated_at: chrono::DateTime<Utc>,
}

impl From<Secret> for StoredSecret {
    fn from(secret: Secret) -> Self {
        Self {
            id: secret.id,
            user_id: secret.user_id,
            name: secret.name,
            encrypted_value: secret.encrypted_value,
            key_salt: secret.key_salt,
            provider: secret.provider,
            expires_at: secret.expires_at,
            last_used_at: secret.last_used_at,
            usage_count: secret.usage_count,
            created_at: secret.created_at,
            updated_at: secret.updated_at,
        }
    }
}

impl From<StoredSecret> for Secret {
    fn from(record: StoredSecret) -> Self {
        Self {
            id: record.id,
            user_id: record.user_id,
            name: record.name,
            encrypted_value: record.encrypted_value,
            key_salt: record.key_salt,
            provider: record.provider,
            expires_at: record.expires_at,
            last_used_at: record.last_used_at,
            usage_count: record.usage_count,
            created_at: record.created_at,
            updated_at: record.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredKeyCheck {
    encrypted_value: Vec<u8>,
    key_salt: Vec<u8>,
}

fn build_encrypted_secret(
    user_id: &str,
    params: CreateSecretParams,
    crypto: &SecretsCrypto,
) -> Result<Secret, SecretError> {
    let name = normalize_secret_name(&params.name);
    let aad = secret_record_aad(user_id, &name);
    let (encrypted_value, key_salt) =
        crypto.encrypt(params.value.expose_secret().as_bytes(), &aad)?;
    let now = Utc::now();
    Ok(Secret {
        id: Uuid::new_v4(),
        user_id: user_id.to_string(),
        name,
        encrypted_value,
        key_salt,
        provider: params.provider,
        expires_at: params.expires_at,
        last_used_at: None,
        usage_count: 0,
        created_at: now,
        updated_at: now,
    })
}

fn build_key_check_record(crypto: &SecretsCrypto) -> Result<StoredKeyCheck, SecretError> {
    let aad = secret_store_key_check_aad();
    let (encrypted_value, key_salt) =
        crypto.encrypt(SECRET_STORE_KEY_CHECK_PLAINTEXT.as_bytes(), &aad)?;
    Ok(StoredKeyCheck {
        encrypted_value,
        key_salt,
    })
}

fn verify_secret_store_key_check(
    crypto: &SecretsCrypto,
    encrypted_value: &[u8],
    key_salt: &[u8],
) -> Result<(), SecretError> {
    let aad = secret_store_key_check_aad();
    let decrypted = crypto.decrypt(encrypted_value, key_salt, &aad)?;
    if decrypted.expose() != SECRET_STORE_KEY_CHECK_PLAINTEXT {
        return Err(SecretError::DecryptionFailed(
            "secret store key check mismatch".to_string(),
        ));
    }
    Ok(())
}

fn parse_stored_secret(entry: VersionedEntry) -> Result<Secret, SecretError> {
    let record: StoredSecret = parse_entry(entry)?;
    Ok(record.into())
}

fn parse_entry<T: serde::de::DeserializeOwned>(entry: VersionedEntry) -> Result<T, SecretError> {
    entry.entry.parse_json().map_err(|error| {
        SecretError::Database(format!("invalid filesystem secret record: {error}"))
    })
}

fn record_entry<T: Serialize>(kind: &str, value: &T) -> Result<Entry, SecretError> {
    let value = serde_json::to_value(value)
        .map_err(|error| SecretError::Database(format!("serialize secret record: {error}")))?;
    Entry::record(
        RecordKind::new(kind).map_err(secret_filesystem_error)?,
        &value,
    )
    .map_err(|error| SecretError::Database(format!("serialize secret record: {error}")))
}

fn normalize_secret_name(name: &str) -> String {
    name.to_lowercase()
}

fn ensure_secret_not_expired(secret: &Secret) -> Result<(), SecretError> {
    if let Some(expires_at) = secret.expires_at
        && expires_at < Utc::now()
    {
        return Err(SecretError::Expired);
    }
    Ok(())
}

fn records_root_path() -> Result<ScopedPath, SecretError> {
    ScopedPath::new("/secrets/records").map_err(secret_filesystem_error)
}

fn user_records_path(user_id: &str) -> Result<ScopedPath, SecretError> {
    ScopedPath::new(format!("/secrets/records/{}", encode_path_segment(user_id)))
        .map_err(secret_filesystem_error)
}

fn record_path(user_id: &str, name: &str) -> Result<ScopedPath, SecretError> {
    ScopedPath::new(format!(
        "/secrets/records/{}/{}.json",
        encode_path_segment(user_id),
        encode_path_segment(&normalize_secret_name(name))
    ))
    .map_err(secret_filesystem_error)
}

fn key_check_path() -> Result<ScopedPath, SecretError> {
    ScopedPath::new(format!(
        "/secrets/key-check/{SECRET_STORE_KEY_CHECK_ID}.json"
    ))
    .map_err(secret_filesystem_error)
}

fn scoped_from_virtual(path: &VirtualPath) -> Result<ScopedPath, SecretError> {
    ScopedPath::new(path.as_str()).map_err(secret_filesystem_error)
}

fn encode_path_segment(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn secret_filesystem_error(error: impl std::fmt::Display) -> SecretError {
    SecretError::Database(format!("filesystem secret store error: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use secrecy::SecretString;

    use super::*;

    #[tokio::test]
    async fn filesystem_secret_store_persists_records_through_root_filesystem() {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(Arc::clone(&root), "01234567890123456789012345678901");
        store.verify_can_decrypt_existing_secrets().await.unwrap();

        store
            .create(
                "tenant-user",
                CreateSecretParams::new("openai_key", "sk-test-filesystem"),
            )
            .await
            .unwrap();

        let reopened = filesystem_store(root, "01234567890123456789012345678901");
        reopened
            .verify_can_decrypt_existing_secrets()
            .await
            .unwrap();
        let decrypted = reopened
            .get_decrypted("tenant-user", "openai_key")
            .await
            .unwrap();
        assert_eq!(decrypted.expose(), "sk-test-filesystem");

        let refs = reopened.list("tenant-user").await.unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "openai_key");
        assert!(reopened.any_exist().await.unwrap());
    }

    #[tokio::test]
    async fn filesystem_secret_store_key_check_rejects_wrong_key() {
        let root = Arc::new(InMemoryBackend::new());
        let store = filesystem_store(Arc::clone(&root), "01234567890123456789012345678901");
        store.verify_can_decrypt_existing_secrets().await.unwrap();
        store
            .create(
                "tenant-user",
                CreateSecretParams::new("openai_key", "sk-test-filesystem"),
            )
            .await
            .unwrap();

        let wrong_key_store = filesystem_store(root, "abcdefghijklmnopqrstuvwxyzABCDEF");
        let error = wrong_key_store
            .verify_can_decrypt_existing_secrets()
            .await
            .expect_err("wrong key must fail filesystem secret readiness");
        assert!(!format!("{error:?}").contains("sk-test-filesystem"));
    }

    fn filesystem_store(
        root: Arc<InMemoryBackend>,
        key: &str,
    ) -> FilesystemSecretsStore<InMemoryBackend> {
        let crypto = Arc::new(SecretsCrypto::new(SecretString::from(key)).unwrap());
        FilesystemSecretsStore::over_root(root, crypto).unwrap()
    }
}
