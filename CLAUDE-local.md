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

### Fork-release skill + tag-driven release convention — 2026-06-26

- **What:** `.claude/skills/fork-release/SKILL.md` — a Claude Code skill that
  guides cutting a *marked* release tag on this fork via cargo-dist, and that
  doubles as the git-workflow maintenance checklist (remotes, `gh auth setup-git`
  credential helper, branch tracking).
- **Fork-marking convention (local-only):** fork releases use a prerelease
  version suffix `-fork.<N>` (e.g. `0.29.1-fork.1`). cargo-dist requires the
  `ironclaw` `[package]` version in `Cargo.toml` to equal the tag version, so a
  fork release bumps that version line — **this diverges from upstream and will
  conflict on `git rebase upstream/main`**. Resolution: take upstream's base
  version, re-apply the `-fork.<N>` suffix.
- **Why:** releases must never be pushed to `upstream` (nearai) and must be
  visibly distinct from official upstream builds; the prerelease suffix also makes
  cargo-dist auto-flag the GitHub Release as a pre-release.
- **Release targeting repointed to the fork (local-only):** upstream hardcodes
  `nearai/ironclaw` in release generation. We repointed it so fork releases are
  self-consistent: `Cargo.toml` `repository`/`homepage` → `k-l-sorensen/ironclaw`
  (cargo-dist bakes this into the generated installers), `wix/main.wxs`'s
  `ARPHELPLINK` → `k-l-sorensen/ironclaw` (this is a **committed** generated file
  the `msi` installer reads; it is NOT covered by `allow-dirty = ["ci"]`, so
  cargo-dist's `dist host` plan step fails the build if it drifts from
  `Cargo.toml` — it conflicts on rebase like the version line, and the first
  `0.29.1-fork.1` tag attempt failed here before this was caught), and the
  WASM-manifest download URLs in `.github/workflows/release.yml` → `${{ github.repository }}`
  (resolves to whoever runs the build — fork-safe and upstream-safe). `authors`
  and the license are deliberately left as NEAR AI. The `Cargo.toml` change
  conflicts on rebase like the version line; the `release.yml` change is
  conflict-free. `docker.yml`'s `nearai/ironclaw-dind` dispatch is left as-is (it
  no-ops on the fork: `continue-on-error` + gated on an app token we lack).
- **Hard rule:** tags/branches/releases go to `origin` (the fork) only; never
  `git push upstream`, never `git push --tags`.

### Mistral reasoning — implemented (`feat(llm)` landed), live acceptance PASSED — 2026-06-24

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
  - [`docs/providers/mistral-reasoning.md`](docs/providers/mistral-reasoning.md) — API findings, request/response format, the blocker.
  - [`docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md`](docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md) — approved C4 L3 design (decisions D1–D10, findings F1–F12).
  - [`docs/plans/2026-06-24-mistral-reasoning-impl.md`](docs/plans/2026-06-24-mistral-reasoning-impl.md) — work-unit breakdown and the outstanding live-acceptance steps.

#### Retained artifacts (kept on purpose)

- `scripts/test-mistral-reasoning.sh` — raw Mistral API test (PASS: confirms the
  field is honored).
- `tests/e2e_live_mistral_reasoning.rs` — Live-tier acceptance test that drives the
  real agent loop against the Mistral API and asserts a clean reasoning round-trip
  (non-empty reply, no parse-error signature) plus a multi-turn thinking replay. It
  replaced the bespoke bash harness so the live coverage follows the repo's standard
  `#[ignore]` + `LiveTestHarness` convention; the deterministic regression net stays
  the offline matrix in `crates/ironclaw_llm/src/mistral_tests.rs`. Run with:
  `IRONCLAW_LIVE_TEST=1 LLM_BACKEND=mistral MISTRAL_API_KEY=... cargo test --features
  libsql --test e2e_live_mistral_reasoning -- --ignored`.

#### Status

- **Planning committed** — the
  `docs(llm): add Mistral reasoning provider architecture + research` commit
  carries the plan/research docs and `scripts/test-mistral-reasoning.sh`. Docs only.
- **Implementation committed** — the
  `feat(llm): add custom Mistral provider with reasoning_effort support` commit,
  kept **separate** from the planning commit per the Conventional-Commits
  convention above. Offline matrix C1–C12 + U1/U2/G1 pass; `cargo fmt`,
  `cargo clippy --all-features`, and `cargo test` are green.
- **Done (v1 provider).** The bash acceptance harness was replaced by the Live-tier
  Rust test `tests/e2e_live_mistral_reasoning.rs`, which **PASSED against the real API**
  on the v1 path: the reasoning round-trip returns a clean reply with no `ApiResponse`
  parse error, and the multi-turn case confirms the thinking-chunk replay does not 400
  on turn 2. Offline matrix C1–C12 + U1/U2/G1 remain the primary deterministic net. See
  the impl doc's **WU7** (closed).
- **CTR-1 cross-turn replay — v1 fixed (2026-06-25).** Post-ship validation found the
  ThinkChunk was replayed only *within* a turn's tool loop and dropped on every new user
  turn + after DB hydration (the live multi-turn test was green on the degraded path).
  WU-CTR1–3 landed: `Turn`/`ConversationMessage` carry a leak-scanned `reasoning` field
  (dual-backend column, PG `V31`/libSQL `v26`), captured from `RespondResult` and
  re-attached at both context-rebuild gateways. Offline tests CTR-C1–C5 pass; gate green.
  **WU-CTR4** confirmed Reborn has the *same* drop for plain assistant messages
  (`model_gateway.rs::convert_messages`) — deferred to the Reborn follow-up (WU8–WU10)
  as it is a multi-crate Reborn change. See the impl doc's **CTR-1** section.
  **Scope: v1 agent loop only.** The replay applies to the default v1 loop
  (`ENGINE_V2` unset/false — the Mistral deployment runs v1). Under engine v2, the
  `src/bridge/router.rs` sites that persist `assistant`/`tool_calls` rows into the
  v1 `conversation_messages` table write **no** reasoning, so
  `rebuild_chat_messages_from_db` hydrates `reasoning = NULL` and the replay is
  inert (the pre-CTR-1 behavior — not a regression). Enabling engine v2 or
  migrating to Reborn requires the **WU-CTR4** / Reborn follow-up (WU8–WU10) to
  carry reasoning through that path.

- **SIG-1 ThinkChunk `signature` replay — v1 implemented (2026-06-27).** The custom
  Mistral provider dropped the opaque `signature` on every reasoning block on both
  capture and replay, replaying a signature-less, text-flattened approximation of the
  ThinkChunk. SIG-1 carries it end-to-end as a typed sibling `reasoning_signature:
  Option<String>` (never folded into the `reasoning` string), a mechanical mirror of
  the CTR-1 `reasoning` field: captured in `mistral.rs::extract_content`, re-emitted in
  `chat_message_to_wire`, threaded through `ChatMessage`/`CompletionResponse`/
  `ToolCompletionResponse` + `RespondResult` + `Turn`/`TurnPersistSnapshot` and the
  `LoopDelegate` methods, persisted via the widened `add_conversation_message_with_reasoning`
  (dual-backend column, PG `V33`/libSQL `v27`), and re-attached at the single
  `Thread::messages()` / `rebuild_chat_messages_from_db` gateway. **Leak-scan EXEMPT**
  (SIG-D2): the signature is an opaque token routed *around* `redact_reasoning` so the
  redactor cannot corrupt it. Serde stays lenient (no `deny_unknown_fields`). Offline
  tests SIG-C1/C2/C3/C5/C6 (+ caller-level reattach/hydration) pass. The detail lives in
  the plan docs — do not restate here: the architecture doc's **SIG-1** section
  (decisions SIG-D1–D5) and the impl doc's **SIG-1** section. **Scope: v1 agent loop**
  (same as CTR-1). Reborn: the per-tool-call `signature` already round-trips, and the
  Mistral message-level signature now flows through `model_gateway.rs`; the plain-assistant
  cross-turn drop is the **same WU-CTR4 gap deferred to the Reborn follow-up (WU8–WU10)**.

<!-- Add new local changes above this line, newest first. -->
