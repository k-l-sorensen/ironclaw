use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponsesCreateRequest {
    pub model: String,
    pub input: OpenAiResponsesInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OpenAiResponsesInput {
    Text(String),
    Items(Vec<OpenAiResponsesInputItem>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponsesInputItem {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub item_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponseObject {
    pub id: String,
    pub object: String,
    pub created_at: u64,
    pub status: OpenAiResponseStatus,
    pub model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<OpenAiResponseOutputItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OpenAiResponseErrorObject>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incomplete_details: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenAiResponseUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiResponseUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiResponseStatus {
    Queued,
    InProgress,
    Completed,
    Failed,
    Cancelled,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenAiResponseOutputItem {
    pub id: String,
    #[serde(rename = "type")]
    pub item_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<OpenAiResponseOutputItemStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAiResponseOutputItemStatus {
    InProgress,
    Completed,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenAiResponseErrorObject {
    pub code: String,
    pub message: String,
}
