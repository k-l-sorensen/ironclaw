# Mistral Reasoning + CTR-1 — Code Review Findings

**Date:** 2026-06-25 · **Reviewer:** paranoid-architect pass · **Branch:**
`mistral-reasoning-fix` (vs `main`) · **Status:** findings open, to be fixed
**one at a time**.

> Scope of the reviewed branch: custom `ProviderProtocol::Mistral` provider with
> `reasoning_effort` support (`crates/ironclaw_llm/src/mistral.rs` +
> `mistral_tests.rs`), cross-turn reasoning persistence (CTR-1: new `reasoning`
> column on both DB backends, `Turn`/`ConversationMessage` field, rebuild
> re-attachment), and `LeakDetector::redact_all` for scanning reasoning traces.
>
> Companion docs (cite, don't restate):
> `docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md`,
> `docs/plans/2026-06-24-mistral-reasoning-impl.md`, `CLAUDE-local.md`.

## How to use this document

Each finding below is **independently fixable** and carries: location, the
problem, the evidence, a concrete fix, and acceptance criteria. Fix them one at
a time; check the box and append a dated line to the per-finding log when done.
F1 and F2 share a root cause and are cheapest to do together, but each is
written to stand alone.

Overall assessment: the branch is high-quality and well-tested (error mapping,
`SecretString` handling, fail-loud serde, dual-backend migrations, trait
propagation to all delegates + `StubLlm`, centralized leak-scan before
persistence — all verified correct). These findings are refinements, mostly in
the CTR-1 cross-turn layer, not blockers on the core provider.

---

## F1 — `MISTRAL_REASONING` invalid-input handling diverges between the two resolution paths

- **Severity:** Medium
- **Category:** Correctness / consistency
- **Status:** ☑ fixed

### Locations
- `crates/ironclaw_llm/src/resolution.rs:424-439` (`apply_registry_provider_env`) — **fail-closed**.
- `src/config/llm.rs:803-826` (`resolve_registry_provider`) — **fail-open**.

### Problem
The same env var is parsed in two places with **opposite** behavior on an
invalid value:

- `resolution.rs` (catalog / Reborn path): invalid → `Err(LlmError::RequestFailed)`,
  which aborts resolution/startup.
- `src/config/llm.rs` (v1 binary env-boundary path): invalid → `tracing::warn!`
  and default to `Some(High)`, continues.

So `MISTRAL_REASONING=hgih` (typo) boots fine on one path and hard-fails on the
other, depending on which resolver runs. The impl doc (WU4) claims the logic is
"mirrored" in both sites — it is not.

### Evidence
```rust
// resolution.rs — fail-closed
match value.parse::<MistralReasoningEffort>().map_err(|reason| {
    LlmError::RequestFailed { provider: config.provider_id.clone(),
        reason: format!("invalid MISTRAL_REASONING: {reason}") }
})? { /* ... */ }
```
```rust
// src/config/llm.rs — fail-open
Err(e) => {
    tracing::warn!("Invalid MISTRAL_REASONING ({raw}): {e}; defaulting to high");
    Some(MistralReasoningEffort::High)
}
```

### Fix
Pick one policy and apply it to both sites (extract a shared helper — see F2,
which wants the same extraction). Recommended: **fail-open with a warning**, to
match the existing v1 caller-level tests
(`mistral_reasoning_invalid_warns_and_defaults_to_high` in `src/config/llm.rs`)
and the general "a bad toggle shouldn't brick startup" principle. If fail-closed
is preferred instead, update that test accordingly.

### Acceptance
- Both `apply_registry_provider_env` and `resolve_registry_provider` treat an
  invalid `MISTRAL_REASONING` identically.
- A test drives each path with an invalid value and asserts the chosen behavior
  (warn+default, or error) — not just the helper in isolation
  (`.claude/rules/testing.md`: test through the caller).

---

## F2 — The two resolution paths gate Mistral reasoning on different keys (id string vs protocol)

- **Severity:** Medium
- **Category:** Correctness
- **Status:** ☑ fixed

### Locations
- `src/config/llm.rs:809` — gate is `if canonical_id == "mistral"`.
- `crates/ironclaw_llm/src/resolution.rs:424` — gate is `if config.protocol == ProviderProtocol::Mistral`.

### Problem
A provider that uses the **Mistral protocol** but is registered under any id
other than the literal `"mistral"` (a custom overlay entry, a renamed provider,
or the overlay-migrated entry whose id is an alias) is handled inconsistently:

- Catalog/Reborn path (`resolution.rs`, protocol-gated): reasoning defaults to
  `Some(High)`.
- v1 path (`src/config/llm.rs`, string-gated): the `mistral_reasoning` block is
  skipped, leaving the field `None` → param omitted → **reasoning silently
  OFF**, even though the provider is reasoning-capable Mistral.

The string gate is the odd one out; the rest of the system keys on
`ProviderProtocol::Mistral` (factory dispatch in `lib.rs`, the
`reasoning_effort_for` model gate, the overlay migration).

### Fix
Gate the `src/config/llm.rs` block on the resolved `protocol ==
ProviderProtocol::Mistral` rather than `canonical_id == "mistral"`. Combine with
F1 by extracting one shared `resolve_mistral_reasoning_from_env()` helper (in
`ironclaw_llm`, env-read stays at the binary boundary per the crate's
env-agnostic rule — so pass the raw `Option<String>` in and return
`Result<Option<MistralReasoningEffort>, _>`), called by both sites.

### Acceptance
- A Mistral-protocol provider with a non-`"mistral"` id resolves to
  `Some(High)` by default on **both** paths.
- Caller-level test through `crate::config::llm::resolve()` with such a provider
  asserts `mistral_reasoning == Some(High)`.

---

## F3 — One `turn.reasoning` slot is replayed onto two assistant messages; tool-turn replay is unvalidated

- **Severity:** Medium
- **Category:** Correctness / unvalidated contract
- **Status:** ☐ open

### Locations
- `src/agent/session.rs:558` and `:589` (`Thread::messages()`).
- `src/agent/thread_ops.rs:3119` and `:3173` (`rebuild_chat_messages_from_db`).
- Capture sites (last-write-wins): `src/agent/dispatcher.rs` `execute_tool_calls`
  (`turn.reasoning = turn_reasoning`) and `handle_text_response`
  (`turn.reasoning = reasoning`), both guarded by `is_some()`.

### Problem
A `Turn` stores a **single** `reasoning: Option<String>`, but a tool-bearing turn
re-attaches it to **two distinct** assistant messages on rebuild: the
`assistant_with_tool_calls` message *and* the final `assistant` message. Because
the dispatcher captures are last-write-wins (the final text response overwrites
the tool-round trace when both are `Some`, which is the common Mistral case), the
**final answer's** reasoning gets replayed on the **earlier tool-call** message —
a trace that did not actually precede that tool call.

Mistral's documented contract is that each assistant message replays *its own*
`ThinkChunk`. This is an approximation of that contract. Crucially, the live
multi-turn acceptance test (`tests/e2e_live_mistral_reasoning.rs`) only exercises
**pure-text** turns, so the tool-turn cross-turn replay shape (same ThinkChunk on
two messages, possibly mismatched to the tool-call message) has **never been
validated against the real Mistral API**. It may degrade or 400 on turn 2 of a
tool-using Mistral conversation.

### Fix
Preferred: store reasoning per assistant-message rather than one per `Turn` — a
trace for the tool-call round and a (possibly different / absent) trace for the
final answer — and re-attach each to its own message in both
`Thread::messages()` and `rebuild_chat_messages_from_db`. This likely means
carrying reasoning on `TurnToolCall`/the tool-call record and separately on the
final response, plus a second DB column or an enriched `tool_calls` JSON field.

Minimum acceptable: document the single-slot approximation inline at both
re-attach sites, and add a **live** tool-turn multi-turn test (a prompt that
forces a tool call on turn 1, then a follow-up on turn 2) asserting no turn-2
failure, before relying on Mistral multi-turn tool use.

### Acceptance
- Either: a tool-call assistant message and the final assistant message carry
  their own respective reasoning (no cross-stamping), with caller-level tests in
  `session::tests` and `thread_ops::tests` asserting the distinct values; **or**
- the approximation is documented and a live tool-turn multi-turn test exists and
  passes against the real API.

---

## F4 — CTR-1 cross-turn replay does not cover the engine-v2 → v1-DB persistence path

- **Severity:** Low
- **Category:** Coverage / scope confirmation
- **Status:** ☐ open

### Locations
- `src/bridge/router.rs:5217`, `:5473`, `:6003` (`add_conversation_message(cid, "assistant", …)`)
- `src/bridge/router.rs:6134` (`add_conversation_message(cid, "tool_calls", …)`)

### Problem
These engine-v2 sites persist `assistant`/`tool_calls` rows into the **v1
`conversation_messages` table** (comments: "Persist to v1 DB so the history API
renders", "persist v2 tool_calls to v1 DB") with **no reasoning**.
`rebuild_chat_messages_from_db` reads that same table, so under engine v2 the
CTR-1 cross-turn replay is inert — rows hydrate with `reasoning = NULL` and no
ThinkChunk is replayed.

This is **documented** as "engine v2 permanently out of scope" (Reborn replaces
it) and is **not a regression** (NULL → no replay was the pre-CTR-1 behavior). The
risk is silent: if the Mistral deployment actually runs engine v2, the entire
CTR-1 fix does nothing for it, and the only symptom is degraded multi-turn
Mistral quality.

### Fix
No code change required if the Mistral path runs the v1 agent loop. Confirm
`SANDBOX_ENABLED` / engine-v2 enablement is off for the Mistral deployment, and
add a one-line note to `CLAUDE-local.md`'s Mistral status making the v1-only
applicability explicit (so a future engine-v2 switch is a known trigger to
revisit, alongside the already-scoped Reborn WU-CTR4).

### Acceptance
- `CLAUDE-local.md` states CTR-1 cross-turn replay applies to the v1 agent loop
  only, and that enabling engine v2 (or migrating to Reborn) requires the
  WU-CTR4 / Reborn follow-up.

---

## F5 — Replayed reasoning grows context/cost unboundedly on long threads

- **Severity:** Low
- **Category:** Cost / resource bounding
- **Status:** ☐ open

### Locations
- `src/agent/session.rs:589` (and the tool-call branch at `:558`), plus the
  `rebuild_chat_messages_from_db` re-attach sites.

### Problem
Every persisted assistant message now replays its **full** reasoning trace into
every subsequent request (required for Mistral; ignored by OpenAI/Anthropic).
Reasoning rides on the assistant message and is **not** subject to the
tool-result truncation already applied in `Thread::messages()`. On a long thread
with verbose traces this monotonically increases input tokens (and cost) each
turn, with no cap.

### Fix
Decide a bounding policy and apply it at the replay sites: e.g. only replay
reasoning for the last N turns, and/or cap total replayed reasoning bytes,
dropping the oldest first. Keep the most recent turn's trace always (that is the
one Mistral most needs). `log()`/comment any truncation so it is not silent
(`.claude/rules/safety-and-sandbox.md`: bounded resources; no silent caps).

### Acceptance
- Replayed reasoning is bounded (by turn count and/or bytes) with the policy
  documented inline.
- A test asserts that beyond the bound, older turns' reasoning is omitted from
  the rebuilt messages while recent turns retain it.

---

## Per-finding completion log

> Append a dated line when a finding lands. Reference the fix commit by its
> Conventional-Commit subject, not its SHA (carry commits re-hash on rebase —
> see `CLAUDE-local.md`).

- 2026-06-25 — **F1 fixed** in `fix(llm): unify MISTRAL_REASONING invalid-input
  handling across both resolution paths`. Extracted shared
  `resolve_mistral_reasoning_from_env` helper in `ironclaw_llm` (fail-open: warn
  + default to `High`); both `apply_registry_provider_env` (was fail-closed) and
  the v1 `resolve_registry_provider` now call it. Added caller-level regression
  test `mistral_reasoning_invalid_warns_and_defaults_to_high_on_registry_path`
  driving `apply_registry_provider_env`. Gating keys left untouched (F2 scope).
- 2026-06-25 — **F2 fixed** in `fix(llm): gate v1 Mistral reasoning on protocol,
  not id string`. Changed the v1 `resolve_registry_provider` gate from
  `canonical_id == "mistral"` to `protocol == ProviderProtocol::Mistral`, so a
  Mistral-protocol provider under a non-`"mistral"` id (custom overlay, rename,
  alias) is no longer silently left reasoning-off — matching the protocol-keyed
  `apply_registry_provider_env`, factory dispatch, and overlay migration. Added
  caller-level regression test `mistral_reasoning_gates_on_protocol_not_id_string`
  driving `resolve_registry_provider` with a custom-id Mistral `ProviderDefinition`.
