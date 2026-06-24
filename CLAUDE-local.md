# Local Fork Notes (not upstream)

> This file documents **local-only** modifications to this clone. It is not
> part of upstream `nearai/ironclaw` and exists only in this fork. Anything
> described here is a deliberate local carry, not project canon.

## Situation

This is a personal fork of [`nearai/ironclaw`](https://github.com/nearai/ironclaw).
We have **no affiliation** with the project and do **not** intend to upstream
changes via PR. We run a modified build locally and pull upstream updates
periodically.

Git remotes:

- `upstream` → `https://github.com/nearai/ironclaw.git` (the original project; read-only to us)
- `origin` → our personal GitHub fork (where our branch lives)

## Maintenance workflow

We carry local changes as commits on a branch, rebased onto upstream so git
reapplies them automatically:

```bash
git fetch upstream
git rebase upstream/main                       # replays our carry commits on top
git push --force-with-lease origin <branch>
```

A rebase conflict is the signal to look — it usually means upstream touched the
same code (possibly fixing it themselves, at which point the local commit can be
dropped).

### Commit convention

The repo (and we, for our carry commits) use **Conventional Commits** —
`type(scope): subject` (e.g. `docs(llm): …`, `feat(llm): …`, `fix(reborn): …`) —
with a `Co-Authored-By: Claude …` trailer when a commit was authored with Claude.
Keep planning/docs and implementation in **separate** commits.

## Active local changes

### Mistral reasoning — ARCHITECTURE DONE, build pending — 2026-06-24

We want to use Mistral (largest EU provider) to its fullest, which mandates
`reasoning_effort=high`. We are building this **properly** as a first-class path.

- **Goal:** Mistral `mistral-small`/`mistral-medium` with `reasoning_effort=high`,
  fully supported through IronClaw's agent loop.
- **Knowledge:** see [`docs/mistral-reasoning.md`](docs/mistral-reasoning.md) for
  the complete API findings — Mistral's request/response format and the blocker.
- **Architecture (C4 L3):** see
  [`docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md`](docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md)
  — approved component-level design. Build a custom `ProviderProtocol::Mistral`
  (`crates/ironclaw_llm/src/mistral.rs`, modeled on `nearai_chat.rs`) that owns
  all Mistral traffic; reasoning on-by-default; model-gating via
  `reasoning_models.rs`; vision patterns via `vision_models.rs`; default model →
  `mistral-medium-latest`. Reuses the existing `ChatMessage.reasoning` round-trip.
- **Gating decision RESOLVED:** build, don't upgrade. Verified against latest
  `rig-core` 0.39.0 — its dedicated `mistral` client still models assistant
  `content` as `String` (`completion.rs:71`), so the `reasoning_effort=high`
  array response still fails to deserialize. The `panic!` is gone, but only
  because rig now *silently skips* reasoning on the request side; the receive
  path is still broken. A version bump does **not** fix it.
- **Correction:** `reasoning_effort` is `high`/`none` (boolean-ish), **not**
  the OpenAI `low`/`medium`/`high` scale.

#### Retained artifacts (kept on purpose)

- `docs/mistral-reasoning.md` — knowledge doc (above).
- `scripts/test-mistral-reasoning.sh` — raw Mistral API test (PASS: confirms the
  field is honored).
- `scripts/test-mistral-reasoning-ironclaw.sh` — live end-to-end test via
  `ironclaw -m` (currently FAIL: proves the receive-path blocker; will become the
  acceptance test once the real fix lands).

#### Status

- **Exploratory code REVERTED.** The earlier minimal `reasoning_effort` injection
  (config enum/field, env read, `RigAdapter::with_additional_params`, gating
  helper) was rolled back — it could not work end-to-end and modeled the param
  wrong. Working tree is clean of it; only the docs, the two scripts, and the
  `CLAUDE.md` fork pointer remain.
- **Architecture DONE (2026-06-24).** Build-vs-upgrade gating decision resolved
  (build custom provider — `rig-core` 0.39 still can't parse the array response).
  Approved C4 L3 design recorded in
  `docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md`.
- **Planning committed (2026-06-24).** Commit `74b63b0dc`
  (`docs(llm): add Mistral reasoning provider architecture + research`) carries
  the architecture doc, `docs/mistral-reasoning.md`, this file, and
  `scripts/test-mistral-reasoning.sh`. Docs/research only — no implementation.
- **Implementation IN PROGRESS (uncommitted).** Working tree carries the build
  (`crates/ironclaw_llm/src/mistral.rs` + edits to `lib.rs`, `registry.rs`,
  `config.rs`, `reasoning_models.rs`, `vision_models.rs`, `providers.json`,
  `src/config/llm.rs`, etc.). This must land as a **separate `feat(llm): …`
  commit** (Conventional Commits + `Co-Authored-By: Claude …` trailer), distinct
  from the docs commit above — do not fold it into the planning commit.
- **Next:** finish the implementation per the architecture doc, run the quality
  gate (`cargo fmt` / `clippy` / `test`), then commit it as `feat(llm): …`.

<!-- Add new local changes above this line, newest first. -->
