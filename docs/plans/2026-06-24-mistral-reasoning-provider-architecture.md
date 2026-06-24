# Mistral Reasoning — Provider Architecture (C4 Level 3)

**Status:** Approved architecture · **Date:** 2026-06-24 · **Scope:** v1 (non-Reborn)

> This is an **architecture-level** design (C4 model, component level). It does
> not specify line-level code. A code-level implementation plan and the build
> itself are deliberately deferred to a later pass (see the fork workflow in
> `CLAUDE-local.md`).
>
> Companion docs: `docs/mistral-reasoning.md` (the API/blocker research this
> builds on), `CLAUDE-local.md` (fork status). This document supersedes the
> "design implications" sketch in `docs/mistral-reasoning.md` §4.

## Context

This fork wants to use Mistral (largest EU provider) "to the fullest", which
means running `mistral-small` / `mistral-medium` with **`reasoning_effort=high`**
through IronClaw's full agent loop.

Today Mistral is wired in `providers.json` as `protocol: open_ai_completions` and
flows through `rig-core`'s OpenAI Chat Completions client via `RigAdapter`. That
path **cannot consume Mistral's reasoning response**: with `reasoning_effort=high`,
`message.content` becomes an array of typed chunks
(`[{type:"thinking",…},{type:"text",…}]`) instead of a string, and the agent
loop fails every turn (`JsonError: did not match any variant of untagged enum
ApiResponse`). Full request/response details are in `docs/mistral-reasoning.md`.

### Gating decision — RESOLVED: build, don't upgrade

The first required step was build-vs-upgrade (`docs/mistral-reasoning.md` §3a):
does a newer `rig-core` parse Mistral reasoning natively? **Verified against the
latest `rig-core` (0.39.0, Jun 2026): no.**

- The old `panic!("Reasoning content is not currently supported on Mistral via
  Rig")` is gone — but only on the **request** side, where rig now *silently
  skips* reasoning chunks (`providers/mistral/completion.rs` ~L163, L562).
- On the **response** side, the dedicated Mistral client still models
  `Message::Assistant.content` as a plain **`String`** (`completion.rs:71`), with
  no array/untagged handling. Mistral's `high` array response still fails to
  deserialize. The generic OpenAI-compat client has the same string-content
  assumption.

→ A `rig-core` bump does **not** rescue this. IronClaw must **own the Mistral
request/response** via a custom provider. (A bump may still be desirable for
other reasons, but it is orthogonal and out of scope here.)

### Decisions locked

- **Route strategy:** the custom provider owns **all** Mistral traffic (reasoning
  on and off) under a new `ProviderProtocol::Mistral`. Single code path; honours
  the existing registry rule that reasoning-aware providers must not use
  `open_ai_completions`.
- **Reasoning default:** **on (`high`) by default** for supported small/medium
  models; a toggle (high/off) can disable it. Modeled as a typed 2-variant enum
  `MistralReasoningEffort { High, None }`, **not** a `bool` and **not** the OpenAI
  low/medium/high 3-level scale (see Decision 3).

## Target architecture (C4 L3 — components)

The change adds **one new provider component** inside the `ironclaw_llm` crate
and threads a **typed reasoning-effort value** from the binary's env layer through
to it. Everything else (decorator chain, agent loop, reasoning round-trip channel)
is reused unchanged.

```
[ src/config/llm.rs ]  (binary — env-agnostic crate boundary)
   reads MISTRAL_REASONING (+ existing MISTRAL_API_KEY / MISTRAL_MODEL)
        │  populates RegistryProviderConfig (+ new reasoning flag)
        ▼
[ ironclaw_llm::lib.rs  factory dispatch ]
   match config.protocol { … ProviderProtocol::Mistral => create_mistral_from_registry(config) }
        │
        ▼
┌───────────────────────────────────────────────────────────┐
│  NEW: MistralProvider  (crates/ironclaw_llm/src/mistral.rs)│  ← sibling of nearai_chat.rs
│  impl LlmProvider                                          │
│   • own reqwest client + own JSON request/response model   │
│   • REQUEST:  sets reasoning_effort=high|none, gated to     │
│     supported models (small/medium); maps ChatMessage→wire │
│     incl. replaying prior thinking from ChatMessage.reasoning│
│   • RESPONSE: parses array content → splits thinking-chunk  │
│     into CompletionResponse.reasoning + text-chunk into     │
│     .content; also handles string content (reasoning=off)  │
│   • complete() AND complete_with_tools()                    │
│   • maps Mistral error bodies → LlmError at the boundary    │
└───────────────────────────────────────────────────────────┘
        │ returns Arc<dyn LlmProvider>
        ▼
[ build_provider_chain() ]  → Retry → SmartRouting → Failover → CircuitBreaker → Cache → Recording   (UNCHANGED)
        ▼
[ agent loop / Reasoning engine ]  (UNCHANGED)
   round-trips ChatMessage.reasoning via existing .with_reasoning(...) channel
```

### Components touched

| Component | File | Change |
|---|---|---|
| **MistralProvider (new)** | `crates/ironclaw_llm/src/mistral.rs` | New `LlmProvider` impl; own HTTP client + JSON model. Template: `nearai_chat.rs`. |
| Protocol enum | `crates/ironclaw_llm/src/registry.rs` | Add `ProviderProtocol::Mistral` variant. |
| Factory dispatch | `crates/ironclaw_llm/src/lib.rs` | Add `Mistral => create_mistral_from_registry(...)` arm; new constructor fn; `mod mistral;`. |
| **Reasoning-model registry** | `crates/ironclaw_llm/src/reasoning_models.rs` | Add `supports_mistral_reasoning(model)` helper (patterns `mistral-small`, `mistral-medium`), mirroring `supports_openai_reasoning` / `supports_anthropic_*_thinking`. The provider gates `reasoning_effort` through this — **model-gating is NOT hardcoded in the provider.** |
| **Vision-model registry** | `crates/ironclaw_llm/src/vision_models.rs` | Add `mistral-medium` / `mistral-small` to `VISION_PATTERNS` (small/medium are multimodal; `pixtral` is already present). Without this, switching the default to `mistral-medium-latest` would silently drop image attachments (same bug class the tier-first Claude patterns guard against). |
| Provider registry | `providers.json` (repo root) | Switch Mistral entry `protocol: open_ai_completions → mistral`; change `default_model: mistral-large-latest → mistral-medium-latest` (large is **not** reasoning-capable; with reasoning on-by-default the default model must be reasoning-capable). |
| **Overlay migration** | `registry.rs` loader (`~/.ironclaw/providers.json`, `$IRONCLAW_REBORN_HOME/providers.json`) | A user/operator overlay that copied the Mistral block still pins `open_ai_completions` and would **silently keep the broken rig path**. On load, rewrite an overlay Mistral entry whose protocol is `open_ai_completions` → `mistral` (or warn loudly). See Decision 9. |
| Reasoning config field | `crates/ironclaw_llm/src/config.rs` (`RegistryProviderConfig`) | Add a **typed** `Option<MistralReasoningEffort>` field (`None` = omit param). Not a `bool`. Crate stays env-agnostic. |
| Wire JSON model | `crates/ironclaw_llm/src/mistral.rs` | Typed serde model: `MistralMessageContent` (`#[serde(untagged)]` over `Text(String)` / `Chunks(Vec<…>)`) and `MistralContentChunk` (`#[serde(tag="type")]`, `Thinking`/`Text`). The untagged content enum is the actual fix for the original `ApiResponse` error. See Decision 8. |
| Env read | `src/config/llm.rs` (binary) | Parse `MISTRAL_REASONING` (`high`/`on`/`true`/`1` → `High`; `off`/`none`/`false`/`0` → `None`; default `High`) into the typed field at the boundary. |
| Reasoning leak-scan | shared response stage (`src/bridge/router.rs` / agent boundary) + `ironclaw_safety::LeakDetector` | Route the `reasoning` field through the same `LeakDetector` path as `content` before it is stored or replayed. Prefer the shared stage so existing DeepSeek/Gemini/OpenRouter reasoning is covered too. See Decision 7. |
| Registry guard test | `crates/ironclaw_llm/src/registry.rs` | Extend `reasoning_aware_providers_use_dedicated_protocol_not_openai_compat` to assert the built-in Mistral entry resolves to `ProviderProtocol::Mistral` (equality), not merely "not OpenAiCompletions". |
| Provider docs | `docs/capabilities/llm-providers.md` | Update the Mistral row (note reasoning support) and section: `MISTRAL_REASONING`, default `mistral-medium-latest`, supported reasoning models, on-by-default behavior. |
| Env docs | `.env.example` | Document `MISTRAL_REASONING`. |

### Reused — do NOT rebuild

- **Reasoning round-trip channel:** `ChatMessage.reasoning` + `.with_reasoning(...)`
  and `CompletionResponse/ToolCompletionResponse.reasoning` already exist (added
  for DeepSeek/Gemini/OpenRouter, issues #3201/#3225). The new provider populates
  the same fields; the agent loop already replays them. This satisfies Mistral's
  multi-turn requirement (replay the ThinkChunk) with **no agent-loop change**.
- **Decorator chain** (`build_provider_chain` in `lib.rs`): the new provider
  returns `Arc<dyn LlmProvider>` and inherits retry/routing/failover/cache/record
  for free.
- **Tool-schema normalization** patterns from `nearai_chat.rs` / `rig_adapter.rs`
  (OpenAI strict vs flatten-only) — reuse the appropriate one for tool calls.
- **Model-capability registries** (`reasoning_models.rs`, `vision_models.rs`) —
  extend with Mistral patterns rather than introducing ad-hoc model checks in the
  provider. The provider calls these helpers; it does not own model lists.
- **`LlmProvider` trait** is the single integration seam; no trait changes.

## Key architectural decisions

1. **Custom provider, not rig** — rig 0.39 still can't parse array content
   (gating decision above). Owning the JSON model is the only correct route and
   matches IronClaw's own rule against reasoning-aware providers on
   `open_ai_completions`.
2. **One protocol for all Mistral traffic** — `ProviderProtocol::Mistral` handles
   both reasoning-on (array response) and reasoning-off (string response). No
   conditional dual-path.
3. **Reasoning effort is a typed 2-variant enum, default on** —
   `MistralReasoningEffort { High, None }` with `#[serde(rename_all="snake_case")]`
   (emits exactly `"high"`/`"none"`). **Never a `bool`** (would force a
   bool→magic-string `format!`, banned by `types.md`) and **never** the OpenAI
   3-level scale. The env string is converted to this type at the binary boundary
   only. The wire has **three** states, expressed as `Option<MistralReasoningEffort>`:
   `Option::None` = **omit** the param (model-gated off, or unsupported model);
   `Some(High)` = send `"high"`; `Some(None)` = send explicit `"none"`. Do not
   collapse `Option::None` and `Some(None)` — they are distinct wire behaviors (C2
   asserts omit, not an explicit `"none"`). Implementer note: the variant is named
   `None` to mirror the wire value but shadows `Option::None` in match arms; `Off`
   would read cleaner if the wire spelling is set via `#[serde(rename="none")]`.
4. **Model-gating via the existing registry, not the provider** —
   `reasoning_effort` is sent only for supported models (`mistral-small*`,
   `mistral-medium*`) via a new `supports_mistral_reasoning()` helper in
   `reasoning_models.rs`. For others (e.g. `mistral-large`) the param is
   auto-omitted, so the toggle is safe regardless of selected model. Default model
   becomes `mistral-medium-latest` so on-by-default reasoning engages out of the
   box.
5. **Env stays in the binary, toggle is startup-only** — `MISTRAL_REASONING` is
   read and typed in `src/config/llm.rs`; the crate remains env-agnostic. The
   toggle is **startup-env-only, not a hot-reloadable setting** — this sidesteps
   the persist-then-reload atomicity hazard (`error-handling.md`). If it ever
   becomes settings-backed, it must adopt pre-validate or snapshot+rollback.
6. **Error mapping at the channel boundary, cause carried** — Mistral failures map
   to specific `LlmError` variants inside `mistral.rs` (mirrors `nearai_chat.rs`),
   so retry/circuit-breaker classification is correct and no internal detail leaks
   to the user. **Never** `.map_err(|_| …)` (drops the cause; banned and
   non-exemptible) and **never** let `serde_json` parse failures surface as bare
   `LlmError::Json` (non-retryable + doesn't trip the breaker — wrong class for a
   transient body). Mapping table:

   | Mistral response | `LlmError` variant | Class |
   |---|---|---|
   | 401 auth | `AuthFailed { provider: "mistral" }` | non-transient (API-key only; no session renewal) |
   | 429 rate limit | `RateLimited { retry_after }` (parse `Retry-After`) | transient |
   | 413 / context overflow | `ContextLengthExceeded` via shared `error::context_length_error` | non-transient |
   | 5xx | `BadGateway { provider, status, retry_after }` — body to `debug!` only, never on the error | transient |
   | other non-2xx | `RequestFailed { reason: "HTTP {status}: {truncated}" }` | transient |
   | 2xx, envelope won't deserialize | `InvalidResponse { reason }` (not `Json`) | transient |
   | 2xx, well-formed, no `text` chunk / empty choices | `EmptyResponse` | transient |
   | transport / reqwest error | `RequestFailed { reason: e.to_string() }` | transient |

   **Class verification (code-pass):** the crate's enumerated retryable/breaker
   sets (see `ironclaw_llm/CLAUDE.md`) list `RequestFailed`, `RateLimited`,
   `InvalidResponse`, `Http`, `Io` — they do **not** explicitly name `BadGateway`
   or `EmptyResponse`. Confirm `retry.rs::is_retryable()` + the circuit breaker
   actually treat those two as transient; if not, remap the 5xx row to
   `RequestFailed`. Test C10 must assert the **class** (retryable + trips breaker),
   not just the variant.

7. **Reasoning trace rides the same safety scan as content** — the `thinking`
   trace is a new model-output surface that is both **stored** (LLM data is never
   deleted) and **replayed into the LLM** next turn. It must pass through
   `ironclaw_safety::LeakDetector` before storage/replay, exactly as `content`
   does. Hook it at the **shared response stage** (so the existing
   DeepSeek/Gemini/OpenRouter `reasoning` fields get covered too), not only inside
   `mistral.rs`. The scan must occur **pre-persistence** so the stored reasoning is
   already redacted and replay inherits it — do **not** reuse only the existing
   response-before-user `LeakDetector` site, which fires on the outbound-to-user
   path and would leave the reasoning-replay-to-LLM path unscanned (the rule's
   "wrong stage" bug). The thinking-chunk extraction must **fail loud** (`EmptyResponse`/
   `InvalidResponse`) on a malformed reasoning-on array — never `unwrap_or_default()`
   to `""`, which would drop the answer *and* the ThinkChunk and cause the turn-2
   HTTP 400 this design exists to prevent.
8. **Typed wire model, no stringly-typed parsing** — request/response JSON uses
   serde-tagged/untagged enums (`MistralMessageContent` untagged String-or-array;
   `MistralContentChunk` tagged on `type`), never `chunk["type"] == "thinking"`
   string matching. Unknown chunk `type` fails loud rather than being silently
   skipped.
9. **Overlay migration is in-scope** — changing the built-in `providers.json` does
   not touch user/operator overlays, which would silently keep the broken
   `open_ai_completions` route. The registry loader must rewrite (or loudly warn
   on) an overlay Mistral entry pinned to `open_ai_completions`. `open_ai_completions`
   is **not** serde-aliased onto `Mistral` (it remains a live value for other
   providers) — this is a value migration, not an enum-rename.
10. **Build guardrails (architecture-discipline)** — `mistral.rs` targets `< 800`
    lines (the template `nearai_chat.rs` is ~2,988); co-locate tests in a sibling
    rather than inline. Reuse the shared chat-completions wire-shaping seams
    (`tool_schema.rs` normalization, `provider.rs::sanitize_tool_messages`) instead
    of hand-rolling a 4th copy; keep only the genuinely novel logic (array/string
    content parsing + ThinkChunk replay) Mistral-specific. Any unavoidable
    `#[allow(clippy::too_many_arguments)]` carries an `// arch-exempt:` annotation
    with a plan link — never bare; prefer a params struct.

## Out of scope

- `rig-core` version bump (orthogonal; doesn't solve this).
- `prompt_mode` knob (risks layering Mistral's own system prompt over IronClaw's).
- Reborn / engine v2 — this is the v1 path only.
- Streaming reasoning surfacing in the TUI (round-trip + final answer is the goal;
  live thinking display is a possible follow-up, not required).

## Verification (for the future implementation pass)

The regression net is **offline and deterministic**; the live scripts are
complementary smoke tests, not the primary coverage. Both
`supports_mistral_reasoning()` and `is_vision_model()` gate side effects, so per
`testing.md` ("Test Through the Caller") helper unit tests alone are insufficient —
call-site tests are mandatory.

**Offline regression net (`cargo test`, no API key — use the crate `testing` feature / recorded fixtures):**

| # | Driver | Input | Assert |
|---|---|---|---|
| C1 | **`complete()`** entry point | `mistral-medium-latest`, reasoning ON | body **contains** `reasoning_effort: "high"` (drives the public trait method, not a shared sub-helper) |
| C2 | request builder | `mistral-medium-latest`, reasoning OFF | body **omits** `reasoning_effort` |
| C3 | request builder | `mistral-large-latest`, reasoning ON | body **omits** it (model-gate beats toggle) |
| C4 | request builder | `mistral-small-latest`, reasoning ON | body **contains** it |
| C5 | ChatMessage→wire | `mistral-medium-latest` + image attachment | wire request **includes the image part** (proves `is_vision_model` consulted, attachment not dropped) |
| C6 | response parser | recorded **array** fixture `[{thinking},{text}]` | `.reasoning` = thinking, `.content` = text, no error |
| C7 | response parser | recorded **string** fixture | `.content` set, `.reasoning` none |
| C8 | multi-turn builder | turn-1 `.reasoning` set, fed back via `.with_reasoning(...)` | turn-2 request replays the ThinkChunk |
| C9 | `complete_with_tools` | tool-bearing request, reasoning ON | both `reasoning_effort` and tool schema present (test both trait methods) |
| C10 | error mapping | recorded error bodies (401/429/413/5xx/malformed) | each maps to the variant in Decision 6's table; assert the **class** (retryable + trips-breaker), not just the variant |
| C10b | response parser | 2xx well-formed envelope with an **unknown chunk `type`** | fails loud (`InvalidResponse`/`EmptyResponse`), **not** silently skipped (Decision 8) |
| C11 | leak-scan (Decision 7) | planted secret in a `thinking` chunk | redacted in the persisted record **and** in the replayed prompt |
| C12 | overlay migration | overlay Mistral entry pinned to `open_ai_completions` | loader rewrites to `mistral` (or warns) — not silently kept |
| U1 | helper units | `supports_mistral_reasoning`: small/medium `true`, large `false`, case-insensitive, `auto` `false` | — |
| U2 | helper units | `is_vision_model`: mistral small/medium `true`, `pixtral` still `true` | — |
| G1 | registry guard | built-in registry | `find("mistral").protocol == ProviderProtocol::Mistral` |

**Live smoke tests (`-- --ignored`, need `MISTRAL_API_KEY`):**

1. **Acceptance (the locked artifact):** `scripts/test-mistral-reasoning-ironclaw.sh`
   — real agent via `ironclaw -m`, Mistral + reasoning on. Currently **FAILS**
   (`JsonError … ApiResponse`); must **PASS** after the change. Needs
   `DATABASE_BACKEND=libsql` + throwaway `LIBSQL_PATH`.
2. **Raw-API control:** `scripts/test-mistral-reasoning.sh` already PASSes.
3. **Multi-turn live:** ≥2-turn exchange, reasoning on; no HTTP 400 on turn 2.

**Gate:** `cargo fmt`, `cargo clippy --all --benches --tests --examples --all-features`
(zero warnings), `cargo test`.

## Rule-compliance review findings

This architecture was reviewed against six `.claude/rules` (2026-06-24). Verdicts:
doc-hygiene ✅ clean; architecture ✅ compliant-with-guardrails; error-handling,
types, testing, safety ⚠️ concerns — all now folded into the decisions above. The
table records each finding and whether it is **settled at architecture level**
(decided here, code just implements it) or **deferred to code** (a constraint the
implementation must honor, not resolvable in a design doc).

| # | Rule | Finding | Resolution | State |
|---|---|---|---|---|
| F1 | safety | Reasoning trace stored + replayed to LLM but not leak-scanned | Decision 7 — scan via `LeakDetector` at shared response stage | Settled (design); impl deferred |
| F2 | types | `providers.json` overlay still pins `open_ai_completions` → silent broken path | Decision 9 + overlay-migration component + test C12 | Settled |
| F3 | types / arch | Toggle modeled as `bool` | Decision 3 — `MistralReasoningEffort { High, None }` as `Option` | Settled |
| F4 | types | String/array content + chunk union parsed by ad-hoc `type` matching | Decision 8 — untagged + tagged serde enums; fail loud on unknown | Settled |
| F5 | error-handling | Decision 6 under-specified; `Json` vs `InvalidResponse` class trap | Decision 6 — full mapping table; never bare `Json`, never `map_err(|_|)` | Settled |
| F6 | error-handling | Thinking extraction could `unwrap_or_default()` → drops answer + turn-2 400 | Decision 7 — fail loud, never default to `""` | Settled |
| F7 | error-handling | Persist-then-reload hazard for the toggle | Decision 5 — startup-env-only, not hot-reloadable | Settled |
| F8 | testing | Helper-only tests insufficient; over-reliance on live script | Offline matrix C1–C12 + U1/U2/G1; live tests demoted to smoke | Settled (matrix); impl deferred |
| F9 | architecture | `mistral.rs` may exceed file-size budget; risk of 4th wire-shaping copy | Decision 10 — `<800` target, reuse shared seams | Settled (constraint); impl deferred |
| F10 | safety | Mistral endpoint must be pinned/validated; no user base-URL override | Impl constraint — hardcode `https://api.mistral.ai`, no override knob | Deferred to code |
| F11 | safety | `MISTRAL_API_KEY` as `SecretString`; never log `Authorization` header | Impl constraint — `SecretString`, `expose_secret()` only at header build | Deferred to code |
| F12 | doc-hygiene | Future `.env.example` / `llm-providers.md` edits must avoid `op://` ref + abs paths | Impl constraint — placeholder values only; grep before merge | Deferred to code |

**What this leaves for the code-level pass:** F10–F12 are constraints that can only
be verified against real code, plus the mechanical implementation of every
"settled" item. No open architecture-level questions remain.
