import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

// Merged contract (supersedes the desktop fork's "codex must not exist
// anywhere" rule): the web SPA keeps the full provider set, including the
// OpenAI Codex device-code login. The desktop app surfaces NEAR AI Cloud only.
// That narrowing is enforced at RUNTIME via `isDesktopRuntime()` gates — not by
// deleting the codex paths from shared source. These assertions lock in BOTH
// halves: codex stays available (web) AND the desktop NEAR-only narrowing is
// gated.

function read(relativePath) {
  return readFileSync(new URL(relativePath, import.meta.url), 'utf8');
}

test('web provider surfaces keep the OpenAI Codex login path (security mandate)', () => {
  const card = read('../components/provider-card.js');
  const login = read('../hooks/useProviderLogin.js');
  const api = read('../lib/settings-api.js');

  // The Codex sign-in branch on the card + the device-code flow in the hook +
  // the real endpoint must all remain present on the shared (web) surface.
  assert.match(card, /openai_codex/);
  assert.match(card, /codexSignIn/i);
  assert.match(login, /startCodexLogin/);
  assert.match(login, /isLocalDevOrigin/, 'the #4705 local-dev SSO guard must remain');
  assert.match(api, /\/api\/webchat\/v2\/llm\/codex\/login/);
});

test('the desktop NEAR-only provider narrowing is runtime-gated, not hard-coded', () => {
  const management = read('../components/provider-management.js');
  const hook = read('../hooks/useLlmProviders.js');

  // The NEAR-only filter and the desktop-specific UI branch are reached only
  // behind the desktop runtime gate, so web keeps the full provider list.
  assert.match(management, /isDesktopRuntime\(\)/);
  assert.match(management, /filterDesktopVisibleLlmProviders/);
  assert.match(hook, /isDesktopRuntime\(\)/);
  assert.match(hook, /filterDesktopVisibleLlmProviders/);
});

test('the full provider catalog (all adapters) is preserved', () => {
  const providers = read('../lib/llm-providers.js');

  // ADAPTER_OPTIONS must keep every web adapter; the desktop NEAR-only set is a
  // separate, additive narrowing helper, never a replacement of the catalog.
  assert.match(providers, /open_ai_completions/);
  assert.match(providers, /"anthropic"/);
  assert.match(providers, /"ollama"/);
  assert.match(providers, /"nearai"/);
});
