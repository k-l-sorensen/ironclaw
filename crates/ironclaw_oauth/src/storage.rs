use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_secrets::{SecretMaterial, SecretStore};
use secrecy::{ExposeSecret, SecretString};

use crate::error::OAuthError;

#[derive(Clone)]
pub struct TokenSet {
    pub access_token: SecretString,
    pub refresh_token: Option<SecretString>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

impl TokenSet {
    pub fn from_expires_in(
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
        scopes: Vec<String>,
    ) -> Self {
        let expires_at = expires_in.map(|seconds| {
            let seconds = i64::try_from(seconds).unwrap_or(i64::MAX);
            let delta = Duration::try_seconds(seconds).unwrap_or(Duration::MAX);
            Utc::now() + delta
        });
        Self {
            access_token: SecretString::from(access_token.into()),
            refresh_token: refresh_token.map(SecretString::from),
            expires_at,
            scopes,
        }
    }

    pub fn expires_in_buffer(&self, buffer: Duration) -> bool {
        self.expires_at
            .is_some_and(|expiry| expiry <= Utc::now() + buffer)
    }
}

#[derive(Clone)]
pub struct TokenPersister {
    secrets: Arc<dyn SecretStore>,
}

impl TokenPersister {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        Self { secrets }
    }

    pub async fn persist(
        &self,
        scope: &ResourceScope,
        credential_name: &str,
        tokens: &TokenSet,
    ) -> Result<(), OAuthError> {
        self.put_row(scope, credential_name, &tokens.access_token)
            .await?;
        if let Some(refresh_token) = &tokens.refresh_token {
            self.put_row(
                scope,
                &format!("{credential_name}_refresh_token"),
                refresh_token,
            )
            .await?;
        }
        if !tokens.scopes.is_empty() {
            self.put_string(
                scope,
                &format!("{credential_name}_scopes"),
                &serde_json::to_string(&tokens.scopes)?,
            )
            .await?;
        }
        if let Some(expires_at) = tokens.expires_at {
            self.put_string(
                scope,
                &format!("{credential_name}_expiry"),
                &expires_at.timestamp().to_string(),
            )
            .await?;
        }
        Ok(())
    }

    pub async fn load_access_token(
        &self,
        scope: &ResourceScope,
        credential_name: &str,
    ) -> Result<Option<SecretString>, OAuthError> {
        self.load_optional(scope, credential_name).await
    }

    pub async fn load_refresh_token(
        &self,
        scope: &ResourceScope,
        credential_name: &str,
    ) -> Result<Option<SecretString>, OAuthError> {
        self.load_optional(scope, &format!("{credential_name}_refresh_token"))
            .await
    }

    pub async fn load_scopes(
        &self,
        scope: &ResourceScope,
        credential_name: &str,
    ) -> Result<Vec<String>, OAuthError> {
        let Some(scopes) = self
            .load_optional(scope, &format!("{credential_name}_scopes"))
            .await?
        else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_str(scopes.expose_secret())?)
    }

    pub async fn load_expiry(
        &self,
        scope: &ResourceScope,
        credential_name: &str,
    ) -> Result<Option<DateTime<Utc>>, OAuthError> {
        let Some(expiry) = self
            .load_optional(scope, &format!("{credential_name}_expiry"))
            .await?
        else {
            return Ok(None);
        };
        let timestamp = expiry.expose_secret().parse::<i64>().map_err(|error| {
            OAuthError::InvalidTokenResponse {
                reason: format!("stored expiry is not a Unix timestamp: {error}"),
            }
        })?;
        DateTime::from_timestamp(timestamp, 0)
            .ok_or_else(|| OAuthError::InvalidTokenResponse {
                reason: "stored expiry timestamp is out of range".to_string(),
            })
            .map(Some)
    }

    async fn put_row(
        &self,
        scope: &ResourceScope,
        name: &str,
        value: &SecretString,
    ) -> Result<(), OAuthError> {
        self.put_string(scope, name, value.expose_secret()).await
    }

    async fn put_string(
        &self,
        scope: &ResourceScope,
        name: &str,
        value: &str,
    ) -> Result<(), OAuthError> {
        let handle = SecretHandle::new(name.to_string())?;
        self.secrets
            .put(
                scope.clone(),
                handle,
                SecretMaterial::from(value.to_string()),
            )
            .await?;
        Ok(())
    }

    async fn load_optional(
        &self,
        scope: &ResourceScope,
        name: &str,
    ) -> Result<Option<SecretString>, OAuthError> {
        let handle = SecretHandle::new(name.to_string())?;
        if self.secrets.metadata(scope, &handle).await?.is_none() {
            return Ok(None);
        }
        let lease = self.secrets.lease_once(scope, &handle).await?;
        let material = self.secrets.consume(scope, lease.id).await?;
        Ok(Some(material))
    }
}
