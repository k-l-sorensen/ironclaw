//! Assembled Reborn runtime: substrate + drivers + worker, started as one.
//!
//! This module is the "later slice" the crate-level docstring promises:
//! product-level wiring on top of the substrate facades exposed by
//! `build_reborn_services`. It is the **only** place in the workspace where
//! `ironclaw_reborn` (drivers, host factory, model gateway bridge),
//! `ironclaw_threads` (session thread service), and (under the
//! `root-llm-provider` feature) `ironclaw_llm` are composed into a running
//! agent.
//!
//! Downstream callers (the CLI, future channel adapters, e2e harnesses) reach
//! this assembly only through:
//!
//! - [`build_reborn_runtime`] — construct + start the runtime
//! - [`RebornRuntime`] — task-level handle (`new_conversation`,
//!   `send_user_message`, `shutdown`)
//!
//! They never name the underlying `TurnCoordinator`, `SessionThreadService`,
//! `LoopExitApplier`, `HostManagedModelGateway`, etc. directly. That is the
//! property that satisfies the "narrow Reborn public surface" requirement
//! pinned by `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use thiserror::Error;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
    ExtensionId, GrantConstraints, MountAlias, MountGrant, MountPermissions, MountView,
    NetworkPolicy, Principal, RuntimeKind, TenantId, ThreadId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, CapabilitySurfacePolicy,
    ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, JSON_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, SurfaceKind, TIME_CAPABILITY_ID,
    WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    HostIdentityContextBuildError, HostIdentityContextCandidate, HostIdentityContextSource,
    HostInputBatch, HostInputEnvelope, HostInputQueue, HostInputQueueError,
    TurnStateRunCancellationFactory,
};
use ironclaw_reborn::loop_driver_host::LoopCapabilityPortFactory as _;
use ironclaw_reborn::loop_exit_applier::ThreadCheckpointLoopExitEvidencePort;
use ironclaw_reborn::runtime::{
    DefaultPlannedRuntimeBuildError, DefaultPlannedRuntimeConfig, DefaultPlannedRuntimeParts,
    build_default_planned_runtime,
};
use ironclaw_reborn::turn_runner::{TurnRunnerWakeSender, TurnRunnerWorkerConfig};
use ironclaw_threads::{
    AcceptInboundMessageRequest, EnsureThreadRequest, InMemorySessionThreadService, MessageContent,
    MessageKind, MessageStatus, SessionThreadService, ThreadHistoryRequest, ThreadScope,
};
use ironclaw_trust::EffectiveTrustClass;
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, GetRunStateRequest, IdempotencyKey,
    InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryTurnStateStore,
    ReplyTargetBindingRef, RunProfileResolutionRequest, SanitizedCancelReason, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnRunId,
    TurnScope, TurnStatus,
    run_profile::{
        InMemoryLoopHostMilestoneSink, InstructionSafetyContext, LoopHostMilestoneSink,
        LoopModelBudgetAccountant, LoopModelPolicyGuard, LoopRunContext, NoOpBudgetAccountant,
        NoOpPolicyGuard, PromptMode,
    },
};

use crate::product_live_adapters::{
    ProductLiveCapabilityAuthorityResolver, ProductLiveCapabilityIo, ProductLiveModelRouteSettings,
    ProductLivePlannedRuntimeAdapterConfig, ProductLivePlannedRuntimeAdapterError,
    ProductLivePlannedRuntimeAdapters, ProductLiveVisibleCapabilityRequestConfig,
    capability_allowlist,
};
use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};
use crate::{RebornBuildError, RebornCompositionProfile, RebornServices, build_reborn_services};

#[cfg(feature = "root-llm-provider")]
use crate::runtime_input::{ResolvedRebornLlm, ResolvedRebornLlmSource};

/// Stable identifier for a Reborn CLI conversation. Wraps a `ThreadId`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConversationId(pub ThreadId);

/// Final-form assistant reply read back from the session thread service after
/// a `send_user_message` completes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantReply {
    pub conversation: ConversationId,
    pub run_id: TurnRunId,
    pub status: TurnStatus,
    pub text: Option<String>,
}

impl AssistantReply {
    /// True when a caller can treat the reply as a successful single-shot
    /// response. Recovery/failed/cancelled runs may still produce diagnostics,
    /// but they did not produce the requested assistant text.
    pub fn is_successful_final_reply(&self) -> bool {
        self.status == TurnStatus::Completed && self.text.is_some()
    }
}

/// Errors returned by `RebornRuntime` methods.
#[derive(Debug, Error)]
pub enum RebornRuntimeError {
    #[error("reborn runtime build failed: {0}")]
    Build(#[from] RebornBuildError),
    #[error("turn coordinator unavailable for assembled runtime")]
    TurnCoordinatorUnavailable,
    #[error("host runtime unavailable for assembled runtime")]
    HostRuntimeUnavailable,
    #[error("turn submission failed: {0}")]
    TurnSubmission(String),
    #[error("turn submission rejected: {reason}")]
    TurnRejected { reason: String },
    #[error("session thread service error: {0}")]
    ThreadService(String),
    #[error("turn coordinator error: {0}")]
    TurnCoordinator(String),
    #[error("run did not reach a terminal state within {timeout:?}")]
    RunTimeout { timeout: Duration },
    #[error("run cancelled by caller")]
    OperationCancelled,
    #[error("invalid scope or identifier: {reason}")]
    InvalidArgument { reason: String },
    #[cfg(feature = "root-llm-provider")]
    #[error("llm provider construction failed: {0}")]
    LlmProvider(String),
    #[error("turn-runner worker is no longer running")]
    WorkerStopped,
}

impl From<TurnError> for RebornRuntimeError {
    fn from(value: TurnError) -> Self {
        Self::TurnCoordinator(value.to_string())
    }
}

impl From<DefaultPlannedRuntimeBuildError> for RebornRuntimeError {
    fn from(value: DefaultPlannedRuntimeBuildError) -> Self {
        Self::InvalidArgument {
            reason: value.to_string(),
        }
    }
}

/// Started, running Reborn agent runtime.
///
/// `RebornRuntime` is the single user-facing handle returned by
/// [`build_reborn_runtime`]. Downstream code never reaches into the substrate
/// or worker machinery: it talks to the runtime through task-level methods.
pub struct RebornRuntime {
    services: RebornServices,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    thread_service: Arc<InMemorySessionThreadService>,
    thread_scope: ThreadScope,
    worker_handle: JoinHandle<()>,
    worker_cancel: CancellationToken,
    poll_settings: PollSettings,
    actor_user_id: UserId,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
    default_run_profile_id: String,
    wake_sender: TurnRunnerWakeSender,
    send_locks: Mutex<HashMap<ConversationId, Arc<Mutex<()>>>>,
}

impl RebornRuntime {
    /// Snapshot of the substrate facades produced by `build_reborn_services`.
    /// Exposed for diagnostics / readiness reporting; **not** for traffic.
    pub fn services(&self) -> &RebornServices {
        &self.services
    }

    /// Diagnostic id for the no-profile run profile selected by this runtime.
    pub fn default_run_profile_id(&self) -> &str {
        &self.default_run_profile_id
    }

    /// Create a fresh conversation. Returns the opaque conversation id used
    /// in subsequent `send_user_message` calls.
    ///
    /// The thread is materialized inside the session thread service so
    /// `accept_inbound_message` does not error on the first send.
    pub async fn new_conversation(&self) -> Result<ConversationId, RebornRuntimeError> {
        let thread_id =
            ThreadId::new(format!("reborn-conv-{}", Uuid::new_v4())).map_err(|reason| {
                RebornRuntimeError::InvalidArgument {
                    reason: reason.to_string(),
                }
            })?;
        self.thread_service
            .ensure_thread(EnsureThreadRequest {
                scope: self.thread_scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: self.actor_user_id.as_str().to_string(),
                title: None,
                metadata_json: None,
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;
        Ok(ConversationId(thread_id))
    }

    /// Submit a user message into the conversation, wait for the run to
    /// reach a terminal state, and return the assistant reply read back
    /// from the session thread service.
    ///
    /// Without an LLM gateway wired in (i.e. when this crate is built
    /// without the `root-llm-provider` feature or `RebornLlmConfig` is not
    /// provided), the run will fail and the returned reply will surface
    /// that failure via `status = Failed` and `text = None`.
    pub async fn send_user_message(
        &self,
        conversation: &ConversationId,
        text: &str,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        self.send_user_message_with_cancellation(conversation, text, CancellationToken::new())
            .await
    }

    /// Submit a user message with a cooperative cancellation token. If the
    /// token fires while waiting for completion, the runtime cancels the run
    /// before returning.
    pub async fn send_user_message_with_cancellation(
        &self,
        conversation: &ConversationId,
        text: &str,
        cancellation: CancellationToken,
    ) -> Result<AssistantReply, RebornRuntimeError> {
        let send_lock = self.send_lock_for(conversation).await;
        let _send_guard = send_lock.lock().await;
        if self.worker_handle.is_finished() {
            return Err(RebornRuntimeError::WorkerStopped);
        }
        let scope = self.turn_scope_for(&conversation.0);
        let accepted = self
            .thread_service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: self.thread_scope.clone(),
                thread_id: conversation.0.clone(),
                actor_id: self.actor_user_id.as_str().to_string(),
                source_binding_id: Some(self.source_binding_ref.as_str().to_string()),
                reply_target_binding_id: Some(self.reply_target_binding_ref.as_str().to_string()),
                // This task-level API does not receive an upstream stable
                // event id, so mint a best-effort unique id scoped to the
                // caller-provided source binding.
                external_event_id: Some(format!(
                    "{}:{}",
                    self.source_binding_ref.as_str(),
                    Uuid::new_v4()
                )),
                content: MessageContent::text(text.to_string()),
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;

        let accepted_message_ref = AcceptedMessageRef::new(format!("msg:{}", accepted.message_id))
            .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;
        let idempotency_key = IdempotencyKey::new(format!(
            "{}-{}",
            self.source_binding_ref.as_str(),
            Uuid::new_v4()
        ))
        .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?;

        let response = self
            .turn_coordinator
            .submit_turn(SubmitTurnRequest {
                scope: scope.clone(),
                actor: TurnActor::new(self.actor_user_id.clone()),
                accepted_message_ref,
                source_binding_ref: self.source_binding_ref.clone(),
                reply_target_binding_ref: self.reply_target_binding_ref.clone(),
                requested_run_profile: None,
                idempotency_key,
                received_at: Utc::now(),
            })
            .await?;

        let SubmitTurnResponse::Accepted { run_id, .. } = response;
        if cancellation.is_cancelled() {
            self.cancel_run(
                &scope,
                run_id,
                SanitizedCancelReason::UserRequested,
                "caller-cancel",
            )
            .await?;
            return Err(RebornRuntimeError::OperationCancelled);
        }
        self.wake_sender.wake();

        let terminal_status = self
            .wait_for_terminal(&scope, run_id, &cancellation)
            .await?;
        let assistant_text = self
            .read_latest_assistant_text(&conversation.0, run_id)
            .await?;

        Ok(AssistantReply {
            conversation: conversation.clone(),
            run_id,
            status: terminal_status,
            text: assistant_text,
        })
    }

    /// Stop the turn-runner worker. Awaits the worker task to finish before
    /// returning.
    pub async fn shutdown(self) -> Result<(), RebornRuntimeError> {
        self.worker_cancel.cancel();
        if let Err(error) = self.worker_handle.await {
            if error.is_panic() {
                tracing::error!(%error, "reborn worker task panicked during shutdown");
            } else {
                tracing::warn!(%error, "reborn worker task was cancelled during shutdown");
            }
        }
        Ok(())
    }

    fn turn_scope_for(&self, thread_id: &ThreadId) -> TurnScope {
        TurnScope::new(
            self.thread_scope.tenant_id.clone(),
            Some(self.thread_scope.agent_id.clone()),
            self.thread_scope.project_id.clone(),
            thread_id.clone(),
        )
    }

    async fn send_lock_for(&self, conversation: &ConversationId) -> Arc<Mutex<()>> {
        let mut locks = self.send_locks.lock().await;
        Arc::clone(
            locks
                .entry(conversation.clone())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    async fn wait_for_terminal(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        cancellation: &CancellationToken,
    ) -> Result<TurnStatus, RebornRuntimeError> {
        let start = std::time::Instant::now();
        loop {
            if self.worker_handle.is_finished() {
                return Err(RebornRuntimeError::WorkerStopped);
            }
            let state = self
                .turn_coordinator
                .get_run_state(GetRunStateRequest {
                    scope: scope.clone(),
                    run_id,
                })
                .await?;
            if state.status.is_terminal() {
                return Ok(state.status);
            }
            if state.status == TurnStatus::RecoveryRequired {
                // RecoveryRequired keeps the durable turn active because a
                // future recovery worker may resume it. The standalone
                // runtime has no recovery worker, so cancel it before
                // returning to release the conversation lock.
                let response = self
                    .cancel_run(
                        scope,
                        run_id,
                        SanitizedCancelReason::OperatorRequested,
                        "recovery-required-cancel",
                    )
                    .await?;
                return Ok(response.status);
            }
            if start.elapsed() > self.poll_settings.max_total {
                self.cancel_run(
                    scope,
                    run_id,
                    SanitizedCancelReason::Timeout,
                    "timeout-cancel",
                )
                .await?;
                return Err(RebornRuntimeError::RunTimeout {
                    timeout: self.poll_settings.max_total,
                });
            }
            tokio::select! {
                _ = cancellation.cancelled() => {
                    self.cancel_run(
                        scope,
                        run_id,
                        SanitizedCancelReason::UserRequested,
                        "caller-cancel",
                    )
                    .await?;
                    return Err(RebornRuntimeError::OperationCancelled);
                }
                _ = tokio::time::sleep(self.poll_settings.interval) => {}
            }
        }
    }

    async fn cancel_run(
        &self,
        scope: &TurnScope,
        run_id: TurnRunId,
        reason: SanitizedCancelReason,
        idempotency_suffix: &str,
    ) -> Result<CancelRunResponse, RebornRuntimeError> {
        let response = self
            .turn_coordinator
            .cancel_run(CancelRunRequest {
                scope: scope.clone(),
                actor: TurnActor::new(self.actor_user_id.clone()),
                run_id,
                reason,
                idempotency_key: IdempotencyKey::new(format!(
                    "{}-{}-{}",
                    self.source_binding_ref.as_str(),
                    idempotency_suffix,
                    run_id
                ))
                .map_err(|reason| RebornRuntimeError::InvalidArgument { reason })?,
            })
            .await?;
        self.wake_sender.wake();
        Ok(response)
    }

    async fn read_latest_assistant_text(
        &self,
        thread_id: &ThreadId,
        run_id: TurnRunId,
    ) -> Result<Option<String>, RebornRuntimeError> {
        let history = self
            .thread_service
            .list_thread_history(ThreadHistoryRequest {
                scope: self.thread_scope.clone(),
                thread_id: thread_id.clone(),
            })
            .await
            .map_err(|error| RebornRuntimeError::ThreadService(error.to_string()))?;
        let run_id_str = run_id.to_string();
        let reply = history
            .messages
            .into_iter()
            .rev()
            .find(|message| {
                matches!(message.kind, MessageKind::Assistant)
                    && matches!(message.status, MessageStatus::Finalized)
                    && message.turn_run_id.as_deref() == Some(run_id_str.as_str())
            })
            .and_then(|message| message.content);
        Ok(reply)
    }
}

/// Build and start a Reborn agent runtime.
///
/// On return, the turn-runner worker is already running in the background and
/// the returned `RebornRuntime` is ready to accept `send_user_message` calls.
///
/// **Currently supported profiles:** only `RebornCompositionProfile::LocalDev`
/// is wired end-to-end here; production profiles will follow in a later slice
/// (they currently return their substrate-only `RebornServices` and need
/// durable thread/checkpoint stores wired before being driven). Passing a
/// production profile returns a "not yet wired" error rather than partially
/// starting an agent.
pub async fn build_reborn_runtime(
    input: RebornRuntimeInput,
) -> Result<RebornRuntime, RebornRuntimeError> {
    let RebornRuntimeInput {
        services: services_input,
        #[cfg(feature = "root-llm-provider")]
        llm,
        runner,
        poll,
        identity,
        #[cfg(test)]
        model_gateway_override,
    } = input;

    let services_input = services_input.ok_or(RebornRuntimeError::InvalidArgument {
        reason: "RebornRuntimeInput.services is required".to_string(),
    })?;

    let profile = services_input.profile();
    if !matches!(profile, RebornCompositionProfile::LocalDev) {
        return Err(RebornRuntimeError::InvalidArgument {
            reason: format!(
                "profile={profile} is not yet wired end-to-end by build_reborn_runtime; \
                 only local-dev is supported in this slice"
            ),
        });
    }

    let owner_id = services_input.owner_id().to_string();
    let services = build_reborn_services(services_input).await?;

    // For local-dev, we synthesize substrate handles the composition root
    // owns directly. These intentionally do not flow out of the runtime
    // facade — they're an implementation detail of how the runtime stitches
    // the worker to the thread service.
    let turn_state_store = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_state_store = Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store = Arc::new(InMemoryLoopCheckpointStore::default());
    let thread_service = Arc::new(InMemorySessionThreadService::default());

    let validated_identity = validate_runtime_identity(identity)?;

    let tenant_id = validated_identity.tenant_id.clone();
    let agent_id = validated_identity.agent_id.clone();
    let actor_user_id =
        UserId::new(owner_id.clone()).map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("user id: {reason}"),
        })?;
    let thread_scope = ThreadScope {
        tenant_id,
        agent_id,
        project_id: None,
        // Keep this scope aligned with `ThreadCheckpointLoopExitEvidencePort`,
        // which reconstructs thread scope from `TurnScope` for completion
        // evidence and currently has no owner-user dimension there.
        owner_user_id: None,
        mission_id: None,
    };

    #[cfg(feature = "root-llm-provider")]
    let model_gateway = {
        #[cfg(test)]
        if let Some(gateway) = model_gateway_override {
            gateway
        } else {
            match llm {
                Some(cfg) => build_llm_gateway(cfg)?,
                None => build_stub_gateway(),
            }
        }
        #[cfg(not(test))]
        {
            match llm {
                Some(cfg) => build_llm_gateway(cfg)?,
                None => build_stub_gateway(),
            }
        }
    };
    #[cfg(not(feature = "root-llm-provider"))]
    let model_gateway = {
        #[cfg(test)]
        if let Some(gateway) = model_gateway_override {
            gateway
        } else {
            build_stub_gateway()
        }
        #[cfg(not(test))]
        {
            build_stub_gateway()
        }
    };

    let loop_exit_evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new_with_thread_scope(
        Arc::clone(&thread_service),
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::TurnStateStore>,
        Arc::clone(&loop_checkpoint_store) as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        thread_scope.clone(),
    ));
    let milestone_sink = Arc::new(InMemoryLoopHostMilestoneSink::default());
    let product_live_adapters = build_local_dev_repl_adapters(
        &services,
        Arc::clone(&turn_state_store) as Arc<dyn ironclaw_turns::TurnStateStore>,
        actor_user_id.clone(),
        milestone_sink.clone(),
    )?
    .adapters;

    let composition = build_default_planned_runtime(DefaultPlannedRuntimeParts {
        turn_state: Arc::clone(&turn_state_store),
        thread_service: Arc::clone(&thread_service),
        thread_scope: thread_scope.clone(),
        model_gateway,
        checkpoint_state_store: Arc::clone(&checkpoint_state_store)
            as Arc<dyn ironclaw_turns::CheckpointStateStore>,
        loop_checkpoint_store: Arc::clone(&loop_checkpoint_store)
            as Arc<dyn ironclaw_turns::LoopCheckpointStore>,
        milestone_sink,
        capability_factory: product_live_adapters.capability_factory,
        capability_surface_resolver: product_live_adapters.capability_surface_resolver,
        loop_exit_evidence,
        config: DefaultPlannedRuntimeConfig {
            worker: TurnRunnerWorkerConfig {
                heartbeat_interval: runner.heartbeat_interval,
                poll_interval: runner.poll_interval,
                scope_filter: None,
            },
            ..DefaultPlannedRuntimeConfig::default()
        },
        model_route_resolver: Some(product_live_adapters.model_route_resolver),
        cancellation_factory: Some(product_live_adapters.cancellation_factory),
        skill_context_source: None,
        input_queue: Some(product_live_adapters.input_queue),
        identity_context_source: product_live_adapters.identity_context_source,
        model_policy_guard: Some(product_live_adapters.model_policy_guard),
        model_budget_accountant: Some(product_live_adapters.model_budget_accountant),
        safety_context: Some(product_live_adapters.safety_context),
    })?;
    let default_run_profile_id = composition
        .run_profile_resolver
        .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
        .await
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("could not resolve default run profile: {error}"),
        })?
        .profile_id
        .as_str()
        .to_string();

    let worker_cancel = CancellationToken::new();
    let worker = Arc::clone(&composition.worker);
    let worker_cancel_clone = worker_cancel.clone();
    let worker_handle = tokio::spawn(async move {
        worker.run(worker_cancel_clone).await;
    });
    let turn_coordinator: Arc<dyn TurnCoordinator> = composition.coordinator;
    let wake_sender = composition.wake_sender;

    Ok(RebornRuntime {
        services,
        turn_coordinator,
        thread_service,
        thread_scope,
        worker_handle,
        worker_cancel,
        poll_settings: poll,
        actor_user_id,
        source_binding_ref: validated_identity.source_binding_ref,
        reply_target_binding_ref: validated_identity.reply_target_binding_ref,
        default_run_profile_id,
        wake_sender,
        send_locks: Mutex::new(HashMap::new()),
    })
}

struct ValidatedRuntimeIdentity {
    tenant_id: TenantId,
    agent_id: AgentId,
    source_binding_ref: SourceBindingRef,
    reply_target_binding_ref: ReplyTargetBindingRef,
}

fn validate_runtime_identity(
    identity: RebornRuntimeIdentity,
) -> Result<ValidatedRuntimeIdentity, RebornRuntimeError> {
    let tenant_id = TenantId::new(identity.tenant_id).map_err(|reason| {
        RebornRuntimeError::InvalidArgument {
            reason: format!("tenant id: {reason}"),
        }
    })?;
    let agent_id =
        AgentId::new(identity.agent_id).map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("agent id: {reason}"),
        })?;
    let source_binding_ref =
        SourceBindingRef::new(identity.source_binding_id).map_err(|reason| {
            RebornRuntimeError::InvalidArgument {
                reason: format!("source binding id: {reason}"),
            }
        })?;
    let reply_target_binding_ref = ReplyTargetBindingRef::new(identity.reply_target_binding_id)
        .map_err(|reason| RebornRuntimeError::InvalidArgument {
            reason: format!("reply target binding id: {reason}"),
        })?;
    Ok(ValidatedRuntimeIdentity {
        tenant_id,
        agent_id,
        source_binding_ref,
        reply_target_binding_ref,
    })
}

struct LocalDevReplAdapters {
    adapters: ProductLivePlannedRuntimeAdapters,
    #[cfg_attr(not(test), allow(dead_code))]
    capability_io: Arc<ProductLiveCapabilityIo>,
}

fn build_local_dev_repl_adapters(
    services: &RebornServices,
    turn_state_store: Arc<dyn ironclaw_turns::TurnStateStore>,
    actor_user_id: UserId,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
) -> Result<LocalDevReplAdapters, RebornRuntimeError> {
    let capability_io = Arc::new(ProductLiveCapabilityIo::default());
    let allowed_capabilities = local_dev_repl_builtin_capability_ids()?;
    let cancellation_factory = Arc::new(TurnStateRunCancellationFactory::new(turn_state_store));
    let model_routes = ProductLiveModelRouteSettings::new("local-dev", "interactive_model")
        .map_err(|error| RebornRuntimeError::InvalidArgument {
            reason: format!("local-dev model routes: {error}"),
        })?;
    let safety_context = InstructionSafetyContext::new(
        "policy:local-dev-repl",
        "Local-dev REPL runtime. Use host-managed tools only through scoped grants.",
    )
    .map_err(|error| RebornRuntimeError::InvalidArgument {
        reason: format!("local-dev safety context: {error}"),
    })?;

    let adapters = ProductLivePlannedRuntimeAdapters::from_services(
        services,
        ProductLivePlannedRuntimeAdapterConfig {
            capability_authority_resolver: Arc::new(LocalDevReplCapabilityAuthorityResolver {
                user_id: actor_user_id,
            }),
            capability_input_resolver: capability_io.clone(),
            capability_result_writer: capability_io.clone(),
            capability_allow_set: capability_allowlist(allowed_capabilities),
            model_routes,
            cancellation_factory,
            input_queue: Arc::new(EmptyInputQueue),
            identity_context_source: Arc::new(EmptyIdentityContextSource),
            model_policy_guard: Arc::new(NoOpPolicyGuard) as Arc<dyn LoopModelPolicyGuard>,
            model_budget_accountant: Arc::new(NoOpBudgetAccountant)
                as Arc<dyn LoopModelBudgetAccountant>,
            safety_context,
            milestone_sink: Some(milestone_sink),
        },
    )
    .map_err(|error| RebornRuntimeError::InvalidArgument {
        reason: format!("local-dev product-live adapters: {error}"),
    })?;
    Ok(LocalDevReplAdapters {
        adapters,
        capability_io,
    })
}

fn local_dev_repl_builtin_capability_ids() -> Result<Vec<CapabilityId>, RebornRuntimeError> {
    local_dev_repl_builtin_capability_names()
        .into_iter()
        .map(|name| {
            CapabilityId::new(name).map_err(|reason| RebornRuntimeError::InvalidArgument {
                reason: format!("built-in capability id {name}: {reason}"),
            })
        })
        .collect()
}

fn local_dev_repl_builtin_capability_names() -> [&'static str; 9] {
    [
        ECHO_CAPABILITY_ID,
        TIME_CAPABILITY_ID,
        JSON_CAPABILITY_ID,
        READ_FILE_CAPABILITY_ID,
        WRITE_FILE_CAPABILITY_ID,
        LIST_DIR_CAPABILITY_ID,
        GLOB_CAPABILITY_ID,
        GREP_CAPABILITY_ID,
        APPLY_PATCH_CAPABILITY_ID,
    ]
}

struct LocalDevReplCapabilityAuthorityResolver {
    user_id: UserId,
}

#[async_trait::async_trait]
impl ProductLiveCapabilityAuthorityResolver for LocalDevReplCapabilityAuthorityResolver {
    async fn resolve_capability_authority(
        &self,
        run_context: &LoopRunContext,
    ) -> Result<ProductLiveVisibleCapabilityRequestConfig, ProductLivePlannedRuntimeAdapterError>
    {
        let user_id = self.user_id.clone();
        let mounts = local_dev_repl_mounts()?;
        Ok(ProductLiveVisibleCapabilityRequestConfig::new(
            user_id.clone(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            SurfaceKind::new("agent_loop").map_err(|reason| {
                ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
                    reason: format!("surface kind: {reason}"),
                }
            })?,
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_mounts(mounts.clone())
        .with_grants(local_dev_repl_grants(user_id, mounts)?)
        .with_provider_trust_for_effects(
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER).map_err(|reason| {
                ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
                    reason: format!("built-in provider id: {reason}"),
                }
            })?,
            EffectiveTrustClass::user_trusted(),
            local_dev_repl_allowed_effects(),
        ))
    }
}

fn local_dev_repl_grants(
    user_id: UserId,
    mounts: MountView,
) -> Result<CapabilitySet, ProductLivePlannedRuntimeAdapterError> {
    let allowed_effects = local_dev_repl_allowed_effects();
    let grants = local_dev_repl_builtin_capability_names()
        .into_iter()
        .map(|capability| {
            Ok(CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability: CapabilityId::new(capability).map_err(|reason| {
                    ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
                        reason: format!("built-in capability id {capability}: {reason}"),
                    }
                })?,
                grantee: Principal::User(user_id.clone()),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: allowed_effects.clone(),
                    mounts: mounts.clone(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            })
        })
        .collect::<Result<Vec<_>, ProductLivePlannedRuntimeAdapterError>>()?;
    Ok(CapabilitySet { grants })
}

fn local_dev_repl_allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
    ]
}

fn local_dev_repl_mounts() -> Result<MountView, ProductLivePlannedRuntimeAdapterError> {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").map_err(|reason| {
            ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
                reason: format!("workspace mount alias: {reason}"),
            }
        })?,
        VirtualPath::new("/projects").map_err(|reason| {
            ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
                reason: format!("workspace mount target: {reason}"),
            }
        })?,
        MountPermissions::read_write(),
    )])
    .map_err(
        |error| ProductLivePlannedRuntimeAdapterError::InvalidCapabilityScope {
            reason: error.to_string(),
        },
    )
}

struct EmptyInputQueue;

#[async_trait::async_trait]
impl HostInputQueue for EmptyInputQueue {
    async fn next_after(
        &self,
        _run_id: TurnRunId,
        after: ironclaw_turns::run_profile::LoopInputCursorToken,
        _limit: usize,
    ) -> Result<HostInputBatch, HostInputQueueError> {
        Ok(HostInputBatch {
            inputs: Vec::<HostInputEnvelope>::new(),
            next_cursor: after,
        })
    }

    async fn ack_consumed(
        &self,
        _run_id: TurnRunId,
        _tokens: Vec<ironclaw_turns::run_profile::LoopInputAckToken>,
    ) -> Result<(), HostInputQueueError> {
        Ok(())
    }
}

struct EmptyIdentityContextSource;

#[async_trait::async_trait]
impl HostIdentityContextSource for EmptyIdentityContextSource {
    async fn load_identity_candidates(
        &self,
        _run_context: &LoopRunContext,
        _mode: PromptMode,
    ) -> Result<Vec<HostIdentityContextCandidate>, HostIdentityContextBuildError> {
        Ok(Vec::new())
    }
}

#[cfg(feature = "root-llm-provider")]
fn build_llm_gateway(
    llm: ResolvedRebornLlm,
) -> Result<Arc<dyn ironclaw_loop_support::HostManagedModelGateway>, RebornRuntimeError> {
    use ironclaw_llm::RegistryProviderConfig;
    use ironclaw_reborn::model_gateway::{LlmModelProfilePolicy, LlmProviderModelGateway};
    use ironclaw_turns::run_profile::ModelProfileId;

    let model = llm.model().to_string();
    let provider = match llm.source {
        ResolvedRebornLlmSource::Catalog(cfg) => {
            let protocol = parse_provider_protocol(&cfg.protocol)?;
            let registry_config = RegistryProviderConfig::generic(
                protocol,
                cfg.provider_id.clone(),
                cfg.api_key.clone(),
                cfg.base_url.clone(),
                cfg.model.clone(),
            )
            .with_extra_headers(cfg.extra_headers.clone());
            ironclaw_llm::create_registry_provider(&registry_config, cfg.request_timeout_secs)
        }
        ResolvedRebornLlmSource::RegistryProvider {
            config,
            request_timeout_secs,
        } => ironclaw_llm::create_registry_provider(&config, request_timeout_secs),
    }
    .map_err(|error| RebornRuntimeError::LlmProvider(error.to_string()))?;

    let model_profile_id = ModelProfileId::new("interactive_model").map_err(|reason| {
        RebornRuntimeError::LlmProvider(format!("invalid interactive model profile id: {reason}"))
    })?;
    let policy = LlmModelProfilePolicy::new().allow_model_profile(model_profile_id, Some(model));
    let gateway = LlmProviderModelGateway::new(provider, policy);
    Ok(Arc::new(gateway))
}

#[cfg(feature = "root-llm-provider")]
fn parse_provider_protocol(
    protocol: &str,
) -> Result<ironclaw_llm::ProviderProtocol, RebornRuntimeError> {
    use ironclaw_llm::ProviderProtocol;

    match protocol {
        "open_ai_completions" | "openai_completions" | "openai" => {
            Ok(ProviderProtocol::OpenAiCompletions)
        }
        "anthropic" => Ok(ProviderProtocol::Anthropic),
        "ollama" => Ok(ProviderProtocol::Ollama),
        "github_copilot" => Ok(ProviderProtocol::GithubCopilot),
        "deep_seek" | "deepseek" => Ok(ProviderProtocol::DeepSeek),
        "gemini" => Ok(ProviderProtocol::Gemini),
        "open_router" | "openrouter" => Ok(ProviderProtocol::OpenRouter),
        _ => Err(RebornRuntimeError::LlmProvider(format!(
            "unsupported llm protocol: {protocol}"
        ))),
    }
}

fn build_stub_gateway() -> Arc<dyn ironclaw_loop_support::HostManagedModelGateway> {
    use async_trait::async_trait;
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
        HostManagedModelRequest, HostManagedModelResponse,
    };

    #[derive(Debug, Default)]
    struct StubGateway;

    #[async_trait]
    impl HostManagedModelGateway for StubGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "no LLM gateway wired (build with `root-llm-provider` feature)",
            ))
        }
    }

    Arc::new(StubGateway)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use ironclaw_host_api::{AgentId, CapabilityId, TenantId, ThreadId};
    use ironclaw_host_runtime::ECHO_CAPABILITY_ID;
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
        HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
    };
    use ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver;
    use ironclaw_turns::{
        InMemoryTurnStateStore, RunProfileResolutionRequest, RunProfileResolver, TurnId, TurnRunId,
        TurnScope, TurnStatus,
        run_profile::{
            CapabilityInvocation, CapabilityOutcome, LoopCapabilityPort, ProviderToolCall,
            VisibleCapabilityRequest,
        },
    };

    use crate::build_reborn_services;
    use crate::input::RebornBuildInput;
    use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};

    use super::{build_local_dev_repl_adapters, build_reborn_runtime};

    #[derive(Debug)]
    struct RecordingGateway {
        reply: String,
        requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
    }

    #[async_trait]
    impl HostManagedModelGateway for RecordingGateway {
        async fn stream_model(
            &self,
            request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            self.requests
                .lock()
                .expect("recording gateway requests lock poisoned")
                .push(request);
            Ok(HostManagedModelResponse::assistant_reply(
                self.reply.clone(),
            ))
        }
    }

    #[derive(Debug)]
    struct ScriptedToolGateway {
        requests: Arc<StdMutex<Vec<HostManagedModelRequest>>>,
    }

    #[async_trait]
    impl HostManagedModelGateway for ScriptedToolGateway {
        async fn stream_model(
            &self,
            _request: HostManagedModelRequest,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            Err(HostManagedModelError::safe(
                HostManagedModelErrorKind::Unavailable,
                "scripted tool gateway expected capability-aware model calls",
            ))
        }

        async fn stream_model_with_capabilities(
            &self,
            request: HostManagedModelRequest,
            capabilities: Arc<dyn LoopCapabilityPort>,
        ) -> Result<HostManagedModelResponse, HostManagedModelError> {
            let call_index = {
                let mut requests = self
                    .requests
                    .lock()
                    .expect("scripted gateway requests lock poisoned");
                requests.push(request.clone());
                requests.len()
            };
            match call_index {
                1 => {
                    assert!(request.surface_version.is_some());
                    assert!(
                        request
                            .messages
                            .iter()
                            .any(|message| message.content.contains(ECHO_CAPABILITY_ID))
                    );
                    let echo_tool = capabilities
                        .tool_definitions()
                        .map_err(model_gateway_host_error)?
                        .into_iter()
                        .find(|definition| definition.capability_id.as_str() == ECHO_CAPABILITY_ID)
                        .ok_or_else(|| {
                            HostManagedModelError::safe(
                                HostManagedModelErrorKind::InvalidRequest,
                                "builtin echo tool definition unavailable",
                            )
                        })?;
                    let call = capabilities
                        .register_provider_tool_call(ProviderToolCall {
                            provider_id: "scripted-provider".to_string(),
                            provider_model_id: "scripted-model".to_string(),
                            turn_id: Some("scripted-turn".to_string()),
                            id: "scripted-call-1".to_string(),
                            name: echo_tool.name,
                            arguments: serde_json::json!({ "message": "hello repl" }),
                            response_reasoning: None,
                            reasoning: None,
                            signature: None,
                        })
                        .await
                        .map_err(model_gateway_host_error)?;
                    Ok(HostManagedModelResponse::capability_calls(vec![call], ""))
                }
                2 => {
                    let tool_result = request
                        .messages
                        .iter()
                        .find(|message| {
                            message.role == HostManagedModelMessageRole::ToolResult
                                && message.content.contains("capability completed")
                                && message.tool_result_provider_call.is_some()
                        })
                        .expect("provider replay tool-result message");
                    let provider_call = tool_result
                        .tool_result_provider_call
                        .as_ref()
                        .expect("tool result provider call metadata");
                    assert_eq!(provider_call.provider_id, "scripted-provider");
                    assert_eq!(provider_call.provider_model_id, "scripted-model");
                    assert_eq!(provider_call.provider_turn_id, "scripted-turn");
                    assert_eq!(provider_call.provider_call_id, "scripted-call-1");
                    assert_eq!(provider_call.capability_id.as_str(), ECHO_CAPABILITY_ID);
                    assert_eq!(
                        provider_call.arguments["message"],
                        serde_json::json!("hello repl")
                    );
                    Ok(HostManagedModelResponse::assistant_reply("tool worked"))
                }
                _ => Err(HostManagedModelError::safe(
                    HostManagedModelErrorKind::InvalidRequest,
                    "scripted tool gateway received unexpected model call",
                )),
            }
        }
    }

    fn model_gateway_host_error(
        error: ironclaw_turns::run_profile::AgentLoopHostError,
    ) -> HostManagedModelError {
        HostManagedModelError::safe(HostManagedModelErrorKind::InvalidRequest, error.to_string())
    }

    #[tokio::test]
    async fn local_dev_repl_adapters_expose_and_invoke_builtin_echo() {
        let root = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "runtime-tools-owner",
            root.path().join("local-dev"),
        ))
        .await
        .expect("services build");
        let turn_state_store = Arc::new(InMemoryTurnStateStore::default());
        let adapters = build_local_dev_repl_adapters(
            &services,
            turn_state_store,
            ironclaw_host_api::UserId::new("runtime-tools-owner").expect("user id"),
            Arc::new(ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink::default()),
        )
        .expect("adapters build");
        let resolved = default_planned_run_profile_resolver()
            .expect("planned profile resolver")
            .resolve_run_profile(RunProfileResolutionRequest::interactive_default())
            .await
            .expect("planned profile");
        let run_context = ironclaw_turns::run_profile::LoopRunContext::new(
            TurnScope::new(
                TenantId::new("runtime-tools-tenant").expect("tenant id"),
                Some(AgentId::new("runtime-tools-agent").expect("agent id")),
                None,
                ThreadId::new("runtime-tools-thread").expect("thread id"),
            ),
            TurnId::new(),
            TurnRunId::new(),
            resolved,
        );
        let capability_id = CapabilityId::new(ECHO_CAPABILITY_ID).expect("echo capability id");
        let input_ref = adapters
            .capability_io
            .stage_input(&run_context, serde_json::json!({ "message": "hello repl" }))
            .expect("stage input");
        let port = adapters
            .adapters
            .capability_factory
            .create_capability_port(&run_context)
            .await
            .expect("capability port");
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible surface");
        assert!(!surface.descriptors.is_empty());
        assert!(
            surface
                .descriptors
                .iter()
                .any(|descriptor| descriptor.capability_id == capability_id)
        );

        let outcome = port
            .invoke_capability(CapabilityInvocation {
                surface_version: surface.version,
                capability_id,
                input_ref,
            })
            .await
            .expect("invoke echo");
        let CapabilityOutcome::Completed(completed) = outcome else {
            panic!("expected completed echo outcome, got {outcome:?}");
        };
        assert_eq!(
            adapters
                .capability_io
                .result_for_ref(&run_context, &completed.result_ref)
                .expect("echo result"),
            serde_json::json!("hello repl")
        );
    }

    #[tokio::test]
    async fn send_user_message_routes_model_tool_call_through_local_dev_adapters() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(ScriptedToolGateway {
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
            "runtime-tool-loop-owner",
            root.path().join("local-dev"),
        ))
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-tool-loop-tenant".to_string(),
            agent_id: "runtime-tool-loop-agent".to_string(),
            source_binding_id: "runtime-tool-loop-source".to_string(),
            reply_target_binding_id: "runtime-tool-loop-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, "use echo tool"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("tool worked"));
        assert_eq!(
            requests
                .lock()
                .expect("scripted gateway requests lock poisoned")
                .len(),
            2
        );

        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn send_user_message_returns_completed_assistant_text_with_recording_gateway() {
        let root = tempfile::tempdir().expect("tempdir");
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let gateway = Arc::new(RecordingGateway {
            reply: "recorded runtime reply".to_string(),
            requests: Arc::clone(&requests),
        });
        let input = RebornRuntimeInput::from_services(RebornBuildInput::local_dev(
            "runtime-success-owner",
            root.path().join("local-dev"),
        ))
        .with_identity(RebornRuntimeIdentity {
            tenant_id: "runtime-success-tenant".to_string(),
            agent_id: "runtime-success-agent".to_string(),
            source_binding_id: "runtime-success-source".to_string(),
            reply_target_binding_id: "runtime-success-reply".to_string(),
        })
        .with_poll_settings(PollSettings {
            interval: Duration::from_millis(10),
            max_total: Duration::from_secs(3),
        })
        .with_model_gateway_override(gateway);

        let runtime = build_reborn_runtime(input).await.expect("runtime builds");
        let conversation = runtime.new_conversation().await.expect("conversation");
        let reply = tokio::time::timeout(
            Duration::from_secs(3),
            runtime.send_user_message(&conversation, "ping"),
        )
        .await
        .expect("runtime send should finish")
        .expect("runtime send should succeed");

        assert_eq!(reply.status, TurnStatus::Completed);
        assert_eq!(reply.text.as_deref(), Some("recorded runtime reply"));
        let recorded_requests = requests
            .lock()
            .expect("recording gateway requests lock poisoned");
        assert_eq!(recorded_requests.len(), 1);
        assert!(recorded_requests[0].surface_version.is_some());
        assert!(
            recorded_requests[0]
                .messages
                .iter()
                .any(|message| message.content.contains(ECHO_CAPABILITY_ID))
        );
        drop(recorded_requests);

        runtime.shutdown().await.expect("runtime shutdown");
    }
}

#[cfg(all(test, feature = "root-llm-provider"))]
mod llm_provider_tests {
    use ironclaw_llm::ProviderProtocol;

    use super::parse_provider_protocol;

    #[test]
    fn parses_supported_provider_protocols_without_wildcard_mapping() {
        assert_eq!(
            parse_provider_protocol("open_ai_completions").unwrap(),
            ProviderProtocol::OpenAiCompletions
        );
        assert_eq!(
            parse_provider_protocol("openai_completions").unwrap(),
            ProviderProtocol::OpenAiCompletions
        );
        assert_eq!(
            parse_provider_protocol("openai").unwrap(),
            ProviderProtocol::OpenAiCompletions
        );
        assert_eq!(
            parse_provider_protocol("anthropic").unwrap(),
            ProviderProtocol::Anthropic
        );
        assert_eq!(
            parse_provider_protocol("ollama").unwrap(),
            ProviderProtocol::Ollama
        );
        assert_eq!(
            parse_provider_protocol("deep_seek").unwrap(),
            ProviderProtocol::DeepSeek
        );
        assert_eq!(
            parse_provider_protocol("deepseek").unwrap(),
            ProviderProtocol::DeepSeek
        );
        assert_eq!(
            parse_provider_protocol("gemini").unwrap(),
            ProviderProtocol::Gemini
        );
        assert_eq!(
            parse_provider_protocol("open_router").unwrap(),
            ProviderProtocol::OpenRouter
        );
        assert_eq!(
            parse_provider_protocol("openrouter").unwrap(),
            ProviderProtocol::OpenRouter
        );
        assert_eq!(
            parse_provider_protocol("github_copilot").unwrap(),
            ProviderProtocol::GithubCopilot
        );
    }

    #[test]
    fn rejects_unsupported_provider_protocol() {
        assert!(parse_provider_protocol("made_up_protocol").is_err());
    }
}
