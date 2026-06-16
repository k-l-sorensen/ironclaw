import { apiFetch, tauriInvoke } from "../../../lib/api.js";

// Settings endpoints depend on v1 `/api/settings/*`, `/api/tools/*`, etc.
// LLM, extension, and skills reads use v2 endpoints. Remaining settings APIs
// are TODO stubs.

export function fetchSettingsExport() {
  return Promise.resolve({ settings: {}, todo: true });
}
export function fetchSetting(_key) {
  return Promise.resolve(null);
}
export function updateSetting(_key, _value) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 settings endpoint" });
}
export function importSettings(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 settings endpoint" });
}
// LLM provider configuration — v2 native endpoints. The snapshot is the single
// source of truth: a unified provider list (built-in + operator-defined) plus
// the active selection. API-key values are write-only; the snapshot only ever
// reports `api_key_set`.
export function fetchLlmProviders() {
  return apiFetch("/api/webchat/v2/llm/providers");
}
export function upsertLlmProvider(payload) {
  return apiFetch("/api/webchat/v2/llm/providers", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
export function deleteLlmProvider(providerId) {
  return apiFetch(`/api/webchat/v2/llm/providers/${encodeURIComponent(providerId)}/delete`, {
    method: "POST",
  });
}
export function setActiveLlm(payload) {
  return apiFetch("/api/webchat/v2/llm/active", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

// Desktop-only NEAR AI Cloud credential path (additive). Both the API-key paste
// and the browser-login flow funnel through here: store the credential in the
// OS keychain and restart the bundled sidecar so it respawns with the
// credential in its env. This deliberately does NOT call `upsertLlmProvider` —
// a user `nearai` provider definition makes the gateway's list-models return
// 400, so the picker can't load the catalog. An `sk-` value is classified
// Rust-side and routed to cloud-api.near.ai; a session token routes to
// private.near.ai. Callers gate this behind `isDesktopRuntime()`.
async function activeDesktopProfileId() {
  const settings = (await tauriInvoke("get_settings").catch(() => null)) || {};
  const profiles = Array.isArray(settings.profiles) ? settings.profiles : [];
  return settings.activeProfileId || profiles[0]?.id || "default";
}

export async function restartDesktopSidecar() {
  const profileId = await activeDesktopProfileId();
  try {
    await tauriInvoke("stop_sidecar");
  } catch (_) {
    // Not running yet — start fresh below.
  }
  return tauriInvoke("start_sidecar", { providerId: "nearai", profileId });
}

export async function storeNearaiCredential(value) {
  const profileId = await activeDesktopProfileId();
  await tauriInvoke("set_llm_provider_credential", {
    profileId,
    providerId: "nearai",
    value,
  });
  return restartDesktopSidecar();
}

export function testLlmProviderConnection(payload, options = {}) {
  return apiFetch("/api/webchat/v2/llm/test-connection", {
    method: "POST",
    body: JSON.stringify(payload),
    signal: options.signal,
  });
}
export function listLlmProviderModels(payload) {
  return apiFetch("/api/webchat/v2/llm/list-models", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}
// Begin NEAR AI browser login. Returns { auth_url } to open; a background task
// stores the session token and makes NEAR AI active once the user authorizes.
export function startNearaiLogin(payload) {
  return apiFetch("/api/webchat/v2/llm/nearai/login", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

// Complete a NEAR AI wallet (NEP-413) login. `payload` carries the browser
// wallet's signed message; the backend relays it to NEAR AI, stores the session
// token, and makes NEAR AI active. Returns { active }.
export function completeNearaiWalletLogin(payload) {
  return apiFetch("/api/webchat/v2/llm/nearai/wallet", {
    method: "POST",
    body: JSON.stringify(payload),
  });
}

// Begin an OpenAI Codex (ChatGPT subscription) device-code login. Returns
// { user_code, verification_uri } to display; a background task polls for
// authorization, stores the tokens, and makes Codex active once authorized.
export function startCodexLogin() {
  return apiFetch("/api/webchat/v2/llm/codex/login", {
    method: "POST",
  });
}
export function fetchTools() {
  return Promise.resolve({ tools: [], todo: true });
}
export function updateToolPermission(_name, _state) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 tools endpoint" });
}
export function fetchExtensions() {
  return apiFetch("/api/webchat/v2/extensions");
}
export function fetchExtensionRegistry() {
  return apiFetch("/api/webchat/v2/extensions/registry");
}
export function fetchSkills() {
  return apiFetch("/api/webchat/v2/skills");
}
export function fetchSkillContent(name) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`);
}
export function installSkill(payload) {
  return apiFetch("/api/webchat/v2/skills/install", {
    method: "POST",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify(payload),
  });
}
export function updateSkill(name, payload) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`, {
    method: "PUT",
    headers: { "X-Confirm-Action": "true" },
    body: JSON.stringify(payload),
  });
}
export function removeSkill(name) {
  return apiFetch(`/api/webchat/v2/skills/${encodeURIComponent(name)}`, {
    method: "DELETE",
    headers: { "X-Confirm-Action": "true" },
  });
}
// Trace Commons credits — read-only, scoped server-side to the
// authenticated caller. The response is the contributor-local view as
// of the last credit sync; the authoritative ledger is server-side.
export function fetchTraceCredits() {
  return apiFetch("/api/webchat/v2/traces/credit");
}
// Authorize a held (manual-review) trace for submission. No request body —
// the submission id is in the path. Returns { authorized: bool }.
export function authorizeTraceHold(submissionId) {
  return apiFetch(
    `/api/webchat/v2/traces/holds/${encodeURIComponent(submissionId)}/authorize`,
    { method: "POST" }
  );
}
export function fetchUsers() {
  return Promise.resolve({ users: [], todo: true });
}
export function createUser(_payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 users endpoint" });
}
export function updateUser(_id, _payload) {
  return Promise.resolve({ success: false, message: "TODO: requires v2 users endpoint" });
}
