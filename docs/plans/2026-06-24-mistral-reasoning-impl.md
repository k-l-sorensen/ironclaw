# Mistral Reasoning — Implementation Plan

**Status:** v1 Implemented — offline gate green and live acceptance PASSED · Reborn
follow-up (WU8–WU10) **scoped, unstarted** · **Date:** 2026-06-24 · **Scope:** v1
(shipped) + Reborn

> Companion to the design doc
> [`2026-06-24-mistral-reasoning-provider-architecture.md`](2026-06-24-mistral-reasoning-provider-architecture.md)
> (the `-architecture` doc). **That doc is the source of truth for *what* and
> *why*; this doc is *how* and *in what order*.** Decision numbers (D1–D10),
> component rows, and test IDs (C1–C12, U1/U2, G1) referenced below are defined
> there — do not restate them, cite them.
>
> The work-unit breakdown is complete and the per-file detail is filled in.
> Boxes checked `[x]` are done and verified by the offline test matrix + gate;
> boxes left `[ ]` are genuinely outstanding (the final live acceptance run).
> Keep this live until the feature is declared implemented.

## Current state (read first)

- The architecture is **approved and review-converged** (no open
  architecture-level items; see the design doc's "Rule-compliance review
  findings"). Implemented to the decisions as written.
- **The build is committed** as the `feat(llm): …` commit, kept **separate** from
  the planning `docs(llm): …` commit per `CLAUDE-local.md`. It was authored in one
  pass against the post-review architecture doc (D1–D10 + F1–F12 already folded
  in), so there was no pre-review drift to undo — WU0 below is a confirmation
  audit, and every item holds.
- **Offline verified:** the full matrix (C1–C12, U1/U2, G1) passes; `cargo fmt`,
  `cargo clippy --all --benches --tests --examples --all-features` (zero
  warnings), and `cargo test` are green.
- **Not yet declared done:** the live acceptance script ran against the real API
  and the receive path worked (no `ApiResponse` parse error) on small/medium/large,
  but its reasoning-trace detection needed an ANSI fix, so a clean final
  acceptance run is still outstanding (see WU7). Fork-level status lives in
  `CLAUDE-local.md`.

## Goal

Ship the custom `ProviderProtocol::Mistral` provider so `reasoning_effort=high`
works end-to-end through the v1 agent loop, with the Live-tier acceptance test
`tests/e2e_live_mistral_reasoning.rs` passing against the real API. (Why/what: see
the design doc Context + Decisions.)

## Implementation plan (work units, in order)

Each WU is independently reviewable. Acceptance = the listed test IDs pass +
quality gate. All code lands as one `feat(llm): …` commit, kept split from the
planning commit.

### WU0 — Reconcile existing working-tree code with the approved decisions ✅
Audited; all items confirmed (built against the converged decisions, no drift).
- [x] `mistral.rs` reasoning effort is the typed enum, not a `bool` (**D3**) —
      `MistralReasoningEffort` carried as `Option<…>`; no bool/`format!`.
- [x] wire model uses tagged/untagged serde enums, no `chunk["type"]==…` (**D8**) —
      `MistralMessageContent` (`#[serde(untagged)]`), `MistralContentChunk`
      (`#[serde(tag="type", rename_all="snake_case")]`), `MistralTextChunk`.
- [x] error mapping matches **D6** table; no bare `Json`, no `map_err(|_|)` —
      `post_chat` maps 401/429/413/5xx/other/parse-fail/empty per the table.
- [x] thinking extraction fails loud, no `unwrap_or_default()` (**D7**) —
      `extract_content` returns `Result`; missing text chunk → `EmptyResponse`.
- [x] endpoint pinned, key as `SecretString`, no header logging (**F10/F11**) —
      `MISTRAL_BASE_URL` const (no override knob); `api_key: SecretString`,
      `expose_secret()` only at the `Authorization` header; the request-body
      `debug!` does not include the header.
- **Acceptance:** existing code compiles and matches D3/D6/D7/D8/F10/F11. ✅

### WU1 — Types foundation (no behavior yet) ✅
- [x] `ProviderProtocol::Mistral` variant (`registry.rs`) — no serde alias of
      `open_ai_completions` (**D9**); non-`has_dedicated_config`.
- [x] `MistralReasoningEffort { High, None }` + `Option<…>` semantics (**D3**) —
      `config.rs`: `FromStr` (`high|on|true|1`→High, `off|none|false|0`→None) +
      `wire_value()`; re-exported from `lib.rs`.
- [x] wire JSON model: `MistralMessageContent` (untagged) + `MistralContentChunk`
      (tagged), unknown `type` fails loud (**D8**) — `mistral.rs`.
- **Acceptance:** **G1** (registry guard, extended in `registry.rs`), **C6/C7/C10b**
  (parser fixtures in `mistral/tests.rs`). ✅

### WU2 — Capability registries ✅
- [x] `supports_mistral_reasoning()` in `reasoning_models.rs` (**D4**) — patterns
      `mistral-small` / `mistral-medium`, case-insensitive.
- [x] `mistral-small`/`mistral-medium` added to `VISION_PATTERNS` (`vision_models.rs`).
- **Acceptance:** **U1** (`reasoning_models.rs` tests), **U2** (`vision_models.rs` test). ✅

### WU3 — The provider + factory dispatch ✅
- [x] `crates/ironclaw_llm/src/mistral.rs` `impl LlmProvider` (`complete` +
      `complete_with_tools`), reusing `tool_schema.rs` (`FlattenOnly`) +
      `sanitize_tool_messages`; **736 lines** (`<800`, **D10**), tests in sibling
      `mistral/tests.rs` via `#[path]`.
- [x] request build: `reasoning_effort_for()` gates via the WU2 helper (three
      states per D3); `chat_message_to_wire()` replays prior thinking as
      `[{thinking},{text}]`. Builder takes a `MistralRequestParams` struct (D10 —
      avoids an `#[allow(too_many_arguments)]`). `reasoning_effort` is declared as
      the 2nd request field so it survives the 500 B/event debug-log cap
      (behaviour-neutral; JSON key order is irrelevant to Mistral).
- [x] response parse: `extract_content()` splits array → `reasoning` + `content`;
      string path → `content` only; `usage_tokens()`; error mapping (**D6**).
- [x] factory arm `Mistral => create_mistral_from_registry(...)` + `mod mistral;`
      (`lib.rs`); also added the `Mistral` arm to the dedicated-config match in
      `resolution.rs` (non-exhaustive-match fix).
- **Acceptance:** **C1–C5, C8, C9, C10** (`mistral/tests.rs`). ✅

### WU4 — Config wiring + registry switch + overlay migration ✅
- [x] typed `Option<MistralReasoningEffort>` on `RegistryProviderConfig`
      (`config.rs`) (**D3**); `::generic` defaults it to `None`; the three other
      struct-literal sites (`src/config/llm.rs` custom path, `src/cli/models.rs`,
      `tests/support/gateway_workflow_harness.rs`) set `None`.
- [x] `MISTRAL_REASONING` env parse at the boundary (`src/config/llm.rs`
      `resolve_registry_provider`) (**D5**, startup-only): unset/`high`→`Some(High)`;
      `off`→`Option::None` (omit, per C2); invalid→warn+default `Some(High)`. Same
      logic mirrored in `resolution.rs::apply_registry_provider_env` (factory path).
- [x] `providers.json`: `open_ai_completions → mistral`; `default_model →
      mistral-medium-latest`.
- [x] overlay migration in the registry loader (**D9**) — `migrate_mistral_overlay`
      in `registry.rs::try_load_from_path` rewrites a Mistral overlay pinned to
      `open_ai_completions`.
- **Acceptance:** **C12** (`registry.rs` test); env→field mapping covered by
  caller-level tests in `src/config/llm.rs` (`resolve()` through the public
  resolver, incl. the off→omit and default-on cases). ✅

### WU5 — Safety (reasoning leak-scan) ✅
- [x] route `reasoning` through `LeakDetector` at the shared response stage,
      pre-persistence, covering existing DeepSeek/Gemini/OpenRouter too (**D7**).
  - **Implementation note (location):** the design suggested `src/bridge/router.rs`,
    but v1 has no single stage there — reasoning is round-tripped at ~4 sites.
    The real single chokepoint is the crate's `Reasoning` engine: scanned in
    `reasoning.rs::respond_with_tools` right after the provider call
    (`response.reasoning = redact_reasoning(...)`), which feeds every
    `RespondResult::ToolCalls` the agent loop persists and replays. So all
    reasoning-emitting providers are covered, not just Mistral.
  - Added `LeakDetector::redact_all()` to `crates/ironclaw_safety` (masks every
    match, never blocks/fails — unlike `scan_and_clean` which errors on
    Block-action, or `scan` whose `redacted_content` covers only Redact-action).
  - Backed by a `LazyLock<LeakDetector>`; `StubLlm` gained
    `with_response_reasoning` / `with_tool_call` for the caller-level test.
- **Acceptance:** **C11** (caller-level test in `reasoning.rs` through
  `respond_with_tools`; `redact_all` units in `leak_detector.rs`). ✅

### WU6 — Docs + env (mind doc-hygiene **F12**) ✅
- [x] `.env.example`: documented `MISTRAL_REASONING` (placeholder values only).
- [x] `docs/capabilities/llm-providers.md`: Mistral row + a Mistral section
      (`MISTRAL_REASONING`, default `mistral-medium-latest`, reasoning on-by-default).
- **Acceptance:** `grep -rE '/(home|Users)/|op://' .env.example docs/capabilities/llm-providers.md`
  clean. ✅

### WU7 — Acceptance + gate ✅
- [x] `cargo fmt` · `cargo clippy --all --benches --tests --examples --all-features`
      (zero warnings) · `cargo test` — all green.
- [x] **Test alignment:** the bespoke bash harness
      (`scripts/test-mistral-reasoning-ironclaw.sh`) was replaced by a Live-tier Rust
      test, `tests/e2e_live_mistral_reasoning.rs`, following the repo's standard
      `#[ignore]` + `LiveTestHarness` convention (the offline matrix C1–C12 remains
      the primary deterministic net; this is the smoke layer). `MISTRAL_API_KEY` was
      added to the harness `SECRET_TO_ENV` hydration map. The two cases below are
      the two `#[ignore]` tests in that file.
- [x] live, round-trip: `mistral_reasoning_round_trips` PASSED against the real API
      (clean 620-char reply, no `ApiResponse`/parse failure signature).
- [x] multi-turn live: `mistral_reasoning_multi_turn_replays` PASSED — turn 2 succeeded,
      confirming no HTTP 400 when the parsed thinking chunk is replayed. Both ran on the
      v1 path (`engine_v2=false`), the feature's intended scope.
      Run both:
      `IRONCLAW_LIVE_TEST=1 LLM_BACKEND=mistral MISTRAL_API_KEY=... cargo test --features libsql --test e2e_live_mistral_reasoning -- --ignored --nocapture`.
- [x] commit as `feat(llm): …` (separate from the planning commit) — landed.

## Reborn follow-up (WU8–WU10) — scoped, unstarted

Scope: the `ironclaw-reborn` binary only. Architecture/decisions (R1–R3) and the
"already present in Reborn" inventory live in the `-architecture` doc's **"Reborn
architecture (follow-up)"** section — cite, don't restate. Engine v2 is
permanently out of scope (Reborn replaces it). All three WUs land as one
`feat(llm)`/`feat(reborn)` commit, separate from docs.

Why this is small: in Reborn the custom Mistral provider is already reachable
(`build_static_provider_chain` → registry dispatch), the reasoning round-trip
already exists (`model_gateway.rs`), and reasoning is already persisted
(`ironclaw_turns` / `ironclaw_threads`). Only *enable*, *redact*, and *expose in
UI* are missing.

> **Correction (WU-CTR4, 2026-06-25):** the "reasoning round-trip already
> exists / is already persisted" claim above holds only **within a single
> turn's tool loop** (tool-call reasoning rides
> `ProviderToolCallReferenceEnvelope.response_reasoning`). It is **false across
> user turns** for a *plain assistant message*: `model_gateway.rs::convert_messages`
> rebuilds it as `ChatMessage::assistant(content)` with no `.with_reasoning(...)`,
> and there is **no `reasoning` field** on `ThreadMessageRecord` /
> `ContextMessage` / `HostManagedModelMessage` to read back. Closing this is the
> Reborn analogue of v1's CTR-1 fix and belongs in this follow-up (add the field
> to the thread/loop-support records, persist the leak-scanned trace, and
> re-attach it in `convert_messages`). It overlaps WU9's redaction work.

### WU8 — Enable reasoning via the provider catalog (R1) ☐
- [ ] Add `#[serde(default)] reasoning_effort: Option<MistralReasoningEffort>` to
      `ProviderDefinition` (`crates/ironclaw_llm/src/registry.rs`); `#[serde(default)]`
      keeps all other entries valid. Reuse the existing enum (no bool/string).
- [ ] Built-in `providers.json` (repo root): add `"reasoning_effort": "high"` to
      the `mistral` entry.
- [ ] `resolution.rs::resolve_provider_definition`: at the
      `RegistryProviderConfig::generic(...)` builder, set
      `config.mistral_reasoning = provider.reasoning_effort`. This is the single
      point feeding both the Reborn catalog path
      (`llm_catalog::resolve_against_registry` → `build_llm_config_from_resolved_provider`)
      and v1 selection/onboarding.
- [ ] Verify precedence: the v1 primary-chain `apply_registry_provider_env` path
      stays authoritative (catalog = default, env = override); an explicit
      `MISTRAL_REASONING=off` must still win and not be re-defaulted to `High`.
- **Acceptance:** caller-level test driving `resolve_against_registry` /
      `resolve_provider_config_from_selection` for built-in `mistral` →
      `mistral_reasoning == Some(High)`; a `mistral-large` case still omits the
      param (model-gate); `ProviderDefinition` round-trip test that the embedded
      `providers.json` parses the field and other entries (no field) still
      deserialize.

### WU9 — Leak-scan reasoning on the Reborn path (R2) ☐
- [ ] In `crates/ironclaw_reborn/src/model_gateway.rs`, route `response.reasoning`
      through `LeakDetector::redact_all` (added to `ironclaw_safety` in WU5)
      **before** it lands on `HostManagedModelResponse`, so the redacted form is
      what is persisted + replayed. Fail-soft (redact, never block). Covers all
      reasoning-emitting providers on that gateway, not just Mistral.
- **Acceptance:** test in `crates/ironclaw_reborn/tests/llm_gateway.rs` — a planted
      secret in a provider `reasoning` value is redacted on the returned
      `HostManagedModelResponse`. (Note: the store's `validate_optional_provider_text`
      is shape validation, not secret scanning — WU9 is still required.)

### WU10 — Surface the toggle in the Reborn WebUI v2 LLM settings (R3) ☐
Extends the **existing** LLM-provider-config feature with one per-provider field
(no new endpoint). The overlay is itself a `ProviderDefinition`, so the UI edits
the same `reasoning_effort` field WU8 adds.
- [ ] Port DTOs (`crates/ironclaw_product_workflow/src/reborn_services/llm_config.rs`):
      add `reasoning_effort` to `UpsertLlmProviderRequest` (`#[serde(default)]`) and
      `LlmProviderView` (`#[serde(skip_serializing_if = "Option::is_none")]`). No
      trait/facade change.
- [ ] Composition (`crates/ironclaw_reborn_composition/src/llm_config_service.rs`):
      thread the field through `upsert_provider` → `build_overlay_definition`
      (builtin-clone + custom paths) and read it back in `build_snapshot`. Add it
      to `RebornProviderMetadata` and populate in `provider_admin.rs::provider_info`.
- [ ] HTTP: no route change (16 KiB upsert cap). Confirm
      `crates/ironclaw_webui_v2/tests/webui_v2_descriptors_contract.rs` still
      passes unchanged.
- [ ] Frontend (`crates/ironclaw_webui_v2_static/static/js/pages/settings/`):
      `useProviderDialogForm.js` init `reasoning`, a Mistral-adapter-gated select
      in `provider-dialog.js`, and add `reasoning_effort` to the save payload in
      `useLlmProviders.js`. `node --check` the three JS files.
- **Acceptance:** caller-level round-trip in the `llm_config_service.rs` test
      module — `upsert_provider` with `reasoning_effort: Some(High)` persists it in
      the overlay and `snapshot()` reads it back on `LlmProviderView` (test through
      the service, not just the DTO).
- **Per-crate gate (don't wait for the whole graph):**
      `cargo build -p ironclaw_product_workflow --all-features`;
      `cargo build -p ironclaw_webui_v2 --features webui-v2-beta`;
      `cargo build -p ironclaw_reborn_composition --features "root-llm-provider webui-v2-beta libsql"`;
      `cargo build -p ironclaw_reborn_cli`.

### Reborn live acceptance (optional smoke) ☐
- [ ] `ironclaw-reborn serve` with `mistral`: multi-turn exchange returns a clean
      reply, no `ApiResponse` parse error, no turn-2 HTTP 400. Adapt the approach
      from `tests/e2e_live_mistral_reasoning.rs` to the Reborn binary.

## CTR-1 — Cross-turn reasoning replay (found post-ship)

**Status:** v1 fix **implemented** (WU-CTR1–3 landed, offline gate green); WU-CTR4
verification **done — confirmed a Reborn gap**, deferred to the Reborn follow-up.
**Found:** 2026-06-25 by post-ship validation of the shipped v1 path. **v1 fix:**
2026-06-25. Architecture and where-to-handle live in the `-architecture` doc's
**"CTR-1 — Cross-turn reasoning replay defect"** section — cite, don't restate.

Scope: the v1 agent loop's turn persistence + context rebuild, plus a Reborn check.
Engine v2 remains permanently out of scope.

### The one-line problem

Mistral requires replaying the full assistant message **including its `ThinkChunk`**
on every subsequent turn — and explicitly **not** rebuilding it from the answer text
alone (docs: <https://docs.mistral.ai/en/studio-api/conversations/reasoning>; the SDK
example's `messages.append(assistant_message)` with its "Do NOT rebuild the message
with only the answer text" comment is quoted in the architecture doc). v1 does this
**within a single tool loop** (`dispatcher.rs:863` et al.) but **drops it on every new
user turn and after DB hydration**, because the reasoning trace is never stored on
`Turn` and never re-attached in `Thread::messages()`. The shipped feature therefore
does **not** meet the multi-turn requirement it was built for.

### WU-CTR1 — Persist the reasoning trace on the turn ☑
- [x] Added `reasoning: Option<String>` to `Turn` (`src/agent/session.rs`) and persist
      it on the assistant **and** tool_calls rows. `ConversationMessage`
      (`crates/ironclaw_reborn_traces/src/conversation_message.rs`) gained `reasoning`;
      `reasoning TEXT` column added on **both** backends — PG `V31`
      (`migrations/V32__conversation_messages_reasoning.sql` + `checksums.lock`), libSQL
      base SCHEMA + incremental `v26` + idempotent marker. Threaded a new sibling trait
      method `add_conversation_message_with_reasoning` through `ConversationStore`
      (`src/db/mod.rs`), `Store` (`src/history/store.rs`), `postgres.rs`,
      `libsql/conversations.rs` (write + read), and the test mock — chosen over a
      signature change to avoid churning ~58 call sites. Stores the leak-scanned copy
      (redaction at `reasoning.rs`'s `redact_reasoning`). See **CTR-D1/D6**.
- [x] Populated from `RespondResult` on **both** paths. Per **CTR-D5**, extended
      `RespondResult::Text` → `Text { text, reasoning }`; the no-tools branch now
      leak-scans its reasoning too. ChatDelegate captures it onto the turn in
      `execute_tool_calls` (tool path) and `handle_text_response` (text path; trait
      signature gained a `reasoning` param, other delegates ignore it).
- **Acceptance:** **CTR-C1** libSQL dual-backend round-trip
      (`db::libsql::conversations::tests::test_conversation_message_reasoning_round_trips`,
      runs under `cargo test`). The PG half shares the same trait + `Store` SQL; a
      PG-only `--features integration` test needs a live database not available in this
      environment.

### WU-CTR2 — Re-attach on context rebuild ☑
- [x] `Thread::messages()` (`src/agent/session.rs`): `.with_reasoning(turn.reasoning…)`
      on both the `assistant_with_tool_calls` and the final `assistant` message.
- [x] `rebuild_chat_messages_from_db()` (`src/agent/thread_ops.rs`): same on both the
      assistant and tool_calls rebuild sites.
- [x] No-regression for non-Mistral providers confirmed via the existing `rig_adapter`
      round-trip (the field is load-bearing per **CTR-D4**); providers that ignore it
      (OpenAI/Anthropic) are unaffected.
- **Acceptance:** **CTR-C2/C3/C4** caller-level tests —
      `session::tests::test_messages_reattaches_reasoning_{text,tool}_turn` and
      `thread_ops::tests::test_rebuild_chat_messages_reattaches_reasoning`.

### WU-CTR3 — Real multi-turn assertion ☑
- [x] Added offline **CTR-C5** test
      `mistral::tests::ctr_c5_turn_two_request_replays_prior_think_chunk`: builds a
      turn-2 history (prior assistant message carrying reasoning) and asserts the
      serialized request body contains a `thinking` chunk for that prior message — not
      just "no 400". Complements the existing C8 wire test and the CTR-C2/C3 rebuild
      tests.

### WU-CTR4 — Reborn cross-turn check ☑ (verified — **gap confirmed, deferred**)
- [x] Verified `crates/ironclaw_reborn/src/model_gateway.rs`. **Finding:** Reborn has
      the **same** cross-turn drop for plain assistant messages. Tool-call reasoning
      *is* replayed (`provider_tool_roundtrip_messages` → `.with_reasoning(...)` off
      `ProviderToolCallReferenceEnvelope.response_reasoning`), but a plain assistant
      message is rebuilt at `model_gateway.rs` `convert_messages` as
      `ChatMessage::assistant(content)` with **no** `.with_reasoning(...)`, and no
      `reasoning` field exists on `ThreadMessageRecord` / `ContextMessage` /
      `HostManagedModelMessage` to read back. Closing it is a multi-crate Reborn change
      (`ironclaw_threads` contract, `ironclaw_loop_support`, `ironclaw_reborn`) that
      **overlaps WU8–WU10**, so per this WU's own "fold into WU8–WU10 if it overlaps"
      guidance it is **deferred to the Reborn follow-up**, not landed in this v1 pass.
      Engine v2 stays out of scope.

**Gate (v1 pass):** `cargo fmt` clean · `cargo clippy --all --benches --tests --examples
--all-features` **zero warnings** · changed-module tests green (`agent::` 517,
`ironclaw_llm` 901, libSQL conversations 11, history/worker/hooks). Full `cargo test`
also surfaces pre-existing `channels::wasm::wrapper` failures that are an environment
issue (the wasm target std is unavailable here), unrelated to CTR-1. Lands as one
`fix(agent)`/`fix(llm)` commit, separate from docs.

## Open code-level questions (from the review, carry forward)
- [x] **RESOLVED — no remap needed.** `BadGateway` and `EmptyResponse` are both
      classified transient: `retry.rs::is_retryable` (lines 45–57) and
      `circuit_breaker.rs::is_transient` (lines 231–238) explicitly list both. The
      design-doc D6 note (worried the enumerated sets omitted them) was based on a
      stale `CLAUDE.md` summary; the code names them. So 5xx→`BadGateway` and the
      no-text-chunk→`EmptyResponse` mappings stand; C10 asserts the class.

## Progress log
> Append dated markers as WUs land. Move the Status line: Scaffold → in progress
> → Implemented (<commit>).

- 2026-06-24 — scaffold created; implementation already partially in working tree (pre-review). WU0 reconciliation pending.
- 2026-06-24 — WU0–WU6 complete and verified. Offline matrix C1–C12 + U1/U2/G1 all pass; `cargo fmt` / `clippy --all-features` / `cargo test` green. Code uncommitted in working tree.
- 2026-06-24 — Open code-level question resolved: `BadGateway` + `EmptyResponse` confirmed transient (`retry.rs:45-57`, `circuit_breaker.rs:231-238`); no 5xx remap.
- 2026-06-24 — WU5 leak-scan landed in the crate `Reasoning` engine (shared v1 chokepoint) via new `LeakDetector::redact_all`; covers DeepSeek/Gemini/OpenRouter too.
- 2026-06-24 — Live acceptance run against the real API: receive path works on small/medium/large, no `ApiResponse` parse error; thinking trace and answer surfaced separately. Acceptance script rebuilt to log the full interaction across 5 cases; a trace-detection ANSI bug (false FAIL) was found and fixed. WU7 remaining: clean final acceptance run + multi-turn check + `feat(llm)` commit.
- 2026-06-24 — Implementation committed as the `feat(llm): …` commit, separate from the planning `docs(llm): …` commit. WU7 remaining: clean final live-acceptance run + multi-turn check.
- 2026-06-24 — Test alignment: the bespoke bash acceptance harness was replaced by the Live-tier Rust test `tests/e2e_live_mistral_reasoning.rs` (`#[ignore]` + `LiveTestHarness`, skips cleanly without `IRONCLAW_LIVE_TEST`); `MISTRAL_API_KEY` added to the harness `SECRET_TO_ENV` map. **WU7 closed:** both live tests PASSED against the real API (round-trip: clean reply, no parse error; multi-turn: turn 2 OK), on the v1 path (`engine_v2=false`). Offline matrix remains the primary deterministic net.
- 2026-06-25 — **CTR-1 found:** post-ship validation showed the ThinkChunk is replayed
  only within a single turn's tool loop (`dispatcher.rs:863`) and dropped on every new
  user turn + after DB hydration — `Turn` never stores the trace and `Thread::messages()`
  / `rebuild_chat_messages_from_db()` rebuild assistant messages without `.with_reasoning(...)`.
  The live multi-turn test is green on the degraded (no-ThinkChunk) path, so it gave false
  confidence. Logged as CTR-1 (architecture section + WU-CTR1–4) for a fresh architecture
  pass then implementation pass. Docs-only this change; no code.
- 2026-06-24 — Reborn follow-up scoped (WU8–WU10, unstarted). Investigation found Reborn already has the provider reachable, the reasoning round-trip (`model_gateway.rs`), and reasoning persistence (`ironclaw_turns`/`ironclaw_threads`); the only gaps are enabling the catalog field (WU8), the leak-scan on the Reborn path (WU9), and the WebUI toggle (WU10). Engine v2 ruled permanently out of scope (Reborn replaces it). Decisions R1–R3 recorded in the `-architecture` doc's "Reborn architecture (follow-up)" section. Docs-only this pass; no code.
- 2026-06-25 — **CTR-1 v1 fix implemented (WU-CTR1–3).** `Turn`/`ConversationMessage`
  gained a leak-scanned `reasoning` field, persisted via a new
  `add_conversation_message_with_reasoning` trait method on both backends (PG `V31`,
  libSQL `v26`), captured from `RespondResult` (incl. extending `Text` per CTR-D5), and
  re-attached at both context-rebuild gateways (`Thread::messages()` +
  `rebuild_chat_messages_from_db()`). Offline tests CTR-C1 (libSQL round-trip),
  CTR-C2/C3/C4 (rebuild caller-level), CTR-C5 (turn-2 wire replays the ThinkChunk) pass.
  `cargo fmt`/`clippy --all-features` clean; changed-module suites green. Per the user,
  the shared `ConversationMessage` field forced compile-only `reasoning: None` in a few
  Reborn construction sites (Option A) — no Reborn behavior changed. Code only.
- 2026-06-25 — **WU-CTR4 verified — Reborn has the same cross-turn drop, deferred.**
  `model_gateway.rs::convert_messages` rebuilds a prior plain assistant message as
  `ChatMessage::assistant(content)` with no `.with_reasoning(...)`, and no `reasoning`
  field exists on `ThreadMessageRecord`/`ContextMessage`/`HostManagedModelMessage` to
  read back (tool-call reasoning *is* replayed via `ProviderToolCallReferenceEnvelope`).
  Closing it is a multi-crate Reborn change overlapping WU8–WU10, so it is folded into
  the Reborn follow-up rather than landed in this v1 pass.
