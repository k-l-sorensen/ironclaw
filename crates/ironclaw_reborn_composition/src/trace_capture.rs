//! Autonomous Trace Commons turn-end capture for the Reborn runtime.
//!
//! Mirrors the v1 binary's turn-end capture (`src/agent/thread_ops.rs::
//! spawn_autonomous_trace_contribution`) and periodic queue flush
//! (`src/agent/agent_loop.rs::spawn_trace_queue_flush_worker`): every terminal
//! turn lifecycle event spawns a detached best-effort task that reads the
//! owner's standing contribution policy, captures the recent thread
//! transcript, redacts and scores it locally, and queues + flushes eligible
//! envelopes. Non-enrolled users pay one policy-file read per turn and
//! nothing else.
//!
//! Capture must never block or fail the turn lifecycle path: the sink is
//! subscribed best-effort and all work happens on a spawned task whose
//! errors are logged at `debug!` only (`info!`/`warn!` corrupt the REPL).
//!
//! Credit-notice delivery (v1 broadcasts via `ChannelManager`) is
//! intentionally not wired here yet: the composition layer has no outbound
//! notification surface. The notice outbox still accumulates on disk and is
//! delivered when the same scope runs under the v1 binary; a Reborn-native
//! delivery path is a follow-up.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_reborn_traces::ConversationMessage;
use ironclaw_reborn_traces::client::{
    TraceClientAutonomousCaptureOutcome, TraceClientAutonomousCaptureRequest, TraceClientHost,
    TraceClientScope,
};
use ironclaw_reborn_traces::contribution::{self as trace, read_trace_policy_for_scope};
use ironclaw_threads::{
    MessageKind, MessageStatus, SessionThreadError, SessionThreadService, ThreadHistoryRequest,
    ThreadMessageRecord, ThreadScope,
};
use ironclaw_turns::{TurnError, TurnEventKind, TurnEventSink, TurnLifecycleEvent};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Recent-transcript bound, mirroring v1 (last 24 messages, max 5 turns).
const CAPTURE_MESSAGE_LIMIT: usize = 24;
const CAPTURE_MAX_TURNS: usize = 5;
/// Immediate flush limit after queueing one envelope (v1 parity).
const CAPTURE_FLUSH_LIMIT: usize = 10;
/// Periodic queue-flush cadence and per-scope limit (v1 parity).
const TRACE_QUEUE_WORKER_INTERVAL: Duration = Duration::from_secs(300);
const TRACE_QUEUE_WORKER_FLUSH_LIMIT: usize = 25;

/// Scopes whose queues the periodic worker flushes. Seeded with the runtime
/// owner and extended with every scope seen at capture time. Queued items for
/// scopes not seen since boot only flush on that scope's next turn — the
/// composition layer has no user directory to enumerate (v1 lists active
/// users from its database).
pub(crate) type ObservedTraceScopes = Arc<Mutex<BTreeSet<String>>>;

/// Narrow history-read seam so tests don't have to fake the full
/// [`SessionThreadService`] surface.
#[async_trait]
pub(crate) trait TraceCaptureHistorySource: Send + Sync {
    async fn thread_history_messages(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError>;
}

struct SessionThreadHistorySource {
    thread_service: Arc<dyn SessionThreadService>,
}

#[async_trait]
impl TraceCaptureHistorySource for SessionThreadHistorySource {
    async fn thread_history_messages(
        &self,
        request: ThreadHistoryRequest,
    ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
        let history = self.thread_service.list_thread_history(request).await?;
        Ok(history.messages)
    }
}

pub(crate) struct TraceCaptureTurnEventSink {
    history: Arc<dyn TraceCaptureHistorySource>,
    observed_scopes: ObservedTraceScopes,
}

impl TraceCaptureTurnEventSink {
    pub(crate) fn new(
        thread_service: Arc<dyn SessionThreadService>,
        observed_scopes: ObservedTraceScopes,
    ) -> Self {
        Self {
            history: Arc::new(SessionThreadHistorySource { thread_service }),
            observed_scopes,
        }
    }

    #[cfg(test)]
    fn with_history_source(
        history: Arc<dyn TraceCaptureHistorySource>,
        observed_scopes: ObservedTraceScopes,
    ) -> Self {
        Self {
            history,
            observed_scopes,
        }
    }
}

#[async_trait]
impl TurnEventSink for TraceCaptureTurnEventSink {
    async fn publish(&self, event: TurnLifecycleEvent) -> Result<(), TurnError> {
        if !matches!(event.kind, TurnEventKind::Completed | TurnEventKind::Failed) {
            return Ok(());
        }
        // Never capture without an explicit owner: system/sentinel-scoped
        // turns have no contribution policy and no consent.
        let Some(owner_user_id) = event
            .owner_user_id
            .clone()
            .or_else(|| event.scope.explicit_owner_user_id().cloned())
        else {
            return Ok(());
        };
        record_observed_scope(&self.observed_scopes, owner_user_id.as_str());
        let history = Arc::clone(&self.history);
        tokio::spawn(async move {
            capture_turn_trace(history, event, owner_user_id.as_str().to_string()).await;
        });
        Ok(())
    }
}

fn record_observed_scope(observed_scopes: &ObservedTraceScopes, scope: &str) {
    let mut scopes = match observed_scopes.lock() {
        Ok(scopes) => scopes,
        Err(poisoned) => poisoned.into_inner(),
    };
    scopes.insert(scope.to_string());
}

/// One turn's best-effort capture. Errors never propagate — every exit is a
/// `debug!` line keyed by the pseudonymous contributor ref, never raw content.
pub(crate) async fn capture_turn_trace(
    history: Arc<dyn TraceCaptureHistorySource>,
    event: TurnLifecycleEvent,
    scope: String,
) {
    let scope_ref = trace::local_pseudonymous_contributor_id(&scope);
    let policy = match read_trace_policy_for_scope(Some(scope.as_str())) {
        Ok(policy) => policy,
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture could not read policy");
            return;
        }
    };
    if !policy.enabled {
        return;
    }

    let Some(messages) = load_capture_messages(&history, &event, &scope_ref).await else {
        return;
    };
    if messages.is_empty() {
        return;
    }

    let turn_failed = matches!(event.kind, TurnEventKind::Failed);
    let outcome = TraceClientHost
        .prepare_autonomous_envelope_from_messages(TraceClientAutonomousCaptureRequest {
            scope: TraceClientScope::user(scope.clone()),
            // The lifecycle event does not identify the product surface
            // (REPL/WebUI/channel) behind the turn, so the channel is the
            // honest catch-all rather than a guess.
            channel: trace::TraceChannel::Other,
            messages: &messages,
            policy: &policy,
            max_turns: CAPTURE_MAX_TURNS,
            // Reborn thread transcripts carry no structured outcome payload;
            // the lifecycle event's terminal status is authoritative.
            outcome_override: turn_failed.then_some(trace::TaskSuccess::Failure),
        })
        .await;
    match outcome {
        Ok(TraceClientAutonomousCaptureOutcome::Submit(envelope)) => {
            let trace_scope = TraceClientScope::user(scope.clone());
            if let Err(error) = TraceClientHost.queue_envelope_for_scope(&trace_scope, &envelope) {
                tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to queue envelope");
                return;
            }
            if let Err(error) = TraceClientHost
                .flush_scope_queue(&trace_scope, CAPTURE_FLUSH_LIMIT)
                .await
            {
                tracing::debug!(%error, %scope_ref, "Reborn trace queue flush failed; worker retries");
            }
        }
        Ok(TraceClientAutonomousCaptureOutcome::Held {
            submission_id,
            reason,
        }) => {
            tracing::debug!(
                %submission_id,
                %reason,
                %scope_ref,
                "Reborn trace capture held by contribution policy"
            );
        }
        Ok(TraceClientAutonomousCaptureOutcome::Skipped) => {}
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture failed to build envelope");
        }
    }
}

async fn load_capture_messages(
    history: &Arc<dyn TraceCaptureHistorySource>,
    event: &TurnLifecycleEvent,
    scope_ref: &str,
) -> Option<Vec<ConversationMessage>> {
    let Some(agent_id) = event.scope.agent_id.clone() else {
        tracing::debug!(%scope_ref, "Reborn trace capture skipped: turn scope has no agent id");
        return None;
    };
    let owner_user_id = event
        .owner_user_id
        .clone()
        .or_else(|| event.scope.explicit_owner_user_id().cloned());
    let request = ThreadHistoryRequest {
        scope: ThreadScope {
            tenant_id: event.scope.tenant_id.clone(),
            agent_id,
            project_id: event.scope.project_id.clone(),
            owner_user_id,
            mission_id: None,
        },
        thread_id: event.scope.thread_id.clone(),
    };
    match history.thread_history_messages(request).await {
        Ok(records) => Some(conversation_messages_from_records(&records)),
        Err(error) => {
            tracing::debug!(%error, %scope_ref, "Reborn trace capture could not load thread history");
            None
        }
    }
}

/// Adapt Reborn thread transcript records into the neutral conversation
/// shape the trace capture pipeline consumes. Only user/assistant text rows
/// participate; tool-result references carry refs rather than content and
/// are deferred until the capture pipeline grows a Reborn-native tool-call
/// input. Redacted and superseded rows never leave the thread store.
fn conversation_messages_from_records(records: &[ThreadMessageRecord]) -> Vec<ConversationMessage> {
    let now = Utc::now();
    let mut messages: Vec<ConversationMessage> = records
        .iter()
        .filter(|record| {
            matches!(
                record.status,
                MessageStatus::Accepted
                    | MessageStatus::Submitted
                    | MessageStatus::Finalized
                    | MessageStatus::Interrupted
            )
        })
        .filter_map(|record| {
            let role = match record.kind {
                MessageKind::User => "user",
                MessageKind::Assistant => "assistant",
                MessageKind::System
                | MessageKind::Summary
                | MessageKind::CheckpointReference
                | MessageKind::ToolResultReference
                | MessageKind::CapabilityDisplayPreview => return None,
            };
            let content = record.content.clone()?;
            if content.trim().is_empty() {
                return None;
            }
            Some(ConversationMessage {
                id: uuid::Uuid::new_v4(),
                role: role.to_string(),
                content,
                // Thread message records carry no timestamps; capture time
                // is informational only (turn started_at metadata).
                created_at: now,
            })
        })
        .collect();
    if messages.len() > CAPTURE_MESSAGE_LIMIT {
        messages = messages.split_off(messages.len() - CAPTURE_MESSAGE_LIMIT);
    }
    messages
}

pub(crate) struct TraceQueueFlushWorkerHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl TraceQueueFlushWorkerHandle {
    pub(crate) async fn shutdown(self) {
        self.cancel.cancel();
        if let Err(error) = self.handle.await {
            tracing::debug!(%error, "Reborn trace queue flush worker did not shut down cleanly");
        }
    }
}

/// Periodic queue flush, mirroring v1's 300s worker: retries envelopes whose
/// immediate flush failed (network blips, endpoint downtime) for every scope
/// observed since boot.
pub(crate) fn spawn_trace_queue_flush_worker(
    observed_scopes: ObservedTraceScopes,
) -> TraceQueueFlushWorkerHandle {
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(TRACE_QUEUE_WORKER_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // The first tick fires immediately; consume it so the first flush
        // happens one full interval after boot.
        interval.tick().await;
        loop {
            tokio::select! {
                _ = worker_cancel.cancelled() => break,
                _ = interval.tick() => {}
            }
            let scopes: Vec<String> = {
                let scopes = match observed_scopes.lock() {
                    Ok(scopes) => scopes,
                    Err(poisoned) => poisoned.into_inner(),
                };
                scopes.iter().cloned().collect()
            };
            if scopes.is_empty() {
                continue;
            }
            if let Err(error) = TraceClientHost
                .flush_queue_worker_tick(scopes, TRACE_QUEUE_WORKER_FLUSH_LIMIT)
                .await
            {
                tracing::debug!(%error, "Reborn trace queue worker tick failed");
            }
        }
    });
    TraceQueueFlushWorkerHandle { cancel, handle }
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::UserId;
    use ironclaw_threads::ThreadMessageId;
    use ironclaw_turns::{EventCursor, TurnRunId, TurnScope, TurnStatus};
    use uuid::Uuid;

    use super::*;

    struct FixedHistorySource {
        records: Vec<ThreadMessageRecord>,
    }

    #[async_trait]
    impl TraceCaptureHistorySource for FixedHistorySource {
        async fn thread_history_messages(
            &self,
            _request: ThreadHistoryRequest,
        ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
            Ok(self.records.clone())
        }
    }

    struct FailingHistorySource;

    #[async_trait]
    impl TraceCaptureHistorySource for FailingHistorySource {
        async fn thread_history_messages(
            &self,
            _request: ThreadHistoryRequest,
        ) -> Result<Vec<ThreadMessageRecord>, SessionThreadError> {
            Err(SessionThreadError::UnknownThread {
                thread_id: test_thread_id(),
            })
        }
    }

    fn record(kind: MessageKind, status: MessageStatus, content: &str) -> ThreadMessageRecord {
        ThreadMessageRecord {
            message_id: ThreadMessageId::new(),
            thread_id: test_thread_id(),
            sequence: 0,
            kind,
            status,
            actor_id: None,
            source_binding_id: None,
            reply_target_binding_id: None,
            turn_id: None,
            turn_run_id: None,
            tool_result_ref: None,
            tool_result_provider_call: None,
            content: Some(content.to_string()),
            redaction_ref: None,
        }
    }

    fn test_thread_id() -> ironclaw_host_api::ThreadId {
        ironclaw_host_api::ThreadId::new("trace-capture-test-thread").expect("thread id")
    }

    fn terminal_event(kind: TurnEventKind, owner: Option<&str>) -> TurnLifecycleEvent {
        let owner_user_id =
            owner.map(|owner| UserId::new(owner).expect("test owner user id is valid"));
        TurnLifecycleEvent {
            cursor: EventCursor::default(),
            scope: TurnScope::new_with_owner(
                ironclaw_host_api::TenantId::new("trace-capture-test-tenant").expect("tenant"),
                Some(ironclaw_host_api::AgentId::new("trace-capture-test-agent").expect("agent")),
                None,
                test_thread_id(),
                owner_user_id.clone(),
            ),
            occurred_at: None,
            owner_user_id,
            run_id: TurnRunId::new(),
            status: match kind {
                TurnEventKind::Failed => TurnStatus::Failed,
                _ => TurnStatus::Completed,
            },
            kind,
            blocked_gate: None,
            sanitized_reason: None,
        }
    }

    fn enabled_policy() -> trace::StandingTraceContributionPolicy {
        trace::StandingTraceContributionPolicy {
            enabled: true,
            // Loopback endpoint on a closed port: the immediate flush attempt
            // fails fast and locally (no external traffic), leaving the
            // envelope queued for assertion.
            ingestion_endpoint: Some("https://127.0.0.1:1/v1/traces".to_string()),
            min_submission_score: 0.0,
            require_manual_approval_when_pii_detected: false,
            auto_submit_high_value_traces: true,
            ..trace::StandingTraceContributionPolicy::default()
        }
    }

    fn unique_scope(label: &str) -> String {
        format!("reborn-trace-capture-{label}-{}", Uuid::new_v4())
    }

    fn queue_dir(scope: &str) -> std::path::PathBuf {
        trace::trace_contribution_dir_for_scope(Some(scope)).join("queue")
    }

    fn queued_entries(scope: &str) -> Vec<std::path::PathBuf> {
        std::fs::read_dir(queue_dir(scope))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|path| {
                        // Envelope entries only — exclude `.held.json` hold
                        // sidecars the flush path may write next to them.
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| {
                                name.ends_with(".json") && !name.ends_with(".held.json")
                            })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn cleanup_scope(scope: &str) {
        let dir = trace::trace_contribution_dir_for_scope(Some(scope));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn conversation_messages_keep_user_and_assistant_text_only() {
        let records = vec![
            record(MessageKind::User, MessageStatus::Accepted, "hello"),
            record(MessageKind::System, MessageStatus::Finalized, "system row"),
            record(
                MessageKind::ToolResultReference,
                MessageStatus::Finalized,
                "ref-only",
            ),
            record(MessageKind::Assistant, MessageStatus::Finalized, "hi"),
            record(MessageKind::Assistant, MessageStatus::Redacted, "redacted"),
            record(MessageKind::Assistant, MessageStatus::Superseded, "stale"),
        ];
        let messages = conversation_messages_from_records(&records);
        let roles: Vec<&str> = messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, vec!["user", "assistant"]);
        assert_eq!(messages[0].content, "hello");
        assert_eq!(messages[1].content, "hi");
    }

    #[test]
    fn conversation_messages_bound_to_recent_window() {
        let records: Vec<ThreadMessageRecord> = (0..(CAPTURE_MESSAGE_LIMIT + 10))
            .map(|i| {
                record(
                    MessageKind::User,
                    MessageStatus::Accepted,
                    &format!("message {i}"),
                )
            })
            .collect();
        let messages = conversation_messages_from_records(&records);
        assert_eq!(messages.len(), CAPTURE_MESSAGE_LIMIT);
        assert_eq!(messages[0].content, "message 10");
    }

    #[tokio::test]
    async fn capture_queues_envelope_for_enrolled_scope() {
        let scope = unique_scope("enrolled");
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");

        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![
                record(MessageKind::User, MessageStatus::Accepted, "do the thing"),
                record(MessageKind::Assistant, MessageStatus::Finalized, "done"),
            ],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;

        // No ingestion endpoint is configured, so the immediate flush fails
        // locally and the envelope must remain queued for the worker.
        let entries = queued_entries(&scope);
        assert_eq!(entries.len(), 1, "exactly one envelope queued");
        let body = std::fs::read_to_string(&entries[0]).expect("queued envelope readable");
        let envelope: serde_json::Value = serde_json::from_str(&body).expect("envelope is JSON");
        assert_eq!(envelope["outcome"]["task_success"], "success");
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn capture_marks_failed_turns_as_failure_outcome() {
        let scope = unique_scope("failed-turn");
        // auto_submit_failed_traces is on by default in enabled_policy()'s
        // base, so the failed turn is still eligible.
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");

        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![
                record(MessageKind::User, MessageStatus::Accepted, "do the thing"),
                record(
                    MessageKind::Assistant,
                    MessageStatus::Finalized,
                    "attempt output",
                ),
            ],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Failed, Some(&scope)),
            scope.clone(),
        )
        .await;

        let entries = queued_entries(&scope);
        assert_eq!(entries.len(), 1, "failed turn envelope queued");
        let body = std::fs::read_to_string(&entries[0]).expect("queued envelope readable");
        let envelope: serde_json::Value = serde_json::from_str(&body).expect("envelope is JSON");
        assert_eq!(envelope["outcome"]["task_success"], "failure");
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn capture_skips_when_policy_missing_or_disabled() {
        let scope = unique_scope("not-enrolled");
        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FixedHistorySource {
            records: vec![record(MessageKind::User, MessageStatus::Accepted, "hello")],
        });
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;
        assert!(
            !queue_dir(&scope).exists(),
            "no queue dir for non-enrolled scope"
        );
    }

    #[tokio::test]
    async fn capture_survives_history_read_failure() {
        let scope = unique_scope("history-error");
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");
        let history: Arc<dyn TraceCaptureHistorySource> = Arc::new(FailingHistorySource);
        capture_turn_trace(
            history,
            terminal_event(TurnEventKind::Completed, Some(&scope)),
            scope.clone(),
        )
        .await;
        assert!(
            !queue_dir(&scope).exists(),
            "history failure queues nothing"
        );
        cleanup_scope(&scope);
    }

    #[tokio::test]
    async fn sink_ignores_non_terminal_and_ownerless_events() {
        let scopes: ObservedTraceScopes = Arc::new(Mutex::new(BTreeSet::new()));
        let sink = TraceCaptureTurnEventSink::with_history_source(
            Arc::new(FixedHistorySource {
                records: Vec::new(),
            }),
            Arc::clone(&scopes),
        );
        sink.publish(terminal_event(TurnEventKind::Submitted, Some("someone")))
            .await
            .expect("non-terminal event accepted");
        sink.publish(terminal_event(TurnEventKind::Completed, None))
            .await
            .expect("ownerless event accepted");
        assert!(
            scopes.lock().expect("scope set lock").is_empty(),
            "neither event records a capture scope"
        );
    }

    #[tokio::test]
    async fn sink_records_scope_and_spawns_capture_for_terminal_events() {
        let scope = unique_scope("sink-spawn");
        trace::write_trace_policy_for_scope(Some(&scope), &enabled_policy()).expect("write policy");
        let scopes: ObservedTraceScopes = Arc::new(Mutex::new(BTreeSet::new()));
        let sink = TraceCaptureTurnEventSink::with_history_source(
            Arc::new(FixedHistorySource {
                records: vec![
                    record(MessageKind::User, MessageStatus::Accepted, "hello"),
                    record(MessageKind::Assistant, MessageStatus::Finalized, "hi"),
                ],
            }),
            Arc::clone(&scopes),
        );
        sink.publish(terminal_event(TurnEventKind::Completed, Some(&scope)))
            .await
            .expect("terminal event accepted");
        assert!(
            scopes.lock().expect("scope set lock").contains(&scope),
            "terminal event records the owner scope for the flush worker"
        );
        // The capture task is detached; poll briefly for the queued envelope.
        for _ in 0..100 {
            if !queued_entries(&scope).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            queued_entries(&scope).len(),
            1,
            "spawned capture queues the envelope"
        );
        cleanup_scope(&scope);
    }
}
