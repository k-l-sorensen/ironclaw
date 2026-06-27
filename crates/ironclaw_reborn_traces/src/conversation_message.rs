use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A single message in a conversation. Re-exported from the legacy
/// `ironclaw::history::ConversationMessage`; the monolith now re-exports
/// this type so both names refer to the same struct.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    /// Provider-emitted reasoning trace for an assistant/tool_calls message,
    /// already leak-scanned. Persisted so it can be replayed into the LLM on
    /// the next user turn (CTR-1). `None` for user messages and for rows
    /// written before the `reasoning` column existed.
    pub reasoning: Option<String>,
    /// Opaque reasoning-block signature (Mistral ThinkChunk `signature`) for an
    /// assistant/tool_calls message. Persisted and replayed verbatim so the
    /// provider can verify the replayed block; not leak-scanned, since it is an
    /// opaque token. `None` for user messages and rows written before the
    /// `reasoning_signature` column existed.
    pub reasoning_signature: Option<String>,
}
