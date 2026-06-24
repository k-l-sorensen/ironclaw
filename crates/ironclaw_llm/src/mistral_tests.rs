//! Tests for the Mistral provider (co-located sibling per architecture
//! Decision 10 to keep `mistral.rs` under the file-size budget).
//!
//! Coverage maps to the architecture test matrix:
//! C1/C9 drive the public trait methods against a loopback mock server; C2–C5
//! and C8 exercise the request builder / wire conversion; C6/C7 the response
//! parser; C10/C10b the error-mapping boundary.

use super::*;
use crate::provider::ContentPart;
use crate::{ImageUrl, ToolDefinition};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const MEDIUM: &str = "mistral-medium-latest";
const SMALL: &str = "mistral-small-latest";
const LARGE: &str = "mistral-large-latest";

fn provider(
    model: &str,
    reasoning: Option<MistralReasoningEffort>,
    base_url: &str,
) -> MistralProvider {
    MistralProvider::new_with_base_url(
        model,
        SecretString::from("test-key"),
        reasoning,
        30,
        base_url,
    )
    .expect("provider builds")
}

/// Build the (model, reasoning) request and return its serialized JSON body.
fn request_json(
    model: &str,
    reasoning: Option<MistralReasoningEffort>,
    msgs: Vec<ChatMessage>,
    tools: Vec<ToolDefinition>,
) -> String {
    let p = provider(model, reasoning, MISTRAL_BASE_URL);
    let req = p.build_request(MistralRequestParams {
        model: model.to_string(),
        messages: msgs,
        tools,
        temperature: None,
        max_tokens: None,
        stop: None,
        tool_choice: None,
    });
    serde_json::to_string(&req).expect("request serializes")
}

/// Spawn a one-shot mock that captures the request body and replies with
/// `status_line` (+ optional extra headers) and `response_body`. Returns the
/// base URL and a handle yielding the captured request body.
async fn serve_once(
    status_line: &'static str,
    extra_headers: &'static str,
    response_body: String,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        // Read headers, then the declared content-length body.
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        let header_end = loop {
            let n = socket.read(&mut chunk).await.expect("read headers");
            assert!(n > 0, "connection closed before headers");
            buffer.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        while buffer.len() < header_end + content_length {
            let n = socket.read(&mut chunk).await.expect("read body");
            assert!(n > 0, "connection closed before body");
            buffer.extend_from_slice(&chunk[..n]);
        }
        let body =
            String::from_utf8_lossy(&buffer[header_end..header_end + content_length]).to_string();

        let response = format!(
            "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\n{extra_headers}content-length: {}\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = socket.write_all(response.as_bytes()).await;
        body
    });
    (base_url, handle)
}

/// Canned reasoning-on (array content) success body.
fn array_response() -> String {
    serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": [{"type": "text", "text": "let me think"}]},
                    {"type": "text", "text": "the answer"}
                ]
            },
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 11, "completion_tokens": 7}
    })
    .to_string()
}

// ── C1: complete() drives reasoning_effort onto the wire ────────────────────

#[tokio::test]
async fn c1_complete_sends_reasoning_effort_high() {
    let (base_url, handle) = serve_once("200 OK", "", array_response()).await;
    let p = provider(MEDIUM, Some(MistralReasoningEffort::High), &base_url);

    let resp = p
        .complete(CompletionRequest::new(vec![ChatMessage::user("hi")]))
        .await
        .expect("completion succeeds");
    assert_eq!(resp.content, "the answer");
    assert_eq!(resp.reasoning.as_deref(), Some("let me think"));
    assert_eq!(resp.input_tokens, 11);
    assert_eq!(resp.output_tokens, 7);

    let body = handle.await.unwrap();
    assert!(
        body.contains(r#""reasoning_effort":"high""#),
        "request body must carry reasoning_effort high: {body}"
    );
}

// ── C2/C3/C4: request-builder gating ────────────────────────────────────────

#[tokio::test]
async fn c2_reasoning_off_omits_param() {
    // "reasoning OFF" is represented as Option::None at the config boundary.
    let body = request_json(MEDIUM, None, vec![ChatMessage::user("hi")], vec![]);
    assert!(
        !body.contains("reasoning_effort"),
        "off must omit reasoning_effort: {body}"
    );
}

#[tokio::test]
async fn c3_large_model_gate_beats_toggle() {
    let body = request_json(
        LARGE,
        Some(MistralReasoningEffort::High),
        vec![ChatMessage::user("hi")],
        vec![],
    );
    assert!(
        !body.contains("reasoning_effort"),
        "mistral-large is not reasoning-capable; param must be omitted: {body}"
    );
}

#[tokio::test]
async fn c4_small_model_sends_param() {
    let body = request_json(
        SMALL,
        Some(MistralReasoningEffort::High),
        vec![ChatMessage::user("hi")],
        vec![],
    );
    assert!(
        body.contains(r#""reasoning_effort":"high""#),
        "mistral-small must carry reasoning_effort: {body}"
    );
}

/// Documents the non-collapsed `Some(None)` → explicit `"none"` wire behavior
/// (Decision 3). The env boundary does not currently produce this, but the
/// provider must render it faithfully rather than collapsing it with omit.
#[tokio::test]
async fn some_none_renders_explicit_none() {
    let body = request_json(
        MEDIUM,
        Some(MistralReasoningEffort::None),
        vec![ChatMessage::user("hi")],
        vec![],
    );
    assert!(
        body.contains(r#""reasoning_effort":"none""#),
        "Some(None) must send explicit none: {body}"
    );
}

// ── C5: image attachment survives ChatMessage→wire on a vision model ────────

#[tokio::test]
async fn c5_image_part_included_for_vision_model() {
    let msg = ChatMessage::user_with_parts(
        "describe this",
        vec![ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: "data:image/png;base64,AQIDBA==".to_string(),
                detail: None,
            },
        }],
    );
    let body = request_json(
        MEDIUM,
        Some(MistralReasoningEffort::High),
        vec![msg],
        vec![],
    );
    assert!(
        body.contains("image_url") && body.contains("data:image/png;base64,AQIDBA=="),
        "vision model wire request must include the image part: {body}"
    );
}

#[tokio::test]
async fn image_part_dropped_for_non_vision_model() {
    let msg = ChatMessage::user_with_parts(
        "describe this",
        vec![ContentPart::ImageUrl {
            image_url: ImageUrl {
                url: "data:image/png;base64,AQIDBA==".to_string(),
                detail: None,
            },
        }],
    );
    // mistral-large is not in the vision allowlist → image dropped.
    let body = request_json(LARGE, None, vec![msg], vec![]);
    assert!(
        !body.contains("image_url"),
        "non-vision model must drop image parts: {body}"
    );
}

// ── C6/C7: response parser ──────────────────────────────────────────────────

#[test]
fn c6_array_response_splits_reasoning_and_content() {
    let content: MistralMessageContent = serde_json::from_value(serde_json::json!([
        {"type": "thinking", "thinking": [{"type": "text", "text": "step one "}, {"type": "text", "text": "step two"}]},
        {"type": "text", "text": "final answer"}
    ]))
    .unwrap();
    let (text, reasoning) = extract_content(Some(content)).unwrap();
    assert_eq!(text.as_deref(), Some("final answer"));
    assert_eq!(reasoning.as_deref(), Some("step one step two"));
}

#[test]
fn c7_string_response_has_no_reasoning() {
    let content: MistralMessageContent =
        serde_json::from_value(serde_json::json!("plain answer")).unwrap();
    let (text, reasoning) = extract_content(Some(content)).unwrap();
    assert_eq!(text.as_deref(), Some("plain answer"));
    assert!(reasoning.is_none());
}

// ── C8: multi-turn replay reconstructs the ThinkChunk ───────────────────────

#[test]
fn c8_multi_turn_replays_think_chunk() {
    // Turn-1 reasoning fed back via with_reasoning, as the agent loop does.
    let assistant = ChatMessage::assistant("the answer")
        .with_reasoning(Some("prior reasoning trace".to_string()));
    let wire = chat_message_to_wire(assistant, false);
    let json = serde_json::to_value(&wire).unwrap();
    let content = &json["content"];
    assert!(
        content.is_array(),
        "replayed content must be an array: {json}"
    );
    let chunks = content.as_array().unwrap();
    assert_eq!(chunks[0]["type"], "thinking");
    assert_eq!(chunks[0]["thinking"][0]["text"], "prior reasoning trace");
    assert_eq!(chunks[1]["type"], "text");
    assert_eq!(chunks[1]["text"], "the answer");
}

// ── C9: complete_with_tools carries both reasoning_effort and tool schema ────

#[tokio::test]
async fn c9_tools_and_reasoning_effort_both_present() {
    let tool_response = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "abc123def",
                    "type": "function",
                    "function": {"name": "echo", "arguments": "{\"x\":1}"}
                }]
            },
            "finish_reason": "tool_calls"
        }]
    })
    .to_string();
    let (base_url, handle) = serve_once("200 OK", "", tool_response).await;
    let p = provider(MEDIUM, Some(MistralReasoningEffort::High), &base_url);

    let tools = vec![ToolDefinition {
        name: "echo".to_string(),
        description: "Echo input".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"x": {"type": "integer"}}
        }),
    }];
    let resp = p
        .complete_with_tools(ToolCompletionRequest::new(
            vec![ChatMessage::user("call echo")],
            tools,
        ))
        .await
        .expect("tool completion succeeds");
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "echo");
    assert_eq!(resp.tool_calls[0].arguments["x"], 1);

    let body = handle.await.unwrap();
    assert!(
        body.contains(r#""reasoning_effort":"high""#),
        "tool request must carry reasoning_effort: {body}"
    );
    assert!(
        body.contains(r#""name":"echo""#) && body.contains("parameters"),
        "tool request must carry the tool schema: {body}"
    );
}

// ── C10: error mapping at the boundary, with class assertions ───────────────

async fn complete_against(
    status_line: &'static str,
    extra_headers: &'static str,
    body: serde_json::Value,
) -> LlmError {
    let (base_url, _handle) = serve_once(status_line, extra_headers, body.to_string()).await;
    let p = provider(MEDIUM, None, &base_url);
    p.complete(CompletionRequest::new(vec![ChatMessage::user("hi")]))
        .await
        .expect_err("expected error")
}

#[tokio::test]
async fn c10_401_maps_to_auth_failed_non_transient() {
    let err = complete_against(
        "401 Unauthorized",
        "",
        serde_json::json!({"message": "bad key"}),
    )
    .await;
    assert!(matches!(err, LlmError::AuthFailed { .. }), "got {err:?}");
    assert!(
        !crate::retry::is_retryable(&err),
        "auth must not be retryable"
    );
}

#[tokio::test]
async fn c10_429_maps_to_rate_limited_transient() {
    let err = complete_against(
        "429 Too Many Requests",
        "retry-after: 30\r\n",
        serde_json::json!({"message": "slow down"}),
    )
    .await;
    match err {
        LlmError::RateLimited { retry_after, .. } => {
            assert_eq!(retry_after, Some(std::time::Duration::from_secs(30)));
        }
        other => panic!("expected RateLimited, got {other:?}"),
    }
}

#[tokio::test]
async fn c10_413_context_overflow_maps_to_context_length() {
    let err = complete_against(
        "400 Bad Request",
        "",
        serde_json::json!({"message": "This model's maximum context length is 128000 tokens. However, your messages resulted in 150000 tokens."}),
    )
    .await;
    match err {
        LlmError::ContextLengthExceeded { used, limit } => {
            assert_eq!(used, 150000);
            assert_eq!(limit, 128000);
        }
        other => panic!("expected ContextLengthExceeded, got {other:?}"),
    }
    assert!(
        !crate::retry::is_retryable(&err),
        "context overflow must not retry"
    );
}

#[tokio::test]
async fn c10_5xx_maps_to_bad_gateway_transient() {
    let err = complete_against(
        "500 Internal Server Error",
        "",
        serde_json::json!({"traceback": "Traceback (most recent call last) ..."}),
    )
    .await;
    match &err {
        LlmError::BadGateway { status, .. } => assert_eq!(*status, 500),
        other => panic!("expected BadGateway, got {other:?}"),
    }
    // Class: must be retryable (and the circuit breaker treats BadGateway as
    // transient — covered by circuit_breaker's own is_transient tests).
    assert!(crate::retry::is_retryable(&err), "5xx must be retryable");
}

#[tokio::test]
async fn c10_malformed_2xx_maps_to_invalid_response_transient() {
    // 2xx with a body that isn't a valid envelope → InvalidResponse, not Json.
    let err = complete_against("200 OK", "", serde_json::json!({"unexpected": true})).await;
    assert!(
        matches!(err, LlmError::InvalidResponse { .. }),
        "got {err:?}"
    );
    assert!(
        crate::retry::is_retryable(&err),
        "invalid response must be retryable"
    );
}

#[tokio::test]
async fn c10_2xx_array_without_text_chunk_is_empty_response() {
    // Reasoning-on array with only a thinking chunk → answer lost → fail loud.
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": [{"type": "thinking", "thinking": [{"type": "text", "text": "thinking only"}]}]
            },
            "finish_reason": "stop"
        }]
    });
    let err = complete_against("200 OK", "", body).await;
    assert!(matches!(err, LlmError::EmptyResponse { .. }), "got {err:?}");
    assert!(
        crate::retry::is_retryable(&err),
        "empty response must be retryable"
    );
}

// ── C10b: unknown chunk type fails loud ─────────────────────────────────────

#[tokio::test]
async fn c10b_unknown_chunk_type_fails_loud() {
    // An `audio` chunk is a known-to-Mistral but unsupported type; it must
    // surface as a parse failure (InvalidResponse), never be silently skipped.
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": [{"type": "audio", "audio": "..."}, {"type": "text", "text": "answer"}]
            },
            "finish_reason": "stop"
        }]
    });
    let err = complete_against("200 OK", "", body).await;
    assert!(
        matches!(err, LlmError::InvalidResponse { .. }),
        "unknown chunk type must fail loud, got {err:?}"
    );
}
