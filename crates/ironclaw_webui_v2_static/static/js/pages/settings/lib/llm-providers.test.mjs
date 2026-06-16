import assert from "node:assert/strict";
import test from "node:test";

import {
  API_KEY_UNCHANGED,
  filterDesktopVisibleLlmProviders,
  groupProvidersByStatus,
  isDesktopVisibleLlmProvider,
  modelDisplayName,
  nextModelAfterFetch,
  providerAcceptsApiKey,
  providerStatus,
} from "./llm-providers.js";

// Helpers — minimal provider shapes that satisfy the configured/missing
// predicates without coupling the test to the full provider schema.
const builtinReady = (id, opts = {}) => ({
  id,
  name: id,
  builtin: true,
  adapter: "open_ai_completions",
  api_key_required: true,
  base_url_required: false,
  has_api_key: true,
  default_model: "model-x",
  ...opts,
});

const builtinNeedsKey = (id) => ({
  id,
  name: id,
  builtin: true,
  adapter: "anthropic",
  api_key_required: true,
  base_url_required: false,
  has_api_key: false,
  default_model: "claude",
});

const builtinNeedsBaseUrl = (id) => ({
  id,
  name: id,
  builtin: true,
  adapter: "open_ai_completions",
  api_key_required: false,
  base_url_required: true,
  has_api_key: true,
  default_model: "model-x",
});

const customReady = (id) => ({
  id,
  name: id,
  builtin: false,
  adapter: "open_ai_completions",
  base_url: "https://example.com/v1",
  api_key: API_KEY_UNCHANGED,
  default_model: "m",
});

test("providerStatus returns 'active' for the active id regardless of configuration", () => {
  const provider = builtinNeedsKey("anthropic");
  assert.equal(providerStatus(provider, {}, "anthropic"), "active");
});

test("providerStatus returns 'ready' for configured non-active providers", () => {
  assert.equal(providerStatus(builtinReady("nearai"), {}, "other"), "ready");
  assert.equal(providerStatus(customReady("vllm"), {}, "other"), "ready");
});

test("providerStatus returns 'setup' when an API key is missing", () => {
  assert.equal(providerStatus(builtinNeedsKey("anthropic"), {}, "nearai"), "setup");
});

test("providerStatus returns 'setup' when a required base URL is missing", () => {
  assert.equal(providerStatus(builtinNeedsBaseUrl("openai"), {}, "nearai"), "setup");
});

test("providerStatus returns 'setup' for synthetic unavailable fallback providers", () => {
  // Desktop-only: the synthetic offline-NEAR fallback must never be classified
  // ready, regardless of its declared key/base-url requirements.
  assert.equal(
    providerStatus(
      builtinReady("nearai", {
        adapter: "nearai",
        api_key_required: false,
        has_api_key: false,
        synthetic_unavailable: true,
      }),
      {},
      ""
    ),
    "setup"
  );
});

test("providerAcceptsApiKey supports dual-auth NEAR providers", () => {
  const nearai = builtinReady("nearai", {
    adapter: "nearai",
    api_key_required: false,
    accepts_api_key: true,
    has_api_key: false,
  });

  assert.equal(providerAcceptsApiKey(nearai), true);
});

test("providerAcceptsApiKey rejects missing provider input", () => {
  assert.equal(providerAcceptsApiKey(null), false);
  assert.equal(providerAcceptsApiKey(undefined), false);
});

test("providerAcceptsApiKey honors explicit false", () => {
  const loginOnly = builtinReady("openai_codex", {
    adapter: "openai_codex",
    api_key_required: false,
    accepts_api_key: false,
  });

  assert.equal(providerAcceptsApiKey(loginOnly), false);
});

test("groupProvidersByStatus buckets providers into active/ready/setup", () => {
  const providers = [
    builtinReady("nearai"),
    builtinNeedsKey("anthropic"),
    builtinReady("bedrock"),
    builtinNeedsKey("cerebras"),
    customReady("vllm"),
  ];

  const groups = groupProvidersByStatus(providers, {}, "nearai");

  assert.deepEqual(
    groups.active.map((p) => p.id),
    ["nearai"]
  );
  assert.deepEqual(
    groups.ready.map((p) => p.id),
    ["bedrock", "vllm"]
  );
  assert.deepEqual(
    groups.setup.map((p) => p.id),
    ["anthropic", "cerebras"]
  );
});

test("groupProvidersByStatus preserves upstream order inside each bucket", () => {
  const providers = [
    builtinReady("z-provider"),
    builtinReady("a-provider"),
    builtinNeedsKey("y-needs-key"),
    builtinNeedsKey("b-needs-key"),
  ];

  const groups = groupProvidersByStatus(providers, {}, "none");

  assert.deepEqual(
    groups.ready.map((p) => p.id),
    ["z-provider", "a-provider"],
    "ready bucket preserves input ordering (no implicit sort)"
  );
  assert.deepEqual(
    groups.setup.map((p) => p.id),
    ["y-needs-key", "b-needs-key"],
    "setup bucket preserves input ordering"
  );
});

test("groupProvidersByStatus honours builtin overrides when classifying", () => {
  // Provider declares it needs an API key but the override supplies a stored
  // sentinel value — should be considered ready, not in setup.
  const provider = builtinNeedsKey("anthropic");
  const overrides = { anthropic: { api_key: API_KEY_UNCHANGED } };

  const groups = groupProvidersByStatus([provider], overrides, "nearai");

  assert.deepEqual(groups.ready.map((p) => p.id), ["anthropic"]);
  assert.deepEqual(groups.setup.map((p) => p.id), []);
});

test("groupProvidersByStatus returns empty arrays for missing buckets, not undefined", () => {
  const groups = groupProvidersByStatus([], {}, null);
  assert.deepEqual(groups, { active: [], ready: [], setup: [] });
});

test("groupProvidersByStatus treats non-array input as empty", () => {
  const groups = groupProvidersByStatus(null, {}, null);
  assert.deepEqual(groups, { active: [], ready: [], setup: [] });
});

test("nextModelAfterFetch commits the first model when the field is empty", () => {
  // The exact Ollama bug: empty form.model + a single fetched option. The
  // controlled <Select> shows it but never commits, so save would send empty.
  assert.equal(nextModelAfterFetch("", ["qwen3:latest"]), "qwen3:latest");
  assert.equal(nextModelAfterFetch("   ", ["llama3", "qwen2"]), "llama3");
});

test("nextModelAfterFetch commits the first model when the current one is absent from the list", () => {
  assert.equal(nextModelAfterFetch("old-model", ["llama3", "qwen2"]), "llama3");
});

test("nextModelAfterFetch keeps the current model when it is in the fetched list", () => {
  assert.equal(nextModelAfterFetch("qwen2", ["llama3", "qwen2"]), null);
  assert.equal(nextModelAfterFetch(" qwen2 ", ["llama3", "qwen2"]), null);
});

test("nextModelAfterFetch keeps the current model when no models were fetched", () => {
  assert.equal(nextModelAfterFetch("", []), null);
  assert.equal(nextModelAfterFetch("x", null), null);
});

// --- Desktop-only helpers (additive; gated behind isDesktopRuntime() at call
// sites). The catalog itself is never trimmed — these only narrow a list. ---

test("filterDesktopVisibleLlmProviders keeps only NEAR AI Cloud for normal desktop UI", () => {
  const providers = [
    builtinReady("nearai", { adapter: "nearai" }),
    builtinReady("openrouter"),
    builtinReady("anthropic"),
  ];

  assert.deepEqual(
    filterDesktopVisibleLlmProviders(providers).map((provider) => provider.id),
    ["nearai"]
  );
});

test("isDesktopVisibleLlmProvider accepts provider objects and ids", () => {
  assert.equal(isDesktopVisibleLlmProvider({ id: "nearai" }), true);
  assert.equal(isDesktopVisibleLlmProvider("nearai"), true);
  assert.equal(isDesktopVisibleLlmProvider({ id: "openai" }), false);
  assert.equal(isDesktopVisibleLlmProvider(null), false);
});

test("modelDisplayName keeps NEAR model choices readable without provider plumbing", () => {
  assert.equal(modelDisplayName("auto"), "Auto");
  assert.equal(modelDisplayName("z-ai/glm-4.5"), "GLM 4.5");
  assert.equal(modelDisplayName("gpt-oss-120b"), "GPT OSS 120B");
  assert.equal(modelDisplayName("qwen/qwen3.5"), "Qwen3.5");
});

test("modelDisplayName renders the real NEAR AI Cloud catalog id, not a generic tier", () => {
  assert.equal(modelDisplayName("anthropic/claude-haiku-4-5"), "Claude Haiku 4.5");
  assert.equal(modelDisplayName("anthropic/claude-opus-4-7"), "Claude Opus 4.7");
  assert.equal(modelDisplayName("anthropic/claude-sonnet-4.5"), "Claude Sonnet 4.5");
  assert.equal(modelDisplayName("deepseek-ai/DeepSeek-V4-Flash"), "DeepSeek V4 Flash");
  assert.equal(modelDisplayName("google/gemini-2.5-pro"), "Gemini 2.5 Pro");
  assert.equal(modelDisplayName("moonshotai/kimi-k2.6"), "Kimi K2.6");
  assert.equal(modelDisplayName("openai/gpt-5.5"), "GPT 5.5");
  assert.equal(modelDisplayName("zai-org/GLM-5.1-FP8"), "GLM 5.1 FP8");
});
