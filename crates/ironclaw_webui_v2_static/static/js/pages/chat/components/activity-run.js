import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { summarizeActivity } from "../lib/activity-summary.js";
import { MarkdownRenderer } from "./markdown-renderer.js";
import { ToolActivity } from "./tool-activity.js";

export function ActivityRun({ activity }) {
  const summary = summarizeActivity(activity);
  // Show the agent's reasoning + tool steps live while the run is in progress
  // (the user asked to watch the agent think). A completed activity mounts as a
  // quiet collapsed receipt, per the Handled-Receipts design. Manual toggle works.
  const [expanded, setExpanded] = React.useState(!summary.isComplete);

  if (summary.isComplete) {
    return html`
      <${ActivityReceiptCard}
        activity=${activity}
        summary=${summary}
        expanded=${expanded}
        setExpanded=${setExpanded}
      />
    `;
  }

  return html`
    <div className="mr-auto flex w-full max-w-[85%] flex-col">
      <button
        type="button"
        onClick=${() => setExpanded((value) => !value)}
        aria-expanded=${expanded ? "true" : "false"}
        aria-label=${`${summary.label}; ${expanded ? "hide details" : "show details"}`}
        data-testid="activity-summary-row"
        className=${[
          "v2-button flex w-full items-center gap-2 border-0 bg-transparent px-1 py-1.5 text-left text-sm",
          summary.hasError ? "text-[var(--v2-danger-text)]" : "text-iron-400 hover:text-iron-200",
        ].join(" ")}
      >
        <${Icon} name="layers" className="h-4 w-4 shrink-0" />
        <span className="truncate">${summary.label}</span>
        <${Icon}
          name="chevron"
          className=${["ml-auto h-3.5 w-3.5 shrink-0", expanded ? "rotate-180" : ""].join(" ")}
        />
      </button>

      ${expanded &&
      html`
        <div className="mt-2 flex flex-col gap-3">
          ${activity.map(
            (item, index) => html`
              <${ActivityItem}
                key=${item.id || `${item.role || "activity"}-${index}`}
                item=${item}
              />
            `,
          )}
        </div>
      `}
    </div>
  `;
}

function ActivityReceiptCard({ activity, summary, expanded, setExpanded }) {
  const rows = receiptRowsForActivity(activity, summary);
  const link = receiptLinkForActivity(activity);
  return html`
    <div
      className="mr-auto flex w-full max-w-[min(760px,92vw)] flex-col gap-2 rounded-[14px] border border-[color-mix(in_srgb,var(--v2-gold)_28%,var(--v2-panel-border))] bg-[var(--v2-gold-soft)] px-4 py-3 text-sm text-[var(--v2-text-strong)] shadow-[var(--v2-card-shadow)]"
      data-testid="activity-receipt-card"
    >
      <div className="flex min-w-0 items-start gap-3">
        <span
          className="grid h-8 w-8 shrink-0 place-items-center rounded-[9px] border border-[color-mix(in_srgb,var(--v2-gold)_34%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-gold)_16%,transparent)] text-[var(--v2-gold-text)]"
          aria-hidden="true"
        >
          <${Icon} name="check" className="h-4 w-4" />
        </span>
        <div className="min-w-0 flex-1">
          <div
            className="text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--v2-gold-text)]"
          >
            Agent action completed
          </div>
          <div className="truncate text-sm font-semibold text-[var(--v2-text-strong)]">
            ${receiptTitleForActivity(activity, summary)}
          </div>
        </div>
        <button
          type="button"
          onClick=${() => setExpanded((value) => !value)}
          aria-expanded=${expanded ? "true" : "false"}
          aria-label=${`${expanded ? "Hide" : "View"} action details`}
          className="v2-button inline-flex shrink-0 items-center gap-1 rounded-md border border-[color-mix(in_srgb,var(--v2-gold)_26%,var(--v2-panel-border))] bg-transparent px-2 py-1 text-[11px] font-medium text-[var(--v2-gold-text)] hover:bg-[color-mix(in_srgb,var(--v2-gold)_10%,transparent)]"
        >
          ${expanded ? "Hide details" : "View details"}
          <${Icon}
            name="chevron"
            className=${["h-3 w-3", expanded ? "rotate-180" : ""].join(" ")}
          />
        </button>
      </div>

      <div className="grid gap-1.5 text-xs">
        ${rows.map(
          (row) => html`
            <div key=${row.label} className="grid grid-cols-[5.25rem_minmax(0,1fr)] gap-3">
              <span className="text-[var(--v2-gold-text)]">${row.label}</span>
              <span className="truncate text-[var(--v2-text-muted)]" title=${row.value}
                >${row.value}</span
              >
            </div>
          `,
        )}
      </div>

      ${link &&
      html`
        <a
          href=${link.href}
          target="_blank"
          rel="noreferrer"
          className="inline-flex w-fit items-center gap-1.5 rounded-md border border-[color-mix(in_srgb,var(--v2-gold)_28%,var(--v2-panel-border))] px-2 py-1 text-xs font-medium text-[var(--v2-gold-text)] hover:bg-[color-mix(in_srgb,var(--v2-gold)_10%,transparent)]"
        >
          ${link.label}
          <${Icon} name="external" className="h-3 w-3" />
        </a>
      `}
      ${expanded &&
      html`
        <div
          className="mt-1 border-t border-[color-mix(in_srgb,var(--v2-gold)_18%,var(--v2-panel-border))] pt-3"
        >
          <div className="flex flex-col gap-3">
            ${activity.map(
              (item, index) => html`
                <${ActivityItem}
                  key=${item.id || `${item.role || "activity"}-${index}`}
                  item=${item}
                />
              `,
            )}
          </div>
        </div>
      `}
    </div>
  `;
}

function ActivityItem({ item }) {
  if (item.role === "thinking") {
    return html`<${ReasoningItem} content=${item.content} />`;
  }

  if (item.role === "tool_activity" || hasToolCalls(item)) {
    const activity = hasToolCalls(item) ? { id: item.id, toolCalls: item.toolCalls } : item;
    return html`<${ToolActivity} activity=${activity} />`;
  }

  return null;
}

function ReasoningItem({ content }) {
  if (!content) return null;
  // The agent's live reasoning. Gold = agent action (Design Law); v2 tokens so it
  // reads on the light surface (the prior iron-* dark styling was off-theme).
  return html`
    <div className="flex gap-2.5">
      <span
        className="mt-0.5 grid h-6 w-6 shrink-0 place-items-center rounded-full border border-[color-mix(in_srgb,var(--v2-gold)_30%,var(--v2-panel-border))] bg-[var(--v2-gold-soft)] text-[var(--v2-gold-text)]"
        aria-hidden="true"
      >
        <${Icon} name="spark" className="h-3.5 w-3.5" />
      </span>
      <div
        className="min-w-0 flex-1 border-l-2 border-[color-mix(in_srgb,var(--v2-gold)_22%,var(--v2-panel-border))] pl-3"
      >
        <div
          className="mb-0.5 font-mono text-[10.5px] font-semibold uppercase tracking-[0.08em] text-[var(--v2-gold-text)]"
        >
          Thinking
        </div>
        <${MarkdownRenderer}
          content=${content}
          className="text-[13px] text-[var(--v2-text-muted)]"
        />
      </div>
    </div>
  `;
}

function hasToolCalls(item) {
  return item.toolCalls && item.toolCalls.length > 0;
}

function receiptTitleForActivity(activity, summary) {
  const namedTool = activity
    .flatMap((item) => (hasToolCalls(item) ? item.toolCalls : [item]))
    .map((item) => cleanToolLabel(item?.toolName))
    .find(Boolean);
  return namedTool || summary.label;
}

function receiptRowsForActivity(activity, summary) {
  const outcome = receiptOutcomeForActivity(activity);
  const rows = [
    {
      label: "Outcome",
      value: outcome || "Completed successfully",
    },
    {
      label: "Steps",
      value: `${summary.toolCount} ${summary.toolCount === 1 ? "tool step" : "tool steps"}`,
    },
  ];
  const resultRef = firstActivityValue(activity, "resultRef");
  if (resultRef && resultRef !== outcome) {
    rows.push({ label: "Result", value: shortenText(resultRef, 120) });
  }
  return rows;
}

function receiptOutcomeForActivity(activity) {
  const preview =
    firstActivityValue(activity, "toolResultPreview") ||
    firstActivityValue(activity, "toolDetail") ||
    firstActivityValue(activity, "outputKind");
  return shortenText(preview, 140);
}

function receiptLinkForActivity(activity) {
  const href = [
    firstActivityValue(activity, "resultRef"),
    firstActivityValue(activity, "toolResultPreview"),
  ]
    .map((value) => String(value || "").trim())
    .find((value) => /^https?:\/\//i.test(value));
  return href ? { href, label: "Open result" } : null;
}

function firstActivityValue(activity, key) {
  for (const item of activity) {
    const items = hasToolCalls(item) ? item.toolCalls : [item];
    for (const candidate of items) {
      const value = candidate?.[key];
      if (value !== null && value !== undefined && String(value).trim()) {
        return String(value).trim();
      }
    }
  }
  return "";
}

function cleanToolLabel(value) {
  const text = String(value || "").trim();
  if (!text || text === "tool") return "";
  return text.replace(/[_-]+/g, " ");
}

function shortenText(value, maxLength) {
  const text = String(value || "")
    .replace(/\s+/g, " ")
    .trim();
  if (!text || text.length <= maxLength) return text;
  return `${text.slice(0, Math.max(0, maxLength - 1)).trim()}...`;
}
