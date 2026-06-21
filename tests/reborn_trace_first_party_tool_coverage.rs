#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::{collections::BTreeSet, sync::LazyLock, time::Duration};

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID,
    HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID,
    MEMORY_READ_CAPABILITY_ID, MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID,
    MEMORY_WRITE_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, SHELL_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID, TIME_CAPABILITY_ID, TRIGGER_CREATE_CAPABILITY_ID,
    TRIGGER_LIST_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
    builtin_first_party_package,
};
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_memory::stable_learning_document_relative_path;
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{HarnessWaitConfig, RebornBinaryE2EHarness, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const REBORN_FIRST_PARTY_E2E_COVERED_CAPABILITIES: &[&str] = &[
    ECHO_CAPABILITY_ID,
    TIME_CAPABILITY_ID,
    JSON_CAPABILITY_ID,
    HTTP_CAPABILITY_ID,
    HTTP_SAVE_CAPABILITY_ID,
    MEMORY_SEARCH_CAPABILITY_ID,
    MEMORY_WRITE_CAPABILITY_ID,
    MEMORY_READ_CAPABILITY_ID,
    MEMORY_TREE_CAPABILITY_ID,
    SHELL_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID,
    WRITE_FILE_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID,
    GLOB_CAPABILITY_ID,
    GREP_CAPABILITY_ID,
    APPLY_PATCH_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID,
    SKILL_LIST_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID,
    TRIGGER_CREATE_CAPABILITY_ID,
    TRIGGER_LIST_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID,
];

const SKILL_NAME: &str = "reborn-skill-e2e";
const IRONCLAW_LEARNING_ENABLED_ENV: &str = "IRONCLAW_LEARNING_ENABLED";

static LEARNING_ENV_LOCK: LazyLock<tokio::sync::Mutex<()>> =
    LazyLock::new(|| tokio::sync::Mutex::new(()));

struct EnvGuard {
    key: &'static str,
    previous: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: tests that mutate IRONCLAW_LEARNING_ENABLED hold
        // LEARNING_ENV_LOCK for the full mutation scope.
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, previous }
    }

    fn clear(key: &'static str) -> Self {
        let previous = std::env::var(key).ok();
        // SAFETY: tests that mutate IRONCLAW_LEARNING_ENABLED hold
        // LEARNING_ENV_LOCK for the full mutation scope.
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: guard restoration happens while the creating test still
        // holds LEARNING_ENV_LOCK.
        unsafe {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn host_runtime_tool_wait() -> HarnessWaitConfig {
    HarnessWaitConfig {
        timeout: Duration::from_secs(10),
        poll_interval: Duration::from_millis(10),
    }
}

#[test]
fn reborn_builtin_first_party_capability_e2e_coverage_is_complete() {
    let declared = builtin_first_party_package()
        .expect("built-in first-party package builds")
        .capabilities
        .into_iter()
        .map(|capability| capability.id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let covered = REBORN_FIRST_PARTY_E2E_COVERED_CAPABILITIES
        .iter()
        .map(|capability| (*capability).to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        declared, covered,
        "each built-in first-party capability must have Reborn e2e coverage"
    );
}

#[tokio::test]
async fn reborn_trace_process_first_party_tools_parity() {
    let echo = CapabilityId::new(ECHO_CAPABILITY_ID).expect("valid capability id");
    let shell = CapabilityId::new(SHELL_CAPABILITY_ID).expect("valid capability id");
    let spawn_subagent =
        CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                echo.clone(),
                "call_echo_first_party",
                serde_json::json!({"message": "reborn echo e2e"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("process tools trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_process_capabilities(
        "room-trace-process-first-party-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-process-first-party-tools",
            "exercise process first-party tools",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("process tools trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, echo);

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, echo);
    assert_eq!(results[0].output, serde_json::json!("reborn echo e2e"));

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    // The loop approval-gates shell execution; the product-live adapter e2e
    // covers direct shell execution while this test guards model-surface parity.
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content.contains(shell.as_str())),
        "shell must be advertised on the Reborn model-facing first-party surface"
    );
    // Subagent spawning is a special loop path covered by
    // tests/reborn_subagent_spawn_e2e.rs; this first-party tool trace only
    // verifies it remains advertised on the model-facing surface.
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content.contains(spawn_subagent.as_str())),
        "spawn_subagent must be advertised on the Reborn model-facing first-party surface"
    );
    assert_eq!(tool_result_count(&requests[1]), 1);
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_spawn_subagent_is_surface_text_and_structured_tool() {
    let spawn_subagent =
        CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolsThenResponse {
            capability_ids: vec![spawn_subagent.clone()],
            response: HostManagedModelResponse::assistant_reply("spawn surface parity complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_process_capabilities(
        "room-trace-spawn-subagent-surface-parity",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-spawn-subagent-surface-parity",
            "verify spawn subagent is surfaced",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("spawn surface parity complete")
        .await
        .expect("final reply");

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content.contains(spawn_subagent.as_str())),
        "spawn_subagent must be advertised in Reborn model-facing surface text"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_http_save_first_party_tool_parity() {
    let http_save = CapabilityId::new(HTTP_SAVE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                http_save.clone(),
                "call_http_save_first_party",
                serde_json::json!({
                    "url": "https://api.example.test/v1/items",
                    "save_to": "/workspace/http-save-response.json"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("http save trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-trace-http-save-first-party-tool",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-http-save-first-party-tool",
            "exercise http save first-party tool",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            host_runtime_tool_wait(),
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("http save trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, http_save);

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, http_save);
    assert_eq!(results[0].output["status"], serde_json::json!(200));
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_skill_management_first_party_tools_parity() {
    let skill_install =
        CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID).expect("valid capability id");
    let skill_list = CapabilityId::new(SKILL_LIST_CAPABILITY_ID).expect("valid capability id");
    let skill_remove = CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID).expect("valid capability id");
    let skill_content = skill_md(SKILL_NAME, "Reborn skill management e2e");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_install.clone(),
                "call_skill_install_first_party",
                serde_json::json!({
                    "name": SKILL_NAME,
                    "content": skill_content,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_list.clone(),
                "call_skill_list_after_install",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_remove.clone(),
                "call_skill_remove_first_party",
                serde_json::json!({"name": SKILL_NAME}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_list.clone(),
                "call_skill_list_after_remove",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "skill management tools trace complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_skill_management_capabilities(
        "room-trace-skill-management-first-party-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-skill-management-first-party-tools",
            "exercise skill management first-party tools",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("skill management tools trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 4);
    assert_eq!(invocations[0].capability_id, skill_install);
    assert_eq!(invocations[1].capability_id, skill_list);
    assert_eq!(invocations[2].capability_id, skill_remove);
    assert_eq!(invocations[3].capability_id, skill_list);

    let results = harness.capability_results();
    assert_eq!(results.len(), 4);
    assert_eq!(results[0].capability_id, skill_install);
    assert_eq!(results[0].output["installed"], serde_json::json!(true));
    assert_eq!(results[0].output["name"], serde_json::json!(SKILL_NAME));
    assert_skill_list_contains(&results[1].output, SKILL_NAME);
    assert_eq!(results[2].capability_id, skill_remove);
    assert_eq!(results[2].output["removed"], serde_json::json!(true));
    assert_eq!(results[2].output["name"], serde_json::json!(SKILL_NAME));
    assert_skill_list_excludes(&results[3].output, SKILL_NAME);

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 5);
    assert_eq!(tool_result_count(&requests[1]), 1);
    assert_eq!(tool_result_count(&requests[2]), 2);
    assert_eq!(tool_result_count(&requests[3]), 3);
    assert_eq!(tool_result_count(&requests[4]), 4);
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_trigger_management_first_party_tools_parity() {
    let trigger_create =
        CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID).expect("valid capability id");
    let trigger_list = CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID).expect("valid capability id");
    let trigger_remove =
        CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                trigger_create.clone(),
                "call_trigger_create_first_party",
                serde_json::json!({
                    "name": "Daily trace summary",
                    "prompt": "Summarize trace state",
                    "cron": "0 8 * * *",
                    "timezone": "UTC"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                trigger_list.clone(),
                "call_trigger_list_after_create",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                trigger_remove.clone(),
                "call_trigger_remove_first_party",
                serde_json::json!({"trigger_id": "01J00000000000000000000009"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "trigger management tools trace complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_trigger_management_capabilities(
        "room-trace-trigger-management-first-party-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-trigger-management-first-party-tools",
            "exercise trigger management first-party tools",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("trigger management tools trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[0].capability_id, trigger_create);
    assert_eq!(invocations[1].capability_id, trigger_list);
    assert_eq!(invocations[2].capability_id, trigger_remove);

    let results = harness.capability_results();
    assert_eq!(results.len(), 3);
    let trigger_id = results[0].output["trigger"]["trigger_id"]
        .as_str()
        .expect("created trigger id");
    assert_eq!(
        results[0].output["trigger"]["name"],
        serde_json::json!("Daily trace summary")
    );
    assert_eq!(results[1].capability_id, trigger_list);
    assert_eq!(
        results[1].output["triggers"][0]["trigger_id"],
        serde_json::json!(trigger_id)
    );
    assert_eq!(results[2].capability_id, trigger_remove);
    assert_eq!(results[2].output["removed"], serde_json::json!(false));

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(tool_result_count(&requests[1]), 1);
    assert_eq!(tool_result_count(&requests[2]), 2);
    assert_eq!(tool_result_count(&requests[3]), 3);
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_memory_first_party_tools_parity() {
    let memory_write = CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID).expect("valid capability id");
    let memory_read = CapabilityId::new(MEMORY_READ_CAPABILITY_ID).expect("valid capability id");
    let memory_search =
        CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID).expect("valid capability id");
    let memory_tree = CapabilityId::new(MEMORY_TREE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_write.clone(),
                "call_memory_write_first_party",
                serde_json::json!({
                    "target": "projects/alpha/notes.md",
                    "content": "Reborn memory e2e marker for capability search.",
                    "append": false
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_read.clone(),
                "call_memory_read_first_party",
                serde_json::json!({"path": "projects/alpha/notes.md"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_tree.clone(),
                "call_memory_tree_first_party",
                serde_json::json!({"path": "", "depth": 3}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_search.clone(),
                "call_memory_search_first_party",
                serde_json::json!({"query": "capability search marker", "limit": 5}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("memory tools trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-trace-memory-first-party-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-memory-first-party-tools",
            "exercise memory first-party tools",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            host_runtime_tool_wait(),
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("memory tools trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 4);
    assert_eq!(invocations[0].capability_id, memory_write);
    assert_eq!(invocations[1].capability_id, memory_read);
    assert_eq!(invocations[2].capability_id, memory_tree);
    assert_eq!(invocations[3].capability_id, memory_search);

    let results = harness.capability_results();
    assert_eq!(results.len(), 4);
    assert_eq!(results[0].output["status"], serde_json::json!("written"));
    assert!(
        results[1].output["content"]
            .as_str()
            .expect("memory_read content")
            .contains("Reborn memory e2e marker")
    );
    assert!(
        results[2].output.to_string().contains("alpha/"),
        "memory_tree should include alpha directory"
    );
    assert_eq!(results[3].output["result_count"], serde_json::json!(1));
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_memory_learning_schema_is_hidden_when_disabled() {
    let _lock = LEARNING_ENV_LOCK.lock().await;
    let _learning_env = EnvGuard::clear(IRONCLAW_LEARNING_ENABLED_ENV);
    let memory_write = CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolSchemaThenResponse {
            capability_id: memory_write.clone(),
            required_properties: vec!["content", "target", "append", "metadata"],
            forbidden_properties: vec!["key", "category", "confidence", "created_at", "source"],
            response: HostManagedModelResponse::assistant_reply(
                "memory learning disabled schema complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-trace-memory-learning-disabled-schema",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-memory-learning-disabled-schema",
            "verify learning memory schema is disabled",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            host_runtime_tool_wait(),
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("memory learning disabled schema complete")
        .await
        .expect("final reply");

    assert!(
        harness.capability_invocations().is_empty(),
        "schema-only check must not invoke memory_write"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_memory_learning_schema_is_visible_when_enabled() {
    let _lock = LEARNING_ENV_LOCK.lock().await;
    let _learning_env = EnvGuard::set(IRONCLAW_LEARNING_ENABLED_ENV, "true");
    let memory_write = CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolSchemaThenResponse {
            capability_id: memory_write.clone(),
            required_properties: vec![
                "content",
                "target",
                "append",
                "metadata",
                "key",
                "category",
                "confidence",
                "created_at",
                "source",
            ],
            forbidden_properties: Vec::new(),
            response: HostManagedModelResponse::assistant_reply(
                "memory learning enabled schema complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-trace-memory-learning-enabled-schema",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-memory-learning-enabled-schema",
            "verify learning memory schema is enabled",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            host_runtime_tool_wait(),
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("memory learning enabled schema complete")
        .await
        .expect("final reply");

    assert!(
        harness.capability_invocations().is_empty(),
        "schema-only check must not invoke memory_write"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_memory_learning_keyed_write_search_read_e2e() {
    let _lock = LEARNING_ENV_LOCK.lock().await;
    let _learning_env = EnvGuard::set(IRONCLAW_LEARNING_ENABLED_ENV, "true");
    let memory_write = CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID).expect("valid capability id");
    let memory_read = CapabilityId::new(MEMORY_READ_CAPABILITY_ID).expect("valid capability id");
    let memory_search =
        CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID).expect("valid capability id");
    let learning_key = "editor preference";
    let learning_category = "preference";
    let expected_path = stable_learning_document_relative_path(learning_category, learning_key)
        .expect("stable learning path");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_write.clone(),
                "call_memory_learning_write_old",
                serde_json::json!({
                    "key": learning_key,
                    "category": learning_category,
                    "confidence": 2,
                    "created_at": "2026-06-14T00:00:00Z",
                    "source": "reborn-e2e",
                    "content": "learning e2e old_marker prefers nano"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_write.clone(),
                "call_memory_learning_write_new",
                serde_json::json!({
                    "key": learning_key,
                    "category": learning_category,
                    "confidence": 9,
                    "created_at": "2026-06-14T00:01:00Z",
                    "source": "reborn-e2e",
                    "content": "learning e2e new_marker prefers helix region=us-east-1"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_search.clone(),
                "call_memory_learning_search_old",
                serde_json::json!({"query": "learning e2e old_marker", "limit": 5}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_search.clone(),
                "call_memory_learning_search_new",
                serde_json::json!({"query": "learning e2e new_marker", "limit": 5}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                memory_read.clone(),
                "call_memory_learning_read",
                serde_json::json!({"path": expected_path}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "memory learning e2e trace complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-trace-memory-learning-keyed-e2e",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-memory-learning-keyed-e2e",
            "exercise learning memory keyed write search read",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status_with_config(
            submitted.run_id,
            TurnStatus::Completed,
            host_runtime_tool_wait(),
        )
        .await
        .expect("completed run");
    harness
        .assert_final_reply("memory learning e2e trace complete")
        .await
        .expect("final reply");

    let results = harness.capability_results();
    assert_eq!(results.len(), 5, "results: {results:?}");
    assert_eq!(results[0].capability_id, memory_write);
    assert_eq!(results[1].capability_id, memory_write);
    assert_eq!(results[2].capability_id, memory_search);
    assert_eq!(results[3].capability_id, memory_search);
    assert_eq!(results[4].capability_id, memory_read);
    assert_eq!(results[0].output["path"], serde_json::json!(expected_path));
    assert_eq!(results[1].output["path"], serde_json::json!(expected_path));
    assert_eq!(results[1].output["append"], serde_json::json!(false));
    let path = results[1].output["path"].as_str().expect("written path");
    assert_stable_learning_path_is_hashed_and_secret_free(path);

    assert_eq!(results[2].output["result_count"], serde_json::json!(0));
    assert_eq!(results[3].output["result_count"], serde_json::json!(1));
    let search_result = &results[3].output["results"][0];
    assert_eq!(search_result["confidence"], serde_json::json!(9));
    assert_eq!(search_result["is_stale"], serde_json::json!(false));
    let search_content = search_result["content"]
        .as_str()
        .expect("search result content");
    assert!(!search_content.contains("old_marker"));
    assert!(search_content.contains("new_marker"));
    assert!(search_content.contains("region=us-east-1"));
    assert_eq!(search_result["key"], serde_json::json!(learning_key));
    assert_eq!(
        search_result["category"],
        serde_json::json!(learning_category)
    );
    assert_eq!(search_result["source"], serde_json::json!("reborn-e2e"));
    assert_eq!(
        search_result["created_at"],
        serde_json::json!("2026-06-14T00:01:00Z")
    );

    let read_content = results[4].output["content"]
        .as_str()
        .expect("memory_read content");
    assert!(!read_content.contains("old_marker"));
    assert!(read_content.contains("new_marker"));
    assert!(read_content.contains("region=us-east-1"));
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\nSkill body for {name}.\n")
}

fn tool_result_count(request: &ironclaw_loop_support::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}

fn assert_skill_list_contains(output: &serde_json::Value, expected: &str) {
    assert!(
        skill_names(output).contains(&expected),
        "expected skill list to include {expected:?}, got {output:?}"
    );
}

fn assert_skill_list_excludes(output: &serde_json::Value, unexpected: &str) {
    assert!(
        skill_names(output).iter().all(|name| *name != unexpected),
        "expected skill list to exclude {unexpected:?}, got {output:?}"
    );
}

fn skill_names(output: &serde_json::Value) -> Vec<&str> {
    output["skills"]
        .as_array()
        .expect("skill list output should contain skills array")
        .iter()
        .filter_map(|skill| skill["name"].as_str())
        .collect()
}

fn assert_stable_learning_path_is_hashed_and_secret_free(path: &str) {
    let Some(rest) = path.strip_prefix("keyed/") else {
        panic!("stable learning path must use keyed prefix: {path}");
    };
    let Some((category_hash, key_file_name)) = rest.split_once('/') else {
        panic!("stable learning path must include category and key hash segments: {path}");
    };
    let Some(key_hash) = key_file_name.strip_suffix(".md") else {
        panic!("stable learning key hash must end in .md: {path}");
    };
    assert_lowercase_sha256(category_hash);
    assert_lowercase_sha256(key_hash);
    for leaked in [
        "editor",
        "preference",
        "swordfish",
        "hunter2",
        "postgres",
        "sk-proj-test",
        "supersecret",
    ] {
        assert!(
            !path.contains(leaked),
            "stable learning path leaked raw secret fragment {leaked}: {path}"
        );
    }
}

fn assert_lowercase_sha256(value: &str) {
    assert_eq!(value.len(), 64, "expected sha256 hex segment: {value}");
    assert!(
        value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
        "expected lowercase sha256 hex segment: {value}"
    );
}
