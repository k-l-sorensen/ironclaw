//! Mistral AI provider with first-class `reasoning_effort` support.
//!
//! IronClaw owns the Mistral request/response JSON here (rather than delegating
//! to rig-core) because Mistral's `reasoning_effort=high` response models
//! `message.content` as an **array of typed chunks**
//! (`[{type:"thinking",…},{type:"text",…}]`) instead of a string. rig-core's
//! OpenAI-compat client — and rig 0.39's dedicated Mistral client, which still
//! models assistant content as `String` — cannot deserialize that shape, so the
//! agent loop failed every turn with
//! `JsonError: did not match any variant of untagged enum ApiResponse`.
//!
//! The novel logic lives in two places:
//! - **Response parse** (`extract_content`): splits the array into the thinking
//!   trace (`CompletionResponse.reasoning`) and the final answer (`.content`).
//! - **Multi-turn replay** (`chat_message_to_wire`): when an assistant
//!   `ChatMessage` carries a `reasoning` trace, the request content is rebuilt
//!   as `[{thinking:[{text}]},{text}]` so Mistral receives the prior ThinkChunk
//!   — required to avoid degraded multi-turn performance.
//!
//! Everything else (OpenAI-shape chat-completions, tool schema normalization,
//! error mapping) reuses the shared crate seams.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::config::MistralReasoningEffort;
use crate::costs;
use crate::error::LlmError;
use crate::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition, sanitize_tool_messages,
};
use crate::reasoning_models::supports_mistral_reasoning;
use crate::tool_schema::{ToolSchemaPolicy, shape_tool_schema};
use crate::vision_models::is_vision_model;

/// Provider identity used in error variants and logs.
const PROVIDER: &str = "mistral";

/// Mistral's API base URL. Hardcoded and not overridable: a user/operator
/// base-URL knob would be an SSRF / credential-redirection surface, and the
/// reasoning wire contract is Mistral-specific. Tests inject a loopback URL via
/// the `#[cfg(test)]` constructor only.
const MISTRAL_BASE_URL: &str = "https://api.mistral.ai/v1";

/// Mistral provider: owns its HTTP client and request/response JSON model.
pub struct MistralProvider {
    client: Client,
    model: String,
    api_key: SecretString,
    /// Configured `reasoning_effort` toggle. `None` omits the param entirely;
    /// `Some(_)` is further gated on model capability before sending.
    reasoning: Option<MistralReasoningEffort>,
    base_url: String,
    active_model: std::sync::RwLock<String>,
}

impl MistralProvider {
    /// Create a Mistral provider with the production base URL.
    pub fn new(
        model: impl Into<String>,
        api_key: SecretString,
        reasoning: Option<MistralReasoningEffort>,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        Self::new_with_base_url(
            model,
            api_key,
            reasoning,
            request_timeout_secs,
            MISTRAL_BASE_URL,
        )
    }

    fn new_with_base_url(
        model: impl Into<String>,
        api_key: SecretString,
        reasoning: Option<MistralReasoningEffort>,
        request_timeout_secs: u64,
        base_url: impl Into<String>,
    ) -> Result<Self, LlmError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(request_timeout_secs))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER.to_string(),
                reason: format!("Failed to build HTTP client: {e}"),
            })?;
        let model = model.into();
        let active_model = std::sync::RwLock::new(model.clone());
        Ok(Self {
            client,
            model,
            api_key,
            reasoning,
            base_url: base_url.into(),
            active_model,
        })
    }

    /// Resolve the `reasoning_effort` wire value for a given model.
    ///
    /// Two states (Decision 3): `None` omits the param; a `Some(_)` toggle is
    /// gated on model capability first (model-gate beats the toggle, so e.g.
    /// `mistral-large` never receives the param even with reasoning on). When
    /// supported, the on-state renders as `"high"`; otherwise the param is omitted.
    fn reasoning_effort_for(&self, model: &str) -> Option<&'static str> {
        match self.reasoning {
            Some(effort) if supports_mistral_reasoning(model) => Some(effort.wire_value()),
            _ => None,
        }
    }

    /// Build the typed wire request for a (possibly tool-bearing) completion.
    fn build_request(&self, params: MistralRequestParams) -> MistralChatRequest {
        let MistralRequestParams {
            model,
            mut messages,
            tools,
            temperature,
            max_tokens,
            stop,
            tool_choice,
        } = params;
        sanitize_tool_messages(&mut messages);
        let vision = is_vision_model(&model);
        let wire_messages: Vec<MistralMessage> = messages
            .into_iter()
            .map(|m| chat_message_to_wire(m, vision))
            .collect();

        let wire_tools: Vec<MistralTool> = tools.into_iter().map(convert_tool_definition).collect();
        let has_tools = !wire_tools.is_empty();
        let reasoning_effort = self.reasoning_effort_for(&model);

        MistralChatRequest {
            model,
            messages: wire_messages,
            temperature,
            max_tokens,
            stop,
            tools: has_tools.then_some(wire_tools),
            tool_choice: has_tools.then_some(tool_choice).flatten(),
            reasoning_effort,
        }
    }

    /// POST the chat-completions request and map failures to `LlmError` at the
    /// channel boundary (Decision 6). API-key auth only — no session renewal.
    async fn post_chat(&self, body: &MistralChatRequest) -> Result<MistralChatResponse, LlmError> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        if tracing::enabled!(tracing::Level::DEBUG)
            && let Ok(json) = serde_json::to_string(body)
        {
            // The Authorization header is never part of the serialized body.
            tracing::debug!(provider = PROVIDER, "Mistral request body: {json}");
        }

        let response = self
            .client
            .post(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER.to_string(),
                reason: e.to_string(),
            })?;

        let status = response.status();
        // Only `Some(_)` when the header was actually present, so 5xx retries
        // fall through to exponential backoff instead of a fixed default.
        let retry_after_header: Option<Duration> = response
            .headers()
            .get("retry-after")
            .map(crate::retry::parse_retry_after_value);
        let text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER.to_string(),
            reason: format!("Failed to read response body: {e}"),
        })?;

        if tracing::enabled!(tracing::Level::TRACE) {
            tracing::trace!(provider = PROVIDER, "Mistral response body: {text}");
        }

        if !status.is_success() {
            let code = status.as_u16();
            if code == 401 {
                return Err(LlmError::AuthFailed {
                    provider: PROVIDER.to_string(),
                });
            }
            if code == 429 {
                return Err(LlmError::RateLimited {
                    provider: PROVIDER.to_string(),
                    retry_after: retry_after_header.or(Some(Duration::from_secs(60))),
                });
            }
            if let Some(err) = crate::error::context_length_error(code, &text) {
                return Err(err);
            }
            if matches!(code, 500..=599) {
                tracing::debug!(
                    provider = PROVIDER,
                    status = code,
                    body_preview = ironclaw_common::truncate_for_preview(&text, 512).as_str(),
                    "Mistral upstream 5xx response"
                );
                return Err(LlmError::BadGateway {
                    provider: PROVIDER.to_string(),
                    status: code,
                    retry_after: retry_after_header,
                });
            }
            let truncated = ironclaw_common::truncate_for_preview(&text, 512);
            return Err(LlmError::RequestFailed {
                provider: PROVIDER.to_string(),
                reason: format!("HTTP {status}: {truncated}"),
            });
        }

        // A 2xx body that won't deserialize is a transient/upstream-shape issue,
        // not a caller bug — map to InvalidResponse (retryable), never bare Json.
        parse_json(&text)
    }

    fn first_choice(response: MistralChatResponse) -> Result<MistralChoice, LlmError> {
        response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::EmptyResponse {
                provider: PROVIDER.to_string(),
            })
    }
}

#[async_trait]
impl LlmProvider for MistralProvider {
    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        costs::model_cost(&self.active_model_name()).unwrap_or_else(costs::default_cost)
    }

    async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        let request = self.build_request(MistralRequestParams {
            model,
            messages: req.messages,
            tools: Vec::new(),
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            stop: req.stop_sequences,
            tool_choice: None,
        });
        let response = self.post_chat(&request).await?;
        let (input_tokens, output_tokens) = usage_tokens(&response.usage);
        let MistralChoice {
            message,
            finish_reason,
        } = Self::first_choice(response)?;

        let (content, reasoning) = extract_content(message.content)?;
        // For a plain completion the text chunk IS the answer; a reasoning-on
        // array with no text chunk means the answer was lost — fail loud
        // (EmptyResponse, retryable) rather than defaulting to "" (F6).
        let content = content.ok_or_else(|| LlmError::EmptyResponse {
            provider: PROVIDER.to_string(),
        })?;
        emit_reasoning_trace(reasoning.as_deref());

        Ok(CompletionResponse {
            content,
            finish_reason: finish_reason_from(finish_reason.as_deref(), false),
            input_tokens,
            output_tokens,
            reasoning,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        mut req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        let request = self.build_request(MistralRequestParams {
            model,
            messages: req.messages,
            tools: req.tools,
            temperature: req.temperature,
            max_tokens: req.max_tokens,
            stop: req.stop_sequences,
            tool_choice: req.tool_choice,
        });
        let response = self.post_chat(&request).await?;
        let (input_tokens, output_tokens) = usage_tokens(&response.usage);
        let MistralChoice {
            message,
            finish_reason,
        } = Self::first_choice(response)?;

        let tool_calls = parse_tool_calls(message.tool_calls);
        let (content, reasoning) = extract_content(message.content)?;

        // Either a textual answer or at least one tool call is required; both
        // empty means the turn produced nothing usable (fail loud).
        if content.is_none() && tool_calls.is_empty() {
            return Err(LlmError::EmptyResponse {
                provider: PROVIDER.to_string(),
            });
        }
        emit_reasoning_trace(reasoning.as_deref());

        let has_tool_calls = !tool_calls.is_empty();
        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason: finish_reason_from(finish_reason.as_deref(), has_tool_calls),
            input_tokens,
            output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
            reasoning,
        })
    }

    fn active_model_name(&self) -> String {
        match self.active_model.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        match self.active_model.write() {
            Ok(mut guard) => *guard = model.to_string(),
            Err(poisoned) => *poisoned.into_inner() = model.to_string(),
        }
        Ok(())
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .get(&url)
            .header(
                "Authorization",
                format!("Bearer {}", self.api_key.expose_secret()),
            )
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER.to_string(),
                reason: e.to_string(),
            })?;
        if !response.status().is_success() {
            return Err(LlmError::RequestFailed {
                provider: PROVIDER.to_string(),
                reason: format!("models endpoint returned HTTP {}", response.status()),
            });
        }
        let text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER.to_string(),
            reason: format!("Failed to read models response: {e}"),
        })?;
        let parsed: MistralModelsResponse = parse_json(&text)?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }
}

// ── helpers ────────────────────────────────────────────────────────────────

fn parse_json<R: DeserializeOwned>(text: &str) -> Result<R, LlmError> {
    serde_json::from_str(text).map_err(|e| {
        let truncated = ironclaw_common::truncate_for_preview(text, 512);
        LlmError::InvalidResponse {
            provider: PROVIDER.to_string(),
            reason: format!("JSON parse error: {e}. Raw: {truncated}"),
        }
    })
}

/// Split Mistral's `message.content` into `(answer, reasoning_trace)`.
///
/// - String content → `(Some(text), None)` (reasoning off).
/// - Array content → text chunk(s) concatenated as the answer, thinking
///   chunk(s) flattened into the reasoning trace.
///
/// Returns `Ok((None, _))` only when there genuinely is no text chunk; callers
/// decide whether that is an error. The thinking extraction never silently
/// defaults to `""` (F6) — a malformed array surfaces through serde as an
/// `InvalidResponse` before reaching here.
fn extract_content(
    content: Option<MistralMessageContent>,
) -> Result<(Option<String>, Option<String>), LlmError> {
    match content {
        None => Ok((None, None)),
        Some(MistralMessageContent::Text(s)) => Ok((non_empty(s), None)),
        Some(MistralMessageContent::Chunks(chunks)) => {
            let mut answer = String::new();
            let mut thinking = String::new();
            for chunk in chunks {
                match chunk {
                    MistralContentChunk::Text { text } => answer.push_str(&text),
                    MistralContentChunk::Thinking { thinking: parts } => {
                        for MistralTextChunk::Text { text } in parts {
                            thinking.push_str(&text);
                        }
                    }
                    // Assistant responses don't carry image parts; ignore any.
                    MistralContentChunk::ImageUrl { .. } => {}
                }
            }
            Ok((non_empty(answer), non_empty(thinking)))
        }
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.trim().is_empty() { None } else { Some(s) }
}

fn parse_tool_calls(calls: Option<Vec<MistralToolCall>>) -> Vec<ToolCall> {
    calls
        .unwrap_or_default()
        .into_iter()
        .map(|tc| {
            // Mistral tool-call arguments arrive as a JSON-encoded string.
            let parsed: Result<serde_json::Value, _> = serde_json::from_str(&tc.function.arguments);
            let (arguments, arguments_parse_error) = match parsed {
                Ok(value) => (value, None),
                Err(e) => (
                    serde_json::Value::Object(Default::default()),
                    Some(format!("failed to parse tool-call arguments JSON: {e}")),
                ),
            };
            ToolCall {
                id: tc.id,
                name: tc.function.name,
                arguments,
                reasoning: None,
                signature: None,
                arguments_parse_error,
            }
        })
        .collect()
}

fn finish_reason_from(raw: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match raw {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("tool_calls") => FinishReason::ToolUse,
        Some("content_filter") => FinishReason::ContentFilter,
        _ if has_tool_calls => FinishReason::ToolUse,
        _ => FinishReason::Unknown,
    }
}

fn emit_reasoning_trace(reasoning: Option<&str>) {
    if let Some(trace) = reasoning.filter(|s| !s.is_empty()) {
        tracing::trace!(target: "ironclaw_llm::reasoning", "{trace}");
    }
}

/// Convert an IronClaw `ChatMessage` into the Mistral wire message.
///
/// The reasoning-replay branch is the multi-turn requirement: when an assistant
/// message carries a `reasoning` trace, the content is rebuilt as
/// `[{thinking:[{text}]},{text}]` so Mistral receives the prior ThinkChunk.
fn chat_message_to_wire(msg: ChatMessage, vision: bool) -> MistralMessage {
    let role = match msg.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };

    let tool_calls = msg.tool_calls.map(|calls| {
        calls
            .into_iter()
            .map(|tc| MistralToolCall {
                id: tc.id,
                call_type: "function".to_string(),
                function: MistralToolCallFunction {
                    name: tc.name,
                    arguments: tc.arguments.to_string(),
                },
            })
            .collect()
    });

    let reasoning = msg.reasoning.filter(|r| !r.trim().is_empty());

    let content = if let Some(trace) = reasoning {
        // Multi-turn replay: reconstruct the full assistant message including
        // the ThinkChunk, followed by the answer text chunk.
        let mut chunks = vec![MistralContentChunk::Thinking {
            thinking: vec![MistralTextChunk::Text { text: trace }],
        }];
        if !msg.content.is_empty() {
            chunks.push(MistralContentChunk::Text { text: msg.content });
        }
        Some(MistralMessageContent::Chunks(chunks))
    } else if role == "assistant" && tool_calls.is_some() && msg.content.is_empty() {
        None
    } else if vision && !msg.content_parts.is_empty() {
        // Multimodal: forward text + image parts only when the model is
        // vision-capable; otherwise images are dropped below (text only).
        let mut chunks = vec![MistralContentChunk::Text { text: msg.content }];
        for part in msg.content_parts {
            if let crate::ContentPart::ImageUrl { mut image_url } = part {
                image_url.detail = Some(image_url.normalized_openai_detail());
                chunks.push(MistralContentChunk::ImageUrl { image_url });
            }
        }
        Some(MistralMessageContent::Chunks(chunks))
    } else {
        if !vision && !msg.content_parts.is_empty() {
            tracing::warn!(
                provider = PROVIDER,
                "Dropping image attachment(s): model is not vision-capable"
            );
        }
        Some(MistralMessageContent::Text(msg.content))
    };

    MistralMessage {
        role: role.to_string(),
        content,
        tool_call_id: msg.tool_call_id,
        name: msg.name,
        tool_calls,
    }
}

fn convert_tool_definition(tool: ToolDefinition) -> MistralTool {
    let mut description = tool.description.clone();
    // Mistral speaks the OpenAI chat-completions tool format (non-strict); use
    // the shared flatten-only policy so top-level combinators are flattened.
    let parameters = shape_tool_schema(
        ToolSchemaPolicy::FlattenOnly,
        &tool.parameters,
        &mut description,
    );
    MistralTool {
        tool_type: "function".to_string(),
        function: MistralFunction {
            name: tool.name,
            description: Some(description),
            parameters: Some(parameters),
        },
    }
}

/// Inputs to [`MistralProvider::build_request`], bundled into a struct to keep
/// the builder under clippy's argument ceiling (architecture Decision 10 —
/// prefer a params struct over `#[allow(too_many_arguments)]`).
struct MistralRequestParams {
    model: String,
    messages: Vec<ChatMessage>,
    tools: Vec<ToolDefinition>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    stop: Option<Vec<String>>,
    tool_choice: Option<String>,
}

// ── wire model ─────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct MistralChatRequest {
    model: String,
    /// `"high"` / `"none"`, or omitted entirely (model-gated off / unsupported).
    ///
    /// Declared second (right after `model`) so it appears near the start of the
    /// serialized body — the debug request-body log line is capped at 500 bytes
    /// per event, and the messages array easily exceeds that, so a trailing
    /// `reasoning_effort` would be truncated away and invisible in logs.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'static str>,
    messages: Vec<MistralMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<MistralTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MistralMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<MistralMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<MistralToolCall>>,
}

/// `message.content`: a plain string (reasoning off) or an array of typed
/// chunks (reasoning on). The untagged enum is the actual fix for the original
/// `ApiResponse` deserialization failure (Decision 8).
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum MistralMessageContent {
    Text(String),
    Chunks(Vec<MistralContentChunk>),
}

/// One content chunk. Tagged on `type`; an unknown `type` fails loud (no
/// `#[serde(other)]` fallback) rather than being silently skipped (Decision 8 /
/// C10b). `reference` / `file` / `audio` chunks are intentionally unsupported.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MistralContentChunk {
    Thinking { thinking: Vec<MistralTextChunk> },
    Text { text: String },
    ImageUrl { image_url: crate::ImageUrl },
}

/// Inner element of a thinking chunk's `thinking` list (`{type:"text",text}`).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MistralTextChunk {
    Text { text: String },
}

#[derive(Debug, Serialize)]
struct MistralTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: MistralFunction,
}

#[derive(Debug, Serialize)]
struct MistralFunction {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MistralToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: MistralToolCallFunction,
}

#[derive(Debug, Serialize, Deserialize)]
struct MistralToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct MistralChatResponse {
    choices: Vec<MistralChoice>,
    #[serde(default)]
    usage: Option<MistralUsage>,
}

#[derive(Debug, Deserialize)]
struct MistralChoice {
    message: MistralMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct MistralUsage {
    #[serde(default)]
    prompt_tokens: Option<u64>,
    #[serde(default)]
    completion_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MistralModelsResponse {
    #[serde(default)]
    data: Vec<MistralModelEntry>,
}

#[derive(Debug, Deserialize)]
struct MistralModelEntry {
    id: String,
}

/// Extract `(input_tokens, output_tokens)` from the usage block, saturating to
/// `u32` and treating a missing block as zero.
fn usage_tokens(usage: &Option<MistralUsage>) -> (u32, u32) {
    let Some(u) = usage else {
        return (0, 0);
    };
    let saturate = |v: u64| v.min(u32::MAX as u64) as u32;
    (
        u.prompt_tokens.map(saturate).unwrap_or(0),
        u.completion_tokens.map(saturate).unwrap_or(0),
    )
}

#[cfg(test)]
#[path = "mistral_tests.rs"]
mod tests;
