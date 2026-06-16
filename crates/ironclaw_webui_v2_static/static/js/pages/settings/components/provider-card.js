import { Button } from "../../../design-system/button.js";
import { Badge } from "../../../design-system/badge.js";
import { Card } from "../../../design-system/card.js";
import { Icon } from "../../../design-system/icons.js";
import { Select } from "../../../design-system/input.js";
import { React, html } from "../../../lib/html.js";
import { isDesktopRuntime } from "../../../lib/api.js";
import { useT } from "../../../lib/i18n.js";
import {
  adapterLabel,
  isProviderConfigured,
  modelDisplayName,
  providerAcceptsApiKey,
  providerDisplayModel,
  providerEffectiveBaseUrl,
  providerMissingReason,
} from "../lib/llm-providers.js";

// Desktop-only (additive): active model switcher backed by the gateway's live
// model list, applied through the same set-active route the chat popover uses.
// Only the desktop ProviderManagement renders it (behind `isDesktopRuntime()`);
// it is exported here so it sits beside the card it complements.
export function ActiveModelPicker({ provider, currentModel, onListModels, onApplyModel, t }) {
  const [models, setModels] = React.useState(null);
  const [choice, setChoice] = React.useState(currentModel || "");
  const [busy, setBusy] = React.useState(false);
  const [failed, setFailed] = React.useState(false);

  React.useEffect(() => {
    if (models !== null || typeof onListModels !== "function") return undefined;
    let cancelled = false;
    (async () => {
      try {
        const result = await onListModels({
          provider_id: provider.id,
          adapter: provider.adapter || provider.id,
        });
        if (cancelled) return;
        setModels(result?.ok && Array.isArray(result.models) ? result.models : []);
        setFailed(!result?.ok);
      } catch (_) {
        if (!cancelled) {
          setModels([]);
          setFailed(true);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [models, onListModels, provider]);

  if (models === null) {
    return html`<div className="text-xs text-[var(--v2-text-faint)]">${t("common.loading")}</div>`;
  }
  if (models.length === 0) {
    return failed
      ? html`<div className="text-xs text-[var(--v2-warning-text)]">
          ${t("chat.modelPopoverError")}
        </div>`
      : null;
  }

  const apply = async () => {
    if (!choice || busy || typeof onApplyModel !== "function") return;
    setBusy(true);
    try {
      await onApplyModel(provider, choice);
    } finally {
      setBusy(false);
    }
  };

  return html`
    <div className="flex flex-wrap items-center gap-2">
      <${Select}
        aria-label=${t("llm.model")}
        value=${models.includes(choice) ? choice : ""}
        onChange=${(event) => setChoice(event.target.value)}
        wrapperClassName="min-w-[min(18rem,100%)] flex-1 sm:min-w-[18rem]"
        className="h-11 text-xs md:h-9"
      >
        <option value="" disabled>${t("llm.pickModel")}</option>
        ${models.map(
          (entry) => html`<option key=${entry} value=${entry}>${modelDisplayName(entry)}</option>`
        )}
      <//>
      <${Button}
        type="button"
        variant="primary"
        disabled=${busy || !choice || choice === currentModel}
        onClick=${apply}
      >
        ${busy ? t("llm.applying") : t("llm.applyModel")}
      <//>
    </div>
  `;
}

export function ProviderCard({
  provider,
  activeProviderId,
  selectedModel,
  builtinOverrides,
  isBusy,
  onUse,
  onConfigure,
  onDelete,
  onNearaiLogin,
  onNearaiWallet,
  onCodexLogin,
  loginBusy,
}) {
  const t = useT();
  const desktop = isDesktopRuntime();
  const isActive = provider.id === activeProviderId;
  // Desktop relabels the NEAR provider to "NEAR AI Cloud"; web shows the raw
  // provider name/id (behavior equals today's mono).
  const displayName =
    desktop && provider.id === "nearai" ? "NEAR AI Cloud" : provider.name || provider.id;
  const configured = isProviderConfigured(provider, builtinOverrides);
  const baseUrl = providerEffectiveBaseUrl(provider, builtinOverrides);
  const model = providerDisplayModel(provider, builtinOverrides, activeProviderId, selectedModel);
  // Desktop derives a readable model label; web shows the raw id.
  const rawModel = model || provider.default_model || "";
  const modelLabel = desktop ? modelDisplayName(rawModel) : rawModel;
  const missing = providerMissingReason(provider, builtinOverrides);
  const acceptsApiKey = providerAcceptsApiKey(provider);
  // `gateway` is only produced for the desktop synthetic-offline fallback.
  const missingLabel =
    missing === "gateway"
      ? t("llm.gatewayUnavailable")
      : missing === "api_key"
      ? t("llm.missingApiKey")
      : missing === "base_url"
      ? t("llm.missingBaseUrl")
      : t("llm.notConfigured");

  const [expanded, setExpanded] = React.useState(isActive);
  const toggle = React.useCallback(() => setExpanded((v) => !v), []);

  React.useEffect(() => {
    setExpanded(isActive);
  }, [isActive]);

  const inlineMeta = !configured
    ? html`<span className="font-mono text-[11px] text-[var(--v2-warning-text)]">
        ${missingLabel}
      </span>`
    : html`<span className="hidden truncate font-mono text-[11px] text-[var(--v2-text-faint)] sm:inline">
        ${adapterLabel(provider.adapter)} · ${modelLabel || t("llm.none")}
      </span>`;

  // On web the Codex provider keeps its dedicated device-code sign-in; the
  // desktop app surfaces only NEAR AI Cloud, so its filtered list never reaches
  // an `openai_codex` card.
  const isLoginProvider = provider.id === "nearai" || provider.id === "openai_codex";
  const hasApiKey = provider.api_key_set === true || provider.has_api_key === true;
  // Desktop labels the NEAR key action "useNearApiKey"; web uses "addApiKey"
  // (behavior equals today's mono).
  const nearApiKeyLabel = desktop ? t("llm.useNearApiKey") : t("llm.addApiKey");
  const configureLabel = provider.builtin
    ? provider.id === "nearai" && acceptsApiKey && !hasApiKey
      ? nearApiKeyLabel
      : t("llm.configure")
    : t("common.edit");
  const apiKeyAction =
    acceptsApiKey && provider.builtin
      ? html`
          <${Button}
            type="button"
            variant="secondary"
            size="sm"
            disabled=${isBusy}
            onClick=${() => onConfigure(provider)}
          >
            ${configureLabel}
          <//>
        `
      : null;
  const loginActions =
    !isActive && provider.id === "nearai"
      ? html`
          ${apiKeyAction}
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${onNearaiWallet}>
            ${t("onboarding.nearWallet")}
          <//>
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${() => onNearaiLogin("github")}>
            GitHub
          <//>
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${() => onNearaiLogin("google")}>
            Google
          <//>
        `
      : !isActive && provider.id === "openai_codex"
      ? html`
          <${Button} type="button" variant="secondary" size="sm" disabled=${loginBusy} onClick=${onCodexLogin}>
            ${t("onboarding.codexSignIn")}
          <//>
        `
      : null;
  const canUseProvider =
    !isActive &&
    configured &&
    (!isLoginProvider || (provider.id === "nearai" && provider.has_api_key === true));
  const useAction = canUseProvider
    ? html`
        <${Button}
          type="button"
          variant="primary"
          size="sm"
          disabled=${isBusy}
          onClick=${() => onUse(provider)}
        >
          ${t("llm.use")}
        <//>
      `
    : null;
  const setupAction = !configured
    ? html`
        <${Button}
          type="button"
          variant="secondary"
          size="sm"
          disabled=${isBusy}
          onClick=${() => onConfigure(provider)}
        >
          ${missing === "gateway"
            ? t("common.retry")
            : missing === "api_key"
            ? (provider.id === "nearai" ? nearApiKeyLabel : t("llm.addApiKey"))
            : t("llm.configure")}
        <//>
      `
    : null;
  const primaryAction = isActive
    ? null
    : useAction || (isLoginProvider ? loginActions : setupAction);
  const showConfigureAction =
    (!isLoginProvider && ((provider.builtin && provider.id !== "bedrock") || !provider.builtin)) ||
    (provider.id === "nearai" && acceptsApiKey);

  return html`
    <${Card}
      padding="none"
      data-testid="llm-provider-card"
      data-provider-id=${provider.id}
      className=${[
        "transition-colors",
        isActive
          ? "border-[color-mix(in_srgb,var(--v2-positive-text)_36%,var(--v2-panel-border))]"
          : expanded
          ? "border-[color-mix(in_srgb,var(--v2-accent)_32%,var(--v2-panel-border))]"
          : "",
      ].join(" ")}
    >
      <div className="flex w-full flex-col hover:bg-[var(--v2-surface-soft)] sm:flex-row sm:items-stretch">
        <button
          type="button"
          aria-expanded=${expanded ? "true" : "false"}
          aria-label=${expanded ? t("llm.collapseDetails") : t("llm.expandDetails")}
          data-testid="llm-provider-disclosure"
          onClick=${toggle}
          className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 px-4 py-3 text-left focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] sm:pl-5 sm:pr-3"
        >
          <span
            className=${[
              "h-2 w-2 shrink-0 rounded-full",
              isActive
                ? "bg-[var(--v2-positive-text)]"
                : configured
                ? "bg-[var(--v2-accent)]"
                : "bg-[var(--v2-warning-text)]",
            ].join(" ")}
          />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
            <span className="min-w-0 truncate text-sm font-semibold text-[var(--v2-text-strong)]">
              ${displayName}
            </span>
            <span className="font-mono text-[11px] text-[var(--v2-text-faint)]">${provider.id}</span>
            ${isActive && html`<${Badge} tone="positive" label=${t("llm.active")} size="sm" />`}
            ${provider.builtin && !isActive &&
            html`<${Badge} tone="muted" label=${t("llm.builtin")} size="sm" />`}
          </span>
          <span className="hidden min-w-0 max-w-[280px] truncate sm:block">${inlineMeta}</span>
        </button>
        <div className="flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-[var(--v2-panel-border)] px-4 py-3 sm:border-t-0 sm:pl-0 sm:pr-5">
          ${primaryAction}
          <button
            type="button"
            onClick=${toggle}
            data-testid="llm-provider-chevron"
            aria-label=${expanded ? t("llm.collapseDetails") : t("llm.expandDetails")}
            className=${[
              "grid h-11 w-11 place-items-center rounded-md text-[var(--v2-text-faint)] transition-transform hover:bg-[var(--v2-surface-muted)] hover:text-[var(--v2-text-strong)] focus:outline-none focus-visible:ring-2 focus-visible:ring-[var(--v2-accent)] md:h-7 md:w-7",
              expanded ? "rotate-180" : "",
            ].join(" ")}
          >
            <${Icon} name="chevron" className="h-4 w-4" />
          </button>
        </div>
      </div>

      ${expanded &&
      html`
        <div data-testid="llm-provider-details" className="border-t border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-4 py-4 sm:px-5">
          <div className="grid gap-3 text-xs text-[var(--v2-text-muted)] sm:grid-cols-3">
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${t("llm.adapter")}</div>
              <div className="mt-1 truncate">${adapterLabel(provider.adapter)}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${t("llm.baseUrl")}</div>
              <div className="mt-1 truncate font-mono">${baseUrl || t("llm.none")}</div>
            </div>
            <div>
              <div className="font-mono uppercase text-[10px] text-[var(--v2-text-faint)]">${t("llm.model")}</div>
              <div className="mt-1 truncate font-mono">${modelLabel || t("llm.none")}</div>
            </div>
          </div>

          <div className="mt-4 flex flex-wrap justify-end gap-2 border-t border-[var(--v2-panel-border)] pt-3">
            ${showConfigureAction &&
            html`
              <${Button}
                type="button"
                variant="secondary"
                size="sm"
                disabled=${isBusy}
                onClick=${() => onConfigure(provider)}
              >
                ${configureLabel}
              <//>
            `}
            ${!provider.builtin &&
            !isActive &&
            html`
              <${Button}
                type="button"
                variant="danger"
                size="sm"
                disabled=${isBusy}
                onClick=${() => onDelete(provider)}
              >
                ${t("common.delete")}
              <//>
            `}
          </div>
        </div>
      `}
    <//>
  `;
}
