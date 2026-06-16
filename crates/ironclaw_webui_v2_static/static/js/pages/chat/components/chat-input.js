import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Icon } from "../../../design-system/icons.js";
import { Button } from "../../../design-system/button.js";
import { Popover } from "../../../design-system/popover.js";
import { React, html } from "../../../lib/html.js";
import { useT } from "../../../lib/i18n.js";
import { isDesktopRuntime } from "../../../lib/api.js";
import { authScope } from "../../../lib/auth-scope.js";
import { stageFiles } from "../lib/attachments.js";
import { useAttachmentConfig } from "../hooks/useAttachmentConfig.js";
import { formatSize, useComposerAttachments } from "../hooks/useComposerAttachments.js";
import { AttachmentPreviewModal } from "./attachment-preview.js";
import {
  fetchLlmProviders,
  listLlmProviderModels,
  setActiveLlm,
} from "../../settings/lib/settings-api.js";
import {
  filterDesktopVisibleLlmProviders,
  modelDisplayName,
} from "../../settings/lib/llm-providers.js";
import {
  NEW_DRAFT_KEY,
  clearDraft,
  clearStagedAttachments,
  getDraft,
  getStagedAttachments,
  setDraft,
  setStagedAttachments,
} from "../lib/draft-store.js";

// Two composer subsystems share one shell. The web build stages attachments on
// the server (`useAttachmentConfig` + `stageFiles`) and persists drafts per
// thread (`draft-store`); the desktop build runs the client-side composer
// (`useComposerAttachments`) and the NEAR AI Cloud model front-door. The shell
// owns text + send/cancel so both paths keep identical keyboard, status, and
// run-cancellation behavior; each subsystem renders its own attachment UI and
// reports its outgoing payload up through `composerRef`.
//
// `typeof isDesktopRuntime === 'function'` guards the runtime read so the unit
// harness (which strips imports) renders the web shell without a ReferenceError.
function selectDesktopRuntime() {
  return typeof isDesktopRuntime === "function" && isDesktopRuntime();
}

// Neutral payload contract used before a subsystem publishes (and as the
// fallback when the composer ref is not yet populated).
const EMPTY_COMPOSER_API = {
  getPayload: () => ({ images: [], attachments: [] }),
  reset: () => {},
  hasPayload: false,
  sendBlocked: false,
};

export function ChatInput({
  onSend,
  onCancel,
  disabled,
  canCancel = false,
  initialText = "",
  resetKey = "",
  // Default is applied in WebComposer (which owns draft-store) rather than here:
  // the composer unit harness strips imports, so referencing NEW_DRAFT_KEY in
  // this signature would throw before the shell renders.
  draftKey,
  variant = "dock",
  context = {},
  statusText = "",
}) {
  const t = useT();
  const isHero = variant === "hero";
  // useState order is load-bearing: text(0), isSending(1), isCancelling(2).
  // The cancel handler must stay the third state slot — the composer unit test
  // asserts the cancelling setter is index 2.
  const [text, setText] = React.useState("");
  const [isSending, setIsSending] = React.useState(false);
  const [isCancelling, setIsCancelling] = React.useState(false);
  // The active subsystem publishes { getPayload, reset, hasPayload, sendBlocked }
  // here so the shared shell can send and gate the send button without owning
  // the divergent attachment hooks.
  const composerRef = React.useRef(EMPTY_COMPOSER_API);
  const [composerSignal, setComposerSignal] = React.useState(0);
  const onComposerChange = React.useCallback((api) => {
    composerRef.current = api;
    // Re-render the shell so send-button enablement tracks the subsystem state.
    setComposerSignal((value) => value + 1);
  }, []);

  const textareaRef = React.useRef(null);

  const autoResize = React.useCallback(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, []);

  React.useEffect(() => {
    autoResize();
  }, [text, autoResize]);

  const handleSend = React.useCallback(async () => {
    const api = composerRef.current || EMPTY_COMPOSER_API;
    const trimmed = text.trim();
    if (
      (!trimmed && !api.hasPayload) ||
      disabled ||
      api.sendBlocked ||
      isSending
    )
      return;
    setIsSending(true);
    try {
      // The v2 send contract requires non-empty content, so attachments ride
      // along with text rather than sending on their own.
      await onSend(trimmed, api.getPayload());
      setText("");
      api.reset();
      if (textareaRef.current) textareaRef.current.style.height = "auto";
    } catch {
      // The failed optimistic message renders retry details in the thread.
    } finally {
      setIsSending(false);
    }
  }, [text, disabled, isSending, onSend]);

  const handleCancel = React.useCallback(async () => {
    if (!canCancel || isCancelling || !onCancel) return;
    setIsCancelling(true);
    try {
      await onCancel();
    } finally {
      setIsCancelling(false);
    }
  }, [canCancel, isCancelling, onCancel]);

  const onKeyDown = React.useCallback(
    (e) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend]
  );

  const placeholder = isHero
    ? t("chat.heroPlaceholder")
    : t("chat.followUpPlaceholder");
  const shellClass = isHero ? "w-full" : "px-4 py-3 sm:px-5 lg:px-8";
  const composerClass = [
    "relative mx-auto w-full max-w-5xl rounded-[20px] border border-[var(--v2-panel-border)] bg-[var(--v2-card-bg)] shadow-[var(--v2-card-shadow)] p-2.5 transition-colors",
    // Highlight the full rounded container on focus (not just the leaking
    // textarea ring), mirroring the global input:focus accent. Suppressed
    // while disabled so the Working-state composer never looks interactive.
    disabled
      ? ""
      : "focus-within:border-[var(--v2-accent)] focus-within:shadow-[0_0_0_3px_color-mix(in_srgb,var(--v2-accent)_28%,transparent)]",
    isHero ? "min-h-[120px]" : "",
    disabled ? "opacity-70" : "",
  ].join(" ");
  const textClass = [
    "w-full flex-1 resize-none border-0 !border-transparent !bg-transparent px-2 text-[0.9375rem] leading-6",
    "text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:!border-transparent focus:!bg-transparent focus:!outline-none focus:!shadow-none disabled:opacity-50",
    isHero ? "min-h-[72px]" : "min-h-[40px]",
  ].join(" ");

  const desktop = selectDesktopRuntime();
  // The ref may be null before the subsystem publishes (and the unit harness's
  // useRef mock returns { current: null }), so fall back to an empty contract.
  const api = composerRef.current || EMPTY_COMPOSER_API;
  // composerSignal is read so the linter and React both treat this render as
  // dependent on the subsystem's published state.
  void composerSignal;
  const hasPayload = Boolean(text.trim()) || api.hasPayload;
  const sendBlocked = api.sendBlocked;

  const subsystem = desktop
    ? html`<${DesktopComposer}
        disabled=${disabled}
        text=${text}
        onTextChange=${setText}
        onKeyDown=${onKeyDown}
        initialText=${initialText}
        resetKey=${resetKey}
        context=${context}
        textareaRef=${textareaRef}
        onComposerChange=${onComposerChange}
      />`
    : html`<${WebComposer}
        disabled=${disabled}
        text=${text}
        onTextChange=${setText}
        onKeyDown=${onKeyDown}
        initialText=${initialText}
        resetKey=${resetKey}
        draftKey=${draftKey}
        placeholder=${placeholder}
        textareaRef=${textareaRef}
        textClass=${textClass}
        onComposerChange=${onComposerChange}
      />`;

  return html`
    <div className=${shellClass}>
      <div className=${composerClass}>
        ${desktop
          ? html`
              <textarea
                ref=${textareaRef}
                value=${text}
                onChange=${(e) => setText(e.target.value)}
                onKeyDown=${onKeyDown}
                placeholder=${placeholder}
                rows=${1}
                disabled=${disabled}
                className=${textClass}
              />
              ${subsystem}
            `
          : subsystem}

        <div className="mt-2 flex items-center gap-2">
          ${disabled &&
          html`
            <span className="inline-flex items-center gap-2 text-xs text-[var(--v2-text-muted)]">
              <span className="h-2 w-2 rounded-full bg-[var(--v2-accent)]" />
              ${statusText || t("chat.statusWorking")}
            </span>
          `}
          <div className="ml-auto flex items-center gap-1.5">
            ${canCancel
              ? html`
                  <${Button}
                    type="button"
                    variant="danger"
                    size="icon-sm"
                    onClick=${handleCancel}
                    disabled=${isCancelling}
                    aria-label=${t("common.cancel")}
                    title=${t("common.cancel")}
                    className="rounded-full"
                  >
                    <${Icon} name="close" className="h-5 w-5" />
                  <//>
                `
              : html`
                  <${Button}
                    type="button"
                    variant="primary"
                    size="icon-sm"
                    onClick=${handleSend}
                    disabled=${disabled || isSending || sendBlocked || !hasPayload}
                    aria-label=${t("chat.send")}
                    className="rounded-full"
                  >
                    <${Icon} name="send" className="h-5 w-5" />
                  <//>
                `}
          </div>
        </div>
      </div>
    </div>
  `;
}

/* ----------------------------------------------------------------------------
 * Web composer — server-staging attachments + per-thread draft persistence.
 * Mono is the source of truth for web: keep `useAttachmentConfig`/`stageFiles`,
 * the `draft-store` draftKey persistence, debounced draft writes scoped to the
 * authenticated identity, and full i18n.
 * ------------------------------------------------------------------------- */
function WebComposer({
  disabled,
  text,
  onTextChange,
  onKeyDown,
  initialText,
  resetKey,
  draftKey = NEW_DRAFT_KEY,
  placeholder,
  textareaRef,
  textClass,
  onComposerChange,
}) {
  const t = useT();
  const limits = useAttachmentConfig();
  const [attachments, setAttachments] = React.useState(() =>
    getStagedAttachments(draftKey)
  );
  const [attachmentError, setAttachmentError] = React.useState("");
  const [dragOver, setDragOver] = React.useState(false);
  const fileInputRef = React.useRef(null);
  // Mirror of `attachments` plus a serial promise, so overlapping addFiles()
  // calls validate against the latest staged set rather than a stale snapshot
  // (each stageFiles is async; without this two fast drops could both admit
  // files past the per-message budget).
  const attachmentsRef = React.useRef([]);
  const stagingQueueRef = React.useRef(Promise.resolve());
  React.useEffect(() => {
    attachmentsRef.current = attachments;
  }, [attachments]);

  // Debounce draft persistence: localStorage writes are synchronous and
  // disk-backed, so writing on every keystroke can add typing latency. We hold
  // the latest {key, text, scope} and flush after a short idle, but also flush
  // immediately on unmount / thread switch so navigating away never drops the
  // last keystrokes, and cancel outright on send so a queued write can't
  // resurrect a just-sent draft.
  const pendingDraftRef = React.useRef(null);
  const draftTimerRef = React.useRef(null);
  const flushDraft = React.useCallback(() => {
    if (draftTimerRef.current) {
      window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = null;
    }
    const pending = pendingDraftRef.current;
    pendingDraftRef.current = null;
    // Drop the write if the authenticated identity changed since the draft was
    // queued (sign-out / 401 / token swap). Otherwise a flush triggered by the
    // unmount during auth teardown would re-persist the previous user's text
    // after the caches were purged.
    if (pending && pending.scope === authScope()) {
      setDraft(pending.key, pending.text);
    }
  }, []);
  const cancelPendingDraft = React.useCallback(() => {
    if (draftTimerRef.current) {
      window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = null;
    }
    pendingDraftRef.current = null;
  }, []);

  // Restore the persisted draft when the active conversation changes (draftKey
  // switches). The initialText effect below runs after this and overrides when
  // a location.state draft was passed in, so an explicit hand-off draft still
  // wins over the stored one.
  React.useEffect(() => {
    onTextChange(getDraft(draftKey));
    // Flush any queued write (for the previous key) before this key changes or
    // the composer unmounts, so a debounced draft is never lost.
    return () => flushDraft();
  }, [draftKey, flushDraft]);

  // Keep the in-memory staged-attachment store in sync so files survive
  // navigating away from (and back to) this composer, the same way the text
  // draft does. On a conversation switch, *re-read* the new key's files and
  // skip persisting this render — `attachments` still belongs to the previous
  // key, so persisting it here would leak the previous conversation's files
  // into the new one.
  const stagedDraftKeyRef = React.useRef(draftKey);
  React.useEffect(() => {
    if (stagedDraftKeyRef.current !== draftKey) {
      stagedDraftKeyRef.current = draftKey;
      setAttachments(getStagedAttachments(draftKey));
      // The composer stays mounted across conversation switches, so a stale
      // staging error would otherwise persist into every other thread.
      setAttachmentError("");
      return;
    }
    setStagedAttachments(draftKey, attachments);
  }, [draftKey, attachments]);

  React.useEffect(() => {
    if (!initialText) return;
    onTextChange(initialText);
    window.requestAnimationFrame(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.setSelectionRange(
          initialText.length,
          initialText.length
        );
      }
    });
  }, [initialText, resetKey]);

  // Stage dropped/picked/pasted files: validate against the server contract,
  // append the accepted ones, and surface any rejection reasons as a single
  // combined notice. `stageFiles` reads bytes to base64 off the main file list,
  // so this is async.
  const addFiles = React.useCallback(
    (files) => {
      // Paste/drop can call this while disabled; don't stage then.
      if (disabled || !files || files.length === 0) return;
      // Chain on the staging queue so calls run one-at-a-time and each sees the
      // attachments admitted by the previous one (via attachmentsRef). The
      // `.catch` guarantees the shared queue promise always resolves — an
      // unexpected staging failure must not permanently reject it and skip
      // every later add.
      stagingQueueRef.current = stagingQueueRef.current
        .then(async () => {
          const { staged, errors } = await stageFiles(files, {
            limits,
            existing: attachmentsRef.current,
            t,
          });
          if (staged.length > 0) {
            setAttachments((prev) => {
              const next = [...prev, ...staged];
              attachmentsRef.current = next;
              return next;
            });
          }
          setAttachmentError(errors.length > 0 ? errors.join(" ") : "");
        })
        .catch(() => {
          setAttachmentError(t("chat.attachmentStagingFailed"));
        });
    },
    [disabled, limits, t]
  );

  const removeAttachment = React.useCallback((id) => {
    setAttachments((prev) => {
      const next = prev.filter((att) => att.id !== id);
      // Keep the ref in lockstep so a same-tick add validates against the
      // post-removal set, not a stale snapshot (the effect sync is async).
      attachmentsRef.current = next;
      return next;
    });
    setAttachmentError("");
  }, []);

  const openFilePicker = React.useCallback(() => {
    if (disabled) return;
    fileInputRef.current?.click();
  }, [disabled]);

  const onFileInputChange = React.useCallback(
    (e) => {
      const files = Array.from(e.target.files || []);
      addFiles(files);
      // Reset so picking the same file again re-fires `change`.
      e.target.value = "";
    },
    [addFiles]
  );

  const onPaste = React.useCallback(
    (e) => {
      const files = Array.from(e.clipboardData?.files || []);
      if (files.length > 0) {
        e.preventDefault();
        addFiles(files);
      }
    },
    [addFiles]
  );

  const onDrop = React.useCallback(
    (e) => {
      e.preventDefault();
      setDragOver(false);
      const files = Array.from(e.dataTransfer?.files || []);
      if (files.length > 0) addFiles(files);
    },
    [addFiles]
  );

  const onDragOver = React.useCallback(
    (e) => {
      e.preventDefault();
      // `addFiles` no-ops while disabled, so don't tease the drop overlay then.
      if (disabled) return;
      setDragOver(true);
    },
    [disabled]
  );
  const onDragLeave = React.useCallback((e) => {
    if (e.currentTarget.contains(e.relatedTarget)) return;
    setDragOver(false);
  }, []);

  // Publish the send payload + reset for the shared shell. Reset clears the
  // staged files and the persisted draft for this key.
  const reset = React.useCallback(() => {
    setAttachments([]);
    attachmentsRef.current = [];
    setAttachmentError("");
    cancelPendingDraft();
    clearDraft(draftKey);
    clearStagedAttachments(draftKey);
  }, [draftKey, cancelPendingDraft]);

  React.useEffect(() => {
    onComposerChange({
      getPayload: () => ({ images: [], attachments }),
      reset,
      hasPayload: attachments.length > 0,
      sendBlocked: false,
    });
  }, [attachments, reset, onComposerChange]);

  const handleChange = React.useCallback(
    (e) => {
      const next = e.target.value;
      onTextChange(next);
      // Queue a debounced persist instead of writing on every keystroke.
      // Capture the scope so a flush after an identity change is dropped.
      pendingDraftRef.current = { key: draftKey, text: next, scope: authScope() };
      if (draftTimerRef.current) window.clearTimeout(draftTimerRef.current);
      draftTimerRef.current = window.setTimeout(flushDraft, 300);
    },
    [draftKey, flushDraft]
  );

  const acceptAttr =
    limits.accept.length > 0 ? limits.accept.join(",") : undefined;

  return html`
    <div
      onDrop=${onDrop}
      onDragOver=${onDragOver}
      onDragLeave=${onDragLeave}
    >
      ${dragOver &&
      html`
        <div className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]">
          ${t("chat.attachmentDropHint")}
        </div>
      `}
      ${attachmentError &&
      html`
        <div
          role="alert"
          className="mb-3 flex items-start gap-2 rounded-md border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-danger-text)]"
        >
          <span className="min-w-0 flex-1">${attachmentError}</span>
          <button
            type="button"
            onClick=${() => setAttachmentError("")}
            aria-label=${t("common.dismiss")}
            title=${t("common.dismiss")}
            className="-mr-1 -mt-0.5 shrink-0 rounded p-0.5 text-[color-mix(in_srgb,var(--v2-danger-text)_80%,transparent)] transition hover:bg-[color-mix(in_srgb,var(--v2-danger-text)_14%,transparent)] hover:text-[var(--v2-danger-text)]"
          >
            <${Icon} name="close" className="h-3.5 w-3.5" strokeWidth=${2} />
          </button>
        </div>
      `}

      ${attachments.length > 0 &&
      html`
        <div className="mb-2 flex flex-wrap gap-2 px-1">
          ${attachments.map(
            (att) => html`
              <div
                key=${att.id}
                className="group/att relative flex items-center gap-2 rounded-lg border border-iron-700 bg-iron-900/60 py-1.5 pl-1.5 pr-7 text-xs text-iron-100"
              >
                ${att.previewUrl
                  ? html`<img
                      src=${att.previewUrl}
                      alt=${att.filename}
                      className="h-9 w-9 shrink-0 rounded object-cover"
                    />`
                  : html`<span
                      className="grid h-9 w-9 shrink-0 place-items-center rounded bg-iron-800 text-signal"
                    >
                      <${Icon} name="file" className="h-4 w-4" />
                    </span>`}
                <span className="flex min-w-0 flex-col">
                  <span className="max-w-[12rem] truncate font-medium">
                    ${att.filename}
                  </span>
                  <span className="text-[10px] text-iron-400">${att.sizeLabel}</span>
                </span>
                <button
                  type="button"
                  onClick=${() => removeAttachment(att.id)}
                  aria-label=${t("chat.attachmentRemove")}
                  title=${t("chat.attachmentRemove")}
                  className="absolute right-1 top-1 grid h-5 w-5 place-items-center rounded-full text-iron-400 hover:bg-iron-700 hover:text-white"
                >
                  <${Icon} name="close" className="h-3 w-3" />
                </button>
              </div>
            `
          )}
        </div>
      `}

      <textarea
        ref=${textareaRef}
        data-testid="chat-composer"
        value=${text}
        onChange=${handleChange}
        onKeyDown=${onKeyDown}
        onPaste=${onPaste}
        placeholder=${placeholder}
        rows=${1}
        disabled=${disabled}
        className=${textClass}
      />

      <input
        ref=${fileInputRef}
        type="file"
        multiple
        accept=${acceptAttr}
        className="hidden"
        onChange=${onFileInputChange}
      />

      <div className="mt-2 flex items-center justify-end">
        <button
          type="button"
          onClick=${openFilePicker}
          disabled=${disabled}
          aria-label=${t("chat.attachFiles")}
          title=${t("chat.attachFiles")}
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] disabled:cursor-not-allowed disabled:opacity-50"
        >
          <${Icon} name="plus" className="h-5 w-5" />
        </button>
      </div>
    </div>
  `;
}

/* ----------------------------------------------------------------------------
 * Desktop composer — client-side attachment extraction + the NEAR AI Cloud
 * model front-door. This is the heavy automation path; it is only reached when
 * `isDesktopRuntime()` is true (see selectDesktopRuntime in the shell).
 * ------------------------------------------------------------------------- */
function DesktopComposer({
  disabled,
  text,
  initialText,
  resetKey,
  context,
  textareaRef,
  onComposerChange,
}) {
  const t = useT();
  const [attachmentPreview, setAttachmentPreview] = React.useState(null);
  const fileInputRef = React.useRef(null);
  const {
    images,
    attachments,
    rejections,
    addFiles,
    removeImage,
    removeAttachment,
    dismissRejections,
    clearAttachments,
  } = useComposerAttachments();
  const extracting = attachments.some((att) => att.extraction === "extracting");
  const baseReadiness = context.modelReadiness || {
    verified: false,
    sendBlocked: false,
    label: "Verification pending",
    description: "",
    sendBlockReason: "",
  };
  // Prefer the live providers snapshot (single source of truth, refreshed on
  // every apply) over the boot-time gateway fallback for the chip label.
  const providersQuery = useQuery({
    queryKey: ["llm-providers"],
    queryFn: fetchLlmProviders,
    staleTime: 60_000,
  });
  const providersSnapshot = providersQuery.data;
  const visibleProvidersSnapshot = visibleLlmSnapshot(providersSnapshot || {});
  const activeSelection = visibleProvidersSnapshot.active;
  const cloudProvider = visibleProvidersSnapshot.providers[0] || null;
  const providerSnapshotPending = !providersSnapshot && providersQuery.isLoading;
  const providerSnapshotFailed = !providersSnapshot && providersQuery.error;
  const providerSetupRequired = Boolean(providersSnapshot && !activeSelection);
  const providerSetupUnknown = Boolean(providerSnapshotPending || providerSnapshotFailed);
  const cloudBlockReason = providerSetupFailedMessage(providerSnapshotFailed);
  const readiness =
    providerSetupRequired || providerSetupUnknown
      ? {
          ...baseReadiness,
          verified: false,
          sendBlocked: true,
          tone: providerSnapshotPending ? "info" : "warning",
          label: providerSetupUnknown ? "Checking NEAR AI Cloud" : "NEAR AI Cloud setup required",
          description: providerSnapshotPending
            ? "Checking your NEAR AI Cloud session."
            : cloudBlockReason,
          sendBlockReason: providerSnapshotPending
            ? "Checking NEAR AI Cloud. This should only take a moment."
            : cloudBlockReason,
        }
      : baseReadiness;
  const providerNameFromSnapshot =
    visibleProvidersSnapshot.providers.find(
      (provider) => provider.id === activeSelection?.provider_id
    )?.name || "";
  const providerLabel = formatProviderLabel(
    activeSelection?.provider_id || cloudProvider?.id,
    providerNameFromSnapshot || activeSelection?.name || cloudProvider?.name,
    "nearai"
  );
  const fallbackModel = String(
    cloudProvider?.active_model || cloudProvider?.default_model || context.model || "auto"
  );
  const rawModelLabel = String(
    activeSelection?.model ||
      (providerSetupRequired ? "Not connected" : providerSetupUnknown ? "Checking" : fallbackModel)
  );
  const modelLabel =
    providerSetupRequired || providerSetupUnknown ? rawModelLabel : modelDisplayName(rawModelLabel);
  // Calm chip: "Provider · model" with a status dot; the readiness phrase lives
  // in the tooltip and (when blocking) the banner — not shouted inline.
  const modelControlLabel = `${providerLabel} · ${modelLabel}`;
  const readinessDotClass =
    readiness.tone === "positive"
      ? "bg-[var(--v2-positive-text)]"
      : readiness.sendBlocked
        ? "bg-[var(--v2-danger-text)]"
        : "bg-[var(--v2-warning-text)]";

  React.useEffect(() => {
    if (!initialText) return;
    window.requestAnimationFrame(() => {
      if (textareaRef.current) {
        textareaRef.current.focus();
        textareaRef.current.setSelectionRange(initialText.length, initialText.length);
      }
    });
  }, [initialText, resetKey]);

  const onPaste = React.useCallback(
    (e) => {
      const files = Array.from(e.clipboardData.files);
      if (files.length > 0) {
        e.preventDefault();
        addFiles(files);
      }
    },
    [addFiles]
  );

  const [dragOver, setDragOver] = React.useState(false);
  const onDrop = React.useCallback(
    (e) => {
      e.preventDefault();
      setDragOver(false);
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) addFiles(files);
    },
    [addFiles]
  );
  const onDragOver = React.useCallback((e) => {
    e.preventDefault();
    setDragOver(true);
  }, []);
  const onDragLeave = React.useCallback((e) => {
    if (e.currentTarget.contains(e.relatedTarget)) return;
    setDragOver(false);
  }, []);

  const onFileInputChange = React.useCallback(
    (e) => {
      const files = Array.from(e.target.files || []);
      if (files.length > 0) addFiles(files);
      e.target.value = "";
    },
    [addFiles]
  );

  const [modelMenuOpen, setModelMenuOpen] = React.useState(false);
  const [addMenuOpen, setAddMenuOpen] = React.useState(false);
  const openFilePicker = React.useCallback(() => {
    setAddMenuOpen(false);
    fileInputRef.current?.click();
  }, []);

  // Publish payload (images + extracted attachments) + reset + send gating up
  // to the shared shell. extracting/sendBlocked must block send until the
  // client-side extraction settles and the provider is connected.
  React.useEffect(() => {
    onComposerChange({
      getPayload: () => ({ images, attachments }),
      reset: clearAttachments,
      hasPayload: images.length > 0 || attachments.length > 0,
      sendBlocked: Boolean(readiness.sendBlocked || extracting),
    });
  }, [images, attachments, clearAttachments, readiness.sendBlocked, extracting, onComposerChange]);

  return html`
    <div
      onDrop=${onDrop}
      onDragOver=${onDragOver}
      onDragLeave=${onDragLeave}
    >
      ${dragOver &&
      html`
        <div
          className="pointer-events-none absolute inset-1 z-10 flex items-center justify-center rounded-[16px] border border-dashed border-[color-mix(in_srgb,var(--v2-accent)_55%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-canvas)_82%,transparent)] text-sm font-medium text-[var(--v2-accent-text)]"
        >
          ${t("chat.dropToAttach")}
        </div>
      `}
      ${(images.length > 0 || attachments.length > 0) &&
      html`
        <div className="mb-3 flex flex-wrap gap-2">
          ${images.map(
            (img, i) => html`
              <div key=${i} className="group relative">
                <img
                  src=${img.dataUrl}
                  className="h-16 w-16 rounded-lg border border-[var(--v2-panel-border)] object-cover"
                  alt=""
                />
                <button
                  onClick=${() => removeImage(i)}
                  className="absolute -right-1 -top-1 flex h-5 w-5 items-center justify-center rounded-full border border-[color-mix(in_srgb,var(--v2-danger-text)_36%,transparent)] bg-[var(--v2-danger-text)] text-white opacity-0 group-hover:opacity-100"
                  aria-label=${t("chat.removeImage")}
                >
                  <${Icon} name="close" className="h-3 w-3" />
                </button>
              </div>
            `
          )}
          ${attachments.map((att, i) => {
            const statusLabel = attachmentStatusLabel(att, t);
            const warning =
              att.extraction === "no-text" ||
              (att.extraction === "raw" && att.modelReadable === false);
            return html`
              <div
                key=${i}
                className=${`flex max-w-full items-center gap-2 rounded-[10px] border px-3 py-2 text-xs text-[var(--v2-text)] ${
                  warning
                    ? "border-[color-mix(in_srgb,var(--v2-warning-text)_42%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)]"
                    : "border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)]"
                }`}
              >
                <button
                  type="button"
                  onClick=${() => setAttachmentPreview(att)}
                  aria-label=${`Preview ${att.filename}${statusLabel ? `: ${statusLabel}` : ""}`}
                  className="flex min-w-0 flex-1 items-center gap-2 text-left hover:text-[var(--v2-text-strong)]"
                >
                  <${Icon}
                    name="file"
                    className="h-3.5 w-3.5 shrink-0 text-[var(--v2-accent-text)]"
                  />
                  <span className="min-w-0">
                    <span className="block truncate font-medium">${att.filename}</span>
                    <span
                      className=${`block truncate text-[11px] ${
                        warning
                          ? "text-[var(--v2-warning-text)]"
                          : att.extraction === "extracted" ||
                              (att.extraction === "raw" && att.modelReadable !== false)
                            ? "text-[var(--v2-positive-text)]"
                            : "text-[var(--v2-text-muted)]"
                      }`}
                    >
                      ${statusLabel || "Ready to send"}
                    </span>
                  </span>
                </button>
                <span className="shrink-0 text-[var(--v2-text-muted)]"
                  >${formatSize(att.size)}</span
                >
                <button
                  type="button"
                  onClick=${() => setAttachmentPreview(att)}
                  className="shrink-0 font-medium text-[var(--v2-accent-text)] hover:underline"
                >
                  Preview
                </button>
                <button
                  onClick=${() => removeAttachment(i)}
                  className="ml-1 text-[var(--v2-text-muted)] hover:text-[var(--v2-text-strong)]"
                  aria-label=${t("chat.removeAttachment")}
                >
                  <${Icon} name="close" className="h-3.5 w-3.5" />
                </button>
              </div>
            `;
          })}
        </div>
      `}
      ${rejections.length > 0 &&
      html`
        <div
          className="mb-3 flex items-start justify-between gap-3 rounded-[14px] border border-[color-mix(in_srgb,var(--v2-warning-text)_35%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-2 text-xs leading-5 text-[var(--v2-warning-text)]"
          role="status"
        >
          <div>${rejections.map((notice) => html`<div key=${notice}>${notice}</div>`)}</div>
          <button
            type="button"
            onClick=${dismissRejections}
            className="shrink-0 font-semibold hover:underline"
          >
            ${t("common.dismiss")}
          </button>
        </div>
      `}
      ${readiness.sendBlocked &&
      html`
        <div
          className="mb-3 rounded-[14px] border border-[color-mix(in_srgb,var(--v2-warning-text)_35%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-2 text-sm font-semibold leading-5 text-[var(--v2-warning-text)]"
          role="status"
        >
          ${readiness.sendBlockReason || readiness.description}
        </div>
      `}

      <input
        ref=${fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange=${onFileInputChange}
        onPaste=${onPaste}
      />

      <div className="mt-2 flex items-center gap-1.5">
        <${Popover}
          open=${modelMenuOpen}
          onClose=${() => setModelMenuOpen(false)}
          align="end"
          side="top"
          ariaLabel="Chat model settings"
          trigger=${html`
            <button
              type="button"
              aria-label="Chat model settings"
              aria-expanded=${modelMenuOpen}
              title=${`${readiness.label} — ${readiness.description || ""}`}
              onClick=${() => setModelMenuOpen((value) => !value)}
              className="inline-flex h-11 min-w-0 max-w-[16rem] items-center gap-2 rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-xs font-semibold text-[var(--v2-text)] hover:border-[color-mix(in_srgb,var(--v2-accent)_45%,var(--v2-panel-border))] hover:text-[var(--v2-accent-text)]"
            >
              <span className=${`h-1.5 w-1.5 shrink-0 rounded-full ${readinessDotClass}`} />
              <span className="truncate">${modelControlLabel}</span>
              <${Icon} name="chevron" className="h-3 w-3 shrink-0 opacity-60" />
            </button>
          `}
        >
          <${ModelPopover}
            open=${modelMenuOpen}
            onClose=${() => setModelMenuOpen(false)}
            t=${t}
          />
        <//>
        <${Popover}
          open=${addMenuOpen}
          onClose=${() => setAddMenuOpen(false)}
          align="end"
          side="top"
          ariaLabel=${t("chat.addToMessage")}
          trigger=${html`
            <button
              type="button"
              onClick=${() => setAddMenuOpen((value) => !value)}
              className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-accent-text)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]/50 focus-visible:ring-offset-1 focus-visible:ring-offset-[var(--v2-canvas)]"
              title=${t("chat.addToMessage")}
              aria-label=${t("chat.addToMessage")}
              aria-expanded=${addMenuOpen}
            >
              <${Icon} name="plus" className="h-5 w-5" />
            </button>
          `}
        >
          <div className="w-[min(18rem,calc(100vw-2rem))] p-2" data-testid="composer-add-menu">
            <div
              className="px-2 pb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--v2-text-faint)]"
            >
              ${t("chat.addMenuTitle")}
            </div>
            <button
              type="button"
              onClick=${openFilePicker}
              className="flex w-full items-start gap-3 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-3 text-left text-sm text-[var(--v2-text)] hover:border-[color-mix(in_srgb,var(--v2-accent)_40%,var(--v2-panel-border))] hover:text-[var(--v2-text-strong)]"
            >
              <span
                className="grid h-9 w-9 shrink-0 place-items-center rounded-[9px] border border-[var(--v2-panel-border)] text-[var(--v2-accent-text)]"
                aria-hidden="true"
              >
                <${Icon} name="upload" className="h-4 w-4" />
              </span>
              <span className="min-w-0">
                <span className="block font-semibold">${t("chat.attachFiles")}</span>
                <span className="mt-0.5 block text-xs leading-5 text-[var(--v2-text-muted)]">
                  ${t("chat.attachFilesDesc")}
                </span>
              </span>
            </button>
            <p className="px-2 pt-2 text-xs leading-5 text-[var(--v2-text-muted)]">
              ${t("chat.attachFilesHint")}
            </p>
          </div>
        <//>
      </div>
      <${AttachmentPreviewModal}
        open=${Boolean(attachmentPreview)}
        onClose=${() => setAttachmentPreview(null)}
        attachment=${attachmentPreview}
      />
    </div>
  `;
}

// One short status per chip: what actually happens to this file when sent.
function attachmentStatusLabel(att, t) {
  switch (att.extraction) {
    case "extracting":
      return att.progressLabel || t("chat.attachmentExtracting");
    case "extracted":
      return t("chat.attachmentExtracted", {
        chars: `${((att.extractedChars || 0) / 1000).toFixed(1)}k`,
      });
    case "no-text":
      return t("chat.attachmentNoText");
    case "raw":
      // Raw binary the backend cannot inline: the model never sees it. Text-ish
      // raw payloads embed fine and need no caveat.
      return att.modelReadable === false
        ? t("chat.attachmentMetadataOnly")
        : "Model can read this file";
    default:
      return "";
  }
}

function modelSettingsPath() {
  if (typeof window !== "undefined" && window.location?.pathname?.startsWith("/v2")) {
    return "/v2/settings/inference";
  }
  return "/settings/inference";
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

// Compact model switcher anchored to the composer chip. Models come from the
// backend's live list for the ACTIVE provider; applying goes through the same
// set-active route Settings uses, so the snapshot stays the single source of
// truth. One dominant action (Apply); managing providers stays in Settings.
function ModelPopover({ open, onClose, t }) {
  const queryClient = useQueryClient();
  const providersQuery = useQuery({
    queryKey: ["llm-providers"],
    queryFn: fetchLlmProviders,
    staleTime: 60_000,
    enabled: open,
  });
  const snapshot = providersQuery.data || {};
  const { providers, active } = visibleLlmSnapshot(snapshot);
  const providerSnapshotPending = !providersQuery.data && providersQuery.isLoading;
  const providerUnavailable =
    !providerSnapshotPending && (providersQuery.error || providers.length === 0 || !active);
  const activeProvider = providers.find((provider) => provider.id === active?.provider_id) || null;
  const [selectedProviderId, setSelectedProviderId] = React.useState("");
  const [models, setModels] = React.useState(null);
  const [loadError, setLoadError] = React.useState(false);
  const [applying, setApplying] = React.useState("");
  const [selectedModel, setSelectedModel] = React.useState("");
  const [manualModel, setManualModel] = React.useState("");
  const [manualOpen, setManualOpen] = React.useState(false);

  React.useEffect(() => {
    if (!open) return;
    setSelectedProviderId((current) =>
      providers.some((provider) => provider.id === current)
        ? current
        : active?.provider_id || providers[0]?.id || ""
    );
  }, [open, active?.provider_id, providers.length]);

  const selectedProvider =
    providers.find((provider) => provider.id === selectedProviderId) ||
    activeProvider ||
    providers[0] ||
    null;
  const currentProviderModel = modelForProvider(selectedProvider, active);

  React.useEffect(() => {
    if (!open) return;
    setModels(null);
    setLoadError(false);
    setSelectedModel(currentProviderModel);
    setManualModel("");
    setManualOpen(false);
  }, [open, selectedProvider?.id, currentProviderModel]);

  React.useEffect(() => {
    if (!open || !selectedProvider || !active || models !== null) return;
    let cancelled = false;
    (async () => {
      try {
        const result = await listLlmProviderModels({
          provider_id: selectedProvider.id,
          adapter: selectedProvider.adapter || selectedProvider.id,
        });
        if (!cancelled) {
          const normalized = result?.ok ? normalizeModelEntries(result.models) : [];
          setModels(normalized);
          setLoadError(!result?.ok);
          setSelectedModel((current) =>
            normalized.includes(current) ? current : normalized[0] || currentProviderModel
          );
        }
      } catch (_) {
        if (!cancelled) {
          setModels([]);
          setLoadError(true);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open, selectedProvider, active, models, currentProviderModel]);

  const apply = async () => {
    if (!selectedProvider || applying) return;
    const model = (
      manualOpen ? manualModel : selectedModel || currentProviderModel || "auto"
    ).trim();
    if (!model) return;
    setApplying(`${selectedProvider.id}:${model}`);
    try {
      await setActiveLlm({ provider_id: selectedProvider.id, model });
      await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
      await queryClient.invalidateQueries({ queryKey: ["gateway-status"] });
      onClose();
    } catch (_) {
      setLoadError(true);
    } finally {
      setApplying("");
    }
  };

  const applyModel = (
    manualOpen ? manualModel : selectedModel || currentProviderModel || "auto"
  ).trim();
  const isCurrentSelection =
    selectedProvider?.id === active?.provider_id && applyModel === String(active?.model || "auto");
  const canApply = Boolean(selectedProvider && applyModel && !applying && !isCurrentSelection);
  const visibleModels = uniqueModelsByDisplayLabel(models || []);
  const activeModel = currentProviderModel || "auto";
  const activeModelLabel = modelDisplayName(activeModel).toLowerCase();
  const availableModels = visibleModels.filter(
    (model) => model !== activeModel && modelDisplayName(model).toLowerCase() !== activeModelLabel
  );

  return html`
    <div className="w-[min(28rem,calc(100vw-2rem))] p-2">
      <div className="px-2 pb-2 pt-1">
        <div
          className="text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--v2-text-faint)]"
        >
          ${t("chat.modelPopoverProvider")}
        </div>
        <div className="mt-0.5 truncate text-sm font-medium text-[var(--v2-text-strong)]">
          ${selectedProvider
            ? formatProviderLabel(selectedProvider.id, selectedProvider.name)
            : t("chat.modelPopoverNoProvider")}
        </div>
      </div>
      <div className="mb-2 grid max-h-32 gap-1 overflow-y-auto">
        ${providers.length === 0 &&
        html`<div className="px-2 py-2 text-sm text-[var(--v2-text-muted)]">
          ${providersQuery.isLoading ? t("common.loading") : t("chat.modelPopoverNoProvider")}
        </div>`}
        ${providers.map((provider) => {
          const isSelected = provider.id === selectedProvider?.id;
          const isActive = provider.id === active?.provider_id;
          return html`
            <button
              key=${provider.id}
              type="button"
              onClick=${() => setSelectedProviderId(provider.id)}
              className=${`flex min-w-0 items-center justify-between gap-2 rounded-[8px] px-2 py-1.5 text-left text-sm ${
                isSelected
                  ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                  : "text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"
              }`}
            >
              <span className="min-w-0">
                <span className="block truncate font-medium"
                  >${formatProviderLabel(provider.id, provider.name)}</span
                >
              </span>
              ${isActive &&
              html`<span className="shrink-0 text-[11px] font-semibold">${t("llm.active")}</span>`}
            </button>
          `;
        })}
      </div>
      ${providerSnapshotPending
        ? html`
            <div
              className="mx-2 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 py-3"
            >
              <div className="text-sm font-semibold text-[var(--v2-text-strong)]">
                Checking NEAR AI Cloud...
              </div>
              <p className="mt-1 text-sm leading-5 text-[var(--v2-text-muted)]">
                IronClaw is checking your local gateway and model access.
              </p>
            </div>
          `
        : providerUnavailable
          ? html`
              <div
                className="mx-2 rounded-[10px] border border-[color-mix(in_srgb,var(--v2-warning-text)_35%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-3"
              >
                <div className="text-sm font-semibold text-[var(--v2-warning-text)]">
                  ${t("chat.modelPopoverNoProvider")}
                </div>
                <p className="mt-1 text-sm leading-5 text-[var(--v2-text-muted)]">
                  ${t("chat.modelPopoverNeedsSetupDesc")}
                </p>
              </div>
            `
          : html`
              <div className="border-t border-[var(--v2-panel-border)] px-2 pb-2 pt-2">
                <div
                  className="mb-1 text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--v2-text-faint)]"
                >
                  ${t("chat.modelPopoverTitle")}
                </div>
              </div>
              <div className="max-h-64 overflow-y-auto">
                ${models === null &&
                html`<div className="px-2 py-3 text-sm text-[var(--v2-text-muted)]">
                  ${t("common.loading")}
                </div>`}
                ${models !== null &&
                models.length === 0 &&
                html`<div className="px-2 py-3 text-sm text-[var(--v2-text-muted)]">
                  ${loadError ? t("chat.modelPopoverError") : t("chat.modelPopoverEmpty")}
                </div>`}
                ${models !== null &&
                models.length > 0 &&
                html`
                  <div className="px-2 pb-1">
                    <div
                      className="mb-1 font-mono text-[10.5px] uppercase tracking-[0.12em] text-[var(--v2-text-faint)]"
                    >
                      ${t("chat.modelPopoverActive")}
                    </div>
                    <button
                      type="button"
                      disabled=${Boolean(applying)}
                      onClick=${() => {
                        setSelectedModel(activeModel);
                        setManualModel("");
                        setManualOpen(false);
                      }}
                      className=${`flex w-full items-center justify-between gap-2 rounded-[8px] px-2 py-1.5 text-left text-sm ${
                        selectedModel === activeModel && !manualOpen
                          ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                          : "text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"
                      }`}
                    >
                      <span className="truncate">${modelDisplayName(activeModel)}</span>
                      ${selectedModel === activeModel && !manualOpen
                        ? html`<${Icon} name="check" className="h-3.5 w-3.5 shrink-0" />`
                        : null}
                    </button>
                  </div>
                  ${availableModels.length > 0 &&
                  html`
                    <div className="px-2 pt-2">
                      <div
                        className="mb-1 font-mono text-[10.5px] uppercase tracking-[0.12em] text-[var(--v2-text-faint)]"
                      >
                        ${t("chat.modelPopoverAvailable")}
                      </div>
                      <div className="grid gap-1">
                        ${availableModels.map((model) => {
                          const isCurrent = model === selectedModel && !manualOpen;
                          return html`
                            <button
                              key=${model}
                              type="button"
                              disabled=${Boolean(applying)}
                              onClick=${() => {
                                setSelectedModel(model);
                                setManualModel("");
                                setManualOpen(false);
                              }}
                              className=${`flex w-full items-center justify-between gap-2 rounded-[6px] px-2 py-1.5 text-left text-sm ${
                                isCurrent
                                  ? "bg-[var(--v2-accent-soft)] text-[var(--v2-accent-text)]"
                                  : "text-[var(--v2-text)] hover:bg-[var(--v2-surface-soft)]"
                              }`}
                            >
                              <span className="truncate">${modelDisplayName(model)}</span>
                              ${isCurrent
                                ? html`<${Icon} name="check" className="h-3.5 w-3.5 shrink-0" />`
                                : null}
                            </button>
                          `;
                        })}
                      </div>
                    </div>
                  `}
                `}
              </div>
              <div className="mt-2 border-t border-[var(--v2-panel-border)] px-2 pt-2">
                <button
                  type="button"
                  aria-expanded=${manualOpen ? "true" : "false"}
                  onClick=${() => {
                    setManualOpen((value) => !value);
                    setManualModel("");
                  }}
                  className="flex w-full items-center justify-between gap-2 rounded-[8px] px-2 py-1.5 text-left text-xs font-medium text-[var(--v2-text-muted)] hover:bg-[var(--v2-surface-soft)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)]"
                >
                  <span>${t("chat.modelPopoverManualToggle")}</span>
                  <${Icon}
                    name="chevron"
                    className=${`h-3.5 w-3.5 transition-transform ${manualOpen ? "rotate-180" : ""}`}
                  />
                </button>
                ${manualOpen &&
                html`
                  <div
                    className="mt-2 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-2"
                  >
                    <label
                      className="mb-1 block text-[11px] font-semibold uppercase tracking-[0.08em] text-[var(--v2-text-faint)]"
                    >
                      ${t("chat.modelPopoverManualLabel")}
                    </label>
                    <input
                      type="text"
                      value=${manualModel}
                      onChange=${(event) => {
                        setManualModel(event.target.value);
                      }}
                      placeholder=${t("chat.modelPopoverManualPlaceholder")}
                      className="h-9 w-full rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-input-bg)] px-3 font-mono text-xs text-[var(--v2-text-strong)] outline-none placeholder:text-[var(--v2-text-faint)] focus:border-[var(--v2-accent)] focus:ring-2 focus:ring-[color-mix(in_srgb,var(--v2-accent)_28%,transparent)]"
                    />
                    <p className="mt-1.5 text-xs leading-4 text-[var(--v2-text-muted)]">
                      ${t("chat.modelPopoverManualDesc")}
                    </p>
                  </div>
                `}
              </div>
            `}
      <div className="mt-2 border-t border-[var(--v2-panel-border)] px-2 pb-1 pt-2">
        <div className=${active ? "flex items-center justify-between gap-3" : "grid"}>
          ${active
            ? html`
                <a
                  href=${modelSettingsPath()}
                  className="text-xs font-medium text-[var(--v2-accent-text)] hover:underline"
                >
                  ${t("chat.modelPopoverManage")}
                </a>
                <${Button}
                  type="button"
                  variant="primary"
                  size="sm"
                  disabled=${!canApply}
                  onClick=${apply}
                >
                  ${applying ? t("llm.applying") : t("llm.applyModel")}
                <//>
              `
            : html`
                <${Button}
                  as="a"
                  href=${modelSettingsPath()}
                  variant="primary"
                  size="sm"
                  fullWidth=${true}
                >
                  ${t("chat.modelPopoverManage")}
                <//>
              `}
        </div>
      </div>
    </div>
  `;
}

function providerSetupFailedMessage(failed) {
  if (failed) {
    return "IronClaw cannot reach NEAR AI Cloud yet. Open setup or retry when the gateway is ready.";
  }
  return "Connect NEAR AI Cloud before sending your first message.";
}

function formatProviderLabel(providerId, providerName, fallbackId = "nearai") {
  const raw = String(providerId || fallbackId || "nearai").trim();
  const normalized = raw.toLowerCase().replace(/[\s]+/g, "").replace(/[_-]+/g, "_");

  if (normalized === "nearai") return "NEAR AI Cloud";
  if (providerName && providerName.trim()) return providerName.trim();
  if (!providerName) return "External provider";

  const humanized = raw
    .trim()
    .toLowerCase()
    .replace(/[-_]+/g, " ")
    .split(" ")
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");

  return humanized || "NEAR.AI";
}

function normalizeModelEntries(models) {
  if (!Array.isArray(models)) return [];
  return models
    .map((model) =>
      typeof model === "string" ? model : model?.id || model?.model || model?.name || ""
    )
    .map((model) => String(model).trim())
    .filter(Boolean);
}

function uniqueModelsByDisplayLabel(models) {
  const seen = new Set();
  return normalizeModelEntries(models).filter((model) => {
    const label = modelDisplayName(model).toLowerCase();
    if (seen.has(label)) return false;
    seen.add(label);
    return true;
  });
}

function modelForProvider(provider, active) {
  if (!provider) return "";
  if (provider.id === active?.provider_id) return String(active?.model || "auto");
  return String(provider.active_model || provider.default_model || provider.model || "").trim();
}
