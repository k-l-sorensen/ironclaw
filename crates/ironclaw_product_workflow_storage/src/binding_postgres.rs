//! Postgres-backed [`ConversationBindingService`] implementation.
//!
//! Schema lives in `migrations/V28__product_inbound_actions_and_bindings.sql`.
//!
//! ## First-bind atomicity
//!
//! Like the libSQL backend (see `binding_libsql.rs` for the full write-up),
//! this implementation reserves the binding row **before** creating the
//! durable thread. The previous shape — `ensure_thread` then upsert — left
//! orphan threads behind every concurrent first-bind: the loser of the
//! upsert returned the canonical winner's `thread_id`, but had already
//! durably created its own thread for nobody. Telegram retries make this
//! reachable in practice. Fix per @serrrfirat's PR #3590 review finding #2.
//!
//! Postgres-specific trick: `INSERT ... ON CONFLICT (...) DO NOTHING
//! RETURNING thread_id` returns the inserted row on success and *no row*
//! on conflict. We use `query_opt` to branch: `Some(row)` means we won and
//! must now create the thread; `None` means we lost and must re-SELECT
//! for the canonical binding.

use std::sync::Arc;

use async_trait::async_trait;
use deadpool_postgres::Pool;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_threads::{EnsureThreadRequest, SessionThreadService, ThreadScope};
use tokio_postgres::Client;

use crate::error::{pool_error, postgres_error};
use crate::identifiers::derive_user_id;

#[derive(Clone)]
pub struct PostgresConversationBindingService {
    pool: Pool,
    thread_service: Arc<dyn SessionThreadService>,
    default_tenant_id: TenantId,
    default_agent_id: AgentId,
}

impl PostgresConversationBindingService {
    pub fn new(
        pool: Pool,
        thread_service: Arc<dyn SessionThreadService>,
        default_tenant_id: TenantId,
        default_agent_id: AgentId,
    ) -> Self {
        Self {
            pool,
            thread_service,
            default_tenant_id,
            default_agent_id,
        }
    }

    /// Look up an existing canonical binding. Used both for the initial
    /// hit-path and to resolve the lost-race path after a concurrent
    /// winner inserted the canonical row.
    async fn lookup_existing(
        &self,
        client: &Client,
        request: &ResolveBindingRequest,
    ) -> Result<Option<ResolvedBinding>, ProductWorkflowError> {
        let conversation_fingerprint = request.external_conversation_ref.conversation_fingerprint();
        let row = client
            .query_opt(
                "SELECT tenant_id, user_id, thread_id, agent_id, project_id \
                 FROM product_bindings \
                 WHERE adapter_id = $1 \
                   AND installation_id = $2 \
                   AND external_conversation_fingerprint = $3 \
                   AND external_actor_kind = $4 \
                   AND external_actor_id = $5",
                &[
                    &request.adapter_id.as_str(),
                    &request.installation_id.as_str(),
                    &conversation_fingerprint.as_str(),
                    &request.external_actor_ref.kind(),
                    &request.external_actor_ref.id(),
                ],
            )
            .await
            .map_err(postgres_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let tenant_id_str: String = row.get("tenant_id");
        let user_id_str: String = row.get("user_id");
        let thread_id_str: String = row.get("thread_id");
        let agent_id_str: Option<String> = row.get("agent_id");
        let project_id_str: Option<String> = row.get("project_id");
        Ok(Some(ResolvedBinding {
            tenant_id: TenantId::new(tenant_id_str).map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?,
            user_id: UserId::new(user_id_str).map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?,
            thread_id: ThreadId::new(thread_id_str).map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?,
            agent_id: agent_id_str.map(AgentId::new).transpose().map_err(|e| {
                ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                }
            })?,
            project_id: project_id_str
                .map(ProjectId::new)
                .transpose()
                .map_err(|e| ProductWorkflowError::BindingResolutionFailed {
                    reason: e.to_string(),
                })?,
        }))
    }

    /// Best-effort cleanup of the binding row we just inserted, used when
    /// downstream thread creation fails after we won the binding race. If
    /// the rollback itself fails the original error is still the one that
    /// surfaces; we log the cleanup failure so an operator can reconcile.
    async fn rollback_binding(&self, client: &Client, request: &ResolveBindingRequest) {
        let conversation_fingerprint = request.external_conversation_ref.conversation_fingerprint();
        if let Err(e) = client
            .execute(
                "DELETE FROM product_bindings \
                 WHERE adapter_id = $1 \
                   AND installation_id = $2 \
                   AND external_conversation_fingerprint = $3 \
                   AND external_actor_kind = $4 \
                   AND external_actor_id = $5",
                &[
                    &request.adapter_id.as_str(),
                    &request.installation_id.as_str(),
                    &conversation_fingerprint.as_str(),
                    &request.external_actor_ref.kind(),
                    &request.external_actor_ref.id(),
                ],
            )
            .await
        {
            tracing::warn!(
                error = %e,
                adapter = %request.adapter_id.as_str(),
                installation = %request.installation_id.as_str(),
                "Reborn host: failed to roll back binding row after thread \
                 service failure; row may now reference a nonexistent thread \
                 until it is manually reconciled."
            );
        }
    }
}

#[async_trait]
impl ConversationBindingService for PostgresConversationBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let client = self.pool.get().await.map_err(pool_error)?;

        // 1. Hit path — existing binding.
        if let Some(canonical) = self.lookup_existing(&client, &request).await? {
            return Ok(canonical);
        }

        // 2. Miss path — reserve binding first with a pre-minted thread id.
        // The thread does NOT exist yet at this point; we only create it
        // *after* we win the INSERT. This is the fix for finding #2 from
        // @serrrfirat's review on PR #3590.
        let user_id = derive_user_id(&request)?;
        let candidate_thread_id = ThreadId::new(uuid::Uuid::new_v4().to_string()).map_err(|e| {
            ProductWorkflowError::BindingResolutionFailed {
                reason: format!("mint candidate thread id: {e}"),
            }
        })?;
        let conversation_fingerprint = request.external_conversation_ref.conversation_fingerprint();

        // `ON CONFLICT (...) DO NOTHING RETURNING ...` returns the inserted
        // row on success and zero rows on conflict — exactly the branch
        // signal we need to decide whether to create the thread.
        let inserted = client
            .query_opt(
                "INSERT INTO product_bindings \
                 (adapter_id, installation_id, external_conversation_fingerprint, \
                  external_actor_kind, external_actor_id, \
                  tenant_id, user_id, thread_id, agent_id, project_id) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL) \
                 ON CONFLICT (adapter_id, installation_id, external_conversation_fingerprint, external_actor_kind, external_actor_id) \
                 DO NOTHING \
                 RETURNING thread_id",
                &[
                    &request.adapter_id.as_str(),
                    &request.installation_id.as_str(),
                    &conversation_fingerprint.as_str(),
                    &request.external_actor_ref.kind(),
                    &request.external_actor_ref.id(),
                    &self.default_tenant_id.as_str(),
                    &user_id.as_str(),
                    &candidate_thread_id.as_str(),
                    &self.default_agent_id.as_str(),
                ],
            )
            .await
            .map_err(postgres_error)?;

        if inserted.is_some() {
            // We won the upsert. Create the durable thread with our
            // reserved id. On failure, roll the binding back so it never
            // points to a missing thread.
            let scope = ThreadScope {
                tenant_id: self.default_tenant_id.clone(),
                agent_id: self.default_agent_id.clone(),
                project_id: None,
                owner_user_id: Some(user_id.clone()),
                mission_id: None,
            };
            let ensure_request = EnsureThreadRequest {
                scope,
                thread_id: Some(candidate_thread_id.clone()),
                created_by_actor_id: format!(
                    "{}:{}",
                    request.adapter_id.as_str(),
                    request.installation_id.as_str()
                ),
                title: None,
                metadata_json: None,
            };
            if let Err(e) = self.thread_service.ensure_thread(ensure_request).await {
                self.rollback_binding(&client, &request).await;
                return Err(ProductWorkflowError::BindingResolutionFailed {
                    reason: format!("ensure_thread failed after binding insert: {e}"),
                });
            }
            return Ok(ResolvedBinding {
                tenant_id: self.default_tenant_id.clone(),
                user_id,
                thread_id: candidate_thread_id,
                agent_id: Some(self.default_agent_id.clone()),
                project_id: None,
            });
        }

        // Lost the race. Re-SELECT for canonical. We never created a
        // thread, so there's no orphan to clean up.
        self.lookup_existing(&client, &request)
            .await?
            .ok_or_else(|| ProductWorkflowError::BindingResolutionFailed {
                reason: "INSERT ... DO NOTHING returned no row but canonical \
                         row not found on re-SELECT — binding state inconsistent"
                    .to_string(),
            })
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let client = self.pool.get().await.map_err(pool_error)?;
        self.lookup_existing(&client, &request)
            .await?
            .ok_or_else(|| ProductWorkflowError::BindingRequired {
                reason: "no existing binding for adapter+installation+conversation+actor"
                    .to_string(),
            })
    }
}
