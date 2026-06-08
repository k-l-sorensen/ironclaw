# Hooks PR stack — status

The Reborn hooks framework (third-party extension hook activation, durable predicate backends, security audit, production activation) is review-gated, not blocked on CI. The foundation has merged; the remaining open PRs have now had all review feedback addressed and are green.

## TL;DR

- 9 open hooks PRs. All CI-green (modulo one shared pre-existing failure, below). Nothing is actually broken.
- All review feedback (serrrfirat's structural reviews + gemini/codex) has been addressed, replied to, and reviews re-requested.
- Notable: most "blockers" were stale verdicts — the code had already been fixed in later commits; this pass verified correctness, filled the genuine remaining gaps, and replied to re-trigger review.
- The one cross-cutting red (perma-failing production-wiring tests) is tracked and already being fixed by a separate PR.

## Open PRs

| PR | Area | Status |
|----|------|--------|
| [#3931](https://github.com/nearai/ironclaw/pull/3931) | event-triggered security fixes (atomic head_cursor, cross-tenant/replay, lifecycle-owner) | addressed + re-requested |
| [#3938](https://github.com/nearai/ironclaw/pull/3938) | activate hook framework in production | addressed + re-requested |
| [#3922](https://github.com/nearai/ironclaw/pull/3922) | SecurityAuditSink wiring | addressed + re-requested |
| [#3933](https://github.com/nearai/ironclaw/pull/3933) | Postgres predicate-state backend | addressed + re-requested |
| [#3936](https://github.com/nearai/ironclaw/pull/3936) | libSQL predicate-state backend | addressed + replied |
| [#3937](https://github.com/nearai/ironclaw/pull/3937) | cross-backend adversarial parity suite | addressed + re-requested |
| [#3941](https://github.com/nearai/ironclaw/pull/3941) | merged-PR maintainability cleanup | addressed + replied |
| [#3928](https://github.com/nearai/ironclaw/pull/3928) | arguments_digest test-through-caller | addressed + replied |
| [#3951](https://github.com/nearai/ironclaw/pull/3951) | third-party extension hook activation | rebased to MERGEABLE + addressed + re-requested |

## What was actually fixed (genuine gaps)

- **[#3931](https://github.com/nearai/ironclaw/pull/3931)** — `head_cursor` made a required trait op with atomic O(1) overrides per backend (no more non-atomic draining default that could fold concurrent appends into replay); added the missing atomicity contract test. Lifecycle-owner resolution extracted out of the 4.3k-line dispatcher into a table-tested resolver.
- **[#3951](https://github.com/nearai/ironclaw/pull/3951)** — rebased to clear the conflict (now MERGEABLE); install-time quarantine audits now carry the real tenant id (were synthetic); `hooks.rs` (1.7k lines) decomposed into focused modules. Hook boundary already least-privilege (`HookProjectionRegistry`, not the full extension registry) and discovery already per-extension quarantine — verified.
- **[#3938](https://github.com/nearai/ironclaw/pull/3938)** — made the predicate-state boundary explicit: tenant-scoped, deliberately shared across runs (rate/value-cap counters are keyed by tenant, not run), pinned by a test that proves a second run sees the first run's count and is denied.
- **[#3922](https://github.com/nearai/ironclaw/pull/3922)** — audit events now carry the full dimension set including `scope`; distinct block causes stay distinct (no collapse to a generic deny event).
- **[#3936](https://github.com/nearai/ironclaw/pull/3936)** — LRU-eviction query switched to an index-served ordering. Declined a bot suggestion to switch a PRAGMA to `execute` (it breaks this libSQL build — that driver echoes the PRAGMA value back as a row); guarded with a comment so it isn't "fixed" again.
- **[#3941](https://github.com/nearai/ironclaw/pull/3941)** — fixed a real test bug: a cross-scope isolation contract seeded both tenants at the same relative path, so the leak marker could collapse during result fusion and the leak check could pass vacuously. Distinct paths now keep it meaningful.
- **[#3937](https://github.com/nearai/ironclaw/pull/3937)** — Postgres parity setup is now fail-loud (a broken CI DB can no longer silently skip); the LRU-eviction race now runs on Postgres too (was libSQL-only). Identity-hashing and the record state machine were already deduped across backends.
- **[#3933](https://github.com/nearai/ironclaw/pull/3933)** / **[#3928](https://github.com/nearai/ironclaw/pull/3928)** — review blockers verified already-fixed; added cleanups (composite index, u64 length prefix) and strengthened the batch-digest caller test.

## Cross-cutting

- **[#4085](https://github.com/nearai/ironclaw/issues/4085)** — there is a pre-existing perma-failing set of `ironclaw_reborn_composition` tests (`RuntimeProcessPort: Missing` + HTTP-egress mapping) that fail on `reborn-integration` independent of any feature work, so every PR touching that crate inherits the red. Root cause: the production host-runtime builders use the filesystem process services but never wire a `TenantSandboxProcessPort` that the production policy requires. Filed as #4085, and already being fixed by **[#3887](https://github.com/nearai/ironclaw/pull/3887)** (routes the production builders through the factory + wires the port). Recommend closing #4085 when #3887 lands.

## Asks

- Re-reviews requested from @serrrfirat on the structural PRs (#3931, #3938, #3922, #3933, #3937, #3951) — most of the prior change-requests were already satisfied; they just need a fresh look.
- #3887 is the unblocker for the perma-red composition tests — landing it makes the whole hooks/reborn area green again.
