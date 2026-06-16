import { React } from "../../../lib/html.js";
import { isDesktopRuntime } from "../../../lib/api.js";
import { storeNearaiCredential } from "../lib/settings-api.js";
import { useLlmProviders } from "./useLlmProviders.js";

function matchesProvider(provider, query) {
  if (!query) return true;
  const q = query.toLowerCase();
  return [provider.id, provider.name, provider.adapter, provider.base_url, provider.default_model]
    .filter(Boolean)
    .some((value) => String(value).toLowerCase().includes(q));
}

export function useProviderManagementActions({ settings, gatewayStatus, searchQuery, t }) {
  const providerState = useLlmProviders({ settings, gatewayStatus });
  const [dialogProvider, setDialogProvider] = React.useState(null);
  const [isDialogOpen, setIsDialogOpen] = React.useState(false);
  const [message, setMessage] = React.useState(null);
  // Desktop-only: deletes route through the design-system ConfirmDialog (native
  // `window.confirm` does not render on the macOS overlay window). Web keeps
  // `window.confirm` — see `handleDelete`.
  const [confirmRequest, setConfirmRequest] = React.useState(null);
  const messageTimerRef = React.useRef(null);

  const showMessage = React.useCallback((tone, text) => {
    if (messageTimerRef.current) window.clearTimeout(messageTimerRef.current);
    setMessage({ tone, text });
    messageTimerRef.current = window.setTimeout(() => setMessage(null), 3500);
  }, []);

  React.useEffect(
    () => () => {
      if (messageTimerRef.current) window.clearTimeout(messageTimerRef.current);
    },
    []
  );

  const openDialog = React.useCallback((provider = null) => {
    setDialogProvider(provider);
    setIsDialogOpen(true);
  }, []);

  const handleUse = React.useCallback(
    async (provider) => {
      try {
        await providerState.setActiveProvider(provider);
        showMessage("success", t("llm.providerActivated", { name: provider.name || provider.id }));
      } catch (err) {
        if (err.message === "base_url" || err.message === "api_key" || err.message === "model") {
          openDialog(provider);
          showMessage(
            "error",
            t(err.message === "base_url" ? "llm.baseUrlRequired" : err.message === "model" ? "llm.modelRequired" : "llm.configureToUse")
          );
        } else {
          showMessage("error", err.message);
        }
      }
    },
    [openDialog, providerState, showMessage, t]
  );

  const handleSave = React.useCallback(
    async ({ form, apiKey, provider }) => {
      // Desktop NEAR AI Cloud: the pasted key goes to the OS keychain and the
      // sidecar restarts to pick it up — never a provider upsert (a user nearai
      // provider def breaks the gateway's list-models). Rust classifies an
      // `sk-` key to cloud-api.near.ai. Gated behind `isDesktopRuntime()`, so
      // web always falls through to the upsert paths below.
      if (provider?.id === "nearai" && isDesktopRuntime() && apiKey?.trim()) {
        await storeNearaiCredential(apiKey.trim());
        await providerState.refresh();
        showMessage("success", t("llm.providerConfigured", { name: "NEAR AI Cloud" }));
        return;
      }
      if (provider?.builtin) {
        await providerState.saveBuiltinProvider({ provider, form, apiKey });
        showMessage("success", t("llm.providerConfigured", { name: provider.name || provider.id }));
        return;
      }
      const saved = await providerState.saveCustomProvider({ form, apiKey, editingProvider: provider });
      showMessage("success", t(provider ? "llm.providerUpdated" : "llm.providerAdded", { name: saved.name || saved.id }));
    },
    [providerState, showMessage, t]
  );

  const handleDelete = React.useCallback(
    async (provider) => {
      // Desktop: confirm through the design-system ConfirmDialog (native
      // `window.confirm` does not render on the overlay window). Web: keep the
      // synchronous `window.confirm` the hosted UI has always used.
      if (isDesktopRuntime()) {
        setConfirmRequest({
          message: t("llm.confirmDelete", { id: provider.id }),
          title: t("llm.deleteTitle"),
          confirmLabel: t("llm.deleteConfirm"),
          tone: "danger",
          onConfirm: async () => {
            try {
              await providerState.deleteCustomProvider(provider);
              showMessage("success", t("llm.providerDeleted"));
            } catch (err) {
              showMessage("error", err.message);
              throw err; // keep the dialog open with the failure visible
            }
          },
        });
        return;
      }
      if (!window.confirm(t("llm.confirmDelete", { id: provider.id }))) return;
      try {
        await providerState.deleteCustomProvider(provider);
        showMessage("success", t("llm.providerDeleted"));
      } catch (err) {
        showMessage("error", err.message);
      }
    },
    [providerState, showMessage, t]
  );

  return {
    providerState,
    dialogProvider,
    isDialogOpen,
    message,
    confirmRequest,
    dismissConfirm: () => setConfirmRequest(null),
    filteredProviders: providerState.providers.filter((provider) =>
      matchesProvider(provider, searchQuery)
    ),
    allProviderIds: providerState.providers.map((provider) => provider.id),
    openDialog,
    closeDialog: () => setIsDialogOpen(false),
    handleUse,
    handleSave,
    handleDelete,
  };
}
