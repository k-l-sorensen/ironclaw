import { Navigate, useNavigate, useOutletContext } from "react-router";
import { useQueryClient } from "@tanstack/react-query";
import { React, html } from "../../lib/html.js";
import { isDesktopRuntime } from "../../lib/api.js";
import { useT } from "../../lib/i18n.js";
import { Badge } from "../../design-system/badge.js";
import { Button } from "../../design-system/button.js";
import { Card } from "../../design-system/card.js";
import { Icon } from "../../design-system/icons.js";
import { ProviderDialog } from "../settings/components/provider-dialog.js";
import { ProviderLoginStatus } from "../settings/components/provider-login-status.js";
import { useProviderManagementActions } from "../settings/hooks/useProviderManagementActions.js";
import { isLocalDevOrigin, useProviderLogin } from "../settings/hooks/useProviderLogin.js";
import { isProviderConfigured } from "../settings/lib/llm-providers.js";
import { setActiveLlm, testLlmProviderConnection } from "../settings/lib/settings-api.js";
import { ProviderLogo } from "./provider-logos.js";

// First-run model setup. The desktop product contract is NEAR AI Cloud by
// default; generic Reborn provider support stays hidden until an explicit
// advanced-provider mode exists. The web preview shares the same NEAR AI Cloud
// flow; desktop-only behavior is gated on isDesktopRuntime().
const RESUME_SESSION_TIMEOUT_MS = 4000;
const FEATURED = [
  {
    id: "nearai",
    auth: "nearai",
    nameKey: "onboarding.providerNearai",
    descKey: "onboarding.providerNearaiDesc",
  },
];

// One provider row: logo + name/subtitle on the left, the auth action(s) on the
// right. The three NEAR AI Cloud sign-in controls (continue / Google / Wallet)
// are all real tap targets at `md` height — the screen's whole job is sign-in,
// so the secondary actions never drop to the sub-44px `sm` size.
function FeaturedProviderRow({
  entry,
  provider,
  configured,
  showReady,
  isBusy,
  login,
  t,
  onUse,
  onSetUp,
}) {
  const name = t(entry.nameKey);

  // Desktop: NEAR AI sign-in runs in a dedicated app window (the server only
  // accepts private.near.ai callbacks; the window captures the token from that
  // navigation). Keep first-run focused on cloud sign-in; fallback setup stays
  // in Settings.
  let actions;
  if (entry.auth === "nearai") {
    actions = html`
      <${Button}
        type="button"
        variant="primary"
        size="md"
        fullWidth=${true}
        className="col-span-2"
        disabled=${login.nearaiBusy}
        onClick=${() => login.startNearai("github")}
      >
        ${t("onboarding.continue")}
      <//>
      <${Button}
        type="button"
        variant="secondary"
        size="md"
        fullWidth=${true}
        disabled=${login.nearaiBusy}
        onClick=${() => login.startNearai('google')}
      >
        ${t("onboarding.continueGoogle")}
      <//>
      <${Button}
        type="button"
        variant="secondary"
        size="md"
        fullWidth=${true}
        disabled=${login.nearaiBusy}
        onClick=${login.startNearaiWallet}
      >
        ${t("onboarding.continueWallet")}
      <//>
    `;
  } else if (configured) {
    actions = html`<${Button}
      type="button"
      variant="primary"
      size="sm"
      disabled=${isBusy}
      onClick=${() => onUse(provider)}
    >
      ${t("llm.use")}
    <//>`;
  } else {
    actions = html`<${Button}
      type="button"
      variant="primary"
      size="sm"
      disabled=${isBusy}
      onClick=${() => onSetUp(provider)}
    >
      ${t("onboarding.setUp")}
    <//>`;
  }

  return html`
    <div className="grid gap-4">
      <div className="flex min-w-0 items-start gap-3">
        <${ProviderLogo} id=${entry.id} name=${name} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-semibold text-[var(--v2-text-strong)]">${name}</span>
            ${showReady &&
            html`<${Badge} tone="positive" label=${t('onboarding.ready')} size="sm" />`}
          </div>
          <p className="mt-1 text-sm leading-6 text-[var(--v2-text-muted)]">
            ${entry.auth === "nearai" && isDesktopRuntime()
              ? t("onboarding.providerNearaiDescDesktop")
              : t(entry.descKey)}
          </p>
        </div>
      </div>
      <div className="grid grid-cols-2 gap-2">${actions}</div>
    </div>
  `;
}

// First-run trust rows are static product promises, not agent artifacts, so the
// color law applies: gold is the agent's hand and is never used as decoration.
// The icon tile stays on neutral surface/border tokens.
function TrustRow({ icon, title, body }) {
  return html`
    <div
      className="grid grid-cols-[auto_1fr] gap-3 rounded-[12px] border border-[var(--v2-panel-border)] bg-[color-mix(in_srgb,var(--v2-surface-soft)_58%,transparent)] p-3"
    >
      <span
        className="mt-0.5 grid h-8 w-8 place-items-center rounded-[8px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] text-[var(--v2-text-muted)]"
      >
        <${Icon} name=${icon} className="h-3.5 w-3.5" />
      </span>
      <span>
        <span className="block text-sm font-semibold text-[var(--v2-text-strong)]">${title}</span>
        <span className="mt-0.5 block text-sm leading-6 text-[var(--v2-text-muted)]">${body}</span>
      </span>
    </div>
  `;
}

// Compact "Set up" control for NEAR AI Cloud onboarding rows: a single trigger
// that opens a dropdown holding the real auth choices (Add API key, NEAR
// Wallet, GitHub, Google). Keeping the SSO choices behind the dropdown stops the
// first-run row from fanning four buttons across the surface, while the trigger
// stays a single honest tap target. The dropdown closes on Escape and on the
// configure action. Additive — used by the onboarding rows; the inline auth
// buttons remain for the primary NEAR flow.
function NearAiSetupMenu({ provider, isBusy, login, t, onSetUp }) {
  const [open, setOpen] = React.useState(false);
  const containerRef = React.useRef(null);

  // While the dropdown is open, close it on Escape and on an outside click so
  // it behaves like a real menu (focus does not get trapped on first-run).
  React.useEffect(() => {
    if (!open) return undefined;
    const onKeyDown = (event) => {
      if (event.key === "Escape") setOpen(false);
    };
    const onPointerDown = (event) => {
      const node = containerRef.current;
      if (node && typeof node.contains === "function" && !node.contains(event.target)) {
        setOpen(false);
      }
    };
    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("pointerdown", onPointerDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("pointerdown", onPointerDown);
    };
  }, [open]);

  const triggerDisabled = Boolean(isBusy) || Boolean(login?.nearaiBusy);

  return html`
    <div ref=${containerRef} className="relative inline-flex">
      <${Button}
        type="button"
        variant="primary"
        size="sm"
        disabled=${triggerDisabled}
        aria-haspopup="menu"
        aria-expanded=${open ? "true" : "false"}
        onClick=${() => setOpen((value) => !value)}
      >
        ${t("onboarding.setUp")}
        <${Icon} name="chevron-down" className="ml-1 h-3.5 w-3.5" />
      <//>
      ${open &&
      html`
        <div
          role="menu"
          className="absolute right-0 top-full z-10 mt-1 grid min-w-[12rem] gap-1 rounded-[10px] border border-[var(--v2-panel-border)] bg-[var(--v2-surface)] p-1 shadow-lg"
        >
          <${Button}
            type="button"
            variant="secondary"
            size="md"
            fullWidth=${true}
            disabled=${isBusy}
            onClick=${() => {
              setOpen(false);
              onSetUp(provider);
            }}
          >
            ${t("llm.addApiKey")}
          <//>
          <${Button}
            type="button"
            variant="secondary"
            size="md"
            fullWidth=${true}
            disabled=${login?.nearaiBusy}
            onClick=${() => {
              setOpen(false);
              login.startNearaiWallet();
            }}
          >
            ${t("onboarding.nearWallet")}
          <//>
          <${Button}
            type="button"
            variant="secondary"
            size="md"
            fullWidth=${true}
            disabled=${login?.nearaiBusy}
            onClick=${() => {
              setOpen(false);
              login.startNearai('github');
            }}
          >
            ${"GitHub"}
          <//>
          <${Button}
            type="button"
            variant="secondary"
            size="md"
            fullWidth=${true}
            disabled=${login?.nearaiBusy}
            onClick=${() => {
              setOpen(false);
              login.startNearai('google');
            }}
          >
            ${"Google"}
          <//>
        </div>
      `}
    </div>
  `;
}

export function OnboardingPage() {
  // First-run setup is admin-only ONLY when reached as the nested /welcome under
  // the authenticated layout — that route supplies an outlet context carrying the
  // caller's admin status, and a signed-in non-admin bounces to chat. The
  // top-level /welcome is the unauthenticated first-run entry (desktop's
  // unauthenticatedRoute) and the browser static preview: no outlet context, no
  // session yet, so the admin gate does not apply and the onboarding renders
  // directly. Gating it there would redirect every first-run visitor straight
  // back out before they could sign in.
  const outlet = useOutletContext();
  if (outlet) {
    const { isAdmin = false, isChecking = false } = outlet;
    if (isChecking) return null;
    if (!isAdmin) {
      return html`<${Navigate} to="/chat" replace />`;
    }
  }
  return html`<${OperatorOnboardingPage} />`;
}

function OperatorOnboardingPage() {
  const t = useT();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const outletContext = useOutletContext() || {};
  const { gatewayStatus } = outletContext;
  const actions = useProviderManagementActions({
    settings: {},
    gatewayStatus,
    searchQuery: "",
    t,
  });
  const state = actions.providerState;
  const fallbackNearaiProvider = React.useMemo(
    () => ({
      id: "nearai",
      name: t("onboarding.providerNearai"),
      builtin: true,
      adapter: "nearai",
      accepts_api_key: true,
      api_key_required: false,
      base_url_required: false,
      has_api_key: false,
      default_model: "auto",
    }),
    [t]
  );

  const featured = FEATURED.map((entry) => {
    const provider = state.providers.find((candidate) => candidate.id === entry.id);
    const fallbackProvider = entry.id === "nearai" ? fallbackNearaiProvider : null;
    return {
      entry,
      provider: provider || fallbackProvider,
      isFallbackProvider: !provider && Boolean(fallbackProvider),
    };
  }).filter((row) => row.provider);
  const hasSyntheticProvider = featured.some((row) => row.provider?.synthetic_unavailable);
  const hasOnlyFallbackProvider =
    featured.length === 0 || featured.every((row) => row.isFallbackProvider);
  const providerSnapshotPending = Boolean(
    state.isChecking && (hasSyntheticProvider || hasOnlyFallbackProvider)
  );
  const providerSnapshotUnavailable = Boolean(
    state.error || (!providerSnapshotPending && (hasSyntheticProvider || hasOnlyFallbackProvider))
  );
  const providerAccessBlocked = providerSnapshotPending || providerSnapshotUnavailable;
  const showFallbackAccess = providerAccessBlocked;
  const primaryAuthVariant = providerAccessBlocked ? "secondary" : "primary";
  const desktopRuntime = isDesktopRuntime();
  // A browser-served static preview (npm run dev:webui-static, or any loopback
  // origin without the Tauri shim) can never start the local sidecar, so its
  // gateway-unavailable copy must say that plainly instead of implying a desktop
  // restart will fix it. Design law: no fake readiness.
  const staticPreview = !desktopRuntime && isLocalDevOrigin();
  const accessStatusTitle = providerSnapshotPending
    ? desktopRuntime
      ? t("onboarding.gatewayCheckingDesktop")
      : t("onboarding.gatewayCheckingWeb")
    : providerSnapshotUnavailable
      ? desktopRuntime
        ? t("onboarding.gatewayUnavailableDesktop")
        : staticPreview
          ? t("onboarding.staticPreviewUnavailable")
          : t("onboarding.gatewayUnavailableWeb")
      : t("onboarding.gatewayReady");
  const gatewayPendingCopy = desktopRuntime
    ? t("onboarding.gatewayPendingCopyDesktop")
    : t("onboarding.gatewayPendingCopyWeb");
  const gatewayUnavailableCopy = desktopRuntime
    ? t("onboarding.gatewayUnavailableCopyDesktop")
    : staticPreview
      ? t("onboarding.staticPreviewUnavailableCopy")
      : t("onboarding.gatewayUnavailableCopyWeb");
  const gatewayFollowupCopy = desktopRuntime
    ? t("onboarding.gatewayFollowupCopyDesktop")
    : t("onboarding.gatewayFollowupCopyWeb");

  // NEAR AI login shares the same backend flow as the Inference tab; on success
  // here we head straight to chat.
  const navigateToChat = React.useCallback(() => navigate("/chat"), [navigate]);
  const login = useProviderLogin({ onSuccess: navigateToChat });

  // Resume an existing NEAR AI session instead of demanding a fresh sign-in. A
  // returning user (or a machine that signed in via the CLI) already holds a
  // working session the sidecar loaded — verify it with a real connection test,
  // activate it, and go straight to chat. Sign-in stays the path for genuinely
  // new machines.
  const resumeAttemptedRef = React.useRef(false);
  React.useEffect(() => {
    if (resumeAttemptedRef.current || state.isLoading) return;
    const nearai = state.providers.find((provider) => provider.id === "nearai");
    if (!nearai || state.activeProviderId) return;
    resumeAttemptedRef.current = true;
    let cancelled = false;
    const controller = new AbortController();
    const timeout = window.setTimeout(() => controller.abort(), RESUME_SESSION_TIMEOUT_MS);
    (async () => {
      try {
        const probe = await testLlmProviderConnection(
          {
            provider_id: "nearai",
            adapter: nearai.adapter || "nearai",
            model: nearai.active_model || nearai.default_model || "auto",
          },
          {
            signal: controller.signal,
          }
        );
        if (cancelled || !probe?.ok) return;
        await setActiveLlm({
          provider_id: "nearai",
          model: nearai.active_model || nearai.default_model || "auto",
        });
        await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
        if (!cancelled) navigate("/chat");
      } catch (_) {
        // No working session — the sign-in buttons below are the path.
      } finally {
        window.clearTimeout(timeout);
      }
    })();
    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
      controller.abort();
    };
  }, [state.isLoading, state.providers, state.activeProviderId, navigate, queryClient]);

  // Make an already-configured NEAR AI provider the active selection and head to
  // chat without opening the dialog.
  const handleUse = React.useCallback(
    async (provider) => {
      const model = provider.active_model || provider.default_model || "";
      await setActiveLlm({ provider_id: provider.id, model });
      await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
      navigate("/chat");
    },
    [navigate, queryClient]
  );

  const handleOnboardingSave = React.useCallback(
    async ({ form, apiKey, provider }) => {
      // Persist the provider (+ any key) via the shared save path, then make it
      // the active selection and head to chat. The cold-boot reload swaps the
      // placeholder for the real provider — no restart needed.
      await actions.handleSave({ form, apiKey, provider });
      const providerId = provider?.id || form.id.trim();
      const model = form.model?.trim() || provider?.default_model || "";
      await setActiveLlm({ provider_id: providerId, model });
      await queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
      actions.closeDialog();
      navigate("/chat");
    },
    [actions, navigate, queryClient]
  );

  return html`
    <div className="h-full overflow-y-auto bg-[var(--v2-canvas)]">
      <div
        className="mx-auto grid min-h-full max-w-6xl gap-8 px-5 py-6 sm:px-8 sm:py-10 lg:grid-cols-[0.95fr_1.05fr] lg:items-center"
      >
        <div className="max-w-2xl">
          <div
            className="mb-4 inline-flex h-7 items-center rounded-full border border-[var(--v2-panel-border)] bg-[var(--v2-surface-soft)] px-3 text-[11px] font-semibold text-[var(--v2-text-muted)]"
          >
            ${t("onboarding.nativeBadge")}
          </div>
          <h1
            className="max-w-[16ch] text-[32px] font-semibold leading-[1.06] text-[var(--v2-text-strong)] sm:text-[40px]"
          >
            ${t("onboarding.title")}
          </h1>
          <p className="mt-4 max-w-[58ch] text-base leading-7 text-[var(--v2-text-muted)]">
            ${t("onboarding.subtitle")}
          </p>
          <div className="mt-8 hidden gap-3 lg:grid">
            <${TrustRow}
              icon="spark"
              title=${t("onboarding.promiseModelsTitle")}
              body=${t("onboarding.promiseModelsBody")}
            />
            <${TrustRow}
              icon="lock"
              title=${t("onboarding.promiseApprovalsTitle")}
              body=${t("onboarding.promiseApprovalsBody")}
            />
            <${TrustRow}
              icon="file"
              title=${t("onboarding.promiseFilesTitle")}
              body=${t("onboarding.promiseFilesBody")}
            />
          </div>
        </div>

        <div className="grid gap-4">
          <${Card} radius="lg" className="p-5 sm:p-6">
            <div className="mb-4 flex items-center justify-between gap-3">
              <div>
                <div className="text-[11px] font-semibold uppercase text-[var(--v2-accent-text)]">
                  ${t("onboarding.accessLabel")}
                </div>
                <h2 className="mt-1 text-lg font-semibold text-[var(--v2-text-strong)]">
                  ${t("onboarding.accessTitle")}
                </h2>
              </div>
              <${Badge} tone="muted" label=${t("onboarding.firstRun")} size="sm" />
            </div>
            <div className="grid gap-3">
              ${showFallbackAccess
                ? html`
                    <div className="grid gap-3">
                      <div
                        className="grid grid-cols-[auto_1fr] gap-3 rounded-[12px] border border-[color-mix(in_srgb,var(--v2-warning-text)_30%,var(--v2-panel-border))] bg-[var(--v2-warning-soft)] px-3 py-3"
                      >
                        <span
                          className="grid h-8 w-8 place-items-center rounded-[8px] border border-[color-mix(in_srgb,var(--v2-warning-text)_28%,var(--v2-panel-border))] bg-[color-mix(in_srgb,var(--v2-warning-text)_12%,transparent)] text-[var(--v2-warning-text)]"
                        >
                          <${Icon} name="pulse" className="h-3.5 w-3.5" />
                        </span>
                        <span>
                          <span
                            className="block text-sm font-semibold text-[var(--v2-text-strong)]"
                          >
                            ${accessStatusTitle}
                          </span>
                          <span
                            className="mt-0.5 block text-sm leading-6 text-[var(--v2-text-muted)]"
                          >
                            ${providerSnapshotPending
                              ? gatewayPendingCopy
                              : providerSnapshotUnavailable
                                ? gatewayUnavailableCopy
                                : t("onboarding.providerNearaiDescDesktop")}
                          </span>
                        </span>
                      </div>
                      ${providerSnapshotPending &&
                      html`
                        <p className="text-sm leading-6 text-[var(--v2-text-muted)]">
                          ${gatewayFollowupCopy}
                        </p>
                      `}
                      <div className="grid grid-cols-2 gap-2">
                        <${Button}
                          type="button"
                          variant=${primaryAuthVariant}
                          size="md"
                          fullWidth=${true}
                          className="col-span-2"
                          disabled=${providerAccessBlocked || login.nearaiBusy}
                          onClick=${() => login.startNearai("github")}
                        >
                          ${t("onboarding.continue")}
                        <//>
                        <${Button}
                          type="button"
                          variant="secondary"
                          size="md"
                          fullWidth=${true}
                          disabled=${providerAccessBlocked || login.nearaiBusy}
                          onClick=${() => login.startNearai('google')}
                        >
                          ${t("onboarding.continueGoogle")}
                        <//>
                        <${Button}
                          type="button"
                          variant="secondary"
                          size="md"
                          fullWidth=${true}
                          disabled=${providerAccessBlocked || login.nearaiBusy}
                          onClick=${login.startNearaiWallet}
                        >
                          ${t("onboarding.continueWallet")}
                        <//>
                      </div>
                    </div>
                  `
                : featured.map(
                    ({ entry, provider }) => html`
                      <${FeaturedProviderRow}
                        key=${entry.id}
                        entry=${entry}
                        provider=${provider}
                        configured=${isProviderConfigured(provider, state.builtinOverrides)}
                        showReady=${state.activeProviderId === provider.id ||
                        provider.has_api_key === true}
                        isBusy=${state.isBusy}
                        login=${login}
                        t=${t}
                        onUse=${handleUse}
                        onSetUp=${actions.openDialog}
                      />
                    `
                  )}
            </div>
          <//>

          <${ProviderLoginStatus} login=${login} />

          <div className="px-1 text-sm leading-6 text-[var(--v2-text-muted)]">
            ${t("onboarding.moreInSettings")}${" "}
            <span className="font-medium text-[var(--v2-text-strong)]"> ${t("nav.settings")} </span>
          </div>

          <div className="grid gap-3 lg:hidden">
            <${TrustRow}
              icon="spark"
              title=${t("onboarding.promiseModelsTitle")}
              body=${t("onboarding.promiseModelsBody")}
            />
            <${TrustRow}
              icon="lock"
              title=${t("onboarding.promiseApprovalsTitle")}
              body=${t("onboarding.promiseApprovalsBody")}
            />
            <${TrustRow}
              icon="file"
              title=${t("onboarding.promiseFilesTitle")}
              body=${t("onboarding.promiseFilesBody")}
            />
          </div>
        </div>
      </div>

      <${ProviderDialog}
        open=${actions.isDialogOpen}
        provider=${actions.dialogProvider}
        allProviderIds=${actions.allProviderIds}
        builtinOverrides=${state.builtinOverrides}
        onClose=${actions.closeDialog}
        onSave=${handleOnboardingSave}
        onTest=${state.testConnection}
        onListModels=${state.listModels}
      />
    </div>
  `;
}
