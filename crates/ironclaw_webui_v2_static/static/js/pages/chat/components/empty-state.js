import { useQuery } from "@tanstack/react-query";
import { Link } from "react-router";
import { Icon } from "../../../design-system/icons.js";
import { listAutomations } from "../../../lib/api.js";
import { isDesktopRuntime } from "../../../lib/api.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { normalizeAutomations } from "../../automations/lib/automations-presenters.js";
import { filterDesktopVisibleLlmProviders } from "../../settings/lib/llm-providers.js";
import { fetchLlmProviders } from "../../settings/lib/settings-api.js";
import { useThreadStates } from "../../../lib/thread-state.js";
import { buildFrontDoorData } from "../lib/frontdoor-data.js";
import { ChatInput } from "./chat-input.js";

// Two landings share this entry point. Web (mono, source of truth) shows the
// calm hero + three example prompts and forwards `draftKey` so the composer
// keeps its per-thread draft. Desktop shows the anticipatory front door —
// time-of-day greeting, recent threads, automations brief — which depends on
// the heavy front-door data (`frontdoor-data`), automations, and the LLM
// provider snapshot. That whole subsystem is gated behind `isDesktopRuntime()`
// (the guard tolerates the import-stripping unit harness).
export function EmptyState(props) {
  const isDesktop = typeof isDesktopRuntime === "function" && isDesktopRuntime();
  return isDesktop
    ? html`<${DesktopFrontDoor} ...${props} />`
    : html`<${WebLanding} ...${props} />`;
}

/* ----------------------------------------------------------------------------
 * Web landing — mono's hero. Keeps `draftKey` flowing to the composer.
 * ------------------------------------------------------------------------- */
function WebLanding({
  onSuggestion,
  onSend,
  disabled,
  initialText,
  resetKey,
  draftKey,
  context,
  statusText,
  canCancel,
  onCancel,
}) {
  const t = useT();
  const suggestions = [
    {
      icon: "tool",
      title: t("chat.suggestion1"),
      detail: t("chat.suggestion1Desc"),
    },
    {
      icon: "shield",
      title: t("chat.suggestion2"),
      detail: t("chat.suggestion2Desc"),
    },
    {
      icon: "plug",
      title: t("chat.suggestion3"),
      detail: t("chat.suggestion3Desc"),
    },
  ];

  return html`
    <div
      className="v2-page-entrance flex min-h-0 flex-1 flex-col items-center justify-center px-4 py-8 sm:px-8 lg:px-12"
    >
      <div className="w-full max-w-5xl text-center">
        <h2
          className="mx-auto max-w-[16ch] text-4xl font-semibold leading-[1.04] text-white sm:text-5xl lg:text-6xl"
        >
          ${t("chat.heroTitle")}
        </h2>
        <p
          className="mx-auto mt-4 max-w-[64ch] text-base leading-relaxed text-iron-300"
        >
          ${t("chat.heroDesc")}
        </p>
      </div>

      <div className="mt-9 w-full max-w-5xl">
        <${ChatInput}
          onSend=${onSend}
          disabled=${disabled}
          initialText=${initialText}
          resetKey=${resetKey}
          draftKey=${draftKey}
          variant="hero"
          context=${context}
          statusText=${statusText}
          canCancel=${canCancel}
          onCancel=${onCancel}
        />
      </div>

      <div className="mt-8 grid w-full max-w-5xl gap-2">
        ${suggestions.map(
          (item) => html`
            <button
              type="button"
              key=${item.title}
              onClick=${() => onSuggestion(item.title)}
              className="v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 border-t border-white/10 px-2 py-4 text-left hover:border-signal/35"
            >
              <span
                className="grid h-8 w-8 place-items-center rounded-full border border-white/10 bg-white/[0.035] text-iron-300 group-hover:border-signal/35 group-hover:text-signal"
              >
                <${Icon} name=${item.icon} className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block text-sm font-semibold text-iron-100">
                  ${item.title}
                </span>
                <span className="mt-0.5 block text-sm text-iron-300">
                  ${item.detail}
                </span>
              </span>
            </button>
          `
        )}
      </div>
    </div>
  `;
}

/* ----------------------------------------------------------------------------
 * Desktop front door — anticipation over interrogation: greets by time of day,
 * offers the threads you were just working in, and showcases tasks the app can
 * genuinely finish today (document summarize / draft / spreadsheet analysis —
 * all backed by the client-side extractors). Suggestion clicks PREFILL the
 * composer rather than firing blind, since two of the three start with
 * attaching a file. This whole path is desktop-only (see EmptyState gate).
 * ------------------------------------------------------------------------- */
function greetingKey() {
  const hour = new Date().getHours();
  if (hour < 12) return "chat.heroMorning";
  if (hour < 18) return "chat.heroAfternoon";
  return "chat.heroEvening";
}

function relativeAge(iso, t) {
  const stamp = Date.parse(iso || "");
  if (!Number.isFinite(stamp)) return "";
  const minutes = Math.max(1, Math.round((Date.now() - stamp) / 60_000));
  if (minutes < 60) return t("chat.resumeMinutes", { n: String(minutes) });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("chat.resumeHours", { n: String(hours) });
  return t("chat.resumeDays", { n: String(Math.round(hours / 24)) });
}

function visibleLlmSnapshot(snapshot = {}) {
  const providers = filterDesktopVisibleLlmProviders(
    Array.isArray(snapshot.providers) ? snapshot.providers : []
  );
  const rawActive = snapshot.active || null;
  const active =
    rawActive && providers.some((provider) => provider.id === rawActive.provider_id)
      ? rawActive
      : null;
  return { providers, active };
}

const FRONT_DOOR_LAST_SEEN_KEY = "ironclaw:frontdoor:lastSeenAt";

function readFrontDoorLastSeen() {
  try {
    const value = Number(
      window.localStorage && window.localStorage.getItem(FRONT_DOOR_LAST_SEEN_KEY)
    );
    return Number.isFinite(value) && value > 0 ? value : 0;
  } catch {
    return 0;
  }
}

function writeFrontDoorLastSeen(value) {
  try {
    if (window.localStorage) window.localStorage.setItem(FRONT_DOOR_LAST_SEEN_KEY, String(value));
  } catch {
    // localStorage unavailable (private mode) — the brief just stays empty.
  }
}

function DesktopFrontDoor({
  threads = [],
  threadStates,
  onSuggestion: _onSuggestion,
  onSend,
  disabled,
  initialText,
  resetKey,
  context,
  statusText,
  canCancel,
  onCancel,
}) {
  const t = useT();
  // The state map is sourced internally so the shared chat.js does not need a
  // desktop-only prop; the prop is honored when chat.js does pass it. The hook
  // is always called (rules of hooks) and the prop simply overrides its value.
  const liveThreadStates = useThreadStates();
  const resolvedThreadStates = threadStates || liveThreadStates;
  // Reads the cached threads list (loaded by the sidebar); no extra traffic.
  const threadsQuery = useQuery({ queryKey: ["threads"], enabled: false });
  const providersQuery = useQuery({
    queryKey: ["llm-providers"],
    queryFn: fetchLlmProviders,
    staleTime: 60_000,
  });
  // The cached ['threads'] query is an infinite query (paged); flatten its
  // pages. Tolerate the legacy { threads } shape too so this never silently
  // empties.
  const cachedThreads =
    threadsQuery.data?.pages?.flatMap((page) => page?.threads || []) ||
    threadsQuery.data?.threads ||
    [];
  const rawThreads = threads.length > 0 ? threads : cachedThreads;
  const automationsQuery = useQuery({
    queryKey: ["automations"],
    queryFn: () => listAutomations({ limit: 10 }),
    staleTime: 60_000,
  });
  const recentThreads = rawThreads
    .filter((thread) => (thread?.thread_id || thread?.id) && (thread.title || "").trim())
    .slice(0, 3);
  // Device-local last-seen watermark for the "Since your last visit" brief.
  // Captured once on mount (the value when the user arrived), then advanced to
  // now — so what shows is real change since the previous landing visit, not a
  // server "presence" claim.
  const lastSeenRef = React.useRef(readFrontDoorLastSeen());
  React.useEffect(() => {
    writeFrontDoorLastSeen(Date.now());
  }, []);
  const lastSeenAt = lastSeenRef.current;
  const frontDoor = React.useMemo(
    () =>
      buildFrontDoorData({
        threads: rawThreads,
        threadStates: resolvedThreadStates,
        automations: normalizeAutomations(automationsQuery.data),
        lastSeenAt,
      }),
    [rawThreads, resolvedThreadStates, automationsQuery.data, lastSeenAt]
  );
  const providersSnapshot = providersQuery.data;
  const visibleProvidersSnapshot = visibleLlmSnapshot(providersSnapshot || {});
  const providerSetupChecking = Boolean(!providersSnapshot && providersQuery.isLoading);
  const providerSetupRequired = Boolean(providersSnapshot && !visibleProvidersSnapshot.active);
  const providerSetupFailed = Boolean(!providersSnapshot && providersQuery.error);

  const [draft, setDraft] = React.useState("");
  const [draftKey, setDraftKey] = React.useState(0);
  const prefill = (text) => {
    setDraft(text);
    setDraftKey((key) => key + 1);
  };

  const suggestions = [
    {
      icon: "file",
      title: t("chat.suggestion1"),
      detail: t("chat.suggestion1Desc"),
      prompt: t("chat.suggestion1Prompt"),
    },
    {
      icon: "send",
      title: t("chat.suggestion2"),
      detail: t("chat.suggestion2Desc"),
      prompt: t("chat.suggestion2Prompt"),
    },
    {
      icon: "pulse",
      title: t("chat.suggestion3"),
      detail: t("chat.suggestion3Desc"),
      prompt: t("chat.suggestion3Prompt"),
    },
  ];
  const setupBlocked =
    context?.sendBlocked === true || providerSetupRequired || providerSetupFailed;
  const suggestionsBlocked = Boolean(setupBlocked || disabled);
  const briefRows = [
    providerSetupChecking
      ? {
          icon: "pulse",
          title: "Checking NEAR AI Cloud",
          detail: "IronClaw is checking your local gateway and model access.",
          tone: "muted",
        }
      : setupBlocked
        ? {
            icon: "lock",
            title: t("chat.briefNeedsSetupTitle"),
            detail: context?.sendBlockReason || t("chat.briefNeedsSetupDesc"),
            tone: "warning",
          }
        : {
            icon: "check",
            title: t("chat.briefReadyTitle"),
            detail: t("chat.briefReadyDesc"),
            tone: "positive",
          },
    {
      icon: recentThreads.length > 0 ? "chat" : "folder",
      title: recentThreads.length > 0 ? t("chat.briefResumeTitle") : t("chat.briefWorkspaceTitle"),
      detail: recentThreads.length > 0 ? t("chat.briefResumeDesc") : t("chat.briefWorkspaceDesc"),
      tone: "muted",
    },
    {
      icon: "shield",
      title: t("chat.briefSafetyTitle"),
      detail: t("chat.briefSafetyDesc"),
      tone: "gold",
    },
  ];

  return html`
    <div
      className="v2-page-entrance flex min-h-0 flex-1 items-center overflow-y-auto px-4 py-6 sm:px-8 lg:px-12"
    >
      <div
        className="mx-auto grid w-full max-w-6xl gap-7 lg:grid-cols-[1.08fr_0.92fr] lg:items-start"
      >
        <section className="min-w-0">
          <div
            className="mb-3 font-mono text-[11px] font-semibold uppercase tracking-[0.14em] text-[var(--v2-accent-text)]"
          >
            ${t("chat.briefLabel")}
          </div>
          <h2
            className="max-w-[20ch] text-[28px] font-semibold leading-[1.1] tracking-[-0.01em] text-[var(--v2-text-strong)] sm:text-[32px]"
          >
            ${t(greetingKey())}
          </h2>
          <p className="mt-4 max-w-[58ch] text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("chat.heroDesc")}
          </p>

          <div className="mt-7 grid gap-2">
            ${briefRows.map(
              (item) => html`
                <div
                  key=${item.title}
                  className="grid grid-cols-[auto_1fr] gap-3 border-t border-[var(--v2-panel-border)] py-3"
                >
                  <span
                    className=${[
                      "mt-0.5 grid h-8 w-8 place-items-center rounded-[8px] border",
                      item.tone === "positive"
                        ? "border-[color-mix(in_srgb,var(--v2-positive-text)_30%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]"
                        : item.tone === "warning"
                          ? "border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]"
                          : item.tone === "gold"
                            ? "border-[color-mix(in_srgb,var(--v2-gold)_34%,var(--v2-panel-border))] bg-[var(--v2-gold-soft)] text-[var(--v2-gold-text)]"
                            : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]",
                    ].join(" ")}
                  >
                    <${Icon} name=${item.icon} className="h-4 w-4" />
                  </span>
                  <span>
                    <span className="block text-sm font-semibold text-[var(--v2-text-strong)]">
                      ${item.title}
                    </span>
                    <span className="mt-0.5 block text-sm leading-6 text-[var(--v2-text-muted)]">
                      ${item.detail}
                    </span>
                  </span>
                </div>
              `
            )}
          </div>

          ${recentThreads.length > 0 &&
          html`
            <div className="mt-5">
              <div
                className="mb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--v2-text-faint)]"
              >
                ${t("chat.resumeHeading")}
              </div>
              <div className="grid gap-2">
                ${recentThreads.map(
                  (thread) => html`
                    <${Link}
                      key=${thread.thread_id || thread.id}
                      to=${`/chat/${thread.thread_id || thread.id}`}
                      className="group grid min-w-0 grid-cols-[auto_1fr_auto] items-center gap-2 rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 text-sm text-[var(--v2-text)] hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] hover:text-[var(--v2-accent-text)]"
                    >
                      <${Icon} name="chat" className="h-3.5 w-3.5 shrink-0 opacity-70" />
                      <span className="truncate">${thread.title}</span>
                      ${thread.updated_at &&
                      html`<span className="shrink-0 text-xs text-[var(--v2-text-faint)]">
                        ${relativeAge(thread.updated_at, t)}
                      </span>`}
                    <//>
                  `
                )}
              </div>
            </div>
          `}
        </section>

        <section className="min-w-0">
          <${ChatInput}
            onSend=${onSend}
            disabled=${disabled}
            initialText=${draft || initialText}
            resetKey=${`${resetKey}-${draftKey}`}
            variant="hero"
            context=${context}
            statusText=${statusText}
            canCancel=${canCancel}
            onCancel=${onCancel}
          />

          ${setupBlocked &&
          html`
            <div
              className="mt-3 rounded-[12px] border border-[color-mix(in_srgb,var(--v2-warning-text)_34%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-4 py-3"
            >
              <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                Connect NEAR AI Cloud once, then ask naturally.
              </div>
              <div className="mt-1 text-sm leading-5 text-[var(--v2-text-muted)]">
                Normal desktop setup uses NEAR AI Cloud; no separate model-provider keys are needed.
              </div>
              <${Link}
                to="/settings/inference"
                className="mt-3 inline-flex h-11 items-center rounded-[8px] bg-[var(--v2-accent-btn)] px-4 text-sm font-semibold text-white"
              >
                Open setup
              <//>
            </div>
          `}

          <${FrontDoorPanel}
            sinceAway=${frontDoor.sinceAway}
            sinceAwayTotal=${frontDoor.sinceAwayTotal}
            needsYou=${frontDoor.needsYou}
            needsYouTotal=${frontDoor.needsYouTotal}
            handled=${frontDoor.handled}
          />

          <div className="mt-4 grid gap-2">
            ${suggestions.map(
              (item) => html`
                <button
                  type="button"
                  key=${item.title}
                  disabled=${suggestionsBlocked}
                  onClick=${() => prefill(item.prompt)}
                  className=${[
                    "v2-button group grid grid-cols-[auto_1fr_auto] items-center gap-3 rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-3 text-left",
                    suggestionsBlocked
                      ? "cursor-not-allowed opacity-60"
                      : "hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))]",
                  ].join(" ")}
                >
                  <span
                    className="grid h-8 w-8 place-items-center rounded-[8px] border border-[var(--v2-panel-border)] text-[var(--v2-text-muted)] group-hover:border-[var(--v2-accent)] group-hover:text-[var(--v2-accent-text)]"
                  >
                    <${Icon} name=${item.icon} className="h-4 w-4" />
                  </span>
                  <span className="min-w-0">
                    <span className="block text-sm font-semibold text-[var(--v2-text-strong)]">
                      ${item.title}
                    </span>
                    <span className="mt-0.5 block text-sm leading-5 text-[var(--v2-text-muted)]">
                      ${item.detail}
                    </span>
                  </span>
                  <span
                    className="self-start whitespace-nowrap text-xs font-medium text-[var(--v2-text-faint)]"
                  >
                    ${suggestionsBlocked ? "Setup first" : t("chat.suggestionUse")}
                  </span>
                </button>
              `
            )}
          </div>
        </section>
      </div>
    </div>
  `;
}

function FrontDoorPanel({ sinceAway = [], sinceAwayTotal = 0, needsYou, needsYouTotal, handled }) {
  return html`
    <div
      className="mt-4 grid gap-3 rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] p-3"
      data-testid="frontdoor-panel"
    >
      ${sinceAway.length > 0 &&
      html`<${FrontDoorSection}
        title="Since your last visit"
        emptyTitle=""
        emptyDetail=""
        tone="gold"
        items=${sinceAway}
        count=${sinceAwayTotal}
        moreLabel="more on Scheduled"
      />`}
      <${FrontDoorSection}
        title="Needs you"
        emptyTitle="Nothing waiting on you."
        emptyDetail="Approvals and auth gates appear here when they are backed by a thread."
        tone="warning"
        items=${needsYou}
        count=${needsYouTotal}
      />
      <${FrontDoorSection}
        title="Handled"
        emptyTitle="No completed receipts yet."
        emptyDetail="Completed actions, automations, and recent work appear here once IronClaw has evidence."
        tone="gold"
        items=${handled}
      />
    </div>
  `;
}

function FrontDoorSection({
  title,
  emptyTitle,
  emptyDetail,
  tone,
  items,
  count,
  moreLabel = "more in your threads",
}) {
  // The badge reports the TRUE total (count) when provided, even though only the
  // top rows render — never under-report how many items actually need the user.
  const total = typeof count === "number" ? count : items.length;
  // Color is attribution, not decoration: gold/warning only earns its place when
  // there are real items to point at. An empty count stays muted so a quiet desk
  // never glows with a meaningless "0".
  const populated = total > 0;
  const toneClass =
    tone === "gold"
      ? "bg-[var(--v2-gold-soft)] text-[var(--v2-gold-text)]"
      : "bg-[var(--v2-warning-soft)] text-[var(--v2-warning-text)]";
  const countToneClass = populated
    ? toneClass
    : "bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]";
  return html`
    <section className="min-w-0" aria-label=${title} data-testid=${`frontdoor-${tone}`}>
      <div className="mb-2 flex items-center justify-between gap-3">
        <div className="text-[11px] font-semibold uppercase text-[var(--v2-text-faint)]">
          ${title}
        </div>
        <span className=${`rounded-[6px] px-2 py-0.5 text-[11px] font-semibold ${countToneClass}`}>
          ${total}
        </span>
      </div>
      <div className="grid gap-2">
        ${populated
          ? html`
              ${items.map(
                (item) => html`
                  <${Link}
                    key=${item.id}
                    to=${item.href}
                    className="grid min-w-0 grid-cols-[auto_1fr_auto] items-center gap-2 rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-2 hover:border-[color-mix(in_srgb,var(--v2-accent)_42%,var(--v2-panel-border))]"
                  >
                    <span className=${`grid h-8 w-8 place-items-center rounded-[8px] ${toneClass}`}>
                      <${Icon} name=${item.icon} className="h-4 w-4" />
                    </span>
                    <span className="min-w-0">
                      <span className="flex min-w-0 items-center gap-2">
                        <span
                          className="truncate text-sm font-semibold text-[var(--v2-text-strong)]"
                        >
                          ${item.title}
                        </span>
                        <span
                          className="shrink-0 rounded-[4px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-[0.04em] text-[var(--v2-text-faint)]"
                        >
                          ${item.badge}
                        </span>
                      </span>
                      <span
                        className="mt-0.5 line-clamp-2 text-xs leading-[1.45] text-[var(--v2-text-muted)]"
                        title=${item.age ? `${item.age} · ${item.detail}` : item.detail}
                      >
                        ${item.age
                          ? html`<span className="text-[var(--v2-text-faint)]"
                              >${item.age} ·
                            </span>`
                          : ""}${item.detail}
                      </span>
                    </span>
                    <span className="self-start text-xs font-semibold text-[var(--v2-accent-text)]"
                      >Open</span
                    >
                  <//>
                `
              )}
              ${total > items.length
                ? html`<div className="px-3 py-1.5 text-[11px] text-[var(--v2-text-faint)]">
                    +${total - items.length} ${moreLabel}
                  </div>`
                : ""}
            `
          : html`
              <div
                className="rounded-[8px] border border-dashed border-[var(--v2-panel-border)] px-3 py-2"
              >
                <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                  ${emptyTitle}
                </div>
                <div className="mt-0.5 text-xs leading-5 text-[var(--v2-text-muted)]">
                  ${emptyDetail}
                </div>
              </div>
            `}
      </div>
    </section>
  `;
}
