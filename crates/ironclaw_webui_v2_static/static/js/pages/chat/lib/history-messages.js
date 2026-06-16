// Map v2 `ThreadMessageRecord[]` from RebornTimelineResponse into
// the message shape the UI components render. Turn grouping consumes the
// normalized `turnRunId` carried by records and previews. Records carry
// `attachments: AttachmentRef[]`; we project them into the render shape
// `MessageBubble` expects so attachment cards survive a page refresh and a
// thread switch (the timeline is the source of truth — the bytes stay
// behind the project mount, the cards render from the refs).

import { attachmentKindFromMime, formatBytes } from "./attachments.js";
import { attachmentUrl } from "../../../lib/api.js";

// Project a stored `AttachmentRef` (snake_case wire shape) into the
// render shape `MessageBubble` consumes. The timeline never carries bytes,
// so `preview_url` is null here; a landed image instead gets a `fetch_url`
// the bubble lazily resolves into a thumbnail (an authenticated byte fetch,
// since `<img>` cannot send a bearer header). The just-sent optimistic
// message keeps its local data URL in `preview_url` and needs no fetch.
function attachmentsFromRecord(record, threadId) {
  const refs = record.attachments;
  if (!Array.isArray(refs) || refs.length === 0) return undefined;
  return refs.map((ref) => {
    const kind = ref.kind || attachmentKindFromMime(ref.mime_type);
    // Any landed attachment can serve its bytes — for an image thumbnail or
    // for click-to-preview of any kind. A ref without a storage_key never
    // landed, so there are no bytes to fetch. Require every addressing part so
    // a malformed record yields a plain card (no fetch) rather than throwing in
    // `attachmentUrl` mid-projection.
    const fetch_url =
      threadId && ref.storage_key && record.message_id && ref.id
        ? attachmentUrl({
            threadId,
            messageId: record.message_id,
            attachmentId: ref.id,
          })
        : null;
    return {
      id: ref.id,
      filename: ref.filename || "attachment",
      mime_type: ref.mime_type || "",
      kind,
      size_label: Number.isFinite(ref.size_bytes) ? formatBytes(ref.size_bytes) : "",
      preview_url: null,
      fetch_url,
    };
  });
}

export function messagesFromTimeline(records, pendingMessages = [], threadId = null) {
  const seen = new Set();
  const messages = [];

  for (const record of records || []) {
    if (record.kind === "tool_result_reference") {
      // LLM-visible transcript artifact (result_ref + safe_summary).
      // Not a UI message — the matching `capability_display_preview`
      // record renders the tool card.
      continue;
    }

    if (record.kind === "capability_display_preview") {
      const card = toolCardFromPreviewRecord(record);
      if (!card) continue;
      const id = `tool-${card.invocationId}`;
      if (seen.has(id)) continue;
      seen.add(id);
      messages.push({
        id,
        role: "tool_activity",
        ...card,
        timestamp: timestampForRecord(record) || card.updatedAt || null,
        sequence: record.sequence,
        turnRunId: record.turn_run_id || null,
      });
      continue;
    }

    const id = `msg-${record.message_id}`;
    if (seen.has(id)) continue;
    seen.add(id);
    const role = roleForRecord(record);
    const isBusyRejected =
      role === "user" &&
      (record.status === "rejected_busy" || record.status === "deferred_busy");
    messages.push({
      id,
      role,
      content: record.content || "",
      attachments: attachmentsFromRecord(record, threadId),
      timestamp: timestampForRecord(record),
      kind: record.kind,
      status: isBusyRejected ? "error" : record.status,
      ...(isBusyRejected && {
        error:
          "This message wasn't sent because Ironclaw was busy. Resend it to try again.",
      }),
      isFinalReply: isFinalAssistantRecord(record),
      sequence: record.sequence,
      turnRunId: record.turn_run_id || null,
    });
  }

  // Pending rows are dropped from the ref by the caller as soon as
  // `sendMessage` returns (server has accepted the message and the
  // confirmed row will arrive via timeline). The id-based guard
  // remains as defense-in-depth in case a caller passes a pending
  // that was already merged into the timeline.
  for (const pending of pendingMessages) {
    if (seen.has(pending.id)) continue;
    const message = pendingMessageForRender(pending);
    if (message.timelineMessageId && seen.has(`msg-${message.timelineMessageId}`)) {
      continue;
    }
    messages.push(message);
  }

  return messages;
}

function pendingMessageForRender(pending) {
  return {
    ...pending,
    role: pending.role || "user",
    isOptimistic: pending.isOptimistic !== false,
  };
}

// Return the pending messages that the timeline has NOT yet confirmed, so a
// caller (useHistory) can keep only the still-unconfirmed optimistic rows
// after a timeline refresh instead of blanket-clearing them. A blanket wipe
// would durably drop a second in-flight turn whose record hasn't projected
// yet; keeping unconfirmed rows preserves them until the timeline carries them.
//
// Mirrors the id-based dedup in `messagesFromTimeline`: a pending row is
// confirmed (dropped) only when its recorded `timelineMessageId` is present in
// the timeline. Content-based confirmation is intentionally NOT applied — the
// web render contract preserves an equal-text pending row that has no timeline
// id (see history-messages.test "equal pending text without timeline id is
// preserved"), so this stays consistent with that path.
export function pendingMessagesAfterTimeline(records, pendingMessages = []) {
  const timelineIds = new Set(
    (records || [])
      .filter((record) => record && record.message_id != null)
      .map((record) => `msg-${record.message_id}`),
  );
  return (pendingMessages || []).filter((pending) => {
    const message = pendingMessageForRender(pending);
    if (message.timelineMessageId && timelineIds.has(`msg-${message.timelineMessageId}`)) {
      return false;
    }
    return true;
  });
}

function isFinalAssistantRecord(record) {
  return (
    (record.kind === "assistant" || record.kind === "assistant_message") &&
    record.status === "finalized"
  );
}

function roleForRecord(record) {
  switch (record.kind) {
    case "user":
    case "user_message":
      return "user";
    case "assistant":
    case "assistant_message":
    case "tool_result":
      return "assistant";
    case "system":
      return "system";
    default:
      return record.actor_id ? "user" : "assistant";
  }
}

function timestampForRecord(record) {
  // ThreadMessageRecord has no top-level timestamp; surfaces use
  // the sequence ordering for now. Browsers render the wall-clock
  // when an event arrives (FinalReplyView.generated_at).
  return record.received_at || record.created_at || null;
}

function toolCardFromPreviewRecord(record) {
  if (!record.content) return null;
  let envelope;
  try {
    envelope = JSON.parse(record.content);
  } catch (err) {
    console.warn("Failed to parse capability_display_preview envelope", err);
    return null;
  }
  if (!envelope || !envelope.invocation_id) return null;
  return toolCardFromPreview(envelope);
}

// Map a `CapabilityDisplayPreviewEnvelope` (timeline) or
// `CapabilityDisplayPreviewView` (SSE) into the field set
// `ToolActivityCard` destructures.
export function toolCardFromPreview(preview) {
  const failed = preview.status === "failed" || preview.status === "killed";
  return {
    invocationId: preview.invocation_id,
    callId: preview.invocation_id,
    toolName: preview.title || preview.capability_id || "tool",
    toolStatus: toolStatusFromActivityStatus(preview.status),
    toolDetail: preview.subtitle || null,
    toolParameters: preview.input_summary || null,
    // On failure the output fields carry the error text — surface it
    // only through `toolError` so the card renders it once in red,
    // not twice (once as a teal result preview and once as the error).
    toolResultPreview: failed
      ? null
      : preview.output_preview || preview.output_summary || null,
    toolError: failed
      ? preview.output_summary ||
        preview.output_preview ||
        preview.result_ref ||
        null
      : null,
    toolDurationMs: null,
    updatedAt: preview.updated_at || null,
    resultRef: preview.result_ref || null,
    truncated: Boolean(preview.truncated),
    outputBytes: preview.output_bytes ?? null,
    outputKind: preview.output_kind || null,
    turnRunId: preview.turn_run_id || null,
  };
}

// Map a `CapabilityActivityView` (SSE lifecycle frame) into the same
// card shape. Activity frames carry only metadata — no title, no
// parameters, no output — so the resulting card is intentionally
// sparse and is meant to be enriched by the next preview frame.
export function toolCardFromActivity(activity) {
  return {
    invocationId: activity.invocation_id,
    callId: activity.invocation_id,
    toolName: activity.capability_id || "tool",
    toolStatus: toolStatusFromActivityStatus(activity.status),
    toolDetail: null,
    toolParameters: null,
    toolResultPreview: null,
    toolError: activity.error_kind || null,
    toolDurationMs: null,
    updatedAt: activity.updated_at || null,
    resultRef: null,
    truncated: false,
    outputBytes: activity.output_bytes ?? null,
    outputKind: null,
    turnRunId: activity.turn_run_id || null,
  };
}

export function isTerminalToolStatus(status) {
  return status === "success" || status === "error";
}

function toolStatusFromActivityStatus(status) {
  switch (status) {
    case "completed":
      return "success";
    case "failed":
    case "killed":
      return "error";
    case "started":
    case "running":
    default:
      return "running";
  }
}

// -----------------------------------------------------------------------------
// Desktop-only attachment + run-completion helpers.
//
// The web build sends attachments as the first-class `WebUiInboundAttachment`
// wire field and renders timeline `attachments` refs via `messagesFromTimeline`
// above — that is the canonical web contract and is unchanged by anything in
// this section. The Tauri desktop build talks to a bundled Reborn sidecar that
// (a) drops the first-class `attachments` field from its timeline echo and
// (b) never feeds attachment bytes to the model, so the desktop send path
// inlines extracted text into `content` as a durable manifest block. These
// helpers back that DESKTOP-ONLY path (gated behind `isDesktopRuntime()` in
// useChat) and must not be wired into the web send/render flow.
// -----------------------------------------------------------------------------

// True once a run has produced a finalized assistant reply in the timeline.
// This is the desktop SSE-drop fallback's completion signal: the gateway
// registers no bare GET /runs/{id}, so the poll watches the registered
// timeline route and treats a landed finalized assistant record for the run as
// completion. Failure is deliberately NOT inferred here.
export function runReplyLandedInTimeline(records, runId) {
  if (!runId || !Array.isArray(records)) return false;
  return records.some((record) => {
    if (!record) return false;
    if (record.kind !== "assistant" && record.kind !== "assistant_message") return false;
    if ((record.turn_run_id || null) !== runId) return false;
    const status = String(record.status || "").toLowerCase();
    return status !== "pending" && status !== "streaming";
  });
}

const MESSAGE_CONTENT_MAX_BYTES = 64 * 1024;
const EMBED_TOTAL_BUDGET_BYTES = 48 * 1024;
const EMBED_SAFETY_RESERVE_BYTES = 2 * 1024;

const TEXT_EMBEDDABLE_MIMES = new Set([
  "application/json",
  "application/ld+json",
  "application/x-ndjson",
  "application/xml",
  "application/yaml",
  "application/x-yaml",
  "application/csv",
  "application/javascript",
  "application/sql",
  "application/rtf",
  "text/rtf",
]);
const TEXT_EMBEDDABLE_EXTENSIONS = new Set([
  "txt",
  "md",
  "markdown",
  "csv",
  "tsv",
  "json",
  "jsonl",
  "ndjson",
  "xml",
  "yaml",
  "yml",
  "toml",
  "log",
  "html",
  "htm",
  "css",
  "js",
  "mjs",
  "ts",
  "py",
  "rs",
  "go",
  "java",
  "rb",
  "sh",
  "sql",
  "rtf",
]);

function isTextEmbeddable(item) {
  const mime = String(item.mime_type || "").toLowerCase();
  if (mime.startsWith("text/")) return true;
  if (TEXT_EMBEDDABLE_MIMES.has(mime)) return true;
  const name = String(item.name || item.filename || "").toLowerCase();
  const dot = name.lastIndexOf(".");
  return dot !== -1 && TEXT_EMBEDDABLE_EXTENSIONS.has(name.slice(dot + 1));
}

// Public form of the embed eligibility test, for the desktop composer: a raw
// payload that is not text-embeddable never reaches the model, and the chip
// must say so instead of implying success.
export function isAttachmentTextEmbeddable(item) {
  return isTextEmbeddable(item || {});
}

function decodeBase64Utf8(base64) {
  try {
    const binary = atob(String(base64 || "").replace(/\s+/g, ""));
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
  } catch (_) {
    return null;
  }
}

function sanitizeEmbeddedText(text) {
  return text
    .replace(/\r\n?/g, "\n")
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, " ");
}

function truncateUtf8(text, maxBytes) {
  const encoder = new TextEncoder();
  if (encoder.encode(text).length <= maxBytes) return { text, truncated: false };
  let low = 0;
  let high = text.length;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if (encoder.encode(text.slice(0, mid)).length <= maxBytes) low = mid;
    else high = mid - 1;
  }
  return { text: text.slice(0, low), truncated: true };
}

// Render the durable attachment manifest appended to message content
// (DESKTOP-ONLY). Carries chip metadata (parsed back on reload) AND, for text
// payloads, the extracted text itself as length-prefixed fenced sections the
// model can read — the only channel the bundled sidecar feeds to the model.
// Never includes raw base64; the content validator rejects oversized payloads.
export function buildDurableAttachmentBlock(attachments, opts = {}) {
  const entries = (attachments || []).filter((item) => item && (item.name || item.filename));
  if (entries.length === 0) return "";

  const encoder = new TextEncoder();
  const contentBytes = Math.max(0, Number(opts.contentBytes) || 0);
  let messageBudget = MESSAGE_CONTENT_MAX_BYTES - contentBytes - EMBED_SAFETY_RESERVE_BYTES;
  let embedBudget = EMBED_TOTAL_BUDGET_BYTES;

  const lines = ["", '<attachments ic="1">'];
  const push = (line) => {
    lines.push(line);
    messageBudget -= encoder.encode(line).length + 1;
  };

  entries.forEach((item, index) => {
    push(`Attachment ${index + 1}:`);
    push(`filename: ${item.name || item.filename}`);
    push(`mime_type: ${item.mime_type || "application/octet-stream"}`);
    if (item.size) push(`size: ${item.size}`);

    if (!isTextEmbeddable(item) || !item.data_base64) return;
    const decoded = decodeBase64Utf8(item.data_base64);
    if (decoded == null) return;
    const clean = sanitizeEmbeddedText(decoded).trim();
    if (!clean) return;

    const allowance = Math.min(embedBudget, messageBudget - 256);
    if (allowance <= 0) {
      push("extraction_status: content_omitted_message_budget");
      push("note: message too large to embed this document; ask for it in smaller parts.");
      return;
    }
    const { text, truncated } = truncateUtf8(clean, allowance);
    embedBudget -= encoder.encode(text).length;
    push(`extraction_status: ${truncated ? "extracted_text_truncated" : "extracted_text"}`);
    if (truncated) {
      push(`note: showing the first ${text.length} of ${clean.length} characters.`);
    }
    push(`extracted_text_chars: ${text.length}`);
    push("extracted_text:");
    push("---");
    push(text);
    push("---");
  });
  lines.push("</attachments>");
  return lines.join("\n");
}
