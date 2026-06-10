//! Durable Slack installation setup and secret boundary.
//!
//! The Reborn Slack host-beta path is enabled at boot, but the Slack app
//! installation is operator-managed at runtime. This module owns the only
//! place where WebUI-submitted Slack secrets are written to the shared
//! `SecretStore` and the only place where Slack runtime code resolves those
//! handles back to material.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId};
use ironclaw_product_adapters::AdapterInstallationId;
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStoreError};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::slack_serve::{SlackApiAppId, SlackTeamId};

const SLACK_BOT_TOKEN_HANDLE_PREFIX: &str = "slack_bot_token_v";
const SLACK_SIGNING_SECRET_HANDLE_PREFIX: &str = "slack_signing_secret_v";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SlackInstallationSetup {
    pub(crate) installation_id: String,
    pub(crate) team_id: String,
    pub(crate) api_app_id: String,
    pub(crate) user_id: String,
    pub(crate) shared_subject_user_id: Option<String>,
    pub(crate) bot_token_handle: SecretHandle,
    pub(crate) signing_secret_handle: SecretHandle,
    pub(crate) revision: u64,
    pub(crate) updated_at: DateTime<Utc>,
}

impl SlackInstallationSetup {
    pub(crate) fn installation_id(&self) -> Result<AdapterInstallationId, SlackSetupError> {
        AdapterInstallationId::new(self.installation_id.clone()).map_err(|reason| {
            SlackSetupError::InvalidField {
                field: "installation_id",
                reason: reason.to_string(),
            }
        })
    }

    pub(crate) fn team_id(&self) -> SlackTeamId {
        SlackTeamId::new(self.team_id.clone())
    }

    pub(crate) fn user_id(&self) -> Result<UserId, SlackSetupError> {
        UserId::new(self.user_id.clone()).map_err(|reason| SlackSetupError::InvalidField {
            field: "user_id",
            reason: reason.to_string(),
        })
    }

    pub(crate) fn shared_subject_user_id(&self) -> Result<Option<UserId>, SlackSetupError> {
        self.shared_subject_user_id
            .as_ref()
            .map(|raw| {
                UserId::new(raw.clone()).map_err(|reason| SlackSetupError::InvalidField {
                    field: "shared_subject_user_id",
                    reason: reason.to_string(),
                })
            })
            .transpose()
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SlackInstallationSetupUpdate {
    pub(crate) installation_id: String,
    pub(crate) team_id: String,
    pub(crate) api_app_id: String,
    pub(crate) user_id: Option<String>,
    pub(crate) shared_subject_user_id: Option<String>,
    pub(crate) bot_token: Option<SecretString>,
    pub(crate) signing_secret: Option<SecretString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SlackInstallationSetupStatus {
    pub(crate) configured: bool,
    pub(crate) installation_id: Option<String>,
    pub(crate) team_id: Option<String>,
    pub(crate) api_app_id: Option<String>,
    pub(crate) user_id: Option<String>,
    pub(crate) shared_subject_user_id: Option<String>,
    pub(crate) bot_token_configured: bool,
    pub(crate) signing_secret_configured: bool,
    pub(crate) revision: Option<u64>,
}

#[derive(Debug, Error)]
pub(crate) enum SlackSetupError {
    #[error("invalid Slack setup field {field}: {reason}")]
    InvalidField { field: &'static str, reason: String },
    #[error("Slack setup requires {field}")]
    MissingField { field: &'static str },
    #[error("Slack setup store unavailable")]
    StoreUnavailable,
    #[error("Slack secret store unavailable: {reason}")]
    SecretStoreUnavailable { reason: &'static str },
}

#[async_trait]
pub(crate) trait SlackInstallationSetupStore: Send + Sync + std::fmt::Debug {
    async fn get_slack_installation_setup(
        &self,
    ) -> Result<Option<SlackInstallationSetup>, SlackSetupError>;

    async fn put_slack_installation_setup(
        &self,
        setup: &SlackInstallationSetup,
    ) -> Result<(), SlackSetupError>;
}

#[derive(Clone)]
pub(crate) struct SlackSetupService {
    tenant_id: TenantId,
    agent_id: AgentId,
    project_id: Option<ProjectId>,
    operator_user_id: UserId,
    store: Arc<dyn SlackInstallationSetupStore>,
    secret_store: Arc<dyn SecretStore>,
}

impl SlackSetupService {
    pub(crate) fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        operator_user_id: UserId,
        store: Arc<dyn SlackInstallationSetupStore>,
        secret_store: Arc<dyn SecretStore>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            operator_user_id,
            store,
            secret_store,
        }
    }

    pub(crate) fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }

    pub(crate) fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    pub(crate) fn project_id(&self) -> Option<&ProjectId> {
        self.project_id.as_ref()
    }

    pub(crate) async fn current_setup(
        &self,
    ) -> Result<Option<SlackInstallationSetup>, SlackSetupError> {
        self.store.get_slack_installation_setup().await
    }

    pub(crate) async fn status(&self) -> Result<SlackInstallationSetupStatus, SlackSetupError> {
        let Some(setup) = self.current_setup().await? else {
            return Ok(SlackInstallationSetupStatus {
                configured: false,
                installation_id: None,
                team_id: None,
                api_app_id: None,
                user_id: None,
                shared_subject_user_id: None,
                bot_token_configured: false,
                signing_secret_configured: false,
                revision: None,
            });
        };
        let bot_token_configured = self
            .secret_store
            .metadata(&self.secret_scope(), &setup.bot_token_handle)
            .await
            .map_err(map_secret_error)?
            .is_some();
        let signing_secret_configured = self
            .secret_store
            .metadata(&self.secret_scope(), &setup.signing_secret_handle)
            .await
            .map_err(map_secret_error)?
            .is_some();
        Ok(SlackInstallationSetupStatus {
            configured: bot_token_configured && signing_secret_configured,
            installation_id: Some(setup.installation_id),
            team_id: Some(setup.team_id),
            api_app_id: Some(setup.api_app_id),
            user_id: Some(setup.user_id),
            shared_subject_user_id: setup.shared_subject_user_id,
            bot_token_configured,
            signing_secret_configured,
            revision: Some(setup.revision),
        })
    }

    pub(crate) async fn save(
        &self,
        update: SlackInstallationSetupUpdate,
    ) -> Result<SlackInstallationSetup, SlackSetupError> {
        let previous = self.current_setup().await?;
        let revision = previous
            .as_ref()
            .map(|setup| setup.revision.saturating_add(1))
            .unwrap_or(1);
        let setup = self.validated_setup(update, previous.as_ref(), revision)?;

        if let Some(bot_token) = setup.pending_bot_token.as_ref() {
            self.put_secret(setup.record.bot_token_handle.clone(), bot_token.clone())
                .await?;
        }
        if let Some(signing_secret) = setup.pending_signing_secret.as_ref() {
            self.put_secret(
                setup.record.signing_secret_handle.clone(),
                signing_secret.clone(),
            )
            .await?;
        }
        self.store
            .put_slack_installation_setup(&setup.record)
            .await?;
        Ok(setup.record)
    }

    pub(crate) async fn signing_secret(
        &self,
        setup: &SlackInstallationSetup,
    ) -> Result<SecretMaterial, SlackSetupError> {
        self.secret_material(&setup.signing_secret_handle).await
    }

    pub(crate) async fn bot_token(
        &self,
        setup: &SlackInstallationSetup,
    ) -> Result<SecretMaterial, SlackSetupError> {
        self.secret_material(&setup.bot_token_handle).await
    }

    fn validated_setup(
        &self,
        update: SlackInstallationSetupUpdate,
        previous: Option<&SlackInstallationSetup>,
        revision: u64,
    ) -> Result<ValidatedSlackSetup, SlackSetupError> {
        let installation_id = validate_required("installation_id", update.installation_id)?;
        AdapterInstallationId::new(installation_id.clone()).map_err(|reason| {
            SlackSetupError::InvalidField {
                field: "installation_id",
                reason: reason.to_string(),
            }
        })?;
        let team_id = validate_required("team_id", update.team_id)?;
        SlackTeamId::new(team_id.clone());
        let api_app_id = validate_required("api_app_id", update.api_app_id)?;
        SlackApiAppId::new(api_app_id.clone());
        let user_id = match validate_optional_user("user_id", update.user_id)? {
            Some(user_id) => user_id,
            None => self.operator_user_id.clone(),
        };
        let shared_subject_user_id =
            validate_optional_user("shared_subject_user_id", update.shared_subject_user_id)?;

        let (bot_token_handle, pending_bot_token) = match update.bot_token {
            Some(secret) if !secret.expose_secret().is_empty() => {
                (bot_token_handle(revision)?, Some(secret))
            }
            _ => {
                let previous =
                    previous.ok_or(SlackSetupError::MissingField { field: "bot_token" })?;
                (previous.bot_token_handle.clone(), None)
            }
        };
        let (signing_secret_handle, pending_signing_secret) = match update.signing_secret {
            Some(secret) if !secret.expose_secret().is_empty() => {
                (signing_secret_handle(revision)?, Some(secret))
            }
            _ => {
                let previous = previous.ok_or(SlackSetupError::MissingField {
                    field: "signing_secret",
                })?;
                (previous.signing_secret_handle.clone(), None)
            }
        };

        Ok(ValidatedSlackSetup {
            record: SlackInstallationSetup {
                installation_id,
                team_id,
                api_app_id,
                user_id: user_id.to_string(),
                shared_subject_user_id: shared_subject_user_id.map(|user_id| user_id.to_string()),
                bot_token_handle,
                signing_secret_handle,
                revision,
                updated_at: Utc::now(),
            },
            pending_bot_token,
            pending_signing_secret,
        })
    }

    async fn put_secret(
        &self,
        handle: SecretHandle,
        material: SecretString,
    ) -> Result<(), SlackSetupError> {
        self.secret_store
            .put(
                self.secret_scope(),
                handle,
                SecretMaterial::from(material.expose_secret().to_string()),
            )
            .await
            .map_err(map_secret_error)?;
        Ok(())
    }

    async fn secret_material(
        &self,
        handle: &SecretHandle,
    ) -> Result<SecretMaterial, SlackSetupError> {
        let scope = self.secret_scope();
        let lease = self
            .secret_store
            .lease_once(&scope, handle)
            .await
            .map_err(map_secret_error)?;
        self.secret_store
            .consume(&scope, lease.id)
            .await
            .map_err(map_secret_error)
    }

    fn secret_scope(&self) -> ResourceScope {
        ResourceScope {
            tenant_id: self.tenant_id.clone(),
            user_id: self.operator_user_id.clone(),
            agent_id: Some(self.agent_id.clone()),
            project_id: self.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        }
    }
}

struct ValidatedSlackSetup {
    record: SlackInstallationSetup,
    pending_bot_token: Option<SecretString>,
    pending_signing_secret: Option<SecretString>,
}

fn validate_required(field: &'static str, value: String) -> Result<String, SlackSetupError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(SlackSetupError::MissingField { field });
    }
    Ok(value)
}

fn validate_optional_user(
    field: &'static str,
    value: Option<String>,
) -> Result<Option<UserId>, SlackSetupError> {
    value
        .map(|raw| {
            let raw = validate_required(field, raw)?;
            UserId::new(raw).map_err(|reason| SlackSetupError::InvalidField {
                field,
                reason: reason.to_string(),
            })
        })
        .transpose()
}

fn bot_token_handle(revision: u64) -> Result<SecretHandle, SlackSetupError> {
    SecretHandle::new(format!("{SLACK_BOT_TOKEN_HANDLE_PREFIX}{revision}")).map_err(|reason| {
        SlackSetupError::InvalidField {
            field: "bot_token",
            reason: reason.to_string(),
        }
    })
}

fn signing_secret_handle(revision: u64) -> Result<SecretHandle, SlackSetupError> {
    SecretHandle::new(format!("{SLACK_SIGNING_SECRET_HANDLE_PREFIX}{revision}")).map_err(|reason| {
        SlackSetupError::InvalidField {
            field: "signing_secret",
            reason: reason.to_string(),
        }
    })
}

fn map_secret_error(error: SecretStoreError) -> SlackSetupError {
    SlackSetupError::SecretStoreUnavailable {
        reason: error.stable_reason(),
    }
}
