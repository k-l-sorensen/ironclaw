import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { isDesktopRuntime } from "../../../lib/api.js";
import {
  deleteLlmProvider,
  fetchLlmProviders,
  listLlmProviderModels,
  setActiveLlm,
  testLlmProviderConnection,
  upsertLlmProvider,
} from "../lib/settings-api.js";
import {
  filterDesktopVisibleLlmProviders,
  isProviderConfigured,
  providerDefaultModel,
  providerMissingReason,
} from "../lib/llm-providers.js";

// The v2 `/llm/providers` snapshot is the single source of truth: a unified
// provider list (built-in + operator-defined) already annotated with the active
// selection, `builtin`, and `api_key_set`. Overrides are no longer a separate
// client-side merge — the backend resolves them — so `builtinOverrides` is kept
// as an empty object purely for the shared helper signatures.
//
// Must be a stable reference: it is threaded down to the provider dialog's reset
// effect dependency array, so a fresh `{}` each render would re-run that effect
// on every parent re-render and wipe the in-progress form (the model the user
// just picked, the base URL they typed). A frozen module-level singleton keeps
// the identity constant across renders.
const EMPTY_BUILTIN_OVERRIDES = Object.freeze({});

// Desktop-only: a synthetic NEAR AI Cloud entry shown while the local sidecar
// gateway is unreachable, so the setup panel still offers a sign-in path
// instead of an empty list. `synthetic_unavailable` keeps it out of the ready
// bucket (see `isProviderConfigured`). Web never builds this — the real
// snapshot or an honest error is shown instead.
function fallbackNearaiSnapshot() {
  return {
    providers: [
      {
        id: "nearai",
        name: "NEAR AI Cloud",
        description: "Model access through NEAR AI Cloud.",
        adapter: "nearai",
        default_model: "auto",
        builtin: true,
        synthetic_unavailable: true,
        api_key_required: false,
        accepts_api_key: true,
        base_url_required: false,
        api_key_set: false,
      },
    ],
    active: null,
  };
}

export function useLlmProviders({ settings: _settings, gatewayStatus, enabled = true }) {
  const desktop = isDesktopRuntime();
  const queryClient = useQueryClient();
  const providersQuery = useQuery({
    queryKey: ["llm-providers"],
    queryFn: fetchLlmProviders,
    enabled,
    staleTime: 60_000,
    // Desktop only: ride out local sidecar boot (HTTP bind can lag the WebView
    // by a few seconds): connection-refused fails in milliseconds and the
    // onboarding gate must not see a settled error for a booting backend. Web
    // keeps react-query's default retry behavior.
    ...(desktop ? { retry: 1, retryDelay: 600 } : {}),
  });

  // Desktop: fall back to the synthetic NEAR entry when the snapshot is missing
  // (gateway still booting / unreachable) so the setup panel never goes blank.
  // Web: an empty snapshot stays empty — we never invent a provider.
  const fallbackSnapshot = desktop
    ? fallbackNearaiSnapshot()
    : { providers: [], active: null };
  const snapshot = enabled
    ? providersQuery.data || fallbackSnapshot
    : { providers: [], active: null };
  // If the providers query failed (e.g. 404 when the route is gated under
  // multi-user / SSO auth, or a transient 5xx / offline), we can't conclude
  // "no LLM configured" — the provider may be set operator-side at boot — so
  // callers must not treat the failure as a reason to onboard.
  const isError = enabled && providersQuery.isError;
  const builtinOverrides = EMPTY_BUILTIN_OVERRIDES;

  const providerSnapshot = deriveProviderSnapshot(snapshot, { gatewayStatus, desktop });
  const {
    allProviders,
    activeProviderId,
    selectedModel,
    builtinProviders,
    customProviders,
    hasActiveProvider,
  } = providerSnapshot;
  // Default provider id used when the user activates/saves without a prior
  // selection. Intentionally falls back to `nearai`; never used for grouping.
  const defaultProviderId = activeProviderId || "nearai";
  const providers = [...allProviders].sort((a, b) => {
    if (a.id === activeProviderId) return -1;
    if (b.id === activeProviderId) return 1;
    return (a.name || a.id).localeCompare(b.name || b.id);
  });

  const refresh = () => {
    queryClient.invalidateQueries({ queryKey: ["llm-providers"] });
  };

  const setActiveMutation = useMutation({
    mutationFn: async (provider) => {
      if (!isProviderConfigured(provider, builtinOverrides)) {
        const reason = providerMissingReason(provider, builtinOverrides);
        throw new Error(reason === "base_url" ? "base_url" : "api_key");
      }
      const model = providerDefaultModel(provider, builtinOverrides);
      if (!model) throw new Error("model");
      await setActiveLlm({ provider_id: provider.id, model });
      return provider;
    },
    onSuccess: refresh,
  });

  // Both custom and built-in saves go through one upsert endpoint. A built-in
  // "override" is just an overlay entry that shadows the compiled-in provider
  // by id; the backend resolves later entries last.
  const saveProviderMutation = useMutation({
    mutationFn: async ({ provider, form, apiKey, editingProvider }) => {
      const isBuiltin = Boolean(provider?.builtin);
      const id = (isBuiltin ? provider.id : form.id.trim()).trim();
      const payload = {
        id,
        name: isBuiltin ? provider.name || provider.id : form.name.trim(),
        adapter: isBuiltin ? provider.adapter : form.adapter,
        base_url: form.baseUrl.trim() || provider?.base_url || "",
        default_model: form.model.trim() || undefined,
      };
      // Only send a key when a new value was typed; otherwise leave the stored
      // one untouched (omitting the field is "unchanged" on the backend).
      if (apiKey.trim()) {
        payload.api_key = apiKey.trim();
      }
      if ((editingProvider || provider)?.id === defaultProviderId && payload.default_model) {
        payload.set_active = true;
        payload.model = payload.default_model;
      }
      await upsertLlmProvider(payload);
      return payload;
    },
    onSuccess: refresh,
  });

  const deleteCustomMutation = useMutation({
    mutationFn: async (provider) => {
      await deleteLlmProvider(provider.id);
      return provider;
    },
    onSuccess: refresh,
  });

  return {
    providers,
    builtinProviders,
    customProviders,
    builtinOverrides,
    activeProviderId,
    selectedModel,
    hasActiveProvider,
    isError,
    // Desktop keeps showing any rows it already has while a refetch is in
    // flight (synthetic fallback never collapses to a spinner); web reports the
    // raw query loading state.
    isLoading: desktop ? providersQuery.isLoading && !providers.length : providersQuery.isLoading,
    isChecking: providersQuery.isLoading,
    error: providersQuery.error,
    setActiveProvider: (provider) => setActiveMutation.mutateAsync(provider),
    saveCustomProvider: (payload) => saveProviderMutation.mutateAsync(payload),
    saveBuiltinProvider: (payload) => saveProviderMutation.mutateAsync(payload),
    deleteCustomProvider: (provider) => deleteCustomMutation.mutateAsync(provider),
    testConnection: testLlmProviderConnection,
    listModels: listLlmProviderModels,
    refresh,
    isBusy:
      setActiveMutation.isPending ||
      saveProviderMutation.isPending ||
      deleteCustomMutation.isPending,
  };
}

// Map the wire snapshot onto the field names the components/helpers expect and
// resolve the active selection. `options.desktop` switches the two behaviors
// the desktop fork diverged on:
//   - provider visibility: desktop narrows to NEAR AI Cloud only; web keeps the
//     full list.
//   - active resolution: web honors a runtime/env LLM surfaced by gateway
//     status (`llm_backend`) so an operator-configured model is not masked at
//     first run; desktop trusts only the persisted snapshot's `active`, hiding
//     stale/non-NEAR active snapshots until an advanced-provider mode exists.
export function deriveProviderSnapshot(snapshot = {}, options = {}) {
  const { gatewayStatus, desktop = false } = options;
  const builtinOverrides = {};
  const rawProviders = (Array.isArray(snapshot.providers) ? snapshot.providers : []).map(
    (provider) => ({
      ...provider,
      name: provider.name || provider.description || provider.id,
      has_api_key: provider.api_key_set === true,
    })
  );
  const allProviders = desktop ? filterDesktopVisibleLlmProviders(rawProviders) : rawProviders;

  if (desktop) {
    const rawActiveProviderId = snapshot.active?.provider_id || "";
    const activeProvider = allProviders.find((provider) => provider.id === rawActiveProviderId);
    const hasActiveProvider = Boolean(activeProvider);
    const activeProviderId = hasActiveProvider ? rawActiveProviderId : "";
    const selectedModel = hasActiveProvider ? snapshot.active?.model || "" : "";
    return {
      allProviders,
      builtinProviders: allProviders.filter((provider) => provider.builtin),
      customProviders: allProviders.filter((provider) => !provider.builtin),
      builtinOverrides,
      hasActiveProvider,
      activeProviderId,
      selectedModel,
    };
  }

  // Web: honor the persisted operator snapshot, but also honor runtime/env
  // LLMs surfaced by gateway status so first-run onboarding does not mask an
  // already-live model. The honest active selection is null when nothing is
  // configured — grouping, sort, and the per-card "active" badge key off this
  // so a clean install does not promote any provider into "ACTIVE" (#4857).
  const hasActiveProvider = Boolean(snapshot.active?.provider_id || gatewayStatus?.llm_backend);
  const activeProviderId = hasActiveProvider
    ? snapshot.active?.provider_id || gatewayStatus?.llm_backend
    : null;
  const selectedModel = snapshot.active?.model || gatewayStatus?.llm_model || "";
  return {
    allProviders,
    builtinProviders: allProviders.filter((provider) => provider.builtin),
    customProviders: allProviders.filter((provider) => !provider.builtin),
    builtinOverrides,
    hasActiveProvider,
    activeProviderId,
    selectedModel,
  };
}
