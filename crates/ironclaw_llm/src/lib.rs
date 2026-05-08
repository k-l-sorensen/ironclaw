//! LLM provider abstraction for IronClaw.
//!
//! This crate owns the reusable provider-facing request/response contract used
//! by Reborn composition code. Concrete backend implementations can be added or
//! migrated here without making Reborn crates depend on the root application
//! crate.

pub mod error;
mod provider;

pub use error::LlmError;
pub use provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, ImageUrl,
    LlmProvider, ModelMetadata, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition, ToolResult, UnsupportedParam, generate_tool_call_id,
    normalize_openai_image_detail, normalized_model_override, sanitize_tool_messages,
    strip_unsupported_completion_params, strip_unsupported_tool_params,
};
