# Mistral Reasoning + CTR-1 — Code-Quality Review Findings

**Date:** 2026-06-25 · **Reviewer:** thermo-nuclear code-quality pass · **Branch:**
`mistral-reasoning-fix` (vs `main`) · **Status:** findings open, to be fixed
**one at a time**.

> Companion to the paranoid-architect correctness pass
> (`docs/plans/2026-06-25-mistral-reasoning-review-findings.md`, F1–F5). That doc
> covers correctness/consistency; this one covers **implementation quality,
> abstraction, spaghetti growth, and decomposition** — the structural health of
> the diff, not its behavior. Cite, don't restate:
> `docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md`,
> `docs/plans/2026-06-24-mistral-reasoning-impl.md`, `CLAUDE-local.md`.

## How to use this document

Each finding carries: location, the problem, the evidence, a concrete fix, and
acceptance criteria. Fix them one at a time; check the box and append a dated
line to the per-finding log when done. `Q1` and `Q3` share a lever (the snapshot
helper that shrinks `thread_ops.rs`) and are cheapest together, but each stands
alone.

Overall assessment: **high-quality, well-tested, well-documented work.** The
provider (`crates/ironclaw_llm/src/mistral.rs`, 736 lines, single new module) is
clean and single-purpose; the wire model is strongly typed; serde fails loud on
unknown chunk types; the dual-backend migration is complete (V31 + libSQL #26 +
inline DDL + `IDEMPOTENT_ADD_COLUMN_MIGRATIONS` entry); leak scanning reuses the
canonical `LeakDetector`; tests drive real callers, not just helpers. These
findings are structural refinements, concentrated in the CTR-1 persistence layer.
`Q1` and `Q2` are the two I'd treat as merge-blockers.

---

## Q1 — The turn-snapshot tuple is hand-destructured 8× in `thread_ops.rs`, and this PR widened every one

- **Severity:** Medium
- **Category:** Spaghetti growth / missing helper / type-contract
- **Status:** ☑ done

### Locations
- `src/agent/thread_ops.rs` — 8 sites: the `tool_call_reasoning.clone()` lines at
  `:888`, `:999`, `:1032`, `:1069`, `:2487`, `:2584`, `:2616`, `:2657` each sit
  inside a `thread.turns.last().map(|t| (…)).unwrap_or_default()` block.

### Problem
This exact shape now appears **8 times**, in two arities (4-tuple and 5-tuple):

```rust
let (turn_number, tool_calls, narrative, reasoning, tool_call_reasoning) = thread
    .turns.last()
    .map(|t| (t.turn_number, t.tool_calls.clone(), t.narrative.clone(),
              t.reasoning.clone(), t.tool_call_reasoning.clone()))
    .unwrap_or_default();
```

The PR *widened every one of these* (3-tuple → 4/5-tuple). That is copy-pasted
logic where a helper belongs, inserted into an already-busy file. It is worse
than ordinary duplication because the two new positional fields — `reasoning`
and `tool_call_reasoning` — are **both `Option<String>` sitting adjacent in the
tuple**. Swapping them at any one call site is a silent cross-stamp bug the
compiler cannot catch — i.e. exactly the bug class F3 was introduced to fix
(`.claude/rules/types.md`: "two values with the same shape but different meanings
must be different types"). The fix re-creates the footgun at every persist site,
and the 4-vs-5 arity split means future fields land inconsistently.

### Evidence
8 near-identical blocks; two arities (the AuthPending / tool-only arms omit
`reasoning` and destructure a 4-tuple, the success arms a 5-tuple). `grep -n
"tool_call_reasoning.clone()" src/agent/thread_ops.rs` → 8 hits.

### Fix
Add one method returning a named struct and have `PersistToolCallsInput` carry
it (code-judo: collapses 8 blocks into 8 one-liners, deletes the arity split,
makes every field access compiler-checked instead of positional):

```rust
#[derive(Default)]
struct TurnPersistSnapshot {
    turn_number: u32,
    tool_calls: Vec<TurnToolCall>,
    narrative: Option<String>,
    reasoning: Option<String>,            // final answer's trace
    tool_call_reasoning: Option<String>,  // tool-call round's trace
}
impl Thread { fn last_turn_snapshot(&self) -> TurnPersistSnapshot { … } }
```

This also *reduces* the production footprint of a file that is over-budget (see
`Q3`), which is the right direction.

### Acceptance
- The `thread.turns.last().map(|t| (…tuple…)).unwrap_or_default()` pattern appears
  **once** (inside the helper), not 8×.
- `reasoning` / `tool_call_reasoning` are accessed by field name, not tuple
  position, at every persist site.
- No behavior change; existing CTR-1 tests still pass.

---

## Q2 — `MistralReasoningEffort::None` is built but never reachable; the doc promises a three-state contract the wiring delivers as one

- **Severity:** Medium
- **Category:** Boundary / abstraction not earning its keep / stale contract
- **Status:** ☐ open

### Locations
- `crates/ironclaw_llm/src/config.rs` — `enum MistralReasoningEffort { High, None }`,
  `wire_value`, `FromStr`, `Display`, and `resolve_mistral_reasoning_from_env`.
- `crates/ironclaw_llm/src/mistral.rs:113` — `reasoning_effort_for` renders
  `wire_value()`.

### Problem
The `config.rs` doc block documents three live wire states:

- `Option::None` → omit the param
- `Some(High)` → `"high"`
- `Some(None)` → send explicit `"none"`

But **nothing ever constructs `Some(MistralReasoningEffort::None)`.**
`resolve_mistral_reasoning_from_env` maps the parsed `None` variant straight to
`Option::None` (omit), and that resolver is the *only* producer of
`RegistryProviderConfig.mistral_reasoning`. Consequences:

- The `None` variant exists only transiently inside `FromStr`, immediately
  collapsed to omit.
- `wire_value() == "none"`, the `Display` impl, and the `"none"` rendering in
  `reasoning_effort_for` are **dead in production**.
- The doc comment promises a capability (explicit `"none"`) the wiring never
  delivers — a stale-contract bug waiting to mislead the next maintainer. In
  practice the enum only ever carries `High`.

### Evidence
`resolve_mistral_reasoning_from_env`:
```rust
Ok(MistralReasoningEffort::None) => None,  // collapses Some(None) → omit
```
No call site stores `Some(MistralReasoningEffort::None)`.

### Fix
Pick one and make the contract match the code:

- **(a)** If explicit `"none"` is a real requirement → the resolver has a bug:
  `"off"`/`"none"` should yield `Some(None)`, not `Option::None`. Update the C2
  contract note accordingly.
- **(b)** If it is not (architecture C2 says "off → omit") → delete the `None`
  variant and the `wire_value`/`Display`/`"none"` machinery, represent the toggle
  by the `Option`'s presence (or a one-field marker), and drop the contradictory
  doc block. ~40 lines and one confusing tri-state go away.

Recommended: **(b)** — it matches the architecture's stated C2 contract and the
actual behavior. Reserve a richer enum for if/when Mistral adds graded effort.

### Acceptance
- The wire-state contract documented in `config.rs` is the one the resolver
  actually produces (no state described that no code path yields).
- No dead variant / `wire_value` arm that production cannot reach (or a test that
  exercises the explicit-`"none"` path end-to-end if (a) is chosen).

---

## Q3 — `thread_ops.rs` is 5,725 lines; this PR adds production lines without decomposition

- **Severity:** Low
- **Category:** File size / decomposition
- **Status:** ☐ open

### Locations
- `src/agent/thread_ops.rs` (5,725 lines).

### Problem
The file is well past `.claude/rules/architecture.md`'s 3,000-line "file a
tracking issue for decomposition" threshold, and PRs adding >200 lines to such
files are asked to carry an inline justification. This PR threads reasoning
through ~8 scattered persist sites plus tests, growing the file rather than
shrinking it.

### Fix
No full decomposition required in this PR, but: land `Q1`'s snapshot helper so
the *production* side of the file ends up **smaller**, not larger, and add a
one-line tracking note (issue/plan reference) acknowledging the file is
over-budget per `architecture.md`. `src/agent/session.rs` (2,259) is also over
the 1,500 soft line; its additions here are small and cohesive (two documented
`Turn` fields) — no action beyond not feeding it further.

### Acceptance
- Net production-line delta in `thread_ops.rs` from this PR is ≤ 0 after `Q1`
  (tests excluded), or an inline `// arch-exempt: large_file, …, plan #NNNN`
  justification is present.

---

## Q4 — Minor cleanups (non-blocking)

- **Severity:** Low
- **Category:** Wrapper/abstraction polish
- **Status:** ☐ open

### Items
1. **Trait method duplication.** `ConversationStore` now requires both
   `add_conversation_message` and `add_conversation_message_with_reasoning`, and
   each backend (`src/db/libsql/conversations.rs`, `src/db/postgres.rs`,
   `src/history/store.rs`) hand-writes the `..(.., None)` delegation. Make
   `add_conversation_message` a **provided default** on the trait
   (`{ self.add_conversation_message_with_reasoning(.., None).await }`) so the
   shim lives once, not per-impl (`.claude/rules/review-discipline.md`:
   decorator/wrapper trait delegation — fewer hand-copied delegations to drift).
2. **Two redaction call sites, not one choke point.** In
   `crates/ironclaw_llm/src/reasoning.rs`, `respond_with_tools` redacts after
   `complete_with_tools`, while the no-tools branch redacts separately, with
   comments asserting which path scanned what. It is correct (and `redact_all`
   is idempotent, so even a double-scan is safe), but the "scanned exactly once
   before leaving the engine" invariant is maintained by prose rather than
   structure. If a single exit point is feasible, prefer it.
3. **Third lock-and-mutate-last-turn block.** `dispatcher.rs`
   `handle_text_response` opens its own `session.lock() → get_mut →
   last_turn_mut → set reasoning` block (`:840`), mirroring the tool-call handler
   (`:945`). Consistent with existing style; a small `with_last_turn(|t| …)`
   helper on the delegate would dedupe the sites. Optional.

### Acceptance
- (1) is the only one worth gating on; (2)/(3) are reviewer's-discretion.

---

## What is right — do not change

- `RespondResult::Text(String)` → `Text { text, reasoning }` is the correct call
  (named field, not a second tuple slot).
- The **two** reasoning slots on `Turn` (`reasoning` vs `tool_call_reasoning`)
  are *not* over-engineering: given the existing `Turn` model stores `tool_calls`
  and `response` separately, distinct ThinkChunks per assistant message is the
  inevitable shape, and it is exactly F3's fix. Affirmed.
- Protocol-keyed gating (F2), value-scoped overlay migration, fail-open invalid
  input (F1), and leak-scan-before-replay ordering are all correct and tested.

---

## Per-finding completion log

> Append a dated line when a finding lands. Reference the fix commit by its
> Conventional-Commit subject, not its SHA (carry commits re-hash on rebase —
> see `CLAUDE-local.md`).

- 2026-06-26 — **Q1 done**: `refactor(agent): replace hand-destructured turn
  snapshot tuple with named TurnPersistSnapshot`. Added
  `TurnPersistSnapshot` + `Thread::last_turn_snapshot()` in
  `src/agent/session.rs`; collapsed all 8 destructuring blocks in
  `thread_ops.rs` to `let snapshot = thread.last_turn_snapshot();` with by-name
  field access (−91 production lines, also satisfies Q3 for this PR). Two
  regression tests added (empty-thread default; distinct reasoning slots). Pure
  refactor — full agent unit suite (520 tests incl. CTR-1) green, zero clippy.
