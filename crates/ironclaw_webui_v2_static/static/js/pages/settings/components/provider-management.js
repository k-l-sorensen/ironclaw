import { useQueryClient } from "@tanstack/react-query";
import { Button } from "../../../design-system/button.js";
import { Card } from "../../../design-system/card.js";
import { ConfirmDialog } from "../../../design-system/confirm-dialog.js";
import { Icon } from "../../../design-system/icons.js";
import { React, html } from "../../../lib/html.js";
import { isDesktopRuntime } from "../../../lib/api.js";
import { useT } from "../../../lib/i18n.js";
import { SettingsSearchEmpty } from "./settings-search-empty.js";
import { ActiveModelPicker, ProviderCard } from "./provider-card.js";
import { ProviderDialog } from "./provider-dialog.js";
import { ProviderLoginStatus } from "./provider-login-status.js";
import { useProviderManagementActions } from "../hooks/useProviderManagementActions.js";
import { useProviderLogin } from "../hooks/useProviderLogin.js";
import {
  filterDesktopVisibleLlmProviders,
  groupProvidersByStatus,
  modelDisplayName,
} from "../lib/llm-providers.js";
import { setActiveLlm } from "../lib/settings-api.js";

const GROUP_ORDER = [
  { key: "active", labelKey: "llm.groupActive", dotClass: "bg-[var(--v2-positive-text)]" },
  { key: "ready", labelKey: "llm.groupReady", dotClass: "bg-[var(--v2-accent)]" },
  { key: "setup", labelKey: "llm.groupSetup", dotClass: "bg-[var(--v2-warning-text)]" },
];

function GroupHeader({ label, count, dotClass }) {
  return html`
    <div className="mb-2 mt-1 flex items-center gap-2 px-1">
      <span className=${"h-1.5 w-1.5 rounded-full " + dotClass} />
      <span className="font-mono text-[10.5px] uppercase tracking-[0.14em] text-[var(--v2-text-faint)]">
        ${label}
      </span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">·</span>
      <span className="font-mono text-[10.5px] text-[var(--v2-text-faint)]">${count}</span>
      <span className="ml-2 h-px flex-1 bg-[var(--v2-panel-border)]" />
    </div>
  `;
}

// Desktop-only (additive): the active NEAR AI Cloud model selector, surfaced
// above the provider rows. Web never renders this (it is gated behind
// `isDesktopRuntime()` in ProviderManagement).
export function ActiveModelPanel({ provider, currentModel, onListModels, onApplyModel, t }) {
  if (!provider || provider.can_list_models === false) return null;
  const displayName = provider.id === "nearai" ? "NEAR AI Cloud" : provider.name || provider.id;
  const currentModelLabel = modelDisplayName(currentModel);
  return html`
    <section
      data-testid="active-model-panel"
      className="mb-4 rounded-[12px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] p-3 sm:p-3.5"
    >
      <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
        <div className="min-w-0">
          <div className="text-xs font-semibold uppercase tracking-[0.08em] text-[var(--v2-accent-text)]">
            Current model
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-2">
            <span className="text-sm font-semibold text-[var(--v2-text-strong)]">
              ${displayName}
            </span>
            <span
              className="rounded-full border border-[color-mix(in_srgb,var(--v2-positive-text)_34%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] px-2 py-0.5 text-[11px] font-semibold text-[var(--v2-positive-text)]"
            >
              ${currentModelLabel || t("llm.none")}
            </span>
          </div>
        </div>
        <div className="w-full lg:max-w-[34rem]">
          <${ActiveModelPicker}
            provider=${provider}
            currentModel=${currentModel}
            onListModels=${onListModels}
            onApplyModel=${onApplyModel}
            t=${t}
          />
        </div>
      </div>
    </section>
  `;
}

export function ProviderManagement({ settings, gatewayStatus, searchQuery = "" }) {
  const t = useT();
  const desktop = isDesktopRuntime();
  const actions = useProviderManagementActions({ settings, gatewayStatus, searchQuery, t });
  const state = actions.providerState;
  // NEAR AI / Codex authenticate via login flows; on success the snapshot
  // refresh re-renders the now-active card in place (no navigation here).
  const login = useProviderLogin();
  const loginBusy = login.nearaiBusy || login.codexBusy;
  const queryClient = useQueryClient();
  // Desktop only: apply a model on the ACTIVE provider via the same set-active
  // route the chat popover uses; the snapshot invalidation re-renders every
  // consumer.
  const applyModel = React.useCallback(
    async (provider, model) => {
      await setActiveLlm({ provider_id: provider.id, model });
      await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
    },
    [queryClient]
  );

  if (desktop) {
    return DesktopProviderManagement({ t, actions, state, login, loginBusy, applyModel, searchQuery });
  }

  // --- Web (canonical mono behavior) ---
  if (searchQuery && actions.filteredProviders.length === 0) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  const groups = groupProvidersByStatus(
    actions.filteredProviders,
    state.builtinOverrides,
    state.activeProviderId
  );

  return html`
    <${Card} className="p-4 sm:p-6">
      <div className="mb-4 flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="font-mono text-[11px] uppercase tracking-[0.14em] text-[var(--v2-accent-text)]">
            ${t("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${t("llm.providersDesc")}</p>
        </div>
        <${Button} type="button" variant="secondary" size="sm" className="gap-2" onClick=${() => actions.openDialog(null)}>
          <${Icon} name="plus" className="h-3.5 w-3.5" />
          ${t("llm.addProvider")}
        <//>
      </div>

      ${actions.message &&
      html`
        <div
          className=${[
            "mb-4 rounded-md border px-3 py-2 text-sm",
            actions.message.tone === "error"
              ? "border-red-400/30 bg-red-500/10 text-red-200"
              : "border-mint/30 bg-mint/10 text-mint",
          ].join(" ")}
          role="status"
        >
          ${actions.message.text}
        </div>
      `}

      <${ProviderLoginStatus} login=${login} />

      ${state.isLoading
        ? html`<div className="text-sm text-[var(--v2-text-muted)]">${t("common.loading")}</div>`
        : state.error
        ? html`<div className="text-sm text-red-200">${t("error.loadFailed", { what: t("llm.providers"), message: state.error.message })}</div>`
        : html`
            <div className="space-y-1">
              ${GROUP_ORDER.flatMap((group) => {
                const items = groups[group.key];
                if (!items.length) return [];
                return [
                  html`
                    <section
                      key=${group.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${group.key}
                      className="mb-3"
                    >
                      <${GroupHeader}
                        label=${t(group.labelKey)}
                        count=${items.length}
                        dotClass=${group.dotClass}
                      />
                      <div className="space-y-2">
                      ${items.map(
                        (provider) => html`
                          <${ProviderCard}
                            key=${provider.id}
                            provider=${provider}
                            activeProviderId=${state.activeProviderId}
                            selectedModel=${state.selectedModel}
                            builtinOverrides=${state.builtinOverrides}
                            isBusy=${state.isBusy}
                            onUse=${actions.handleUse}
                            onConfigure=${actions.openDialog}
                            onDelete=${actions.handleDelete}
                            onNearaiLogin=${login.startNearai}
                            onNearaiWallet=${login.startNearaiWallet}
                            onCodexLogin=${login.startCodex}
                            loginBusy=${loginBusy}
                          />
                        `
                      )}
                      </div>
                    </section>
                  `,
                ];
              })}
            </div>
          `}

      <${ProviderDialog}
        open=${actions.isDialogOpen}
        provider=${actions.dialogProvider}
        allProviderIds=${actions.allProviderIds}
        builtinOverrides=${state.builtinOverrides}
        onClose=${actions.closeDialog}
        onSave=${actions.handleSave}
        onTest=${state.testConnection}
        onListModels=${state.listModels}
      />
    <//>
  `;
}

// --- Desktop (NEAR AI Cloud only) ---
// Narrows the provider list to NEAR AI Cloud, collapses the active provider
// into an ActiveModelPanel, hides custom-provider creation, surfaces
// gateway-unreachable guidance for the synthetic offline fallback, and confirms
// deletes through the design-system ConfirmDialog. Reached only behind
// `isDesktopRuntime()`.
function DesktopProviderManagement({ t, actions, state, login, loginBusy, applyModel, searchQuery }) {
  const visibleProviders = filterDesktopVisibleLlmProviders(actions.filteredProviders);

  if (searchQuery && visibleProviders.length === 0) {
    return html`<${SettingsSearchEmpty} query=${searchQuery} />`;
  }

  const groups = groupProvidersByStatus(
    visibleProviders,
    state.builtinOverrides,
    state.activeProviderId
  );
  const activeProvider = groups.active[0] || null;
  const providersNeedingSetup = activeProvider
    ? visibleProviders.filter((provider) => provider.id !== activeProvider.id)
    : visibleProviders;
  const rowGroups = groupProvidersByStatus(
    providersNeedingSetup,
    state.builtinOverrides,
    state.activeProviderId
  );
  const hasProviderRows = providersNeedingSetup.length > 0;

  return html`
    <${Card} className="p-4 sm:p-6">
      <div className="mb-4">
        <div>
          <h3 className="text-xs font-semibold uppercase tracking-[0.08em] text-[var(--v2-accent-text)]">
            ${t("llm.providers")}
          </h3>
          <p className="mt-1 text-sm text-[var(--v2-text-muted)]">${t("llm.providersDesc")}</p>
        </div>
      </div>

      ${actions.message &&
      html`
        <div
          className=${[
            "mb-4 rounded-md border px-3 py-2 text-sm",
            actions.message.tone === "error"
              ? "border-[color-mix(in_srgb,var(--v2-danger-text)_34%,var(--v2-panel-border))] bg-[var(--v2-danger-soft)] text-[var(--v2-danger-text)]"
              : "border-[color-mix(in_srgb,var(--v2-positive-text)_34%,var(--v2-panel-border))] bg-[var(--v2-positive-soft)] text-[var(--v2-positive-text)]",
          ].join(" ")}
          role="status"
        >
          ${actions.message.text}
        </div>
      `}

      <${ProviderLoginStatus} login=${login} />

      <${ActiveModelPanel}
        provider=${activeProvider}
        currentModel=${state.selectedModel}
        onListModels=${state.listModels}
        onApplyModel=${applyModel}
        t=${t}
      />

      ${state.error &&
      html`
        <div
          className="mb-4 rounded-[12px] border border-[color-mix(in_srgb,var(--v2-warning-text)_36%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-2 text-sm text-[var(--v2-warning-text)]"
          role="status"
        >
          IronClaw cannot reach the local gateway yet. Choose a NEAR AI Cloud sign-in path; the app
          will verify the connection when the gateway is back.
        </div>
      `}
      ${state.isLoading && !hasProviderRows
        ? html`<div className="text-sm text-[var(--v2-text-muted)]">${t("common.loading")}</div>`
        : state.error && !hasProviderRows
        ? html`
            <div
              className="rounded-[12px] border border-[color-mix(in_srgb,var(--v2-warning-text)_36%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-2 text-sm leading-6 text-[var(--v2-warning-text)]"
              role="status"
            >
              IronClaw cannot reach NEAR AI Cloud yet. Start the local gateway, then retry from
              this setup panel.
            </div>
          `
        : html`
            <div className="space-y-1">
              ${GROUP_ORDER.flatMap((group) => {
                const items = rowGroups[group.key];
                if (!items.length) return [];
                return [
                  html`
                    <section
                      key=${group.key}
                      data-testid="llm-provider-group"
                      data-provider-status=${group.key}
                      className="mb-3"
                    >
                      <${GroupHeader}
                        label=${t(group.labelKey)}
                        count=${items.length}
                        dotClass=${group.dotClass}
                      />
                      <div className="space-y-2">
                        ${items.map(
                          (provider) => html`
                            <${ProviderCard}
                              key=${provider.id}
                              provider=${provider}
                              activeProviderId=${state.activeProviderId}
                              selectedModel=${state.selectedModel}
                              builtinOverrides=${state.builtinOverrides}
                              isBusy=${state.isBusy}
                              onUse=${actions.handleUse}
                              onConfigure=${actions.openDialog}
                              onDelete=${actions.handleDelete}
                              onNearaiLogin=${login.startNearai}
                              onNearaiWallet=${login.startNearaiWallet}
                              loginBusy=${loginBusy}
                            />
                          `
                        )}
                      </div>
                    </section>
                  `,
                ];
              })}
            </div>
          `}

      <${ProviderDialog}
        open=${actions.isDialogOpen}
        provider=${actions.dialogProvider}
        allProviderIds=${visibleProviders.map((provider) => provider.id)}
        builtinOverrides=${state.builtinOverrides}
        onClose=${actions.closeDialog}
        onSave=${actions.handleSave}
        onTest=${state.testConnection}
        onListModels=${state.listModels}
      />
      <${ConfirmDialog} request=${actions.confirmRequest} onClose=${actions.dismissConfirm} />
    <//>
  `;
}
