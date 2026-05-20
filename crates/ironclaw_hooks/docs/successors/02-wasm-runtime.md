# Successor PR: WASM hook execution path

> Successor work from PR #3573. The manifest schema already accepts
> `HookManifestBody::Wasm`; the registrar rejects it with
> `RegistryConstruction` for now. This PR makes WASM hook bodies
> executable inside a `wasmtime` sandbox.

## Scope

Add a programmatic-hook execution path for Installed-tier hooks. The
WASM module exports a single function — by convention named
`evaluate`, but the actual export name is whatever the manifest
declares in `HookManifestBody::Wasm.export` — and the dispatcher
invokes it inside a sandbox. The signature is intentionally simple:
`(): ()`. Hook context flows in through the `ic:hooks/context@1` host
imports (`ctx_size` / `ctx_read`); decisions, patches, and observer
facts flow out through the per-point host imports
(`ic:hooks/before-capability@1`, `ic:hooks/before-prompt@1`,
`ic:hooks/observer@1`).

This is deliberately distinct from `WitToolRuntime`, which hard-codes
a richer WIT-bindgen-derived interface for the tool surface; the
hook runtime uses a hand-rolled `wasmtime::Linker` per the design
discussion below so the host-import surface area stays explicit and
auditable.

## What lands in this PR

1. **`crates/ironclaw_hooks/src/wasm/` module** with:
   - `WasmHookRuntime`: wasmtime store + instance per hook invocation.
   - Host imports: typed shims for `RestrictedGateSink` / `RestrictedMutatorSink`
     / observer sink.
   - Budget enforcement via `wasmtime` fuel + memory + wall-clock timeout
     (manifest's `WasmBudget` already declares all three).
2. **`HookManifestBody::Wasm`** routes through the new runtime. The
   registrar's current "WASM not implemented" rejection is removed.
3. **WASM-aware `HookId::derive` input**. Reuse the existing
   `HookId::derive(extension_id, extension_version, hook_local_id,
   hook_version)` constructor, but include the compiled module digest in
   the version material the registrar passes to it. Do not add a
   separate `for_wasm` constructor unless the identity contract changes.
4. **New threat model**: `crates/ironclaw_hooks/docs/threat-model-wasm.md`
   covering the wasmtime boundary, host-import surface, time/memory
   exhaustion, side-channels.

## What this PR does NOT do

- Module signing / supply-chain checks. Those belong in the extension
  installer (separate slice).
- Caching compiled modules across hosts (perf optimization, follow-up).
- Self-authored WASM hooks (governance separate, tracked at #3567).

## Threat-model deltas

The current `threat-model.md` says WASM execution is out of scope. This
PR brings it in scope; the new threat-model-wasm.md must cover:

- **Host-import surface**: each exported host fn is an attack channel.
  Enumerate. Restrict to typed sinks — no ambient access to
  filesystem, network, system time, RNG.
- **Fuel exhaustion**: trap → FailIsolated for observers, FailClosed
  for gates (existing failure_policy matrix already covers this).
  Terminology: `FailIsolated` / `FailClosed` are
  `FailureDisposition` values (`crate::failure_policy`) that the
  dispatcher derives from a `FailureCategory` (Panic / Timeout /
  Malformed / etc.) crossed with the binding's
  `HookTrustClass`. They are *not* `HookFailureMode` variants —
  `HookFailureMode::{FailOpen, FailClosed}` is the older
  installed-hook policy switch used elsewhere in the crate and
  applies to predicates, not the WASM dispatch path. The WASM
  runtime never produces `FailOpen`; gates fail closed, observers
  fail isolated.
- **Memory exhaustion**: `memory_mb` cap enforced via the tool-WASM
  `WasmResourceLimiter` resource limiter.
- **Wall-clock exhaustion**: `wall_ms` cap enforced via
  `tokio::time::timeout` wrapped around `tokio::task::spawn_blocking`
  on the host side, plus epoch-interrupt on the wasmtime side. The
  blocking-pool indirection is load-bearing: a bare
  `tokio::time::timeout` over a synchronous wasmtime call cannot cancel
  the call (wasmtime runs on the awaiting task), so the timeout would
  never fire and a wedged module would pin the dispatcher's caller.
  The wasmtime epoch interrupt is the authoritative in-WASM cancel
  signal; the outer timeout governs *when the dispatcher's future
  resolves* if a blocking-pool worker is still spinning when the
  deadline passes.
- **Module substitution**: `HookId` content-addressing must include the
  module bytes (or a digest of them).
- **Side channels**: WASM doesn't get access to ambient time, RNG, or
  syscalls — but constant-time predicates still leak via execution
  time. Acknowledge residual.

## Required tests

1. **Happy path**: Installed-tier WASM hook denies → outcome is
   `Denied` with the predicate's reason.
2. **Fuel exhaustion**: WASM loops forever → trap → FailClosed for gate
   point (Denied), FailIsolated for observer point (no outer impact).
3. **Module substitution**: same `HookLocalId`, different module bytes
   → different `HookId` → checkpoint replay refuses with
   `UnknownHook` / `unknown_hook_id_at_replay`, not a registry collision.
4. **Host-import surface negative**: WASM tries to call an undeclared
   import → link error at instantiation → fail closed.
5. **Memory cap**: WASM tries to grow past `memory_mb` → trap → fail
   closed.

## Required design discussion before implementation

- **Module loading**: where does the WASM blob live? Extension registry
  has bytes; hook framework needs a `WasmModuleRef` resolver. Likely
  reuses the existing tool-WASM loader path.
- **Wit-bindgen vs hand-rolled ABI**: pick one. Tool subsystem already
  uses one; lean toward that for consistency.
- **Per-build vs per-invocation runtime**: per-build (one `Store` per
  hook lifetime, recycled across invocations) is cheaper; per-invocation
  is safer (no state leakage). Bias toward per-invocation for v1 with
  a fast-path optimization later.

## Risk

- Large. Wasmtime integration touches the existing tool-WASM stack;
  any shared host-import infrastructure has to be hardened.
- Requires a separate threat-model artifact (drafted in this PR).
- May surface design questions about which existing primitives can be
  reused vs duplicated.

## Effort

Large. Plan for at least one design-review iteration before
implementation lands.

## Codex review addenda (2026-05-14)

Codex's design-review pass surfaced two scope-shaping issues and one
recommendation.

Human design ack for the implementation PR:

1. **Module loading**: reuse `ironclaw_wasm`'s existing tool-WASM
   loader primitives where the hook crate needs a wasmtime boundary:
   cache compiled modules by byte digest, fetch bytes through a
   resolver handoff, and compile with wasmtime. Do not build a second
   extension-loader stack in `ironclaw_hooks/`.
2. **Runtime lifetime**: v1 uses a fresh `wasmtime::Store` for every
   hook invocation. This lines up with the existing FU8 fresh-dispatcher
   per-build pattern; cross-invocation store reuse is future work.
3. **ABI**: use `wasmtime::Linker`, not `wit-bindgen`, for parity with
   the current tool-WASM runtime. Revisit only when tool-WASM migrates.
4. **Host-import budgets**: enforce at the sink shim:
   `max_sink_calls_per_invocation = 64`,
   `max_total_patch_bytes = 4 * 1024`,
   `max_observer_facts_per_invocation = 32`, and
   `max_decision_calls_per_invocation = 1` (second decision call =
   `FailureCategory::Malformed`).

### Critical: host-import calls + outputs need budgets

The original scope budgets Wasm fuel/memory/wall-time but says nothing
about host-side sink call counts, emitted facts, patch bytes, or
decision attempts. A module that respects the Wasm-side budgets can
still exhaust host memory by issuing thousands of small import calls
(each appending a small string to a sink-internal `Vec`, for instance).

**Mitigation**: the host-import shims must enforce per-invocation
budgets:

- `max_sink_calls_per_invocation` (default ~64)
- `max_total_patch_bytes` (default the existing 4 KiB envelope budget)
- `max_observer_facts_per_invocation` (default ~32)
- `max_decision_calls_per_invocation` (1; second call = protocol violation)

Each budget overflow trips a `FailureCategory::Malformed` and the
slot poisons per the existing failure-policy matrix. Negative tests
in the implementation PR will fire each cap explicitly.

### Critical: module-substitution test claim was internally inconsistent

The original scope said "same `HookLocalId`, different module bytes →
different `HookId` → **registry rejects on `HookId` collision check**."
Those two clauses can't both be true: different `HookId`s by
definition aren't a collision.

**Corrected statement**: a different module produces a different
`HookId`, so the resume / checkpoint replay path refuses with a
**checkpoint-hook-id-mismatch** error (the existing pinned-id replay
guard), not a registry-side collision. The test should:

1. Install hook with module A → `HookId_A`.
2. Persist a checkpoint referencing `HookId_A`.
3. Re-install the same `HookLocalId` with module B → `HookId_B`.
4. Replay against the checkpoint → refused with
   `unknown_hook_id_at_replay` (or equivalent).

### Recommendation: ABI choice should land in the draft

The original scope says "wit-bindgen vs hand-rolled ABI: pick one"
under design discussion. Codex was right that this is implementation-
shaping — the budgets above depend on which crate intercepts the host
imports, and the existing tool-WASM stack already chose one path.

**Decision for this PR**: follow `ironclaw_wasm`'s existing pattern
(linked via `wasmtime::Linker`) for parity. The implementation PR
documents this choice and reuses tool-WASM's memory/fuel sandbox
primitives where applicable; hook-specific sink budgets live in the
hook host-import shim.
