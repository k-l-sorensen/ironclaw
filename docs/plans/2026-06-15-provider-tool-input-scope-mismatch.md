# Fusion Design: Fix provider-tool-call input `ScopeMismatch` on auth-gate resume

**Status:** ACCEPTED. Both council slots signed off with no blockers (Opus 4.8 `ACCEPT_WITH_NONBLOCKING_NOTES`, GPT-5.5 xhigh `ACCEPT_WITH_NONBLOCKING_NOTES`). Ready to implement.

## 0. Implementation notes (from signoff — read before coding)

- **Use `is_provider_call: bool`, not `provider_replay`, on `CapabilityInvocation`** — the 1284 guard needs only the origin bit; a bool keeps the type minimal and avoids a second source of truth (the host port already gets replay via the candidate path).
- **`CapabilityInvocation` derives `Serialize`/`Deserialize` (`host.rs:1380-1394`)** — add the new field with `#[serde(default)]` so persisted/in-flight payloads deserialize as `false` (forward-compat).
- **Update EVERY `CapabilityInvocation { .. }` constructor** — the two builders in `capability_helpers.rs` *and* all test constructors. A non-exhaustive literal defaulting `is_provider_call=false` would silently regress the InvalidInput-vs-host-error classification. (Make it a required field, not `#[serde(default)]`-only-on-the-struct, so the compiler flags every site.)
- **"Durable" here = survives a per-port surface refresh within one `serve` process, NOT cross-process persistence.** Both stores are process-local in-memory (`local_dev.rs:168-172`, `product_live_adapters.rs:97-103`). That's sufficient for resume within a run; restart-recovery is the separate out-of-scope concern.
- **Product-live consume-on-read subtlety:** `ProductLiveCapabilityIo::resolve_capability_input` does a `remove` (consume-on-read); local-dev does not. First dispatch returns `AuthRequired` *before* consuming input, so the parked ref is still present at resume — but the auth-resume path should prefer **re-registering via `provider_replay`** rather than relying on the parked ref surviving on product-live. Verify with a product-live-specific test.
- **`register_then_resolve_survives_surface_refresh` must resolve against a SECOND port instance** sharing the same `Arc<dyn LoopCapabilityInputResolver>` (simulating `for_run_context` rebuild) — testing within one port would pass even under the old broken design.
- **Production-wiring guard test must exercise the full adapter/factory path** (`product_live_adapters.rs:716-720, 785-790`), not just a direct `ProductLiveCapabilityIo` helper call, to catch wiring-layer regressions.
- **Add a tracking note** (issue or doc line) for durable-product migration of any pre-fix `BlockedAuth` checkpoints carrying the old `input:provider-tool-<hash>` ref — out of scope here, but don't lose it.

## 1. Problem statement

Running `ironclaw-reborn serve`, a first-party provider capability that requires OAuth (e.g. `gmail.list_messages`) blocks on an auth gate, and **after OAuth completes / on resume** the run terminally fails:

```
WARN ironclaw_agent_loop::executor::mapping: capability host error mapped to HostUnavailable
     kind="scope_mismatch" safe_summary="capability input ref is not scoped to this loop run"
```

Diagnostic (temporary, in `ensure_local_dev_ref_scope`) proved:
```
current_run_id=4c070440-…  reference=input:provider-tool-f53f0cb8…  expected_prefix=input:4c070440-…:
```
The capability `input_ref` is `input:provider-tool-<sha256>` but the durable local-dev store only accepts `input:<run_id>:`. The `run_id` is **stable** across block→resume (not a cross-run/retry issue). The failed run is retryable, so it can loop.

## 2. Confirmed root cause

There are **two input stores with incompatible ref formats**, and the auth/resume path crosses between them:

1. **Loop side** (`crates/ironclaw_loop_support/src/capability_port.rs`): `HostRuntimeLoopCapabilityPort::new` wraps the injected resolver in `ProviderToolCallInputResolver` (`:622-623`, struct `:79-82`). That wrapper mints `input:provider-tool-<sha256>` (`PROVIDER_TOOL_CALL_INPUT_REF_PREFIX` `:56`; `provider_tool_call_input_ref` `:1897`, whose digest payload includes `run_id` but is opaque to a prefix check) and stages the input JSON in its **own in-memory `provider_inputs: Mutex<HashMap>`** (`:95-117`, `:119-141`).
2. **Durable side** (`crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`): `LocalDevCapabilityIo` is the wired durable resolver (`:86-92`), mints `input:<run_id>:<uuid>` (`:408-430`), and on read runs `ensure_local_dev_ref_scope` (`:778-808`) which accepts only the `input:<run_id>:` prefix. `ProductLiveCapabilityIo` (`product_live_adapters.rs:186, 209-222, 408-430`) uses the same convention.

**Why the in-memory map never covers the read:** the `provider_inputs` map lives *inside* `HostRuntimeLoopCapabilityPort`, and `RefreshingLocalDevCapabilityPort` rebuilds a **fresh, empty** port on every `visible_capabilities()` call (`refreshing_capability_port.rs:115, 164-172, 217-223` → `factory.for_run_context`). The executor refreshes the surface (`executor/prompt.rs:235`) immediately *before* building the resume candidate (`:260-263`). So for any flow where staging and reading are separated by a surface refresh — which **resume always is** — the map is empty and the read falls through to the durable `LocalDevCapabilityIo`, which rejects the `input:provider-tool-<hash>` format → `ScopeMismatch` → `HostUnavailable{Capability}` → terminal failure.

**Why it surfaces only after auth:** a first-party capability checks auth *first* and returns `auth_required` **before reading its staged input** on first dispatch (so the broken read never runs → `BlockedAuth`). After OAuth/resume the handler proceeds, reads its input, and the read lands in the durable store with the foreign ref. The first-party host runtime receives input **already materialized by value** in `RuntimeCapabilityRequest` (`ironclaw_host_runtime` `first_party.rs:20-34`), so the failing read is the **loop-port resolve site**, not a host read path.

Net: the loop side invents a second input-ref format and a non-durable store; the durable store (the only one that survives a refresh/resume, and the one the first-party path actually reads through) neither has the input nor accepts the format.

## 3. Goals / non-goals

**Goals:** one input-ref format and one durable store for provider-tool inputs; auth-gate resume of a first-party capability reads its input and completes; keep run-scoping strict (no cross-run leakage); local_dev and product_live stay consistent; surgical change.

**Non-goals:** changing the auth-gate/resume protocol (#4746/#4839 invocation-identity behavior stays); migrating already-blocked pre-fix checkpoints that carry the old hash ref (separate concern; non-issue for local-dev `serve` since the map is in-memory and a restart clears it).

## 4. Constraints

- `ironclaw_agent_loop` must not depend on safety/host/DB crates; `ironclaw_loop_support` is adapter glue; `ironclaw_reborn_composition` owns local-dev composition.
- No `unwrap`/`expect` in production code.
- Remove the temporary diagnostic `tracing::warn!` in `ensure_local_dev_ref_scope`.

## 5. Final design (fix = "unify on the durable store", DELETE shape)

**Owning layer:** `ironclaw_loop_support::capability_port` — the single layer that invents the divergent format and store.

### 5.1 Delete the second format + store
- Delete `struct ProviderToolCallInputResolver` + its impls (`capability_port.rs:79-143`).
- Delete `PROVIDER_TOOL_CALL_INPUT_REF_PREFIX` (`:56`), `provider_tool_call_input_ref` (`:1897-1937`), `is_provider_tool_call_input_ref` (`:1939-1943`).
- `HostRuntimeLoopCapabilityPort::new`: stop wrapping; hold the injected durable resolver directly (remove the wrap at `:622-623`).
- Provider registration now flows through the durable resolver's `register_provider_tool_call_input` (`LocalDevCapabilityIo` `local_dev.rs:408-430`; `ProductLiveCapabilityIo` `product_live_adapters.rs:209-222`), minting `input:<run_id>:<uuid>` and staging durably.
- The `LoopCapabilityInputResolver` trait **already** has an erroring default `register_provider_tool_call_input` (`capability_port.rs:71-76`) that returns a loud "not supported" — this is the correct behavior for any resolver that genuinely can't stage. **No `supports_*` flag, no in-memory fallback** (every production resolver supports durable registration — see §6).

### 5.2 Re-source the schema-invalid-provider-args guard (required)
The guard at `capability_port.rs:1284-1285` converts schema-invalid arguments of a **model-minted provider call** into a model-visible `CapabilityOutcome::Failed{InvalidInput}` (retryable by the model) instead of a terminal host `Err`. It currently keys on `is_provider_tool_call_input_ref(effective_input_ref)`. After unification the ref is `input:<run_id>:<uuid>`, so that predicate would stop firing — a real regression on the common path.

`CapabilityInvocation` does **not** currently carry the provider-origin signal (`host.rs:1381-1394`), and the executor drops candidate `provider_replay` when building invocations (`capability_helpers.rs:30-40`). So we must **plumb** it:
- Add a provider-origin field to `CapabilityInvocation` — `is_provider_call: bool` (preferred: minimal, no payload) **or** carry `provider_replay` — sourced from `CapabilityCallCandidate.provider_replay` (`host.rs:1212-1219`).
- Set it in both invocation builders: `capability_invocation_from_candidate` and `capability_invocation_from_auth_resume_candidate` (`capability_helpers.rs:30-72`).
- Replace the `is_provider_tool_call_input_ref(...)` condition at `capability_port.rs:1284-1285` with that field.

### 5.3 Remove diagnostic
- Remove the temporary `tracing::warn!` block in `ensure_local_dev_ref_scope` (`local_dev.rs`); keep the strict prefix check unchanged.

### 5.4 Corrected data flow
- **First dispatch (first-party):** model emits provider call → `register_provider_tool_call` (`:1059-1093`) → durable resolver mints `input:<run_id>:<uuid>`, stages durably → candidate carries that ref + `provider_replay` → auth check returns `auth_required` → `BlockedAuth` (durable input row persists).
- **Resume after OAuth:** surface refresh (harmless now — no per-port input map) → `pending_auth_resume_candidate` (`capability_helpers.rs:87-120`): replay path re-registers → durable store mints a fresh `input:<run_id>:<uuid>`; non-replay path reuses the parked `resume.input_ref` (already run-scoped, still durably staged) → invocation resolves the run-scoped ref from the durable store → handler reads input → completes. **No ScopeMismatch.**

## 6. Key decisions & alternatives rejected

- **(b) Unify on the durable store — CHOSEN.** Collapses to one format + one store; the durable store is the only one that survives a surface refresh/resume and is the one the first-party path reads through.
- **(a) Teach the scope checks to accept `input:provider-tool-<hash>` — REJECTED.** Loosens the only run-scoping guard (the matcher can't verify run-binding inside an opaque digest), spreads the second format into two host crates. Worst on security + sprawl.
- **(c) Reroute first-party reads through the loop resolver — REJECTED.** Misdiagnoses the site: the first-party host receives input already materialized by value (`first_party.rs:20-34`); the failing read is the loop-port resolve.
- **(d) Persist `provider_inputs` across resume — REJECTED.** Keeps the wrong owner and two formats; the bug *is* a second source of truth (violates repo `types.md` / `architecture.md §4`).
- **`supports_*` flag + in-memory fallback (GPT's round-1 shape) — REJECTED.** Production resolver inventory: the only non-test `LoopCapabilityInputResolver` impls are `LocalDevCapabilityIo`, `ProductLiveCapabilityIo` (both durable), and `UnavailableCapabilityIo` (fail-closed stub; never reaches a fallback, `production.rs:39,69-78`). All others are `#[cfg(test)]`. So the fallback is dead code in production and a hazard in tests; the trait's existing erroring default is the correct loud failure. A `supports_*()` that is `true` for every production impl is the type system lying (repo `architecture.md §2`).

## 7. Security / run-scoping

The fix **tightens** scoping: refs become `input:<run_id>:<uuid>` and the durable stores verify the stored `run_id` on read (`product_live_adapters.rs:203-205`; local-dev prefix-checks). The deleted in-memory map did **no** prefix scope check at all. Cross-run isolation is strictly improved. The strict prefix checks in `ensure_local_dev_ref_scope` / `ensure_ref_scoped_to_run` are unchanged.

## 8. Test & validation plan

1. **Reborn binary-E2E (primary): `local_dev_gmail_auth_gate_resume_reads_staged_input`** (`tests/support/reborn` harness + gsuite/auth harness). Drive a first-party OAuth-gated capability via `model_replay` → assert `BlockedAuth` → resolve the gate / resume → assert the resumed invocation **reads its input and completes** (`CapabilityOutcome::Completed`, no `ScopeMismatch`, no `HostUnavailable{Capability}`); assert no scope-diag warning and the temp diagnostic is gone.
2. **Contract: `register_then_resolve_survives_surface_refresh`** (`loop_support`). Register a provider tool-call input → simulate a `visible_capabilities` refresh (new port instance sharing the same `Arc<dyn LoopCapabilityInputResolver>`) → resolve from the durable inner. Locks the property the deleted in-memory map lacked.
3. **Unit: `provider_registration_uses_run_scoped_ref`** for both `LocalDevCapabilityIo` and `ProductLiveCapabilityIo` — returned ref has `input:<run_id>:` prefix and resolves to the staged payload.
4. **Unit: `local_dev_ref_scope_rejects_provider_tool_ref`** — `ensure_local_dev_ref_scope` rejects an `input:provider-tool-*` ref **and** a cross-run `input:<other_run>:<uuid>` ref (cross-run isolation lock). (No WARN — diagnostic removed.)
5. **Unit: schema-invalid provider call → `Failed{InvalidInput}` (not `Err`)** — verifies the `provider_replay`/`is_provider_call`-based replacement of the deleted guard predicate on the durable path.
6. **Production-wiring guard test** — assert no production-wired resolver returns an `input:provider-tool-*` ref (trivially true once the minter is deleted; locks the regression shut).
7. **Remove/replace** the internal wrapper test at `capability_port.rs:3454-3470`.

## 9. Risks & mitigations

- **Guard regression** if §5.2 is skipped → mitigated by the explicit plumbing + test #5.
- **Staging capacity** — provider inputs now use the bounded durable store (`LocalDevCapabilityIo` 1024/4 MiB, fail-closed). Same shape as all other staged inputs; acceptable.
- **Legacy in-flight `BlockedAuth` checkpoints** carrying old hash refs — disappear under DELETE for new runs; non-issue for local-dev `serve` (in-memory, restart clears). Durable-product migration is a separate, explicitly-scoped concern.
- **Tracking note:** durable product deployments should separately migrate, clear, or fail-closed any pre-fix `BlockedAuth` checkpoints that already carry `input:provider-tool-<hash>` refs; that compatibility migration is intentionally out of scope for this fix.

## 10. Agreement ledger

| Decision | Opus 4.8 | GPT-5.5 xhigh |
|---|---|---|
| Root cause (two stores, ephemeral map wiped on refresh) | ✅ (found refresh-wipe detail) | ✅ |
| Fix (b): unify on durable store | ✅ | ✅ |
| DELETE wrapper+format+helpers (vs `supports_*`+fallback) | ✅ (proposed) | ✅ (conceded; fallback is test-only) |
| Reject (a)/(c)/(d) | ✅ | ✅ |
| Guard at 1284 must be re-sourced | ✅ (flagged) | ✅ (conceded) |
| Re-source needs plumbing onto `CapabilityInvocation` (not just `request.provider_replay`) | ✅ (accepts) | ✅ (found: field absent at host.rs:1381-1394, dropped at capability_helpers.rs:30-40) |
| Keep strict scope check, remove diagnostic | ✅ | ✅ |
| Test matrix (E2E + survives-refresh + scope reject + InvalidInput + wiring guard) | ✅ | ✅ |

## 11. Unresolved blockers

None.

## 12. Council trace

Slots: anthropic = Opus 4.8; openai = GPT-5.5 xhigh. Rounds: 1 independent draft + 1 cross-review. Both → `ACCEPT_WITH_CHANGES`, converged on this fused design. Final signoff pending.
