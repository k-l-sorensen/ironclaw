export const API_KEY_UNCHANGED = "\u2022\u2022\u2022\u2022\u2022\u2022\u2022\u2022";

// The full web provider model: every adapter the hosted gateway can route to.
// The desktop app narrows the *visible* set to NEAR AI Cloud at the
// `isDesktopRuntime()` gate (see `filterDesktopVisibleLlmProviders` /
// `useLlmProviders`), but the catalog itself is never trimmed \u2014 web keeps the
// complete set.
export const ADAPTER_OPTIONS = [
  { value: "open_ai_completions", label: "OpenAI Compatible" },
  { value: "anthropic", label: "Anthropic" },
  { value: "ollama", label: "Ollama" },
  { value: "nearai", label: "NEAR AI" },
];

// Desktop-only: the single provider id the packaged app surfaces in normal
// (non-advanced) mode. Additive \u2014 web ignores it.
export const DESKTOP_PRIMARY_LLM_PROVIDER_ID = "nearai";

export function adapterLabel(adapter) {
  return ADAPTER_OPTIONS.find((item) => item.value === adapter)?.label || adapter;
}

// Desktop-only helpers (additive). The desktop app presents NEAR AI Cloud as
// the sole normal model path; these narrow a provider list to it. Callers gate
// their use behind `isDesktopRuntime()` so web keeps the full provider list.
export function isDesktopVisibleLlmProvider(providerOrId) {
  const id = typeof providerOrId === "string" ? providerOrId : providerOrId?.id;
  return String(id || "").toLowerCase() === DESKTOP_PRIMARY_LLM_PROVIDER_ID;
}

export function filterDesktopVisibleLlmProviders(providers) {
  if (!Array.isArray(providers)) return [];
  return providers.filter(isDesktopVisibleLlmProvider);
}

// Desktop-only (additive): derive a readable label from the bare model id the
// gateway returns. The NEAR AI Cloud catalog carries no display-name field, so
// the desktop picker formats the id rather than collapsing every entry into one
// generic tier label. Web does not call this (it shows raw model ids), so it is
// inert there.
function modelDisplaySegment(segment) {
  const value = String(segment || "").trim();
  if (!value) return "";
  const lower = value.toLowerCase();
  const acronym = {
    ai: "AI",
    api: "API",
    fp8: "FP8",
    glm: "GLM",
    gpt: "GPT",
    llm: "LLM",
    oss: "OSS",
    tee: "TEE",
    vl: "VL",
  }[lower];
  if (acronym) return acronym;
  if (/^\d+(?:\.\d+)?[a-z]$/i.test(value)) {
    return value.replace(/[a-z]$/i, (match) => match.toUpperCase());
  }
  if (/^\d/.test(value)) return value.toUpperCase();
  return value.charAt(0).toUpperCase() + value.slice(1);
}

export function modelDisplayName(model) {
  const raw = String(model || "").trim();
  if (!raw) return "";
  if (raw.toLowerCase() === "auto") return "Auto";

  // Derive a readable label from the bare model id the gateway returns: drop
  // the vendor prefix, space the separators, preserve version dots.
  const token =
    raw
      .split(/[/:]/)
      .map((part) => part.trim())
      .filter(Boolean)
      .at(-1) || raw;
  // A trailing "major-minor" version written with a dash reads better as a
  // dot; only collapse two digits AT THE END so a trailing size/precision tag
  // keeps its structure.
  const normalized = token.replace(/(\d+)-(\d+)$/, "$1.$2");
  return normalized
    .replace(/\.(?=\d)/g, "DOTMARK")
    .split(/[\s._-]+/)
    .map((part) => modelDisplaySegment(part.replace(/DOTMARK/g, ".")))
    .filter(Boolean)
    .join(" ");
}

export function parseCustomProviders(value) {
  if (Array.isArray(value)) return value;
  if (!value || typeof value !== "string") return [];
  try {
    const parsed = JSON.parse(value);
    return Array.isArray(parsed) ? parsed : [];
  } catch (_) {
    return [];
  }
}

export function parseBuiltinOverrides(value) {
  if (value && typeof value === "object" && !Array.isArray(value)) return value;
  if (!value || typeof value !== "string") return {};
  try {
    const parsed = JSON.parse(value);
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? parsed : {};
  } catch (_) {
    return {};
  }
}

export function providerEffectiveBaseUrl(provider, overrides) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  return override.base_url || provider.env_base_url || provider.base_url || "";
}

export function providerDisplayModel(provider, overrides, activeId, selectedModel) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  if (provider.id === activeId) {
    return selectedModel || override.model || provider.env_model || provider.default_model || "";
  }
  return override.model || provider.env_model || provider.default_model || "";
}

export function providerDefaultModel(provider, overrides) {
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  return override.model || provider.env_model || provider.default_model || "";
}

export function providerAcceptsApiKey(provider) {
  if (!provider) return false;
  if (!provider.builtin) return provider.adapter !== "ollama";
  if (provider.accepts_api_key !== undefined) return provider.accepts_api_key !== false;
  return provider.api_key_required !== false;
}

export function isProviderConfigured(provider, overrides) {
  // Desktop-only: a synthetic offline-NEAR fallback entry is never "configured"
  // (the gateway is unreachable). The flag is only set by the desktop
  // `fallbackNearaiSnapshot`; web snapshots never carry it, so this is inert on
  // web.
  if (provider?.synthetic_unavailable) return false;
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  const needsKey = provider.builtin ? provider.api_key_required !== false : provider.adapter !== "ollama";
  const storedKey = provider.builtin ? override.api_key : provider.api_key;
  const hasDbKey = storedKey === API_KEY_UNCHANGED || (typeof storedKey === "string" && storedKey.length > 0);
  const keyOk = !needsKey || provider.has_api_key === true || hasDbKey;
  if (!keyOk) return false;

  const needsBaseUrl = provider.builtin ? provider.base_url_required === true : true;
  if (!needsBaseUrl) return true;
  return providerEffectiveBaseUrl(provider, overrides).trim().length > 0;
}

export function providerStatus(provider, overrides, activeProviderId) {
  if (provider.id === activeProviderId) return "active";
  return isProviderConfigured(provider, overrides) ? "ready" : "setup";
}

export function groupProvidersByStatus(providers, overrides, activeProviderId) {
  const buckets = { active: [], ready: [], setup: [] };
  if (!Array.isArray(providers)) return buckets;
  for (const provider of providers) {
    const status = providerStatus(provider, overrides, activeProviderId);
    if (buckets[status]) buckets[status].push(provider);
  }
  return buckets;
}

export function providerMissingReason(provider, overrides) {
  // Desktop-only synthetic offline-NEAR fallback (see `isProviderConfigured`).
  if (provider?.synthetic_unavailable) return "gateway";
  const override = provider.builtin ? overrides[provider.id] || {} : {};
  const needsKey = provider.builtin ? provider.api_key_required !== false : provider.adapter !== "ollama";
  const storedKey = provider.builtin ? override.api_key : provider.api_key;
  const hasDbKey = storedKey === API_KEY_UNCHANGED || (typeof storedKey === "string" && storedKey.length > 0);
  if (needsKey && provider.has_api_key !== true && !hasDbKey) return "api_key";

  const needsBaseUrl = provider.builtin ? provider.base_url_required === true : true;
  if (needsBaseUrl && !providerEffectiveBaseUrl(provider, overrides).trim()) return "base_url";
  return "ok";
}

export function providerPayload(provider, form, apiKey, overrides) {
  const baseUrl = form.baseUrl.trim();
  const model = form.model.trim();
  const payload = {
    adapter: provider?.builtin ? provider.adapter : form.adapter,
    base_url: baseUrl || provider?.base_url || "",
    provider_id: provider?.id || form.id.trim(),
    provider_type: provider?.builtin ? "builtin" : "custom",
  };
  if (model) payload.model = model;
  if (apiKey.trim()) payload.api_key = apiKey.trim();

  const override = provider?.builtin ? overrides[provider.id] || {} : {};
  if (!payload.api_key && override.api_key === API_KEY_UNCHANGED) {
    payload.api_key = undefined;
  }
  return payload;
}

export function providerIdFromName(name) {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9_]+/g, "-")
    .replace(/^-|-$/g, "");
}

export function isValidProviderId(id) {
  return /^[a-z0-9_-]+$/.test(id);
}

// After a model-discovery fetch, decide which model to commit to the form.
// The model field is a controlled <Select>: when it is empty (or holds a value
// no longer in the fetched list) the browser shows the first <option> while the
// form value stays stale, and re-picking that already-shown option fires no
// change event — so a save would persist an empty/wrong model. Returns the
// model to commit, or `null` to keep the current selection (already valid).
export function nextModelAfterFetch(currentModel, fetchedModels) {
  if (!Array.isArray(fetchedModels) || fetchedModels.length === 0) return null;
  const trimmed = (currentModel || "").trim();
  if (!trimmed || !fetchedModels.includes(trimmed)) return fetchedModels[0];
  return null;
}
