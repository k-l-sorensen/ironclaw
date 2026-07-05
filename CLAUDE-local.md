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
dropped). The 2026-07-05 catch-up did exactly that for the entire Mistral
reasoning carry (see "Retired" below).

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
  version suffix `-fork.<N>` (e.g. `0.29.1-fork.4`). cargo-dist requires the
  `ironclaw` `[package]` version in `Cargo.toml` to equal the tag version, so a
  fork release bumps that version line — **this diverges from upstream and will
  conflict on `git rebase upstream/main`**. Resolution: take upstream's base
  version, re-apply the `-fork.<N>` suffix.
- **Release targeting repointed to the fork (local-only):** upstream hardcodes
  `nearai/ironclaw` in release generation. We repointed it so fork releases are
  self-consistent: `Cargo.toml` `repository`/`homepage` → `k-l-sorensen/ironclaw`
  (cargo-dist bakes this into the generated installers), `wix/main.wxs`'s
  `ARPHELPLINK` → `k-l-sorensen/ironclaw` (a **committed** generated file the
  `msi` installer reads; NOT covered by `allow-dirty = ["ci"]`, so cargo-dist's
  `dist host` plan step fails the build if it drifts from `Cargo.toml` — it
  conflicts on rebase like the version line), and the WASM-manifest download URLs
  in `.github/workflows/release.yml` → `${{ github.repository }}` (resolves to
  whoever runs the build — fork-safe and upstream-safe). `authors` and the
  license are deliberately left as NEAR AI.
- **Hard rule:** tags/branches/releases go to `origin` (the fork) only; never
  `git push upstream`, never `git push --tags`.

### Worker job-status fix

- **What:** a small cluster of `fix(worker)` / `refactor(worker)` /
  `docs(worker)` commits that emit status on container result events, persist
  detached sandbox job status, collapse duplicated terminal-finalization into one
  helper, type all `job.rs` status arms, and restore the "unknown job result
  status" warning via provenance.
- **Why:** carried locally ahead of (or instead of) an upstream fix; re-verify on
  each catch-up whether upstream has landed an equivalent and drop if so.

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

**Reference material retained in-tree to seed the re-architecture** (so it need
not be reinvented):

- `docs/providers/mistral-reasoning.md` — provider-agnostic API research + the
  rig-core parse blocker (still valid).
- `docs/plans/2026-06-24-mistral-reasoning-provider-architecture.md` — the
  superseded design (bannered), **plus the acceptance criteria** the re-arch must
  re-satisfy (clean round-trip, multi-turn replay).
- `docs/plans/2026-06-24-mistral-reasoning-impl.md` — superseded work breakdown,
  kept for the edge cases it enumerates.
- `scripts/test-mistral-reasoning.sh` — raw Mistral API probe (no code coupling).

The retired tests — `tests/e2e_live_mistral_reasoning.rs` (behavioral acceptance)
and `crates/ironclaw_llm/src/mistral/tests.rs` (offline parser matrix, C1–C12) —
live at the backup tag; their intent is captured in the acceptance criteria above.

<!-- Add new local changes above the "Retired" section, newest first. -->
