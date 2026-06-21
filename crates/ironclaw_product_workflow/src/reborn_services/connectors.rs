//! Read-only connector proxy contract for the Reborn WebUI surface.
//!
//! The Workbench needs to fetch the user's real connected-account data
//! (Gmail/Calendar/etc., brokered through Composio) deterministically,
//! without going through the LLM agent path and without ever exposing the
//! provider API key to the browser.
//!
//! This module defines the *contract* only. The facade
//! ([`super::RebornServicesApi`]) delegates to an injected
//! [`ConnectorReadPort`]; the concrete implementation (key resolution from
//! the encrypted secret store + the Composio REST calls + the read-only
//! allowlist) lives in host composition, which is the only layer that owns a
//! secret store. Keeping the port abstract here means `ironclaw_product_workflow`
//! never depends on an HTTP client or the secret store, and the WebUI handler
//! crate calls a single stable surface.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Request body for `POST /api/webchat/v2/connectors/read`.
#[derive(Debug, Clone, Deserialize)]
pub struct RebornConnectorReadRequest {
    /// Toolkit slug whose connected account owns the read (e.g. `"gmail"`).
    pub toolkit: String,
    /// Composio tool slug to execute. The implementation enforces a
    /// read-only allowlist (`*_FETCH_* / *_LIST_* / *_GET_* / *_SEARCH_*`)
    /// and rejects everything else with `400`.
    pub tool: String,
    /// Arguments forwarded verbatim as the tool's `arguments` payload.
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// Response body for `POST /api/webchat/v2/connectors/read`.
#[derive(Debug, Clone, Serialize)]
pub struct RebornConnectorReadResponse {
    pub successful: bool,
    pub data: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request body for `POST /api/webchat/v2/connectors/write`.
///
/// The write path is DELIBERATELY separate from the read path: reads can never
/// reach a mutating tool, and writes are confined to a small, explicit
/// allowlist (draft-creation by default; sends only when the gateway is started
/// with the send capability enabled). The provider key is still resolved
/// server-side and never crosses to the browser.
#[derive(Debug, Clone, Deserialize)]
pub struct RebornConnectorWriteRequest {
    /// Toolkit slug whose connected account owns the write (e.g. `"gmail"`).
    pub toolkit: String,
    /// Composio tool slug to execute. The implementation enforces the
    /// write allowlist and rejects everything else with `400`.
    pub tool: String,
    /// Arguments forwarded verbatim as the tool's `arguments` payload.
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// A single connected account, redacted to the fields the Workbench needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebornConnectedAccount {
    pub toolkit: String,
    pub status: String,
    pub user_id: String,
}

/// Response body for `GET /api/webchat/v2/connectors/connected`.
#[derive(Debug, Clone, Serialize)]
pub struct RebornConnectedAccountsResponse {
    pub accounts: Vec<RebornConnectedAccount>,
}

/// Failure surface for connector reads. The facade maps these to the
/// stable `RebornServicesError` HTTP vocabulary; no provider error text or
/// secret material ever flows through these variants except the bounded,
/// already-redacted upstream message attached to [`ConnectorReadError::Upstream`].
#[derive(Debug, Clone)]
pub enum ConnectorReadError {
    /// The requested tool is not on the read-only allowlist, or another
    /// request field failed validation. Maps to `400`.
    InvalidRequest { reason: String },
    /// The connector subsystem (secret store / provider client) is not
    /// wired in this profile, or the API key is not bound. Maps to `503`.
    Unavailable { retryable: bool },
    /// The upstream provider returned an error. `message` is bounded and
    /// must never contain secret material. Maps to `502`-ish via `Unavailable`
    /// at the facade (kept retryable=false).
    Upstream { message: String },
    /// An internal invariant failed (serialization, etc.). Maps to `500`.
    Internal,
}

/// Read-only connector port. Implemented by host composition.
///
/// Implementations MUST:
/// - resolve the provider API key server-side from the encrypted secret
///   store and send it only as the upstream auth header — never return or
///   log it;
/// - enforce the read-only allowlist before issuing any execute call.
#[async_trait]
pub trait ConnectorReadPort: Send + Sync {
    /// List the active connected accounts for the owner entity.
    async fn connected(&self) -> Result<RebornConnectedAccountsResponse, ConnectorReadError>;

    /// Execute a single read-only connector tool and return its output.
    async fn read(
        &self,
        request: RebornConnectorReadRequest,
    ) -> Result<RebornConnectorReadResponse, ConnectorReadError>;

    /// Execute a single GATED write connector tool and return its output.
    ///
    /// Implementations MUST classify the tool with [`classify_connector_write`]
    /// and reject anything not on the write allowlist with `InvalidRequest`.
    /// A [`ConnectorWriteKind::Send`] tool is permitted ONLY when the gateway
    /// was started with the send capability enabled; otherwise it is rejected
    /// even though it is a recognized send tool. Draft-creation tools are always
    /// permitted. The provider key is resolved server-side as for reads.
    async fn write(
        &self,
        request: RebornConnectorWriteRequest,
    ) -> Result<RebornConnectorReadResponse, ConnectorReadError>;

    /// Persist the provider API key from a `setup … {action:"configure"}`
    /// request into the encrypted secret store, scoped to the same owner the
    /// read path resolves against. The key is the only secret consumed; any
    /// other entries are ignored. The value is never returned or logged.
    ///
    /// Binding the write here (rather than in a lifecycle facade that owns no
    /// secret store) guarantees the configure write and the connector read use
    /// one owner scope and therefore cannot drift apart.
    async fn configure_secrets(
        &self,
        secrets: std::collections::HashMap<String, String>,
    ) -> Result<(), ConnectorReadError>;
}

/// Read-only allowlist for connector tool slugs, exposed so both the port
/// implementation and tests share one definition.
///
/// Composio slugs are `TOOLKIT_<action…>` with the verb in any action position
/// (`GMAIL_FETCH_EMAILS`, `GOOGLECALENDAR_EVENTS_LIST`, `GOOGLECALENDAR_FIND_EVENT`).
/// A slug is permitted iff: it has a real uppercase/digit toolkit prefix that is
/// not itself a verb, no empty segments, at least one action segment is a known
/// READ verb, and NO segment is a known WRITE/mutate verb. The write denylist is
/// authoritative — a read verb does not rescue a slug that also mutates
/// (e.g. `GOOGLECALENDAR_CALENDAR_LIST_INSERT` is rejected by `INSERT`).
const READ_VERBS: &[&str] = &["FETCH", "LIST", "GET", "SEARCH", "FIND", "READ"];
const WRITE_VERBS: &[&str] = &[
    "SEND",
    "CREATE",
    "DELETE",
    "UPDATE",
    "MODIFY",
    "INSERT",
    "MOVE",
    "WATCH",
    "TRASH",
    "UNTRASH",
    "REMOVE",
    "ADD",
    "PATCH",
    "REPLY",
    "DRAFT",
    "CLEAR",
    "WRITE",
    "APPEND",
    "SET",
    "ENABLE",
    "DISABLE",
    "ARCHIVE",
    "UNARCHIVE",
    "MARK",
    "IMPORT",
    "COPY",
    "RENAME",
    "FORMAT",
    "DUPLICATE",
];

pub fn is_read_only_tool(tool: &str) -> bool {
    let segments: Vec<&str> = tool.split('_').collect();
    if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
        return false;
    }
    // Every segment must be uppercase/digit; the toolkit prefix must not be a verb.
    if !segments.iter().all(|s| {
        s.bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    }) {
        return false;
    }
    let toolkit = segments[0];
    if READ_VERBS.contains(&toolkit) || WRITE_VERBS.contains(&toolkit) {
        return false;
    }
    let action = &segments[1..];
    if action.iter().any(|s| WRITE_VERBS.contains(s)) {
        return false;
    }
    action.iter().any(|s| READ_VERBS.contains(s))
}

/// Gated-write allowlist. Unlike reads (verb-classified), writes are an
/// EXPLICIT, exhaustive allowlist — a write tool is permitted only if it is
/// named here, so the blast radius of the write path is auditable at a glance.
///
/// `DRAFT_WRITE_TOOLS` create reviewable drafts and never deliver anything;
/// they are always permitted. `SEND_WRITE_TOOLS` actually deliver and are
/// permitted ONLY when the gateway is started with the send capability enabled
/// (see [`ConnectorWriteKind`]). Everything else is `Forbidden`.
pub const DRAFT_WRITE_TOOLS: &[&str] = &["GMAIL_CREATE_EMAIL_DRAFT"];
pub const SEND_WRITE_TOOLS: &[&str] = &[
    "GMAIL_SEND_EMAIL",
    "GMAIL_SEND_DRAFT",
    "GMAIL_REPLY_TO_THREAD",
];

/// Classification of a connector write tool against the explicit allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorWriteKind {
    /// Creates a reviewable draft; delivers nothing. Always permitted.
    Draft,
    /// Actually delivers (send/post/reply). Permitted only when the send
    /// capability is enabled on the gateway.
    Send,
    /// Not on the write allowlist. Always rejected.
    Forbidden,
}

/// Classify a connector write tool slug. The slug must be an exact, whole match
/// of an allowlisted tool (after trimming) — no fuzzy/verb matching, so a
/// near-miss like `GMAIL_SEND_EMAILS` is `Forbidden`, not a send.
pub fn classify_connector_write(tool: &str) -> ConnectorWriteKind {
    let tool = tool.trim();
    if DRAFT_WRITE_TOOLS.contains(&tool) {
        ConnectorWriteKind::Draft
    } else if SEND_WRITE_TOOLS.contains(&tool) {
        ConnectorWriteKind::Send
    } else {
        ConnectorWriteKind::Forbidden
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConnectorWriteKind, DRAFT_WRITE_TOOLS, SEND_WRITE_TOOLS, classify_connector_write,
        is_read_only_tool,
    };

    #[test]
    fn write_classifier_separates_draft_send_forbidden() {
        assert_eq!(
            classify_connector_write("GMAIL_CREATE_EMAIL_DRAFT"),
            ConnectorWriteKind::Draft
        );
        assert_eq!(
            classify_connector_write("  GMAIL_CREATE_EMAIL_DRAFT  "),
            ConnectorWriteKind::Draft
        );
        assert_eq!(
            classify_connector_write("GMAIL_SEND_EMAIL"),
            ConnectorWriteKind::Send
        );
        assert_eq!(
            classify_connector_write("GMAIL_REPLY_TO_THREAD"),
            ConnectorWriteKind::Send
        );
        // Not on the allowlist — including destructive and near-miss slugs.
        for forbidden in [
            "",
            "GMAIL_DELETE_MESSAGE",
            "GMAIL_TRASH_MESSAGE",
            "GMAIL_SEND_EMAILS",
            "NOTION_UPDATE_PAGE",
            "GMAIL_FETCH_EMAILS",
        ] {
            assert_eq!(
                classify_connector_write(forbidden),
                ConnectorWriteKind::Forbidden,
                "{forbidden} must be forbidden"
            );
        }
    }

    #[test]
    fn draft_tools_are_never_also_read_only() {
        // The draft/send tools must never sneak through the read path.
        for tool in DRAFT_WRITE_TOOLS.iter().chain(SEND_WRITE_TOOLS.iter()) {
            assert!(!is_read_only_tool(tool), "{tool} must not be read-only");
        }
    }

    #[test]
    fn allows_canonical_read_tools() {
        assert!(is_read_only_tool("GMAIL_FETCH_EMAILS"));
        assert!(is_read_only_tool("GOOGLECALENDAR_LIST_EVENTS"));
        assert!(is_read_only_tool("GMAIL_GET_MESSAGE"));
        assert!(is_read_only_tool("GITHUB_SEARCH_REPOS"));
        assert!(is_read_only_tool("NOTION_FETCH_PAGE"));
        // Real Composio slugs put the verb in a trailing/middle segment.
        assert!(is_read_only_tool("GOOGLECALENDAR_EVENTS_LIST"));
        assert!(is_read_only_tool("GOOGLECALENDAR_FIND_EVENT"));
        assert!(is_read_only_tool("GOOGLECALENDAR_FIND_FREE_SLOTS"));
        assert!(is_read_only_tool("GOOGLECALENDAR_GET_CURRENT_DATE_TIME"));
        assert!(is_read_only_tool("GMAIL_LIST_THREADS"));
        assert!(is_read_only_tool("GOOGLECALENDAR_LIST_CALENDARS"));
    }

    #[test]
    fn rejects_writes() {
        assert!(!is_read_only_tool("GMAIL_SEND_EMAIL"));
        assert!(!is_read_only_tool("GMAIL_CREATE_DRAFT"));
        assert!(!is_read_only_tool("GMAIL_DELETE_MESSAGE"));
        assert!(!is_read_only_tool("GMAIL_TRASH_MESSAGE"));
        assert!(!is_read_only_tool("GMAIL_MODIFY_LABELS"));
        assert!(!is_read_only_tool("GMAIL_REPLY_TO_THREAD"));
        assert!(!is_read_only_tool("NOTION_UPDATE_PAGE"));
        // A read verb must NOT rescue a slug that also mutates.
        assert!(!is_read_only_tool("GOOGLECALENDAR_CALENDAR_LIST_INSERT"));
        assert!(!is_read_only_tool("GOOGLECALENDAR_EVENTS_WATCH"));
        assert!(!is_read_only_tool("GOOGLECALENDAR_EVENTS_MOVE"));
        assert!(!is_read_only_tool("GMAIL_LIST_ADD_LABEL"));
    }

    #[test]
    fn rejects_malformed_or_partial() {
        assert!(!is_read_only_tool(""));
        assert!(!is_read_only_tool("FETCH_EMAILS"));
        assert!(!is_read_only_tool("_FETCH_EMAILS"));
        assert!(!is_read_only_tool("gmail_fetch_emails"));
        assert!(!is_read_only_tool("GMAIL_FETCH_"));
        assert!(!is_read_only_tool("GMAIL_FETCHEMAILS"));
        assert!(!is_read_only_tool("GMAIL"));
        // verb must be a whole underscore-delimited segment
        assert!(!is_read_only_tool("GMAIL_GETTER_THING"));
    }
}
