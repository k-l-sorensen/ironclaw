# Mistral Reasoning — Implementation Plan

**Status:** Implementation written — offline gate green; live acceptance + `feat(llm)` commit pending · **Date:** 2026-06-24 · **Scope:** v1 (non-Reborn)

> Companion to the design doc
> [`2026-06-24-mistral-reasoning-provider-architecture.md`](2026-06-24-mistral-reasoning-provider-architecture.md)
> (the `-architecture` doc). **That doc is the source of truth for *what* and
> *why*; this doc is *how* and *in what order*.** Decision numbers (D1–D10),
> component rows, and test IDs (C1–C12, U1/U2, G1) referenced below are defined
> there — do not restate them, cite them.
>
> The work-unit breakdown is complete and the per-file detail is filled in.
> Boxes checked `[x]` are done and verified by the offline test matrix + gate;
> boxes left `[ ]` are genuinely outstanding (final live acceptance run + the
> commit). Keep this live until the feature is declared implemented.

## Current state (read first)

- The architecture is **approved and review-converged** (no open
  architecture-level items; see the design doc's "Rule-compliance review
  findings"). Implemented to the decisions as written.
- **The build is written and uncommitted in the working tree.** It was authored
  in one pass against the post-review architecture doc (D1–D10 + F1–F12 already
  folded in), so there was no pre-review drift to undo — WU0 below is a
  confirmation audit, and every item holds.
- **Offline verified:** the full matrix (C1–C12, U1/U2, G1) passes; `cargo fmt`,
  `cargo clippy --all --benches --tests --examples --all-features` (zero
  warnings), and `cargo test` are green.
- **Not yet declared done:** the live acceptance script ran against the real API
  and the receive path worked (no `ApiResponse` parse error) on small/medium/large,
  but its reasoning-trace detection needed an ANSI fix, so a clean final
  acceptance run — and the `feat(llm): …` commit — are still outstanding.
- The planning docs are already committed (`docs(llm): …`, `74b63b0dc`). The
  implementation must land as a **separate `feat(llm): …` commit** (Conventional
  Commits + `Co-Authored-By: Claude …` trailer) — see `CLAUDE-local.md`.

## Goal

Ship the custom `ProviderProtocol::Mistral` provider so `reasoning_effort=high`
works end-to-end through the v1 agent loop, with the acceptance test
`scripts/test-mistral-reasoning-ironclaw.sh` flipping FAIL → PASS. (Why/what: see
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
  (parser fixtures in `mistral_tests.rs`). ✅

### WU2 — Capability registries ✅
- [x] `supports_mistral_reasoning()` in `reasoning_models.rs` (**D4**) — patterns
      `mistral-small` / `mistral-medium`, case-insensitive.
- [x] `mistral-small`/`mistral-medium` added to `VISION_PATTERNS` (`vision_models.rs`).
- **Acceptance:** **U1** (`reasoning_models.rs` tests), **U2** (`vision_models.rs` test). ✅

### WU3 — The provider + factory dispatch ✅
- [x] `crates/ironclaw_llm/src/mistral.rs` `impl LlmProvider` (`complete` +
      `complete_with_tools`), reusing `tool_schema.rs` (`FlattenOnly`) +
      `sanitize_tool_messages`; **736 lines** (`<800`, **D10**), tests in sibling
      `mistral_tests.rs` via `#[path]`.
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
- **Acceptance:** **C1–C5, C8, C9, C10** (`mistral_tests.rs`). ✅

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

### WU7 — Acceptance + gate ◻ (outstanding)
- [x] `cargo fmt` · `cargo clippy --all --benches --tests --examples --all-features`
      (zero warnings) · `cargo test` — all green.
- [ ] live: `scripts/test-mistral-reasoning-ironclaw.sh` clean PASS. The rebuilt,
      logging script was run against the real API and the receive path worked on
      small/medium/large with no parse error; a script-side ANSI trace-detection
      bug (false FAIL) was then fixed. **Re-run for a clean PASS verdict.**
- [ ] multi-turn live: confirm no HTTP 400 on turn 2 (the single-message runs so
      far resolved in one turn — `replay` not yet observed live).
- [ ] commit as `feat(llm): …` (separate from the planning commit).

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
