//! libSQL-backed [`ConversationBindingService`] implementation.
//!
//! Looks up existing bindings in the `product_bindings` table; on miss,
//! reserves the binding row atomically before creating the durable thread.
//! Mapping shape: `(adapter, installation, conversation, actor) -> (tenant,
//! user, thread, agent_id?, project_id?)`.
//!
//! Schema lives in `src/db/libsql_migrations.rs` migration V26.
//!
//! ## First-bind atomicity
//!
//! The earlier implementation called `ensure_thread` **before** the binding
//! insert. Two concurrent first-inbounds for the same external conversation
//! both missed the SELECT, both minted a thread, then one INSERT won the
//! UNIQUE constraint while the other returned a transient error. The
//! loser's thread became an orphan — never bound, never referenced. Telegram
//! retries make this reachable in practice (see PR #3590 review finding
//! [#2](https://github.com/nearai/ironclaw/pull/3590#issuecomment-4454525610)).
//!
//! Fix: reserve the binding row first with a pre-minted candidate
//! `ThreadId`, then create the durable thread only on insert success. The
//! loser of the race re-reads the canonical binding and returns it
//! immediately — no orphan thread, no transient bounce.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_workflow::{
    ConversationBindingService, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
};
use ironclaw_threads::{EnsureThreadRequest, SessionThreadService, ThreadScope};

use crate::error::libsql_error;
use crate::identifiers::derive_user_id;

#[derive(Clone)]
pub struct LibSqlConversationBindingService {
    db: Arc<::libsql::Database>,
    thread_service: Arc<dyn SessionThreadService>,
    default_tenant_id: TenantId,
    default_agent_id: AgentId,
}

impl LibSqlConversationBindingService {
    pub fn new(
        db: Arc<::libsql::Database>,
        thread_service: Arc<dyn SessionThreadService>,
        default_tenant_id: TenantId,
        default_agent_id: AgentId,
    ) -> Self {
        Self {
            db,
            thread_service,
            default_tenant_id,
            default_agent_id,
        }
    }

    async fn connect(&self) -> Result<::libsql::Connection, ProductWorkflowError> {
        self.db.connect().map_err(libsql_error)
    }

    /// Look up an existing canonical binding. Returns `None` if no row
    /// matches the conversation/actor coordinates. Used both for the
    /// initial hit-path and to resolve the lost-race path after a
    /// concurrent winner inserted the canonical row.
    async fn lookup_existing(
        &self,
        conn: &::libsql::Connection,
        request: &ResolveBindingRequest,
    ) -> Result<Option<ResolvedBinding>, ProductWorkflowError> {
        let conversation_fingerprint = request.external_conversation_ref.conversation_fingerprint();
        let mut rows = conn
            .query(
                "SELECT tenant_id, user_id, thread_id, agent_id, project_id \
                 FROM product_bindings \
                 WHERE adapter_id = ?1 \
                   AND installation_id = ?2 \
                   AND external_conversation_fingerprint = ?3 \
                   AND external_actor_kind = ?4 \
                   AND external_actor_id = ?5",
                ::libsql::params![
                    request.adapter_id.as_str(),
                    request.installation_id.as_str(),
                    conversation_fingerprint.as_str(),
                    request.external_actor_ref.kind(),
                    request.external_actor_ref.id(),
                ],
            )
            .await
            .map_err(libsql_error)?;
        let Some(row) = rows.next().await.map_err(libsql_error)? else {
            return Ok(None);
        };
        let tenant_id_str: String = row.get(0).map_err(libsql_error)?;
        let user_id_str: String = row.get(1).map_err(libsql_error)?;
        let thread_id_str: String = row.get(2).map_err(libsql_error)?;
        let agent_id_str: Option<String> = row.get(3).map_err(libsql_error)?;
        let project_id_str: Option<String> = row.get(4).map_err(libsql_error)?;
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
    async fn rollback_binding(&self, conn: &::libsql::Connection, request: &ResolveBindingRequest) {
        let conversation_fingerprint = request.external_conversation_ref.conversation_fingerprint();
        if let Err(e) = conn
            .execute(
                "DELETE FROM product_bindings \
                 WHERE adapter_id = ?1 \
                   AND installation_id = ?2 \
                   AND external_conversation_fingerprint = ?3 \
                   AND external_actor_kind = ?4 \
                   AND external_actor_id = ?5",
                ::libsql::params![
                    request.adapter_id.as_str(),
                    request.installation_id.as_str(),
                    conversation_fingerprint.as_str(),
                    request.external_actor_ref.kind(),
                    request.external_actor_ref.id(),
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
impl ConversationBindingService for LibSqlConversationBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let conn = self.connect().await?;

        // 1. Hit path — existing binding.
        if let Some(canonical) = self.lookup_existing(&conn, &request).await? {
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

        let insert_result = conn
            .execute(
                "INSERT INTO product_bindings \
                 (adapter_id, installation_id, external_conversation_fingerprint, \
                  external_actor_kind, external_actor_id, \
                  tenant_id, user_id, thread_id, agent_id, project_id, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL, \
                         strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                ::libsql::params![
                    request.adapter_id.as_str(),
                    request.installation_id.as_str(),
                    conversation_fingerprint.as_str(),
                    request.external_actor_ref.kind(),
                    request.external_actor_ref.id(),
                    self.default_tenant_id.as_str(),
                    user_id.as_str(),
                    candidate_thread_id.as_str(),
                    self.default_agent_id.as_str(),
                ],
            )
            .await;

        match insert_result {
            Ok(_) => {
                // We won. Create the durable thread with our reserved id.
                // If this fails, roll back the binding so we don't leave a
                // row pointing at a nonexistent thread.
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
                    self.rollback_binding(&conn, &request).await;
                    return Err(ProductWorkflowError::BindingResolutionFailed {
                        reason: format!("ensure_thread failed after binding insert: {e}"),
                    });
                }
                Ok(ResolvedBinding {
                    tenant_id: self.default_tenant_id.clone(),
                    user_id,
                    thread_id: candidate_thread_id,
                    agent_id: Some(self.default_agent_id.clone()),
                    project_id: None,
                })
            }
            // UNIQUE violation means another concurrent inbound created the
            // binding between our SELECT and INSERT. libsql 0.6 surfaces the
            // extended SQLite code 2067 (SQLITE_CONSTRAINT_UNIQUE), not the
            // primary code 19; matching on 19 alone silently fails to catch
            // the concurrent case.
            Err(::libsql::Error::SqliteFailure(2067, _)) => {
                // Lost the race. Re-SELECT for canonical. We never created
                // a thread, so there's no orphan to clean up.
                self.lookup_existing(&conn, &request).await?.ok_or_else(|| {
                    ProductWorkflowError::BindingResolutionFailed {
                        reason: "UNIQUE conflict on binding insert but \
                                 canonical row not found on re-SELECT — \
                                 binding state inconsistent"
                            .to_string(),
                    }
                })
            }
            Err(other) => Err(libsql_error(other)),
        }
    }

    async fn lookup_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        let conn = self.connect().await?;
        self.lookup_existing(&conn, &request).await?.ok_or_else(|| {
            ProductWorkflowError::BindingRequired {
                reason: "no existing binding for adapter+installation+conversation+actor"
                    .to_string(),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_product_adapters::AuthRequirement;
    use ironclaw_product_adapters::{
        AdapterInstallationId, ExternalActorRef, ExternalConversationRef, ExternalEventId,
        ProductAdapterId, ProtocolAuthEvidence,
    };
    use ironclaw_threads::InMemorySessionThreadService;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Mirrors `src/db/libsql_migrations.rs` migration V26.
    const TEST_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS product_bindings (
    adapter_id TEXT NOT NULL,
    installation_id TEXT NOT NULL,
    external_conversation_fingerprint TEXT NOT NULL,
    external_actor_kind TEXT NOT NULL,
    external_actor_id TEXT NOT NULL,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    agent_id TEXT,
    project_id TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (
        adapter_id,
        installation_id,
        external_conversation_fingerprint,
        external_actor_kind,
        external_actor_id
    )
);
"#;

    async fn service() -> (LibSqlConversationBindingService, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("binding.db");
        let db = ::libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build db");
        let conn = db.connect().expect("connect");
        conn.execute_batch(TEST_SCHEMA).await.expect("schema");
        let thread_service: Arc<dyn SessionThreadService> =
            Arc::new(InMemorySessionThreadService::default());
        let tenant = TenantId::new("tenant_default").expect("tenant");
        let agent = AgentId::new("agent_default").expect("agent");
        let svc =
            LibSqlConversationBindingService::new(Arc::new(db), thread_service, tenant, agent);
        (svc, dir)
    }

    fn request(actor_id: &str, conversation_id: &str) -> ResolveBindingRequest {
        let evidence = ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Telegram-Bot-Api-Secret-Token".into(),
            },
            "telegram_install_default",
        );
        let auth_claim = evidence.claim().expect("verified claim").clone();
        ResolveBindingRequest {
            adapter_id: ProductAdapterId::new("telegram_v2").expect("adapter"),
            installation_id: AdapterInstallationId::new("install_default").expect("install"),
            external_actor_ref: ExternalActorRef::new("user", actor_id, None::<String>)
                .expect("actor"),
            external_conversation_ref: ExternalConversationRef::new(
                None,
                conversation_id,
                None,
                None,
            )
            .expect("conv"),
            external_event_id: ExternalEventId::new(format!("evt:{actor_id}:{conversation_id}"))
                .expect("event id"),
            route_kind: ironclaw_product_workflow::ProductConversationRouteKind::Direct,
            auth_claim,
        }
    }

    #[tokio::test]
    async fn first_resolve_creates_thread_and_persists_binding() {
        let (svc, _dir) = service().await;
        let binding = svc
            .resolve_binding(request("12345", "67890"))
            .await
            .expect("resolve");
        assert_eq!(binding.tenant_id.as_str(), "tenant_default");
        assert_eq!(
            binding.agent_id.as_ref().map(|a| a.as_str()),
            Some("agent_default")
        );
        assert!(binding.user_id.as_str().contains("12345"));
    }

    #[tokio::test]
    async fn repeated_resolve_returns_same_binding() {
        let (svc, _dir) = service().await;
        let first = svc
            .resolve_binding(request("12345", "67890"))
            .await
            .expect("first");
        let second = svc
            .resolve_binding(request("12345", "67890"))
            .await
            .expect("second");
        assert_eq!(first.user_id.as_str(), second.user_id.as_str());
        assert_eq!(first.thread_id.as_str(), second.thread_id.as_str());
    }

    #[tokio::test]
    async fn different_actor_in_same_conversation_gets_different_binding() {
        let (svc, _dir) = service().await;
        let alice = svc
            .resolve_binding(request("alice", "shared_chat"))
            .await
            .expect("alice");
        let bob = svc
            .resolve_binding(request("bob", "shared_chat"))
            .await
            .expect("bob");
        assert_ne!(alice.user_id.as_str(), bob.user_id.as_str());
        assert_ne!(alice.thread_id.as_str(), bob.thread_id.as_str());
    }

    /// Tiny [`SessionThreadService`] decorator that counts `ensure_thread`
    /// calls so the concurrent test can assert exactly-one durable thread
    /// creation under contention. Non-`ensure_thread` methods are not
    /// reachable from the binding service in this test, so they panic to
    /// keep the decorator small.
    struct CountingThreadService {
        inner: Arc<InMemorySessionThreadService>,
        ensure_thread_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl SessionThreadService for CountingThreadService {
        async fn ensure_thread(
            &self,
            request: EnsureThreadRequest,
        ) -> Result<ironclaw_threads::SessionThreadRecord, ironclaw_threads::SessionThreadError>
        {
            self.ensure_thread_calls.fetch_add(1, Ordering::SeqCst);
            self.inner.ensure_thread(request).await
        }

        async fn accept_inbound_message(
            &self,
            _request: ironclaw_threads::AcceptInboundMessageRequest,
        ) -> Result<ironclaw_threads::AcceptedInboundMessage, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call accept_inbound_message")
        }

        async fn replay_accepted_inbound_message(
            &self,
            _request: ironclaw_threads::ReplayAcceptedInboundMessageRequest,
        ) -> Result<
            Option<ironclaw_threads::AcceptedInboundMessageReplay>,
            ironclaw_threads::SessionThreadError,
        > {
            unreachable!("binding service tests do not call replay_accepted_inbound_message")
        }

        async fn mark_message_submitted(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ThreadId,
            _message_id: ironclaw_threads::ThreadMessageId,
            _turn_id: String,
            _turn_run_id: String,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call mark_message_submitted")
        }

        async fn mark_message_deferred_busy(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ThreadId,
            _message_id: ironclaw_threads::ThreadMessageId,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call mark_message_deferred_busy")
        }

        async fn append_assistant_draft(
            &self,
            _request: ironclaw_threads::AppendAssistantDraftRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call append_assistant_draft")
        }

        async fn append_tool_result_reference(
            &self,
            _request: ironclaw_threads::AppendToolResultReferenceRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call append_tool_result_reference")
        }

        async fn update_assistant_draft(
            &self,
            _request: ironclaw_threads::UpdateAssistantDraftRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call update_assistant_draft")
        }

        async fn finalize_assistant_message(
            &self,
            _scope: &ThreadScope,
            _thread_id: &ThreadId,
            _message_id: ironclaw_threads::ThreadMessageId,
            _content: ironclaw_threads::MessageContent,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call finalize_assistant_message")
        }

        async fn redact_message(
            &self,
            _request: ironclaw_threads::RedactMessageRequest,
        ) -> Result<ironclaw_threads::ThreadMessageRecord, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call redact_message")
        }

        async fn load_context_window(
            &self,
            _request: ironclaw_threads::LoadContextWindowRequest,
        ) -> Result<ironclaw_threads::ContextWindow, ironclaw_threads::SessionThreadError> {
            unreachable!("binding service tests do not call load_context_window")
        }

        async fn load_context_messages(
            &self,
            _request: ironclaw_threads::LoadContextMessagesRequest,
        ) -> Result<ironclaw_threads::ContextMessages, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call load_context_messages")
        }

        async fn list_thread_history(
            &self,
            _request: ironclaw_threads::ThreadHistoryRequest,
        ) -> Result<ironclaw_threads::ThreadHistory, ironclaw_threads::SessionThreadError> {
            unreachable!("binding service tests do not call list_thread_history")
        }

        async fn create_summary_artifact(
            &self,
            _request: ironclaw_threads::CreateSummaryArtifactRequest,
        ) -> Result<ironclaw_threads::SummaryArtifact, ironclaw_threads::SessionThreadError>
        {
            unreachable!("binding service tests do not call create_summary_artifact")
        }
    }

    /// Caller-level regression test for finding #2 of @serrrfirat's PR #3590
    /// review. Under contention, the OLD code minted one thread per concurrent
    /// caller (orphans for losers). This test asserts the new code mints
    /// exactly one thread total, and every caller returns the same canonical
    /// `thread_id`.
    #[tokio::test]
    async fn concurrent_first_bind_creates_exactly_one_thread() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("binding.db");
        let db = ::libsql::Builder::new_local(path)
            .build()
            .await
            .expect("build db");
        let conn = db.connect().expect("connect");
        conn.execute_batch(TEST_SCHEMA).await.expect("schema");
        let counter = Arc::new(AtomicUsize::new(0));
        let thread_service = Arc::new(CountingThreadService {
            inner: Arc::new(InMemorySessionThreadService::default()),
            ensure_thread_calls: Arc::clone(&counter),
        }) as Arc<dyn SessionThreadService>;
        let tenant = TenantId::new("tenant_default").expect("tenant");
        let agent = AgentId::new("agent_default").expect("agent");
        let svc = Arc::new(LibSqlConversationBindingService::new(
            Arc::new(db),
            thread_service,
            tenant,
            agent,
        ));

        // 16 concurrent first-bind attempts for the same conversation.
        let concurrency = 16;
        let handles: Vec<_> = (0..concurrency)
            .map(|_| {
                let svc = Arc::clone(&svc);
                tokio::spawn(async move { svc.resolve_binding(request("99999", "shared")).await })
            })
            .collect();

        let mut thread_ids = Vec::with_capacity(concurrency);
        for handle in handles {
            let binding = handle.await.expect("task join").expect("resolve_binding");
            thread_ids.push(binding.thread_id.as_str().to_string());
        }

        // Every caller returned the same canonical thread_id …
        thread_ids.sort();
        thread_ids.dedup();
        assert_eq!(
            thread_ids.len(),
            1,
            "all concurrent callers must observe the same canonical thread_id, got {thread_ids:?}"
        );

        // … and exactly one durable thread was created (the winner's).
        let total_ensure_calls = counter.load(Ordering::SeqCst);
        assert_eq!(
            total_ensure_calls, 1,
            "expected exactly one ensure_thread call (winner only), got {total_ensure_calls}"
        );
    }
}
