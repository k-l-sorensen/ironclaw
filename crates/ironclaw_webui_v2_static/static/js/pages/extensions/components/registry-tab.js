import { React, html } from "../../../lib/html.js";
import { useNavigate } from "react-router";
import { Button } from "../../../design-system/button.js";
import { Icon } from "../../../design-system/icons.js";
import { useT } from "../../../lib/i18n.js";
import { ConnectorAppIcon, ExtensionCard, RegistryCard } from "./extension-card.js";

// Curated chief-of-staff connection set. These are the high-leverage apps
// IronClaw should support; they render even when the gateway exposes no live
// catalog so the Connections surface stays a prepared desk (design law: no
// blank prompt) while staying honest about what is actually connectable
// (design law: no fake readiness — buttons stay disabled until the catalog is
// real).
export const CORE_CONNECTIONS = [
  {
    id: "gmail",
    display_name: "Gmail",
    kind: "wasm_tool",
    description: "Read, triage, draft, and prepare email work with approval gates.",
    package_ref: { kind: "extension", id: "tools/gmail" },
    keywords: ["email", "google", "inbox"],
  },
  {
    id: "google-calendar",
    display_name: "Google Calendar",
    kind: "wasm_tool",
    description: "Find meetings, protect focus blocks, and prepare schedule changes.",
    package_ref: { kind: "extension", id: "tools/google_calendar" },
    keywords: ["calendar", "google", "schedule"],
  },
  {
    id: "google-drive",
    display_name: "Google Drive",
    kind: "wasm_tool",
    description: "Ground prep, summaries, and answers in Drive documents and folders.",
    package_ref: { kind: "extension", id: "tools/google_drive" },
    keywords: ["drive", "docs", "files"],
  },
  {
    id: "google-sheets",
    display_name: "Google Sheets",
    kind: "wasm_tool",
    description: "Append CRM rows, bug reports, and recurring tracker output to Sheets.",
    package_ref: { kind: "extension", id: "tools/google_sheets" },
    keywords: ["sheets", "spreadsheet", "crm"],
  },
  {
    id: "notion",
    display_name: "Notion",
    kind: "mcp_server",
    description: "Search team knowledge, draft pages, and keep decisions visible.",
    package_ref: { kind: "extension", id: "mcp-servers/notion" },
    keywords: ["knowledge", "docs", "wiki"],
  },
  {
    id: "slack",
    display_name: "Slack",
    kind: "wasm_channel",
    description: "Summarize channels, prepare replies, and surface urgent asks.",
    package_ref: { kind: "extension", id: "channels/slack" },
    keywords: ["messages", "team", "channels"],
  },
  {
    id: "telegram",
    display_name: "Telegram",
    kind: "wasm_channel",
    description: "Send scheduled digests and bot messages through Telegram.",
    package_ref: { kind: "extension", id: "channels/telegram" },
    keywords: ["bot", "news", "dm"],
  },
  {
    id: "github",
    display_name: "GitHub",
    kind: "wasm_tool",
    description: "Watch releases, summarize changes, and route follow-up tasks.",
    package_ref: { kind: "extension", id: "tools/github" },
    keywords: ["releases", "issues", "code"],
  },
  {
    id: "web-http",
    display_name: "Web & HTTP",
    kind: "builtin",
    description: "Fetch pages, search public sources, and watch endpoint health.",
    package_ref: null,
    keywords: ["web", "http", "monitor"],
  },
  {
    id: "routines",
    display_name: "Routines",
    kind: "builtin",
    description: "Schedule recurring checks, prep work, and delivery loops.",
    package_ref: null,
    keywords: ["schedule", "trigger", "automation"],
  },
  {
    id: "workspace",
    display_name: "Workspace files",
    kind: "builtin",
    description: "Use local documents, spreadsheets, PDFs, and generated work products in chat.",
    package_ref: null,
    keywords: ["files", "documents", "exports"],
  },
];

// Outcome-first workflows the connection layer should unlock. Each maps to the
// curated surfaces it needs, plus a chat draft so a user can start from the
// outcome rather than wiring apps first.
export const ACCEPTANCE_WORKFLOWS = [
  {
    id: "daily-news-digest",
    title: "Daily news digest",
    outcome: "Find NEAR AI news, summarize it, and deliver a short Telegram digest on a routine.",
    surfaces: ["telegram", "web-http", "routines"],
    prompt:
      "Create a recurring Telegram digest of the most important NEAR AI news. Start by drafting today's summary, then schedule the routine.",
  },
  {
    id: "calendar-prep-assistant",
    title: "Calendar prep assistant",
    outcome:
      "Prepare meeting briefs from Gmail, Calendar, Drive documents, public news, and a timed routine.",
    surfaces: ["gmail", "google-calendar", "google-drive", "web-http", "routines"],
    prompt:
      "Thirty minutes before my next meeting, prepare a company brief from Gmail, Calendar, Drive docs, and recent news.",
  },
  {
    id: "deployment-health-watcher",
    title: "Deployment health watcher",
    outcome: "Ping an endpoint on a schedule and send a Slack DM when health checks fail.",
    surfaces: ["slack", "web-http", "routines"],
    prompt:
      "Watch a deployment endpoint every five minutes and DM me in Slack if the status is not healthy.",
  },
  {
    id: "competitor-release-tracker",
    title: "Competitor release tracker",
    outcome: "Watch GitHub releases, summarize meaningful changes, and email the result.",
    surfaces: ["gmail", "github", "routines"],
    prompt:
      "Track competitor GitHub releases, summarize meaningful changes, and email me a recurring update.",
  },
  {
    id: "slack-ama",
    title: "AMA in Slack",
    outcome:
      "Answer Slack questions from a Drive strategy document without losing source grounding.",
    surfaces: ["slack", "google-drive"],
    prompt:
      "Use the strategy document in Drive as a knowledge base and answer Slack DMs with grounded answers.",
  },
  {
    id: "crm-inbound-tracker",
    title: "CRM inbound tracker",
    outcome: "Find inbound Gmail from near.ai domains and append structured rows to Google Sheets.",
    surfaces: ["gmail", "google-sheets", "routines"],
    prompt:
      "Every thirty minutes, find new near.ai-domain inbound emails and append the right fields to a Google Sheet.",
  },
  {
    id: "slack-sheet-bug-logger",
    title: "Slack to Sheet bug logger",
    outcome: 'Turn Slack messages that start with "bug:" into rows in a product bug spreadsheet.',
    surfaces: ["slack", "google-sheets", "routines"],
    prompt:
      "Watch the product Slack channel for messages that start with bug: and append them to a Google Sheet.",
  },
  {
    id: "hn-keyword-monitor",
    title: "HN keyword monitor",
    outcome:
      "Search Hacker News for IronClaw and NEAR AI mentions and send Slack summaries hourly.",
    surfaces: ["slack", "web-http", "routines"],
    prompt:
      "Search Hacker News hourly for IronClaw or NEAR AI mentions and send a concise Slack summary.",
  },
];

function packageId(item) {
  return item?.package_ref?.id || "";
}

function catalogItem(entry) {
  return entry.entry || entry.extension || {};
}

export function coreConnectionButtonState({ entry, gatewayOffline, catalogUnavailable, isBusy }) {
  if (!entry.package_ref) return { disabled: true, label: "Built in" };
  if (gatewayOffline) return { disabled: true, label: "Gateway offline" };
  if (catalogUnavailable) return { disabled: true, label: "Not available" };
  if (isBusy) return { disabled: true, label: "Connect" };
  return { disabled: false, label: "Connect" };
}

export function coreConnectionKindLabel(entry) {
  if (entry.id === "web-http") return "Web";
  if (entry.id === "routines") return "Routine";
  if (entry.id === "workspace") return "Files";
  if (entry.kind === "mcp_server") return "Knowledge";
  if (entry.kind === "wasm_channel") return "Messaging";
  if (entry.kind === "builtin") return "Built-in";
  return "Tool";
}

function connectionBySurfaceId(surfaceId) {
  return CORE_CONNECTIONS.find((entry) => entry.id === surfaceId) || null;
}

function catalogKeys(entry) {
  const keys = new Set();
  if (!entry) return keys;
  if (entry.id) keys.add(String(entry.id));
  if (entry.package_ref?.id) keys.add(String(entry.package_ref.id));
  if (entry.packageRef?.id) keys.add(String(entry.packageRef.id));
  if (typeof entry.package_ref === "string") keys.add(entry.package_ref);
  if (typeof entry.packageRef === "string") keys.add(entry.packageRef);
  return keys;
}

export function acceptanceWorkflowStatus({ gatewayOffline, catalogUnavailable, availableEntries }) {
  if (gatewayOffline) return "Gateway offline";
  if (catalogUnavailable) return "Waiting on app catalog";
  if (Array.isArray(availableEntries) && availableEntries.length > 0) return "Catalog loaded";
  return "Connect required apps";
}

export function workflowCatalogStatus(
  workflow,
  { gatewayOffline = false, catalogUnavailable = false, availableEntries = [] } = {}
) {
  if (gatewayOffline) {
    return { label: "Gateway offline", tone: "warning", missingSurfaces: [] };
  }
  if (catalogUnavailable) {
    return { label: "Waiting on app catalog", tone: "muted", missingSurfaces: [] };
  }

  const availableKeys = new Set(
    availableEntries.flatMap((entry) => Array.from(catalogKeys(entry)))
  );
  const missingSurfaces = (workflow?.surfaces || []).filter((surfaceId) => {
    const connection = connectionBySurfaceId(surfaceId);
    if (!connection?.package_ref) return false;
    const requiredKeys = catalogKeys(connection);
    return !Array.from(requiredKeys).some((key) => availableKeys.has(key));
  });

  if (missingSurfaces.length > 0) {
    const label =
      missingSurfaces.length === 1
        ? "1 app missing from catalog"
        : `${missingSurfaces.length} apps missing from catalog`;
    return { label, tone: "warning", missingSurfaces };
  }

  return { label: "Ready to connect", tone: "positive", missingSurfaces: [] };
}

export function RegistryTab({
  catalogEntries,
  loadError,
  onInstall,
  onActivate,
  onConfigure,
  onRemove,
  isBusy,
}) {
  const t = useT();
  const [filter, setFilter] = React.useState("");
  const query = filter.trim().toLowerCase();

  const filtered = query
    ? catalogEntries.filter((entry) => {
        const item = catalogItem(entry);
        return (
          (item.display_name || packageId(item)).toLowerCase().includes(query) ||
          (item.description || "").toLowerCase().includes(query) ||
          (item.keywords || []).some((kw) => kw.toLowerCase().includes(query))
        );
      })
    : catalogEntries;

  const installedEntries = filtered.filter((entry) => entry.installed && entry.extension);
  const registryOnlyInstalledEntries = filtered.filter(
    (entry) => entry.installed && !entry.extension && entry.entry
  );
  const installedCount = installedEntries.length + registryOnlyInstalledEntries.length;
  const availableEntries = filtered.filter((entry) => !entry.installed && entry.entry);

  // Raw registry entries present in the catalog (unfiltered) drive which curated
  // workflow surfaces count as connectable.
  const catalogRawEntries = catalogEntries
    .filter((entry) => entry.entry)
    .map((entry) => entry.entry);

  // "Catalog" means the registry. When the gateway exposes no installable
  // registry entries, show the curated prepared-desk surface — even if some
  // extensions are already installed (those live in the Installed tab).
  if (catalogRawEntries.length === 0) {
    return html`<${CoreConnectionsEmpty}
      loadError=${loadError}
      onInstall=${onInstall}
      isBusy=${isBusy}
    />`;
  }

  return html`
    <div className="space-y-4">
      <div className="flex items-center gap-3">
        <input
          type="text"
          value=${filter}
          onChange=${(e) => setFilter(e.target.value)}
          placeholder=${t("ext.registry.searchPlaceholder")}
          className="h-9 flex-1 rounded-md border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-sm text-[var(--v2-text)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)]"
        />
        <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">
          ${filtered.length} / ${catalogEntries.length}
        </span>
      </div>

      <div className="rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-5 shadow-[var(--v2-shadow-sm)] sm:p-6">
        ${filtered.length === 0
          ? html`<p className="py-4 text-sm text-[var(--v2-text-muted)]">
              ${t("ext.registry.noMatch")}
            </p>`
          : html`
              ${installedCount > 0 &&
              html`
                <h3
                  className="mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
                >
                  ${t("extensions.installed")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${installedEntries.map(
                    (entry) => html`
                      <${ExtensionCard}
                        key=${entry.id}
                        ext=${entry.extension || entry.entry}
                        onActivate=${onActivate}
                        onConfigure=${onConfigure}
                        onRemove=${onRemove}
                        isBusy=${isBusy}
                      />
                    `
                  )}
                  ${registryOnlyInstalledEntries.map(
                    (entry) => html`
                      <${RegistryCard}
                        key=${entry.id}
                        entry=${entry.entry}
                        statusLabel=${t("extensions.installed")}
                        isBusy=${isBusy}
                      />
                    `
                  )}
                </div>
              `}

              ${availableEntries.length > 0 &&
              html`
                <h3
                  className=${[
                    "mb-4 font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]",
                    installedCount > 0 ? "mt-6" : "",
                  ].join(" ")}
                >
                  ${t("ext.registry.availableTitle")}
                </h3>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 2xl:grid-cols-3">
                  ${availableEntries.map(
                    (entry) => html`
                      <${RegistryCard}
                        key=${entry.id}
                        entry=${entry.entry}
                        onInstall=${onInstall}
                        isBusy=${isBusy}
                      />
                    `
                  )}
                </div>
              `}
            `}
      </div>

      <${AcceptanceWorkflowsPanel}
        gatewayOffline=${false}
        catalogUnavailable=${false}
        availableEntries=${catalogRawEntries}
      />
    </div>
  `;
}

function CoreConnectionsEmpty({ loadError, onInstall, isBusy }) {
  const gatewayOffline = Boolean(loadError);
  const catalogUnavailable = !gatewayOffline;
  return html`
    <div className="space-y-4">
      <section
        className="rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-5 shadow-[var(--v2-shadow-sm)] sm:p-6"
      >
        <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="max-w-2xl">
            <p
              className="text-xs font-semibold uppercase tracking-[0.08em] text-[var(--v2-accent-text)]"
            >
              Core connections
            </p>
            <h3 className="mt-2 text-xl font-semibold text-[var(--v2-text-strong)]">
              Connect the tools IronClaw should handle for you.
            </h3>
            <p className="mt-2 text-sm leading-6 text-[var(--v2-text-muted)]">
              ${gatewayOffline
                ? "The local gateway is still starting or unavailable. The apps are shown so setup feels predictable; connect buttons unlock when the gateway responds."
                : "This gateway did not expose installable app catalog entries yet. These are the high-leverage connections IronClaw should support when the catalog is available."}
            </p>
          </div>
          <div
            className="inline-flex items-center gap-2 rounded-full border border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--v2-warning-text)]"
          >
            <${Icon} name="pulse" className="h-3.5 w-3.5" />
            ${gatewayOffline ? "Gateway offline" : "Catalog unavailable"}
          </div>
        </div>
      </section>

      <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
        ${CORE_CONNECTIONS.map(
          (entry) => html`
            <${CoreConnectionCard}
              key=${entry.id}
              entry=${entry}
              gatewayOffline=${gatewayOffline}
              catalogUnavailable=${catalogUnavailable}
              isBusy=${isBusy}
              onInstall=${onInstall}
            />
          `
        )}
      </div>
      <${AcceptanceWorkflowsPanel}
        gatewayOffline=${gatewayOffline}
        catalogUnavailable=${catalogUnavailable}
      />
    </div>
  `;
}

function AcceptanceWorkflowsPanel({ gatewayOffline, catalogUnavailable, availableEntries = [] }) {
  const navigate = useNavigate();
  const connectionsById = Object.fromEntries(CORE_CONNECTIONS.map((entry) => [entry.id, entry]));
  const status = acceptanceWorkflowStatus({
    gatewayOffline,
    catalogUnavailable,
    availableEntries,
  });

  return html`
    <section
      data-testid="acceptance-workflows"
      className="rounded-[18px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-5 shadow-[var(--v2-shadow-sm)] sm:p-6"
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
        <div className="max-w-2xl">
          <p
            className="text-xs font-semibold uppercase tracking-[0.08em] text-[var(--v2-accent-text)]"
          >
            Chief-of-staff workflows
          </p>
          <h3 className="mt-2 text-xl font-semibold text-[var(--v2-text-strong)]">
            Start from outcomes, not app setup.
          </h3>
          <p className="mt-2 text-sm leading-6 text-[var(--v2-text-muted)]">
            These are the work loops the connection layer should unlock once the required apps are
            configured.
          </p>
        </div>
        <span
          className="inline-flex items-center gap-2 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-1.5 text-xs font-semibold text-[var(--v2-text-muted)]"
        >
          <${Icon}
            name=${gatewayOffline || catalogUnavailable ? "pulse" : "check"}
            className="h-3.5 w-3.5"
          />
          ${status}
        </span>
      </div>

      <div className="mt-5 grid grid-cols-1 gap-3 lg:grid-cols-2">
        ${ACCEPTANCE_WORKFLOWS.map((workflow) => {
          const workflowStatus = workflowCatalogStatus(workflow, {
            gatewayOffline,
            catalogUnavailable,
            availableEntries,
          });
          const missingSurfaceSet = new Set(workflowStatus.missingSurfaces);
          return html`
            <article
              key=${workflow.id}
              data-testid="acceptance-workflow-card"
              className="flex h-full flex-col rounded-[14px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-4"
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <h4 className="text-base font-semibold text-[var(--v2-text-strong)]">
                    ${workflow.title}
                  </h4>
                  <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
                    ${workflow.outcome}
                  </p>
                </div>
                <span
                  className="shrink-0 rounded-full bg-[var(--v2-surface-soft)] px-2 py-1 text-[11px] font-medium text-[var(--v2-text-faint)]"
                >
                  ${workflow.surfaces.length} apps
                </span>
              </div>

              <div className="mt-4 flex flex-wrap gap-2">
                ${workflow.surfaces.map((surface) => {
                  const connection = connectionsById[surface];
                  return html`
                    <span
                      key=${surface}
                      data-workflow-surface-state=${missingSurfaceSet.has(surface)
                        ? "missing"
                        : "available"}
                      className=${[
                        "inline-flex items-center gap-1.5 rounded-full border py-1 pl-1 pr-2 text-[11px] font-medium",
                        missingSurfaceSet.has(surface)
                          ? "border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]"
                          : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
                      ].join(" ")}
                    >
                      <${ConnectorAppIcon} source=${connection} className="h-5 w-5 rounded-[6px]" />
                      <span>${connection?.display_name || surface}</span>
                    </span>
                  `;
                })}
              </div>

              <div className="mt-auto flex items-center justify-between gap-3 pt-4">
                <span
                  data-workflow-status-tone=${workflowStatus.tone}
                  className=${[
                    "text-xs",
                    workflowStatus.tone === "positive"
                      ? "font-semibold text-[var(--v2-positive-text)]"
                      : workflowStatus.tone === "warning"
                        ? "font-semibold text-[var(--v2-warning-text)]"
                        : "text-[var(--v2-text-faint)]",
                  ].join(" ")}
                >
                  ${workflowStatus.label}
                </span>
                <button
                  type="button"
                  onClick=${() =>
                    navigate("/chat", { state: { composerDraft: workflow.prompt } })}
                  aria-label=${`Draft prompt for ${workflow.title}`}
                  className="inline-flex h-11 shrink-0 items-center rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-xs font-semibold text-[var(--v2-text-strong)] hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] hover:text-[var(--v2-accent-text)]"
                >
                  Draft prompt
                </button>
              </div>
            </article>
          `;
        })}
      </div>
    </section>
  `;
}

function CoreConnectionCard({ entry, gatewayOffline, catalogUnavailable, isBusy, onInstall }) {
  const canInstall = Boolean(entry.package_ref && onInstall);
  const { disabled, label } = coreConnectionButtonState({
    entry,
    gatewayOffline,
    catalogUnavailable,
    isBusy,
  });
  return html`
    <article
      className="rounded-[16px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-4 shadow-[var(--v2-shadow-sm)]"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-start gap-3">
          <${ConnectorAppIcon} source=${entry} />
          <div className="min-w-0">
            <h4 className="text-base font-semibold text-[var(--v2-text-strong)]">
              ${entry.display_name}
            </h4>
            <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
              ${entry.description}
            </p>
          </div>
        </div>
        <span
          className="rounded-full bg-[var(--v2-surface-soft)] px-2 py-1 font-mono text-[10px] uppercase tracking-[0.1em] text-[var(--v2-text-faint)]"
        >
          ${coreConnectionKindLabel(entry)}
        </span>
      </div>
      <div className="mt-4 flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-wrap gap-1.5">
          ${(entry.keywords || [])
            .slice(0, 3)
            .map(
              (keyword) => html`
                <span
                  key=${keyword}
                  className="rounded-full bg-[var(--v2-surface-soft)] px-2 py-1 text-[11px] text-[var(--v2-text-muted)]"
                >
                  ${keyword}
                </span>
              `
            )}
        </div>
        <${Button}
          type="button"
          variant=${canInstall && !disabled ? "primary" : "secondary"}
          size="sm"
          className="min-h-[44px] shrink-0 px-3.5"
          disabled=${disabled}
          onClick=${() =>
            canInstall &&
            onInstall({ packageRef: entry.package_ref, displayName: entry.display_name })}
        >
          ${label}
        <//>
      </div>
    </article>
  `;
}
