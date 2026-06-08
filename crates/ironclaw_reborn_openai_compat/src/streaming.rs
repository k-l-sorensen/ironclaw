//! OpenAI-compatible SSE translation for projection-backed streams.
//!
//! This module is intentionally route-owned: it consumes projection-safe
//! outbound envelopes supplied by host composition and emits only OpenAI wire
//! events. Projection cursors stay internal to drain requests and never appear
//! as SSE ids or payload fields.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use futures_core::Stream;
use ironclaw_product_adapters::{
    ProductInboundAck, ProductOutboundEnvelope, ProductOutboundPayload, ProductProjectionItem,
    ProductProjectionState, ProjectionCursor,
};
use serde::Serialize;
use serde_json::json;

use crate::{
    OpenAiChatCompletionChunk, OpenAiChatCompletionId, OpenAiChatDelta, OpenAiChatFinishReason,
    OpenAiChatMessageRole, OpenAiChatModelOnlyTools, OpenAiChatStreamChoice,
    OpenAiCompatActorScope, OpenAiCompatErrorCode, OpenAiCompatErrorKind,
    OpenAiCompatErrorResponse, OpenAiCompatHttpError, OpenAiCompatResourceMapping,
    OpenAiResponseErrorObject, OpenAiResponseId, OpenAiResponseObject, OpenAiResponseOutputItem,
    OpenAiResponseOutputItemStatus, OpenAiResponseStatus, OpenAiResponsesMessageRole,
};

const STREAM_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiChatProjectionStreamRequest {
    pub public_id: OpenAiChatCompletionId,
    pub actor_scope: OpenAiCompatActorScope,
    pub accepted_ack: ProductInboundAck,
    pub requested_model: String,
    pub model_only_tools: Option<OpenAiChatModelOnlyTools>,
    pub mapping: OpenAiCompatResourceMapping,
    pub after_cursor: Option<ProjectionCursor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenAiResponseProjectionStreamRequest {
    pub public_id: OpenAiResponseId,
    pub actor_scope: OpenAiCompatActorScope,
    pub accepted_ack: ProductInboundAck,
    pub requested_model: String,
    pub mapping: OpenAiCompatResourceMapping,
    pub after_cursor: Option<ProjectionCursor>,
}

#[async_trait]
pub trait OpenAiCompatProjectionStreamer: Send + Sync {
    async fn drain_chat(
        &self,
        request: OpenAiChatProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError>;

    async fn drain_response(
        &self,
        request: OpenAiResponseProjectionStreamRequest,
    ) -> Result<Vec<ProductOutboundEnvelope>, OpenAiCompatHttpError>;
}

pub(crate) fn chat_sse_response(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiChatProjectionStreamRequest,
) -> Response {
    Sse::new(chat_sse_stream(streamer, request)).into_response()
}

pub(crate) fn response_sse_response(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiResponseProjectionStreamRequest,
) -> Response {
    Sse::new(response_sse_stream(streamer, request)).into_response()
}

fn chat_sse_stream(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiChatProjectionStreamRequest,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let created = unix_timestamp_now();
        let public_id = request.public_id.clone();
        let model = request.requested_model.clone();
        let mut after_cursor = request.after_cursor.clone();
        let mut state = TextDeltaState::default();

        yield Ok(chat_chunk_event(OpenAiChatCompletionChunk {
            id: public_id.clone(),
            object: "chat.completion.chunk".to_string(),
            created,
            model: model.clone(),
            choices: vec![OpenAiChatStreamChoice {
                index: 0,
                delta: OpenAiChatDelta {
                    role: Some(OpenAiChatMessageRole::Assistant),
                    content: None,
                    tool_calls: None,
                },
                finish_reason: None,
            }],
            usage: None,
        }));

        loop {
            let mut drain_request = request.clone();
            drain_request.after_cursor = after_cursor.clone();
            match streamer.drain_chat(drain_request).await {
                Ok(envelopes) => {
                    if envelopes.is_empty() {
                        tokio::time::sleep(STREAM_POLL_INTERVAL).await;
                        continue;
                    }
                    for envelope in envelopes {
                        after_cursor = Some(envelope.projection_cursor().clone());
                        let payload = envelope.payload();
                        match text_from_payload(payload) {
                            PayloadText::None => {}
                            PayloadText::Update(text) => match state.delta_for(text) {
                                Ok(Some(delta)) => yield Ok(chat_text_delta_event(&public_id, created, &model, delta)),
                                Ok(None) => {}
                                Err(error) => {
                                    yield Ok(openai_error_event(error));
                                    return;
                                }
                            },
                            PayloadText::Final(text) => {
                                match state.delta_for(text) {
                                    Ok(Some(delta)) => yield Ok(chat_text_delta_event(&public_id, created, &model, delta)),
                                    Ok(None) => {}
                                    Err(error) => {
                                        yield Ok(openai_error_event(error));
                                        return;
                                    }
                                }
                                yield Ok(chat_finish_event(&public_id, created, &model));
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                        }
                        match terminal_status_from_payload(payload) {
                            TerminalStatus::None => {}
                            TerminalStatus::Completed => {
                                yield Ok(chat_finish_event(&public_id, created, &model));
                                yield Ok(Event::default().data("[DONE]"));
                                return;
                            }
                            TerminalStatus::Failed | TerminalStatus::Cancelled => {
                                yield Ok(openai_error_event(OpenAiCompatHttpError::internal()));
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    yield Ok(openai_error_event(error));
                    return;
                }
            }
        }
    }
}

fn response_sse_stream(
    streamer: Arc<dyn OpenAiCompatProjectionStreamer>,
    request: OpenAiResponseProjectionStreamRequest,
) -> impl Stream<Item = Result<Event, Infallible>> {
    async_stream::stream! {
        let created = unix_timestamp_now();
        let public_id = request.public_id.clone();
        let model = request.requested_model.clone();
        let item_id = format!("msg_{}", public_id.as_str());
        let mut sequence_number = 0_u64;
        let mut after_cursor = request.after_cursor.clone();
        let mut state = TextDeltaState::default();

        yield Ok(response_event(
            "response.created",
            json!({
                "type": "response.created",
                "sequence_number": sequence_number,
                "response": response_object(public_id.clone(), created, model.clone(), OpenAiResponseStatus::InProgress, ""),
            }),
        ));
        sequence_number += 1;

        loop {
            let mut drain_request = request.clone();
            drain_request.after_cursor = after_cursor.clone();
            match streamer.drain_response(drain_request).await {
                Ok(envelopes) => {
                    if envelopes.is_empty() {
                        tokio::time::sleep(STREAM_POLL_INTERVAL).await;
                        continue;
                    }
                    for envelope in envelopes {
                        after_cursor = Some(envelope.projection_cursor().clone());
                        let payload = envelope.payload();
                        match text_from_payload(payload) {
                            PayloadText::None => {}
                            PayloadText::Update(text) => match state.delta_for(text) {
                                Ok(Some(delta)) => {
                                    yield Ok(response_text_delta_event(
                                        &public_id,
                                        &item_id,
                                        sequence_number,
                                        delta,
                                    ));
                                    sequence_number += 1;
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    yield Ok(response_stream_error_event(error));
                                    return;
                                }
                            },
                            PayloadText::Final(text) => {
                                match state.delta_for(text) {
                                    Ok(Some(delta)) => {
                                        yield Ok(response_text_delta_event(
                                            &public_id,
                                            &item_id,
                                            sequence_number,
                                            delta,
                                        ));
                                        sequence_number += 1;
                                    }
                                    Ok(None) => {}
                                    Err(error) => {
                                        yield Ok(response_stream_error_event(error));
                                        return;
                                    }
                                }
                                yield Ok(response_text_done_event(&item_id, sequence_number, state.text()));
                                sequence_number += 1;
                                yield Ok(response_terminal_event(
                                    "response.completed",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Completed,
                                    state.text(),
                                ));
                                return;
                            }
                        }
                        match terminal_status_from_payload(payload) {
                            TerminalStatus::None => {}
                            TerminalStatus::Completed => {
                                yield Ok(response_text_done_event(&item_id, sequence_number, state.text()));
                                sequence_number += 1;
                                yield Ok(response_terminal_event(
                                    "response.completed",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Completed,
                                    state.text(),
                                ));
                                return;
                            }
                            TerminalStatus::Failed => {
                                yield Ok(response_terminal_event(
                                    "response.failed",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Failed,
                                    state.text(),
                                ));
                                return;
                            }
                            TerminalStatus::Cancelled => {
                                yield Ok(response_terminal_event(
                                    "response.cancelled",
                                    sequence_number,
                                    public_id.clone(),
                                    created,
                                    model.clone(),
                                    OpenAiResponseStatus::Cancelled,
                                    state.text(),
                                ));
                                return;
                            }
                        }
                    }
                }
                Err(error) => {
                    yield Ok(response_stream_error_event(error));
                    return;
                }
            }
        }
    }
}

#[derive(Default)]
struct TextDeltaState {
    text: String,
}

impl TextDeltaState {
    fn delta_for(&mut self, next: &str) -> Result<Option<String>, OpenAiCompatHttpError> {
        if next == self.text {
            return Ok(None);
        }
        let Some(delta) = next.strip_prefix(&self.text) else {
            return Err(OpenAiCompatHttpError::from_kind(
                500,
                false,
                OpenAiCompatErrorKind::Internal,
                None,
            ));
        };
        self.text = next.to_string();
        Ok(Some(delta.to_string()))
    }

    fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalStatus {
    None,
    Completed,
    Failed,
    Cancelled,
}

fn terminal_status_from_payload(payload: &ProductOutboundPayload) -> TerminalStatus {
    match payload {
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => terminal_status_from_state(state),
        _ => TerminalStatus::None,
    }
}

fn terminal_status_from_state(state: &ProductProjectionState) -> TerminalStatus {
    state
        .items
        .iter()
        .rev()
        .find_map(|item| match item {
            ProductProjectionItem::RunStatus { status, .. } => match status.as_str() {
                "completed" => Some(TerminalStatus::Completed),
                "failed" | "killed" => Some(TerminalStatus::Failed),
                "cancelled" => Some(TerminalStatus::Cancelled),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(TerminalStatus::None)
}

enum PayloadText<'a> {
    None,
    Update(&'a str),
    Final(&'a str),
}

fn text_from_payload(payload: &ProductOutboundPayload) -> PayloadText<'_> {
    match payload {
        ProductOutboundPayload::FinalReply(reply) => PayloadText::Final(&reply.text),
        ProductOutboundPayload::ProjectionSnapshot { state }
        | ProductOutboundPayload::ProjectionUpdate { state } => state_text(state)
            .map(PayloadText::Update)
            .unwrap_or(PayloadText::None),
        ProductOutboundPayload::KeepAlive
        | ProductOutboundPayload::Progress(_)
        | ProductOutboundPayload::CapabilityActivity(_)
        | ProductOutboundPayload::CapabilityDisplayPreview(_)
        | ProductOutboundPayload::GatePrompt(_)
        | ProductOutboundPayload::AuthPrompt(_) => PayloadText::None,
    }
}

fn state_text(state: &ProductProjectionState) -> Option<&str> {
    state.items.iter().rev().find_map(|item| match item {
        ProductProjectionItem::Text { body, .. } => Some(body.as_str()),
        _ => None,
    })
}

fn chat_text_delta_event(
    public_id: &OpenAiChatCompletionId,
    created: u64,
    model: &str,
    delta: String,
) -> Event {
    chat_chunk_event(OpenAiChatCompletionChunk {
        id: public_id.clone(),
        object: "chat.completion.chunk".to_string(),
        created,
        model: model.to_string(),
        choices: vec![OpenAiChatStreamChoice {
            index: 0,
            delta: OpenAiChatDelta {
                role: None,
                content: Some(delta),
                tool_calls: None,
            },
            finish_reason: None,
        }],
        usage: None,
    })
}

fn chat_finish_event(public_id: &OpenAiChatCompletionId, created: u64, model: &str) -> Event {
    chat_chunk_event(OpenAiChatCompletionChunk {
        id: public_id.clone(),
        object: "chat.completion.chunk".to_string(),
        created,
        model: model.to_string(),
        choices: vec![OpenAiChatStreamChoice {
            index: 0,
            delta: OpenAiChatDelta {
                role: None,
                content: None,
                tool_calls: None,
            },
            finish_reason: Some(OpenAiChatFinishReason::Stop),
        }],
        usage: None,
    })
}

fn chat_chunk_event(chunk: OpenAiChatCompletionChunk) -> Event {
    data_event(None, &chunk)
}

fn response_text_delta_event(
    public_id: &OpenAiResponseId,
    item_id: &str,
    sequence_number: u64,
    delta: String,
) -> Event {
    response_event(
        "response.output_text.delta",
        json!({
            "type": "response.output_text.delta",
            "sequence_number": sequence_number,
            "response_id": public_id.as_str(),
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "delta": delta,
        }),
    )
}

fn response_text_done_event(item_id: &str, sequence_number: u64, text: &str) -> Event {
    response_event(
        "response.output_text.done",
        json!({
            "type": "response.output_text.done",
            "sequence_number": sequence_number,
            "item_id": item_id,
            "output_index": 0,
            "content_index": 0,
            "text": text,
        }),
    )
}

fn response_terminal_event(
    event_name: &'static str,
    sequence_number: u64,
    public_id: OpenAiResponseId,
    created: u64,
    model: String,
    status: OpenAiResponseStatus,
    text: &str,
) -> Event {
    response_event(
        event_name,
        json!({
            "type": event_name,
            "sequence_number": sequence_number,
            "response": response_object(public_id, created, model, status, text),
        }),
    )
}

fn response_event(event_name: &'static str, payload: serde_json::Value) -> Event {
    data_event(Some(event_name), &payload)
}

fn openai_error_event(error: OpenAiCompatHttpError) -> Event {
    data_event(Some("error"), error.body())
}

fn response_stream_error_event(error: OpenAiCompatHttpError) -> Event {
    let body = compat_error_body(error.body());
    data_event(
        Some("error"),
        &json!({
            "type": "error",
            "error": body.get("error").cloned().unwrap_or_else(generic_error_value),
        }),
    )
}

fn data_event<T: Serialize>(event_name: Option<&'static str>, payload: &T) -> Event {
    let data = serde_json::to_string(payload).unwrap_or_else(|_| generic_error_json());
    let event = Event::default().data(data);
    if let Some(event_name) = event_name {
        event.event(event_name)
    } else {
        event
    }
}

fn compat_error_body(body: &OpenAiCompatErrorResponse) -> serde_json::Value {
    serde_json::to_value(body).unwrap_or_else(|_| json!({ "error": generic_error_value() }))
}

fn generic_error_json() -> String {
    json!({ "error": generic_error_value() }).to_string()
}

fn generic_error_value() -> serde_json::Value {
    json!({
        "message": OpenAiCompatErrorCode::InternalError.sanitized_message(),
        "type": "server_error",
        "param": null,
        "code": "internal_error",
    })
}

fn response_object(
    public_id: OpenAiResponseId,
    created_at: u64,
    model: String,
    status: OpenAiResponseStatus,
    text: &str,
) -> OpenAiResponseObject {
    let output = if text.is_empty() {
        Vec::new()
    } else {
        vec![OpenAiResponseOutputItem::Message {
            id: format!("msg_{}", public_id.as_str()),
            status: Some(OpenAiResponseOutputItemStatus::Completed),
            role: OpenAiResponsesMessageRole::Assistant,
            content: json!([{ "type": "output_text", "text": text }]),
        }]
    };
    let error = if matches!(status, OpenAiResponseStatus::Failed) {
        Some(OpenAiResponseErrorObject::from_kind(
            OpenAiCompatErrorKind::Internal,
        ))
    } else {
        None
    };
    OpenAiResponseObject {
        id: public_id,
        object: "response".to_string(),
        created_at,
        status,
        model,
        output,
        error,
        incomplete_details: None,
        usage: None,
    }
}

fn unix_timestamp_now() -> u64 {
    let timestamp = Utc::now().timestamp();
    if timestamp < 0 { 0 } else { timestamp as u64 }
}
