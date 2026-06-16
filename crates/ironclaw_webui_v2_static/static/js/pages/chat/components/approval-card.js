/**
 * ApprovalCard v2 — tool-approval gate.
 *
 * Adds a heuristic risk badge derived client-side from the tool name (the
 * backend does not currently send a risk classification) and a literal safety
 * summary. Missing target/destination/data stays visibly unknown instead of
 * being inferred from prose.
 *
 * Gate-shape compatibility: `gates.js#gateFromEvent` (mono) emits structured
 * `approvalDetails` (and a newline-joined `parameters` string), while the
 * desktop fork emitted a JSON `parameters` string. This card consumes both —
 * it parses JSON parameters when present and otherwise reconstructs a
 * parameters object from `approvalDetails` so the safety summary renders the
 * same rows under either gate shape.
 */
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { Button } from "../../../design-system/button.js";
import { Badge } from "../../../design-system/badge.js";
import { Icon } from "../../../design-system/icons.js";
import { classifyRisk } from "../lib/approval-risk.js";

function isEditableTarget(target) {
  const tag = String(target?.tagName || "").toLowerCase();
  if (tag === "textarea" || tag === "select") return true;
  if (tag !== "input") return false;
  return !["button", "checkbox", "radio", "submit"].includes(String(target?.type || ""));
}

const TARGET_KEYS = ["target", "resource", "path", "file", "filename", "name", "id", "repo"];
const DESTINATION_KEYS = [
  "destination",
  "recipient",
  "recipients",
  "to",
  "email",
  "channel",
  "workspace",
  "account",
  "endpoint",
];
// Every key whose value genuinely leaves the machine. Recipient-copy fields
// (cc/bcc) are listed here too: they are additional parties the data is sent to,
// so the gate must disclose them as outbound even though the primary recipient
// renders under Destination. Keep this list a superset, never a subset — under-
// reporting an outbound field on the gate is a no-fake-readiness violation.
const OUTBOUND_KEYS = [
  "outbound_data",
  "data",
  "payload",
  "subject",
  "body",
  "body_html",
  "html",
  "content",
  "message",
  "text",
  "note",
  "comment",
  "caption",
  "cc",
  "bcc",
  "attachment",
  "attachment_name",
  "attachment_names",
  "attachments",
  "document",
  "diff",
  "patch",
];

// Human labels for outbound fields so a multi-field disclosure reads as
// "subject: ..., cc: ..." instead of bare values. Falls back to the raw key.
const OUTBOUND_FIELD_LABELS = {
  subject: "subject",
  body: "body",
  body_html: "body",
  html: "body",
  content: "content",
  message: "message",
  text: "text",
  note: "note",
  comment: "comment",
  caption: "caption",
  cc: "cc",
  bcc: "bcc",
  attachment: "attachment",
  attachment_name: "attachment",
  attachment_names: "attachments",
  attachments: "attachments",
  document: "document",
  diff: "diff",
  patch: "patch",
};

function parseParameters(parameters) {
  if (!parameters) return null;
  if (typeof parameters === "object") return parameters;
  try {
    return JSON.parse(parameters);
  } catch (_) {
    return null;
  }
}

// Mono's `gateFromEvent` carries structured `approvalDetails`
// (`[{ label, value }]`) rather than a JSON parameters blob. When the
// parameters string isn't JSON-parseable, reconstruct a parameters object
// from those details (keyed by lowercased label) so the safety summary still
// has something to match TARGET/DESTINATION/OUTBOUND keys against.
function parametersFromApprovalDetails(approvalDetails) {
  if (!Array.isArray(approvalDetails) || approvalDetails.length === 0) return null;
  const parameters = {};
  for (const detail of approvalDetails) {
    const label = String(detail?.label || "").trim();
    if (!label || detail?.value == null) continue;
    parameters[label.toLowerCase().replace(/\s+/g, "_")] = String(detail.value);
  }
  return Object.keys(parameters).length > 0 ? parameters : null;
}

function summarizeValue(value) {
  if (value == null || value === "") return "";
  if (Array.isArray(value)) {
    if (value.length === 0) return "";
    if (value.every((item) => typeof item === "string")) return value.slice(0, 3).join(", ");
    return `${value.length} ${value.length === 1 ? "item" : "items"}`;
  }
  if (typeof value === "object") {
    const label = value.name || value.filename || value.title || value.id || value.url;
    if (label) return String(label);
    const keys = Object.keys(value);
    return keys.length ? `${keys.length} fields` : "";
  }
  const text = String(value).trim().replace(/\s+/g, " ");
  return text.length > 120 ? `${text.slice(0, 117)}...` : text;
}

function findSummary(parameters, keys) {
  if (!parameters || typeof parameters !== "object") return "";
  for (const key of keys) {
    if (Object.prototype.hasOwnProperty.call(parameters, key)) {
      const summary = summarizeValue(parameters[key]);
      if (summary) return summary;
    }
  }
  for (const value of Object.values(parameters)) {
    if (!value || typeof value !== "object" || Array.isArray(value)) continue;
    for (const key of keys) {
      if (Object.prototype.hasOwnProperty.call(value, key)) {
        const summary = summarizeValue(value[key]);
        if (summary) return summary;
      }
    }
  }
  return "";
}

// "What leaves the machine" must disclose every outbound field, not just the
// first match — a send with both a subject and a body sends both, so showing
// only one under-reports. Collects each present outbound key (top level plus one
// nesting level, matching findSummary's search depth) and labels it. Distinct
// keys are distinct disclosures and all show; dedupe only collapses an identical
// label+value pair (e.g. the same key reachable at both depths), so the line is
// never padded with a literal repeat.
function findOutboundSummary(parameters, keys, labels) {
  if (!parameters || typeof parameters !== "object") return "";
  const parts = [];
  const seen = new Set();

  const collect = (source) => {
    if (!source || typeof source !== "object" || Array.isArray(source)) return;
    for (const key of keys) {
      if (!Object.prototype.hasOwnProperty.call(source, key)) continue;
      const summary = summarizeValue(source[key]);
      if (!summary) continue;
      const label = labels[key] || key;
      const dedupeKey = `${label}:${summary}`;
      if (seen.has(dedupeKey)) continue;
      seen.add(dedupeKey);
      parts.push(`${label}: ${summary}`);
    }
  };

  collect(parameters);
  for (const value of Object.values(parameters)) {
    if (value && typeof value === "object" && !Array.isArray(value)) collect(value);
  }

  return parts.join("; ");
}

export function ApprovalCard({ gate, onApprove, onDeny, onAlways }) {
  const t = useT();
  const { toolName, headline, body, parameters, allowAlways, approvalDetails } = gate;
  const description = gate.description || body || "";
  const displayName = toolName || headline || t("approval.thisTool");
  const visibleDescription =
    description && description !== displayName && description !== headline ? description : "";
  const [always, setAlways] = React.useState(false);
  const parsedParameters = React.useMemo(
    () => parseParameters(parameters) || parametersFromApprovalDetails(approvalDetails),
    [parameters, approvalDetails],
  );

  const risk = React.useMemo(
    () => classifyRisk(toolName || headline, description, parameters),
    [toolName, headline, description, parameters],
  );
  const allowAlwaysAvailable = Boolean(allowAlways && risk.allowAlways === true);
  const summaryRows = React.useMemo(
    () => [
      {
        label: t("approval.actionLabel"),
        value: headline || displayName,
      },
      {
        label: t("approval.touchesLabel"),
        value: findSummary(parsedParameters, TARGET_KEYS),
      },
      {
        label: t("approval.destinationLabel"),
        value: findSummary(parsedParameters, DESTINATION_KEYS),
        emphasis: true,
      },
      {
        label: t("approval.whatLeavesMachineLabel"),
        value: findOutboundSummary(parsedParameters, OUTBOUND_KEYS, OUTBOUND_FIELD_LABELS),
        emphasis: true,
      },
    ],
    [displayName, headline, parsedParameters, t],
  );

  const onPrimary = React.useCallback(() => {
    if (always && allowAlwaysAvailable) {
      onAlways?.();
    } else {
      onApprove?.();
    }
  }, [always, allowAlwaysAvailable, onAlways, onApprove]);

  React.useEffect(() => {
    const onKeyDown = (event) => {
      if (isEditableTarget(event.target)) return;
      const key = String(event.key || "").toLowerCase();
      if ((event.metaKey || event.ctrlKey) && key === "enter") {
        event.preventDefault();
        onPrimary();
      } else if (key === "escape") {
        event.preventDefault();
        onDeny?.();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [onDeny, onPrimary]);

  return html`
    <div
      role="group"
      aria-label=${t("approval.title")}
      className="mx-auto w-full max-w-xl rounded-[16px] border border-[color-mix(in_srgb,var(--v2-gold)_40%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-gold-soft)_72%,var(--v2-card-bg))] p-4"
    >
      <div className="mb-3 flex items-start gap-3">
        <span
          className="grid h-9 w-9 shrink-0 place-items-center rounded-[10px] border border-[color-mix(in_srgb,var(--v2-gold)_34%,var(--v2-panel-border))] bg-[var(--v2-gold-soft)] text-[var(--v2-gold-text)]"
        >
          <${Icon} name="lock" className="h-4 w-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div
            className="text-[11px] font-semibold uppercase tracking-[0.14em] text-[var(--v2-gold-text)]"
          >
            ${t("approval.agentContext")}
          </div>
          <h3 className="mt-1 text-base font-semibold leading-6 text-[var(--v2-text-strong)]">
            ${displayName}
          </h3>
          ${visibleDescription &&
          html`
            <p className="mt-1 text-sm leading-5 text-[var(--v2-text-muted)]">
              ${visibleDescription}
            </p>
          `}
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          <span
            className="text-[10px] font-semibold uppercase tracking-[0.14em] text-[var(--v2-text-faint)]"
          >
            ${t("projects.card.risk")}
          </span>
          <${Badge} tone=${risk.tone} label=${t(risk.key)} dot=${false} size="sm" />
        </div>
      </div>

      <div
        className="mb-3 rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-3"
      >
        <div
          className="mb-3 flex items-center gap-2 rounded-[8px] border border-[color-mix(in_srgb,var(--v2-gold)_28%,var(--v2-panel-border))] bg-[var(--v2-gold-soft)] px-3 py-2 text-sm font-semibold text-[var(--v2-text-strong)]"
        >
          <${Icon} name="shield" className="h-4 w-4 shrink-0 text-[var(--v2-gold-text)]" />
          ${t("approval.nothingSentYet")}
        </div>
        <dl className="grid gap-3 text-xs sm:grid-cols-2">
          ${summaryRows.map(
            (row) => html`
              <div
                key=${row.label}
                className=${"grid gap-1" +
                (row.emphasis && row.value
                  ? " rounded-[8px] border border-[color-mix(in_srgb,var(--v2-gold)_22%,var(--v2-panel-border))] bg-[var(--v2-canvas-strong)] px-2.5 py-2"
                  : "")}
              >
                <dt
                  className="font-semibold uppercase tracking-[0.08em] text-[var(--v2-text-faint)]"
                >
                  ${row.label}
                </dt>
                <dd
                  className=${"text-sm leading-5 " +
                  (row.emphasis && row.value
                    ? "font-medium text-[var(--v2-text-strong)]"
                    : "text-[var(--v2-text-muted)]")}
                >
                  ${row.value || t("approval.notSpecified")}
                </dd>
              </div>
            `,
          )}
        </dl>
      </div>

      ${parameters &&
      html`<div className="mb-3">
        <div
          className="mb-1 text-xs font-semibold uppercase tracking-[0.08em] text-[var(--v2-text-faint)]"
        >
          ${t("approval.parametersLabel")}
        </div>
        <pre
          className="max-h-44 overflow-auto rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-canvas-strong)] p-3 font-mono text-xs leading-5 text-[var(--v2-text)]"
        >
${parameters}</pre
        >
      </div>`}
      ${allowAlwaysAvailable &&
      html`
        <label
          className="mb-3 flex items-start gap-2 rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-text-muted)]"
        >
          <input
            type="checkbox"
            checked=${always}
            onChange=${(event) => setAlways(event.currentTarget.checked)}
            className="mt-0.5 h-3.5 w-3.5 accent-[var(--v2-accent)]"
          />
          ${t("approval.alwaysAllowToolLabel", { tool: displayName })}
        </label>
      `}
      ${allowAlways &&
      !allowAlwaysAvailable &&
      html`
        <div
          className="mb-3 rounded-[8px] border border-[color-mix(in_srgb,var(--v2-warning-text)_32%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-warning-text)]"
        >
          ${t("approval.alwaysUnavailable")}
        </div>
      `}

      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="text-xs leading-5 text-[var(--v2-text-faint)]">
          ${t("approval.shortcutHint")}
        </div>
        <div className="flex flex-wrap gap-2">
          <${Button} variant="secondary" onClick=${() => onDeny?.()}> ${t("approval.deny")} <//>
          <${Button} variant="primary" onClick=${onPrimary}>
            ${always && allowAlwaysAvailable
              ? t("approval.approveAndAlways")
              : t("approval.approve")}
          <//>
        </div>
      </div>
    </div>
  `;
}
