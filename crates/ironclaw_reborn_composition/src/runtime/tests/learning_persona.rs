use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use ironclaw_host_api::runtime_policy::{
    ApprovalPolicy, AuditMode, DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind,
    NetworkMode, ProcessBackendKind, RuntimeProfile, SecretMode,
};
use ironclaw_host_api::{
    AgentId, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, CorrelationId,
    EffectKind, ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountPermissions,
    NetworkPolicy, Principal, ProjectId, ResourceScope, RuntimeKind, TenantId, TrustClass, UserId,
};
use ironclaw_host_runtime::{
    MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
    MEMORY_WRITE_CAPABILITY_ID, RuntimeFailureKind,
};
use ironclaw_loop_support::{
    HostManagedModelError, HostManagedModelErrorKind, HostManagedModelGateway,
    HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
};
use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
use ironclaw_turns::TurnStatus;
use ironclaw_turns::run_profile::{LoopCapabilityPort, ProviderToolCall, VisibleCapabilityRequest};
use serde_json::{Value, json};

use crate::input::RebornBuildInput;
use crate::runtime_input::{PollSettings, RebornRuntimeIdentity, RebornRuntimeInput};

use super::build_reborn_runtime;

const TENANT_ID: &str = "learning-ws2-tenant";
const USER_ID: &str = "learning-ws2-user";
const AGENT_ID: &str = "learning-ws2-agent";
const RUNTIME_SEND_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::test]
async fn learning_persona_scores_reports_corrects_and_recalls_learning() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let gateway = Arc::new(LearningFlowGateway::default());
    let input = runtime_input(storage_root, gateway.clone(), true);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");

    let save = send(&runtime, &conversation, "save editor preference").await;
    assert_eq!(
        save.text.as_deref(),
        Some("Saved editor preference with confidence 8.")
    );

    let recall = send(&runtime, &conversation, "recall editor preference").await;
    assert_eq!(recall.text.as_deref(), Some("Use helix (confidence 8)."));

    let correction = send(&runtime, &conversation, "correct editor preference").await;
    assert_eq!(
        correction.text.as_deref(),
        Some("Updated editor preference with confidence 9.")
    );

    let old = send(&runtime, &conversation, "check old editor preference").await;
    assert_eq!(
        old.text.as_deref(),
        Some("Old editor preference is unreachable.")
    );

    let new_value = send(
        &runtime,
        &conversation,
        "recall corrected editor preference",
    )
    .await;
    assert_eq!(
        new_value.text.as_deref(),
        Some("Use kakoune (confidence 9).")
    );

    assert!(
        gateway
            .recorded_requests()
            .iter()
            .any(request_has_learning_persona),
        "learning-enabled runtime must inject the learning persona"
    );
    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn learn_management_surface_uses_memory_tools_and_redacts_export() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let gateway = Arc::new(LearnManagementGateway::default());
    let input = runtime_input(storage_root, gateway.clone(), true);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    seed_management_learnings(&runtime).await;
    let conversation = runtime.new_conversation().await.expect("conversation");

    let search = send(&runtime, &conversation, "/learn search management marker").await;
    assert_eq!(
        search.text.as_deref(),
        Some("/learn search found learning metadata.")
    );

    let stats = send(&runtime, &conversation, "/learn stats").await;
    assert_eq!(
        stats.text.as_deref(),
        Some("/learn stats counted confidence buckets.")
    );

    let prune = send(&runtime, &conversation, "/learn prune").await;
    assert_eq!(
        prune.text.as_deref(),
        Some("/learn prune identified stale low-confidence candidates without deleting.")
    );
    let stale_after_prune = invoke_memory_json(
        &runtime,
        MEMORY_SEARCH_CAPABILITY_ID,
        json!({"query": "management marker stale secret", "limit": 5}),
    )
    .await
    .expect("stale learning search");
    assert_eq!(
        stale_after_prune["result_count"],
        json!(1),
        "/learn prune must not delete before an explicit user-approved change"
    );

    let export = send(&runtime, &conversation, "/learn export").await;
    assert_eq!(
        export.text.as_deref(),
        Some("/learn export returned redacted output.")
    );
    let redacted_read = invoke_memory_json(
        &runtime,
        MEMORY_READ_CAPABILITY_ID,
        json!({"path": "keyed/fact/management_secret.md"}),
    )
    .await
    .expect("redacted export read");
    let redacted_content = redacted_read["content"]
        .as_str()
        .expect("redacted content string");
    assert!(redacted_content.contains("[REDACTED - sensitive]"));
    assert!(!redacted_content.contains("hunter2"));
    assert!(!redacted_content.contains("sk-proj-management-secret"));

    assert_eq!(
        gateway.stream_model_fallback_calls(),
        0,
        "learning management must use the capability-aware model path"
    );
    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn fp_learning_loop_exact_dismissal_does_not_generalize() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let gateway = Arc::new(FalsePositiveGateway::default());
    let input = runtime_input(storage_root, gateway, true);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");

    let dismiss = send(
        &runtime,
        &conversation,
        "dismiss false positive RUST-LINT-123",
    )
    .await;
    assert_eq!(
        dismiss.text.as_deref(),
        Some("Dismissed exact false positive.")
    );

    let exact = send(&runtime, &conversation, "should flag RUST-LINT-123").await;
    assert_eq!(
        exact.text.as_deref(),
        Some("Not flagging exact dismissed pattern.")
    );

    let different = send(&runtime, &conversation, "should flag RUST-LINT-456").await;
    assert_eq!(
        different.text.as_deref(),
        Some("No dismissal for different pattern.")
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
async fn learning_flag_off_omits_persona_and_does_not_trigger_learning_write() {
    let root = tempfile::tempdir().expect("tempdir");
    let storage_root = root.path().join("local-dev");
    let gateway = Arc::new(DisabledLearningGateway::default());
    let input = runtime_input(storage_root, gateway.clone(), false);

    let runtime = build_reborn_runtime(input).await.expect("runtime builds");
    let conversation = runtime.new_conversation().await.expect("conversation");

    let reply = send(&runtime, &conversation, "save disabled learning marker").await;
    assert_eq!(reply.text.as_deref(), Some("Learning is disabled."));
    assert!(
        gateway
            .recorded_requests()
            .iter()
            .all(|request| !request_has_learning_persona(request)),
        "learning-disabled runtime must not inject the learning persona"
    );

    let search = invoke_memory_json(
        &runtime,
        MEMORY_SEARCH_CAPABILITY_ID,
        json!({"query": "disabled learning marker", "limit": 5}),
    )
    .await
    .expect("disabled learning search");
    assert_eq!(
        search["result_count"],
        json!(0),
        "flag-off flow must not write learning memory"
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[derive(Debug, Default)]
struct LearningFlowGateway {
    requests: StdMutex<Vec<HostManagedModelRequest>>,
    stream_model_fallback_calls: StdMutex<usize>,
}

#[async_trait]
impl HostManagedModelGateway for LearningFlowGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request);
        self.record_fallback();
        Err(model_error("expected capability-aware learning flow"))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request.clone());
        assert!(request_has_learning_persona(&request));
        let user = latest_user_message(&request)?;
        if let Some(tool_result) = latest_tool_result(&request) {
            return Ok(learning_flow_reply(&user, tool_result));
        }

        let (capability_id, arguments) = match user.as_str() {
            "save editor preference" => (
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "content": "learning marker editor preference use helix",
                    "key": "editor_preference",
                    "category": "preference",
                    "confidence": 8,
                    "created_at": Utc::now().to_rfc3339(),
                    "source": "user_correction"
                }),
            ),
            "recall editor preference" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "learning marker editor preference", "limit": 5}),
            ),
            "correct editor preference" => (
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "content": "learning marker editor preference use kakoune corrected_unique",
                    "key": "editor_preference",
                    "category": "preference",
                    "confidence": 9,
                    "created_at": Utc::now().to_rfc3339(),
                    "source": "user_correction"
                }),
            ),
            "check old editor preference" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "use helix", "limit": 5}),
            ),
            "recall corrected editor preference" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "corrected_unique", "limit": 5}),
            ),
            other => {
                return Err(model_error(format!(
                    "unexpected learning flow user message: {other}"
                )));
            }
        };
        let call_id = test_call_id("learning-flow", &user);
        capability_response(capabilities, capability_id, &call_id, arguments).await
    }
}

impl LearningFlowGateway {
    fn record_request(&self, request: HostManagedModelRequest) {
        self.requests
            .lock()
            .expect("learning flow request lock")
            .push(request);
    }

    fn record_fallback(&self) {
        *self
            .stream_model_fallback_calls
            .lock()
            .expect("learning flow fallback lock") += 1;
    }

    fn recorded_requests(&self) -> Vec<HostManagedModelRequest> {
        self.requests
            .lock()
            .expect("learning flow request lock")
            .clone()
    }
}

fn learning_flow_reply(user: &str, tool_result: &str) -> HostManagedModelResponse {
    match user {
        "save editor preference" => {
            assert!(tool_result.contains("written"));
            HostManagedModelResponse::assistant_reply("Saved editor preference with confidence 8.")
        }
        "recall editor preference" => {
            assert!(tool_result.contains("helix"), "tool result: {tool_result}");
            assert!(
                tool_result.contains("\"confidence\":8")
                    || tool_result.contains("\"confidence\": 8")
                    || tool_result.contains("confidence 8"),
                "tool result: {tool_result}"
            );
            HostManagedModelResponse::assistant_reply("Use helix (confidence 8).")
        }
        "correct editor preference" => {
            assert!(tool_result.contains("written"));
            HostManagedModelResponse::assistant_reply(
                "Updated editor preference with confidence 9.",
            )
        }
        "check old editor preference" => {
            assert!(
                tool_result.contains("\"result_count\":0")
                    || tool_result.contains("\"result_count\": 0")
                    || tool_result.contains("result_count 0"),
                "tool result: {tool_result}"
            );
            HostManagedModelResponse::assistant_reply("Old editor preference is unreachable.")
        }
        "recall corrected editor preference" => {
            assert!(
                tool_result.contains("kakoune"),
                "tool result: {tool_result}"
            );
            assert!(
                tool_result.contains("\"confidence\":9")
                    || tool_result.contains("\"confidence\": 9")
                    || tool_result.contains("confidence 9"),
                "tool result: {tool_result}"
            );
            HostManagedModelResponse::assistant_reply("Use kakoune (confidence 9).")
        }
        other => HostManagedModelResponse::assistant_reply(format!("unexpected user: {other}")),
    }
}

#[derive(Debug, Default)]
struct LearnManagementGateway {
    requests: StdMutex<Vec<HostManagedModelRequest>>,
    stream_model_fallback_calls: StdMutex<usize>,
}

#[async_trait]
impl HostManagedModelGateway for LearnManagementGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request);
        *self
            .stream_model_fallback_calls
            .lock()
            .expect("learn management fallback lock") += 1;
        Err(model_error(
            "expected capability-aware learn management flow",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request.clone());
        assert!(request_has_learning_persona(&request));
        let user = latest_user_message(&request)?;
        if let Some(tool_message) = latest_tool_result_message(&request) {
            let call_id = tool_message
                .tool_result_provider_call
                .as_ref()
                .map(|provider_call| provider_call.provider_call_id.as_str());
            return learn_management_after_tool(
                user.as_str(),
                call_id,
                tool_message.content.as_str(),
                capabilities,
            )
            .await;
        }

        let (capability_id, arguments) = match user.as_str() {
            "/learn search management marker" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "management marker", "limit": 10}),
            ),
            "/learn stats" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "management marker", "limit": 10}),
            ),
            "/learn prune" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "management marker", "limit": 10}),
            ),
            "/learn export" => (
                MEMORY_TREE_CAPABILITY_ID,
                json!({"path": "keyed", "depth": 3}),
            ),
            other => {
                return Err(model_error(format!(
                    "unexpected learn management user message: {other}"
                )));
            }
        };
        let call_id = test_call_id("learn-management", &user);
        capability_response(capabilities, capability_id, &call_id, arguments).await
    }
}

impl LearnManagementGateway {
    fn record_request(&self, request: HostManagedModelRequest) {
        self.requests
            .lock()
            .expect("learn management request lock")
            .push(request);
    }

    fn stream_model_fallback_calls(&self) -> usize {
        *self
            .stream_model_fallback_calls
            .lock()
            .expect("learn management fallback lock")
    }
}

async fn learn_management_after_tool(
    user: &str,
    call_id: Option<&str>,
    tool_result: &str,
    capabilities: Arc<dyn LoopCapabilityPort>,
) -> Result<HostManagedModelResponse, HostManagedModelError> {
    match user {
        "/learn search management marker" => {
            assert!(
                tool_result.contains("management marker high"),
                "tool result: {tool_result}"
            );
            assert!(
                tool_result.contains("preference"),
                "tool result: {tool_result}"
            );
            assert!(
                tool_result.contains("management_high"),
                "tool result: {tool_result}"
            );
            Ok(HostManagedModelResponse::assistant_reply(
                "/learn search found learning metadata.",
            ))
        }
        "/learn stats" => {
            assert!(
                tool_result.contains("\"confidence\":9")
                    || tool_result.contains("\"confidence\": 9")
                    || tool_result.contains("confidence 9"),
                "tool result: {tool_result}"
            );
            assert!(
                tool_result.contains("\"confidence\":2")
                    || tool_result.contains("\"confidence\": 2")
                    || tool_result.contains("confidence 2"),
                "tool result: {tool_result}"
            );
            assert!(
                tool_result.contains("\"result_count\":2")
                    || tool_result.contains("\"result_count\": 2")
                    || tool_result.contains("result_count 2"),
                "tool result: {tool_result}"
            );
            Ok(HostManagedModelResponse::assistant_reply(
                "/learn stats counted confidence buckets.",
            ))
        }
        "/learn prune" => {
            assert!(
                tool_result.contains("management marker stale secret"),
                "tool result: {tool_result}"
            );
            assert!(
                tool_result.contains("\"confidence\":2")
                    || tool_result.contains("\"confidence\": 2")
                    || tool_result.contains("confidence 2"),
                "tool result: {tool_result}"
            );
            assert!(
                tool_result.contains("\"is_stale\":true")
                    || tool_result.contains("\"is_stale\": true")
                    || tool_result.contains("is_stale true"),
                "tool result: {tool_result}"
            );
            Ok(HostManagedModelResponse::assistant_reply(
                "/learn prune identified stale low-confidence candidates without deleting.",
            ))
        }
        "/learn export" if call_id != Some("learn-export-read") => {
            assert!(tool_result.contains("fact"), "tool result: {tool_result}");
            assert!(
                tool_result.contains("management_secret.md"),
                "tool result: {tool_result}"
            );
            capability_response(
                capabilities,
                MEMORY_READ_CAPABILITY_ID,
                "learn-export-read",
                json!({"path": "keyed/fact/management_secret.md"}),
            )
            .await
        }
        "/learn export" => {
            assert!(
                tool_result.contains("REDACTED - sensitive"),
                "tool result: {tool_result}"
            );
            assert!(!tool_result.contains("hunter2"));
            assert!(!tool_result.contains("sk-proj-management-secret"));
            assert!(
                tool_result.contains("\"confidence\":2")
                    || tool_result.contains("\"confidence\": 2")
                    || tool_result.contains("confidence 2"),
                "tool result: {tool_result}"
            );
            Ok(HostManagedModelResponse::assistant_reply(
                "/learn export returned redacted output.",
            ))
        }
        other => Err(model_error(format!(
            "unexpected learn management follow-up: {other}"
        ))),
    }
}

#[derive(Debug, Default)]
struct FalsePositiveGateway {
    requests: StdMutex<Vec<HostManagedModelRequest>>,
}

#[async_trait]
impl HostManagedModelGateway for FalsePositiveGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request);
        Err(model_error("expected capability-aware false-positive flow"))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request.clone());
        assert!(request_has_learning_persona(&request));
        let user = latest_user_message(&request)?;
        if let Some(tool_result) = latest_tool_result(&request) {
            return Ok(false_positive_reply(&user, tool_result));
        }

        let (capability_id, arguments) = match user.as_str() {
            "dismiss false positive RUST-LINT-123" => (
                MEMORY_WRITE_CAPABILITY_ID,
                json!({
                    "content": "fp learning marker exact dismissed pattern rust_lint_123 RUST-LINT-123",
                    "key": "rust_lint_123",
                    "category": "fp",
                    "confidence": 8,
                    "created_at": Utc::now().to_rfc3339(),
                    "source": "user_dismissal"
                }),
            ),
            "should flag RUST-LINT-123" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "rust_lint_123", "limit": 5}),
            ),
            "should flag RUST-LINT-456" => (
                MEMORY_SEARCH_CAPABILITY_ID,
                json!({"query": "rust_lint_456", "limit": 5}),
            ),
            other => {
                return Err(model_error(format!(
                    "unexpected false-positive user message: {other}"
                )));
            }
        };
        let call_id = test_call_id("false-positive", &user);
        capability_response(capabilities, capability_id, &call_id, arguments).await
    }
}

impl FalsePositiveGateway {
    fn record_request(&self, request: HostManagedModelRequest) {
        self.requests
            .lock()
            .expect("false positive request lock")
            .push(request);
    }
}

fn false_positive_reply(user: &str, tool_result: &str) -> HostManagedModelResponse {
    match user {
        "dismiss false positive RUST-LINT-123" => {
            assert!(tool_result.contains("written"));
            HostManagedModelResponse::assistant_reply("Dismissed exact false positive.")
        }
        "should flag RUST-LINT-123" => {
            assert!(tool_result.contains("fp"), "tool result: {tool_result}");
            assert!(
                tool_result.contains("RUST-LINT-123"),
                "tool result: {tool_result}"
            );
            HostManagedModelResponse::assistant_reply("Not flagging exact dismissed pattern.")
        }
        "should flag RUST-LINT-456" => {
            assert!(
                tool_result.contains("\"result_count\":0")
                    || tool_result.contains("\"result_count\": 0")
                    || tool_result.contains("result_count 0"),
                "tool result: {tool_result}"
            );
            HostManagedModelResponse::assistant_reply("No dismissal for different pattern.")
        }
        other => HostManagedModelResponse::assistant_reply(format!("unexpected user: {other}")),
    }
}

#[derive(Debug, Default)]
struct DisabledLearningGateway {
    requests: StdMutex<Vec<HostManagedModelRequest>>,
}

#[async_trait]
impl HostManagedModelGateway for DisabledLearningGateway {
    async fn stream_model(
        &self,
        request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request.clone());
        assert!(!request_has_learning_persona(&request));
        Ok(HostManagedModelResponse::assistant_reply(
            "Learning is disabled.",
        ))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.record_request(request.clone());
        assert!(!request_has_learning_persona(&request));
        Ok(HostManagedModelResponse::assistant_reply(
            "Learning is disabled.",
        ))
    }
}

impl DisabledLearningGateway {
    fn record_request(&self, request: HostManagedModelRequest) {
        self.requests
            .lock()
            .expect("disabled learning request lock")
            .push(request);
    }

    fn recorded_requests(&self) -> Vec<HostManagedModelRequest> {
        self.requests
            .lock()
            .expect("disabled learning request lock")
            .clone()
    }
}

fn test_call_id(prefix: &str, user: &str) -> String {
    let suffix = user
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    format!("{prefix}-{suffix}")
}

async fn capability_response(
    capabilities: Arc<dyn LoopCapabilityPort>,
    capability_id: &str,
    call_id: &str,
    arguments: Value,
) -> Result<HostManagedModelResponse, HostManagedModelError> {
    let capability_id = CapabilityId::new(capability_id).map_err(model_error)?;
    let surface = capabilities
        .visible_capabilities(VisibleCapabilityRequest)
        .await
        .map_err(model_error)?;
    assert!(
        surface
            .descriptors
            .iter()
            .any(|descriptor| descriptor.capability_id == capability_id),
        "expected capability {capability_id} to be visible"
    );
    let tool = capabilities
        .tool_definitions()
        .map_err(model_error)?
        .into_iter()
        .find(|definition| definition.capability_id == capability_id)
        .expect("provider tool definition");
    let candidate = capabilities
        .register_provider_tool_call(ProviderToolCall {
            provider_id: "test-provider".to_string(),
            provider_model_id: "test-model".to_string(),
            turn_id: Some("provider-turn".to_string()),
            id: call_id.to_string(),
            name: tool.name,
            arguments,
            response_reasoning: None,
            reasoning: None,
            signature: None,
        })
        .await
        .map_err(model_error)?;
    Ok(HostManagedModelResponse::capability_calls(
        vec![candidate],
        "",
    ))
}

async fn seed_management_learnings(runtime: &super::RebornRuntime) {
    invoke_memory_json(
        runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "content": "management marker high preference",
            "key": "management_high",
            "category": "preference",
            "confidence": 9,
            "created_at": Utc::now().to_rfc3339(),
            "source": "test"
        }),
    )
    .await
    .expect("seed high learning");

    invoke_memory_json(
        runtime,
        MEMORY_WRITE_CAPABILITY_ID,
        json!({
            "content": "management marker stale secret password: hunter2 OPENAI_API_KEY=sk-proj-management-secret000000",
            "key": "management_secret",
            "category": "fact",
            "confidence": 2,
            "created_at": (Utc::now() - ChronoDuration::days(500)).to_rfc3339(),
            "source": "test"
        }),
    )
    .await
    .expect("seed stale secret learning");
}

async fn invoke_memory_json(
    runtime: &super::RebornRuntime,
    capability_id: &str,
    input: Value,
) -> Result<Value, RuntimeFailureKind> {
    crate::approval_test_support::invoke_json_with_local_dev_approval(
        runtime.services(),
        capability_id,
        memory_context(capability_id),
        input,
        trust_decision(),
    )
    .await
}

fn memory_context(capability_id: &str) -> ExecutionContext {
    let capability = CapabilityId::new(capability_id).expect("valid capability id");
    let extension_id = ExtensionId::new("learning-ws2-test").expect("valid extension id");
    let invocation_id = InvocationId::new();
    let tenant_id = TenantId::new(TENANT_ID).expect("valid tenant id");
    let user_id = UserId::new(USER_ID).expect("valid user id");
    let agent_id = AgentId::new(AGENT_ID).expect("valid agent id");
    let project_id: Option<ProjectId> = None;
    let memory_mounts =
        crate::local_dev_mounts::memory_mount_view(MountPermissions::read_write_list_delete())
            .expect("memory mounts");
    let resource_scope = ResourceScope {
        tenant_id: tenant_id.clone(),
        user_id: user_id.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id,
    };
    let context = ExecutionContext {
        invocation_id,
        correlation_id: CorrelationId::new(),
        process_id: None,
        parent_process_id: None,
        tenant_id,
        user_id,
        agent_id: Some(agent_id),
        project_id,
        mission_id: None,
        thread_id: None,
        extension_id: extension_id.clone(),
        runtime: RuntimeKind::FirstParty,
        trust: TrustClass::UserTrusted,
        grants: CapabilitySet {
            grants: vec![CapabilityGrant {
                id: CapabilityGrantId::new(),
                capability,
                grantee: Principal::Extension(extension_id),
                issued_by: Principal::HostRuntime,
                constraints: GrantConstraints {
                    allowed_effects: allowed_effects(),
                    mounts: memory_mounts.clone(),
                    network: NetworkPolicy::default(),
                    secrets: Vec::new(),
                    resource_ceiling: None,
                    expires_at: None,
                    max_invocations: None,
                },
            }],
        },
        mounts: memory_mounts,
        resource_scope,
    };
    context.validate().expect("valid execution context");
    context
}

fn runtime_input(
    storage_root: std::path::PathBuf,
    gateway: Arc<dyn HostManagedModelGateway>,
    learning_enabled: bool,
) -> RebornRuntimeInput {
    RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev_with_profile(
            crate::RebornCompositionProfile::LocalDevYolo,
            USER_ID,
            storage_root.clone(),
        )
        .with_runtime_policy(local_yolo_runtime_policy()),
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: TENANT_ID.to_string(),
        agent_id: AGENT_ID.to_string(),
        source_binding_id: "learning-ws2-source".to_string(),
        reply_target_binding_id: "learning-ws2-reply".to_string(),
    })
    .with_poll_settings(PollSettings {
        interval: Duration::from_millis(10),
        max_total: RUNTIME_SEND_TIMEOUT,
    })
    .with_boot_config(learning_boot(&storage_root, learning_enabled))
    .with_model_gateway_override(gateway)
}

fn local_yolo_runtime_policy() -> EffectiveRuntimePolicy {
    let mut policy =
        crate::local_dev_yolo_runtime_policy(true).expect("local-yolo policy resolves");
    policy.deployment = DeploymentMode::LocalSingleUser;
    policy.requested_profile = RuntimeProfile::LocalYolo;
    policy.resolved_profile = RuntimeProfile::LocalYolo;
    policy.filesystem_backend = FilesystemBackendKind::HostWorkspace;
    policy.process_backend = ProcessBackendKind::LocalHost;
    policy.network_mode = NetworkMode::DirectLogged;
    policy.secret_mode = SecretMode::ScrubbedEnv;
    policy.approval_policy = ApprovalPolicy::Minimal;
    policy.audit_mode = AuditMode::LocalMinimal;
    policy
}

fn learning_boot(storage_root: &std::path::Path, learning_enabled: bool) -> RebornBootConfig {
    let home = RebornHome::resolve_from_env_parts(
        Some(storage_root.as_os_str().to_os_string()),
        None,
        None,
    )
    .expect("reborn home");
    RebornBootConfig::new_with_learning_enabled(home, RebornProfile::LocalDevYolo, learning_enabled)
}

async fn send(
    runtime: &super::RebornRuntime,
    conversation: &super::ConversationId,
    text: &str,
) -> super::AssistantReply {
    let reply = tokio::time::timeout(
        RUNTIME_SEND_TIMEOUT,
        runtime.send_user_message(conversation, text),
    )
    .await
    .expect("runtime send should finish")
    .expect("runtime send should succeed");
    assert_eq!(reply.status, TurnStatus::Completed);
    reply
}

fn request_has_learning_persona(request: &HostManagedModelRequest) -> bool {
    request.messages.iter().any(|message| {
        message.role == HostManagedModelMessageRole::System
            && message.content.contains("Reborn Learning Persona")
    })
}

fn latest_user_message(request: &HostManagedModelRequest) -> Result<String, HostManagedModelError> {
    request
        .messages
        .iter()
        .rev()
        .find(|message| message.role == HostManagedModelMessageRole::User)
        .map(|message| message.content.clone())
        .ok_or_else(|| model_error("missing latest user message"))
}

fn latest_tool_result(request: &HostManagedModelRequest) -> Option<&str> {
    latest_tool_result_message(request).map(|message| message.content.as_str())
}

fn latest_tool_result_message(
    request: &HostManagedModelRequest,
) -> Option<&ironclaw_loop_support::HostManagedModelMessage> {
    request
        .messages
        .last()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
}

fn allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::ReadFilesystem,
        EffectKind::WriteFilesystem,
    ]
}

fn trust_decision() -> TrustDecision {
    TrustDecision {
        effective_trust: EffectiveTrustClass::user_trusted(),
        authority_ceiling: AuthorityCeiling {
            allowed_effects: allowed_effects(),
            max_resource_ceiling: None,
        },
        provenance: TrustProvenance::Default,
        evaluated_at: Utc::now(),
    }
}

fn model_error(error: impl std::fmt::Display) -> HostManagedModelError {
    HostManagedModelError::safe(HostManagedModelErrorKind::InvalidRequest, error.to_string())
}
