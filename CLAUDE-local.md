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

**Don't cite carry-commit SHAs in committed docs.** Rebasing onto upstream (and
any history rewrite) re-hashes our carry commits, so a pinned SHA goes stale on
the next `git rebase upstream/main`. Reference carry commits by their
Conventional-Commit subject instead.

## Active local changes

### Mistral reasoning — implemented (`feat(llm)` landed), live acceptance pending — 2026-06-24

We want to use Mistral (largest EU provider) to its fullest, which mandates
`reasoning_effort=high`. We built this **properly** as a first-class path: a
custom `ProviderProtocol::Mistral` that owns all Mistral traffic.

- **Goal:** Mistral `mistral-small`/`mistral-medium` with `reasoning_effort=high`,
  fully supported through IronClaw's agent loop.
- **Why custom, not a `rig-core` bump:** verified against `rig-core` 0.39.0 — its
  dedicated `mistral` client still models assistant `content` as `String`, so the
  `reasoning_effort=high` array response still fails to deserialize. A version bump
  does **not** fix it. (`reasoning_effort` is `high`/`none`, boolean-ish — **not**
  the OpenAI `low`/`medium`/`high` scale.)
- **Detail lives in the plan docs — do not restate here:**
  - [`docs/mistral-reasoning.md`](docs/mistral-reasoning.md) — API findings, request/response format, the blocker.
  - [`docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md`](docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md) — approved C4 L3 design (decisions D1–D10, findings F1–F12).
  - [`docs/plans/2026-06-24-mistral-reasoning-impl.md`](docs/plans/2026-06-24-mistral-reasoning-impl.md) — work-unit breakdown and the outstanding live-acceptance steps.

#### Retained artifacts (kept on purpose)

- `scripts/test-mistral-reasoning.sh` — raw Mistral API test (PASS: confirms the
  field is honored).
- `scripts/test-mistral-reasoning-ironclaw.sh` — live end-to-end acceptance test
  via `ironclaw -m`, logging the full interaction (request `reasoning_effort`,
  parsed thinking trace, answer) across both reasoning models, a non-reasoning
  model, and the off toggle.

#### Status

- **Planning committed** — the
  `docs(llm): add Mistral reasoning provider architecture + research` commit
  carries the plan/research docs and `scripts/test-mistral-reasoning.sh`. Docs only.
- **Implementation committed** — the
  `feat(llm): add custom Mistral provider with reasoning_effort support` commit,
  kept **separate** from the planning commit per the Conventional-Commits
  convention above. Offline matrix C1–C12 + U1/U2/G1 pass; `cargo fmt`,
  `cargo clippy --all-features`, and `cargo test` are green.
- **Not yet declared done.** The live acceptance script ran against the real API
  and the receive path worked on small/medium/large with no `ApiResponse` parse
  error, but a clean final acceptance run (post-ANSI-fix) and a multi-turn replay
  check are still pending. See the impl doc's **WU7** for the remaining steps.

<!-- Add new local changes above this line, newest first. -->
