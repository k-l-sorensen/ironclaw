use ironclaw_reborn_openai_compat::{
    OpenAiChatCompletionChunk, OpenAiChatCompletionRequest, OpenAiChatCompletionResponse,
    OpenAiChatFinishReason, OpenAiChatMessageRole, OpenAiResponseObject, OpenAiResponseStatus,
    OpenAiResponseUsage, OpenAiResponsesCreateRequest, OpenAiResponsesInput,
};
use serde_json::json;

#[test]
fn chat_completion_request_round_trips_explicit_compat_fields() {
    let request: OpenAiChatCompletionRequest = serde_json::from_value(json!({
        "model": "gpt-reborn",
        "messages": [
            {"role": "developer", "content": "follow product policy"},
            {"role": "user", "content": "hello"}
        ],
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "lookup_order",
                "description": "Look up an order",
                "parameters": {"type": "object"},
                "strict": true
            }
        }],
        "tool_choice": {"type": "function", "function": {"name": "lookup_order"}},
        "future_openai_option": "ignored until explicitly supported"
    }))
    .expect("chat request");

    assert_eq!(request.model, "gpt-reborn");
    assert_eq!(request.messages[0].role, OpenAiChatMessageRole::Developer);
    assert_eq!(request.stream, Some(true));
    assert_eq!(
        request.tools.as_ref().expect("tools")[0].function.name,
        "lookup_order"
    );

    let serialized = serde_json::to_value(&request).expect("serialize request");
    assert!(serialized.get("future_openai_option").is_none());
}

#[test]
fn responses_create_request_accepts_text_or_item_input() {
    let text: OpenAiResponsesCreateRequest = serde_json::from_value(json!({
        "model": "gpt-reborn",
        "input": "hello",
        "stream": false
    }))
    .expect("text input");
    assert!(matches!(text.input, OpenAiResponsesInput::Text(_)));

    let items: OpenAiResponsesCreateRequest = serde_json::from_value(json!({
        "model": "gpt-reborn",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]
        }],
        "tools": [{"type": "web_search_preview"}],
        "tool_choice": "auto"
    }))
    .expect("item input");
    assert!(matches!(items.input, OpenAiResponsesInput::Items(_)));
    assert_eq!(items.tools.as_ref().expect("tools").len(), 1);
}

#[test]
fn response_dtos_serialize_openai_shapes() {
    let chat = OpenAiChatCompletionResponse {
        id: "chatcmpl-test".to_string(),
        object: "chat.completion".to_string(),
        created: 1_777_777_777,
        model: "gpt-reborn".to_string(),
        choices: vec![ironclaw_reborn_openai_compat::OpenAiChatChoice {
            index: 0,
            message: ironclaw_reborn_openai_compat::OpenAiChatMessage {
                role: OpenAiChatMessageRole::Assistant,
                content: Some(json!("hello")),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            finish_reason: Some(OpenAiChatFinishReason::Stop),
        }],
        usage: None,
    };
    let chat_json = serde_json::to_value(chat).expect("chat response json");
    assert_eq!(chat_json["object"], "chat.completion");
    assert_eq!(chat_json["choices"][0]["finish_reason"], "stop");

    let chunk = OpenAiChatCompletionChunk {
        id: "chatcmpl-test".to_string(),
        object: "chat.completion.chunk".to_string(),
        created: 1_777_777_777,
        model: "gpt-reborn".to_string(),
        choices: vec![ironclaw_reborn_openai_compat::OpenAiChatStreamChoice {
            index: 0,
            delta: ironclaw_reborn_openai_compat::OpenAiChatDelta {
                role: Some(OpenAiChatMessageRole::Assistant),
                content: Some("he".to_string()),
                tool_calls: Some(vec![
                    ironclaw_reborn_openai_compat::OpenAiChatToolCallDelta {
                        index: 0,
                        id: Some("call_1".to_string()),
                        kind: Some(ironclaw_reborn_openai_compat::OpenAiChatToolKind::Function),
                        function: Some(
                            ironclaw_reborn_openai_compat::OpenAiChatToolCallFunctionDelta {
                                name: Some("lookup_order".to_string()),
                                arguments: Some("{".to_string()),
                            },
                        ),
                    },
                ]),
            },
            finish_reason: None,
        }],
        usage: None,
    };
    let chunk_json = serde_json::to_value(chunk).expect("chunk json");
    assert_eq!(chunk_json["object"], "chat.completion.chunk");
    assert_eq!(chunk_json["choices"][0]["delta"]["content"], "he");
    assert_eq!(
        chunk_json["choices"][0]["delta"]["tool_calls"][0]["index"],
        0
    );
    assert_eq!(
        chunk_json["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "{"
    );

    let response = OpenAiResponseObject {
        id: "resp_test".to_string(),
        object: "response".to_string(),
        created_at: 1_777_777_777,
        status: OpenAiResponseStatus::Completed,
        model: "gpt-reborn".to_string(),
        output: Vec::new(),
        error: None,
        incomplete_details: None,
        usage: Some(OpenAiResponseUsage {
            input_tokens: 3,
            output_tokens: 5,
            total_tokens: 8,
        }),
    };
    let response_json = serde_json::to_value(response).expect("response json");
    assert_eq!(response_json["object"], "response");
    assert_eq!(response_json["status"], "completed");
    assert_eq!(response_json["usage"]["input_tokens"], 3);
    assert!(response_json["usage"].get("prompt_tokens").is_none());
}
