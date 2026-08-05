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

For a routine catch-up (a handful of upstream commits, no structural changes),
carry local changes as commits on a branch, rebased onto upstream so git
reapplies them automatically:

```bash
git fetch upstream
git rebase upstream/main                       # replays our carry commits on top
git push --force-with-lease origin <branch>
```

A rebase conflict is the signal to look — it usually means upstream touched the
same code (possibly fixing it themselves, at which point the local commit can be
dropped). The 2026-07-05 catch-up did exactly that for the entire Mistral
reasoning carry (see "Retired" below).

**For a large-scale catch-up** (upstream deleted/renamed whole subsystems, or
the commit gap is in the hundreds), rebase spends nearly all its effort
resolving conflicts on carry commits that are dead anyway. The 2026-08-05
catch-up (upstream 0.29.1-era → 1.1.0-rc.1, ~608 commits, full v1 `src/`
monolith deletion + repo-wide Reborn crate de-prefixing) instead: branched a
new `catchup/<label>` branch straight off `upstream/main` in a separate git
worktree (`git worktree add`, so `main` and the in-progress feature branch are
never touched mid-catch-up), then hand-authored each surviving carry item as a
fresh, independently-validated commit against the clean tree rather than
replaying old commit hashes. Safety tags (`safety/<branch>-pre-catchup`) were
cut on both `main` and the feature branch first. See fork issue tracking for
the specific catch-up if one exists.

### Commit convention

The repo (and we, for our carry commits) use **Conventional Commits** —
`type(scope): subject` (e.g. `chore(fork): …`, `fix(worker): …`) — with a
`Co-Authored-By: Claude …` trailer when a commit was authored with Claude.
Keep planning/docs and implementation in **separate** commits.

**Don't cite carry-commit SHAs in committed docs.** Rebasing onto upstream (and
any history rewrite) re-hashes our carry commits, so a pinned SHA goes stale on
the next `git rebase upstream/main`. Reference carry commits by their
Conventional-Commit subject instead.

## Active local changes

### Fork-release skill + tag-driven release convention

- **What:** `.claude/skills/fork-release/SKILL.md` — a Claude Code skill that
  guides cutting a *marked* release tag on this fork via cargo-dist, and that
  doubles as the git-workflow maintenance checklist (remotes, `gh auth setup-git`
  credential helper, branch tracking).
- **Fork-marking convention (local-only):** fork releases use a prerelease
  version suffix `-fork.<N>` (e.g. `0.29.1-fork.4`, `1.1.0-rc.1-fork.1`).
  cargo-dist requires the `ironclaw` `[package]` version to equal the tag
  version, so a fork release bumps that version line — **this diverges from
  upstream and will conflict on `git rebase upstream/main`** (or need
  re-applying on a reset-and-reapply catch-up). Resolution: take upstream's
  base version, re-apply the `-fork.<N>` suffix, reset `N` to 1 on a new base.
- **Release targeting repointed to the fork (local-only):** upstream hardcodes
  `nearai/ironclaw` in release generation. We repoint `repository`/`homepage`
  → `k-l-sorensen/ironclaw` on the sole dist-able package,
  **`crates/ironclaw_cli/Cargo.toml`** (cargo-dist bakes this into the
  generated installers, including the WiX MSI's ARPHELPLINK — cargo-dist 0.31
  generates the WiX config directly from package metadata, there's no separate
  `wix/main.wxs` template to patch anymore). The release workflow itself is
  `.github/workflows/ironclaw-release.yml` (renamed from `release.yml`
  upstream at some point past our old base). `authors` and the license are
  deliberately left as NEAR AI. Upstream's new `cut-ironclaw-release.yml` is
  their own App-token-gated release tooling — not usable by, or relevant to,
  the fork; a direct annotated-tag push still triggers `ironclaw-release.yml`
  as before.
- **Hard rule:** tags/branches/releases go to `origin` (the fork) only; never
  `git push upstream`, never `git push --tags`.

### Advisory ignore: `RUSTSEC-2026-0187` (lopdf DoS)

- **What:** `deny.toml` ignores `RUSTSEC-2026-0187` (lopdf stack-overflow DoS via
  deeply nested PDF objects), reachable through `pdf-extract` document text
  extraction. Fork-tracked in issue #2.
- **Why:** unblocks `cargo deny` on the fork (which has no upstream CI secrets).
  Remove once `pdf-extract`/`lopdf` is bumped or extraction input is sandboxed.

## Retired local changes

### Custom Mistral `reasoning_effort` provider + reasoning replay (CTR-1 / SIG-1 / ReasoningBlock)

**Retired in the 2026-07-05 upstream catch-up.** The fork previously carried a
custom `crates/ironclaw_llm/src/mistral.rs` provider plus a large reasoning-replay
threading (CTR-1 cross-turn reasoning, SIG-1 ThinkChunk signature, and a
`ReasoningBlock` bundling refactor) spanning ~35 files.

During the catch-up we found that **upstream independently built a more general
reasoning-replay system** — `reasoning` + `reasoning_details`
(`ReasoningDetail::{ Text { text, signature }, Encrypted, Redacted, Summary }`),
`with_reasoning` / `with_reasoning_details` builders, rig-adapter support — which
supersedes ours and already carries a per-block signature. Rather than keep
re-conflicting these files on every rebase, we **dropped the entire Mistral
reasoning carry** and adopted upstream's system.

Re-architecting Mistral support onto upstream's native reasoning path is tracked
in **[fork issue #8](https://github.com/k-l-sorensen/ironclaw/issues/8)**. The
full pre-catch-up state (all Mistral code + tests) is preserved at the
`backup/main-pre-catchup` tag.

**Status (2026-07-05): was IMPLEMENTED** on the approved design
(`docs/plans/2026-07-05-mistral-reasoning-native-arch.md`, which supersedes the
2026-06-24 design) — one thin `MistralProvider` at the wire boundary
translating Mistral's chunk array onto `ReasoningDetail::Text{text,signature}`,
a `ProviderProtocol::Mistral` variant, a `supports_mistral_reasoning()` gate,
and `providers.json` config; no `ReasoningBlock`/CTR-1/SIG-1 rebuild, no new DB
migrations. Scope: `reasoning_effort=high` for Mistral Medium 3.5
(`mistral-medium-2604`) and Mistral Small 4 (`mistral-small-2603`).

**Status (2026-08-05): pending re-landing on this catch-up.** The 608-commit
2026-08-05 catch-up branched fresh off `upstream/main` rather than rebasing
(see "Maintenance workflow" above), so the implemented provider above needs to
be re-created against the new tree, not just carried — the integration points
moved (`src/config/llm.rs` and `src/cli/models.rs`, where the env plumbing
lived, no longer exist; `providers.json` moved from repo root to
`crates/ironclaw_llm/assets/providers.json`). The design and acceptance
criteria in `docs/plans/2026-07-05-mistral-reasoning-native-arch.md` still
hold — confirmed upstream still has no native Mistral reasoning support at the
new base either. Once re-landed, move this entry back to *Active local
changes* above.

**Reference material retained in-tree to seed the re-architecture** (so it need
not be reinvented):

- `docs/providers/mistral-reasoning.md` — provider-agnostic API research + the
  rig-core parse blocker (still valid).
- `docs/plans/2026-07-05-mistral-reasoning-native-arch.md` — the **current
  approved** C4 L3 architecture (native `reasoning_details` path, model catalog,
  components, rule-compliance, acceptance criteria).
- `docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md` — the
  superseded design (bannered), **plus the acceptance criteria** the re-arch must
  re-satisfy (clean round-trip, multi-turn replay).
- `docs/plans/2026-06-24-mistral-reasoning-impl.md` — superseded work breakdown,
  kept for the edge cases it enumerates.
- `scripts/test-mistral-reasoning.sh` — raw Mistral API probe (no code coupling).

The retired tests — `tests/e2e_live_mistral_reasoning.rs` (behavioral acceptance)
and `crates/ironclaw_llm/src/mistral/tests.rs` (offline parser matrix, C1–C12) —
live at the backup tag; their intent is captured in the acceptance criteria above.

### Worker job-status fix

**Dropped in the 2026-08-05 upstream catch-up.** A small cluster of
`fix(worker)` / `refactor(worker)` / `docs(worker)` commits that emitted status
on container result events, persisted detached sandbox job status, collapsed
duplicated terminal-finalization into one helper, typed all `job.rs` status
arms, and restored the "unknown job result status" warning via provenance.

All of it targeted files that no longer exist: `src/agent/job_monitor.rs`,
`src/orchestrator/api.rs`, `src/tools/builtin/job.rs`,
`src/worker/{container,job}.rs` were deleted with the v1 `src/` monolith, and
`crates/ironclaw_common/src/event.rs` (the shared event-type file the fix also
touched) was deleted too — with no successor naming found anywhere in
`crates/` (`JobStatus`/`job_status`/`ContainerResult` all came up empty).
Dropped outright rather than re-homed; if the same class of bug resurfaces in
Reborn's job/sandbox lane, it needs a fresh fix against that code, not a port
of this one.

<!-- Add new local changes above the "Retired" section, newest first. -->
