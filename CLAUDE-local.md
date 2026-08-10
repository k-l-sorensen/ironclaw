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

The **2026-08-10 catch-up** (upstream 19b1fb715 → 4e05a033d, 68 commits) hit
the same large-scale trigger on a much smaller commit count: upstream's WS7
"family directory moves" (#7206, #7212) reorganized the entire `crates/` tree
into family subdirectories (`crates/app/`, `crates/contracts/`,
`crates/domains/`, `crates/events/`, `crates/extensions/`, `crates/kernel/`,
`crates/lanes/`, `crates/loop/`, `crates/product/`, `crates/substrates/`) —
4,589 renames across 2,729+ files. Same worktree-branch playbook applied; the
carry itself needed no logic changes, only path updates (`ironclaw_llm` →
`crates/domains/ironclaw_llm`, `ironclaw_cli` → `crates/app/ironclaw_cli`).
This confirms the trigger is "structural renames," independent of raw commit
count — a two-digit commit gap can still warrant the worktree approach.

**Landing a large-scale catch-up branch will show conflicts on the PR —
this is expected, not a sign the port was done wrong.** The catch-up branch
is built fresh off `upstream/main`, so it shares no history with `main`'s own
carry commits past the old merge-base; `main` still has the pre-reorg carry
sitting on the old paths. GitHub's merge preview (and a plain `git merge`)
will show every file the carry touches as conflicting — `main`'s old-path
version vs. the branch's already-relocated version — even though there is no
real disagreement, just two paths for the same superseded content. Resolve by
merging `main` into the catch-up branch locally (`git merge origin/main`) and
taking "ours" (the catch-up branch) for every such conflict, since it is the
supersede-set; genuine identical-content conflicts (git's rename tracking
failing to match a file across two independently-authored paths to the same
destination) resolve either way. Re-run the doc/lint/test gates after
resolving — the merge can surface files the original port forgot entirely
(the 2026-08-10 catch-up caught this way that the Mistral reference docs never
got re-carried, see the Mistral entry below) — then push the merge commit to
the same PR branch rather than force-pushing over `main`.

### Catch-up cadence: sync to upstream releases, not intermediate `main` commits

- **Rule (2026-08-10):** when catching up, stop at the commit on `upstream/main`
  that corresponds to upstream's most recently *published* release — not
  whatever `upstream/main`'s tip happens to be that day. `main` is a moving
  target with no natural stopping point; a named release is.
- **Why:** discovered while cutting the `1.1.0-rc.1-mistral-fork.1` release.
  Upstream's release tags (`ironclaw-v<X.Y.Z>`, `ironclaw-v<X.Y.Z>-rc.N`) are
  cut onto a dedicated `release/<version>` branch and **never merge back into
  `main`** — confirmed with `git branch -r --contains ironclaw-v1.1.0` (lists
  only `upstream/release/1.1.0-rc.1` and `upstream/release/1.1.1`, not
  `upstream/main`) and `git merge-base --is-ancestor ironclaw-v1.1.0-rc.1
  upstream/main` (fails). By the time of our 2026-08-10 catch-up (to
  `4e05a033d`), `upstream/main` had already moved **83 commits** past the point
  where the `1.1.0-rc.1`/`1.1.0` release branch was cut — none of that gap is
  "the next release," it's ordinary in-flight development with no release
  boundary yet. A fork release cut from that point can't honestly claim to be
  "upstream v1.1.0 + fork edits" — it's "v1.1.0's branch point plus 83
  unrelated commits plus fork edits," a materially different (and undocumented)
  set of upstream content.
- **How to apply — finding the stop point:** a release tag's *commit* isn't on
  `main`, but `git merge-base` still finds where the branches diverged, which
  is the right stop point for a `main`-tracking catch-up:
  ```bash
  git fetch upstream --tags
  git merge-base ironclaw-v<latest-non-rc-version> upstream/main
  ```
  Catch up `main` to that commit (not `upstream/main`'s tip), using the routine
  or large-scale playbook above as the commit-gap size dictates. This is a
  **fork release doesn't have to wait for a fork catch-up** to happen the same
  day, either — cut the release from the merge-base point directly, then catch
  `main` up to it separately if/when there's other reason to.
- **Note the gap, don't chase it:** the 3 commits that exist *only* on the
  release branch (hotfix backports never merged to `main`, e.g. the 1.0→1.1
  migration-state-preservation fix) are real content our `main`-tracking fork
  will never receive through a normal catch-up. If one of those specifically
  matters, it needs an explicit cherry-pick from the release branch, called out
  in the fork issue/PR — don't assume `main` catch-up covers it.

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

### Mistral `reasoning_effort=high` on upstream's native `reasoning_details`

- **What:** a dedicated `crates/domains/ironclaw_llm/src/mistral.rs` provider
  (`ProviderProtocol::Mistral`) that owns Mistral's wire JSON so it can parse
  the `reasoning_effort=high` array-shaped response (`[{thinking},{text}]`)
  that rig-core 0.33 cannot. The thinking chunk (+ opaque signature) is mapped
  onto **upstream's shared `reasoning_details` channel**
  (`ReasoningDetail::Text { text, signature }`) — the same seam
  DeepSeek/Gemini/OpenRouter use — and replayed on the next turn. Supporting
  pieces: `MistralReasoningEffort` + `resolve_mistral_reasoning_from_env`
  (`config.rs`), `supports_mistral_reasoning` gate (`reasoning_models.rs`),
  `ProviderProtocol::Mistral` + factory dispatch (`registry.rs`/`lib.rs`),
  `MISTRAL_REASONING` env wiring in `apply_registry_provider_env`
  (`resolution.rs` — `ironclaw_llm` now owns its full env→config resolution
  itself; there's no more binary-crate env-reading split, since `ironclaw_cli`
  is architecturally forbidden from depending on `ironclaw_llm` directly),
  `crates/domains/ironclaw_llm/assets/providers.json` switch
  (`open_ai_completions` → `mistral`, default model → `mistral-medium-latest`),
  offline parser matrix (`mistral/tests.rs`), and a live contract test
  (`tests/reborn_live_mistral_reasoning_contract.rs` — the old
  `tests/e2e_live_mistral_reasoning.rs` depended on a v1 agent-loop harness
  deleted with the monolith; this one calls `MistralProvider` directly,
  modeled on `tests/reborn_live_github_pat_contract.rs`).
- **Why:** implements fork issue #8 per
  `docs/internal/plans/2026-07-05-mistral-reasoning-native-arch.md`. Re-landed on the
  2026-08-05 catch-up (upstream 0.29.1-era → 1.1.0-rc.1) from the original
  `152c010bc4`; still no upstream native Mistral reasoning support at the new
  base either, so this remains needed. No `ReasoningBlock`/CTR-1/SIG-1 carry,
  no DB migration. Scope: Mistral Medium 3.5 (`mistral-medium-2604`) and Small
  4 (`mistral-small-2603`).
- **Re-landing deviations (2026-08-05):** cost lookup moved crates
  (`crate::costs` → `ironclaw_common::llm_costs`, since `ironclaw_llm` no
  longer owns cost tables); added an explicit `provider_id()` override (the
  `LlmProvider` trait grew this method upstream, matching the
  `github_copilot.rs` pattern); two `retry::is_retryable` test assertions
  pinned to this tree's current policy (`InvalidResponse`/`EmptyResponse` are
  now non-retryable, a policy change upstream made independently of this
  carry).
- **2026-08-10 catch-up:** pure path move, no logic change. `ironclaw_llm`
  relocated to `crates/domains/ironclaw_llm` under upstream's WS7
  family-directory reorg. The crate's own `CLAUDE.md`/`AGENTS.md` are now
  symlink pointers (upstream's guidance-unification work landed in this same
  range); the file-map and sub-owner-map entries for `mistral.rs` /
  `mistral/tests.rs` now live in `crates/domains/ironclaw_llm/CONTRACT.md`
  instead (enforced by `tests/module_charter.rs`).
- **Known gap (tracked):** `ironclaw_common::llm_costs::is_local_model`
  matches `mistral*`, so hosted Mistral currently bills as $0 — fork
  follow-up issue; see the `TODO` in `mistral.rs::cost_per_token`.

### Fork-release skill + tag-driven release convention

- **What:** `.claude/skills/fork-release/SKILL.md` — a Claude Code skill that
  guides cutting a *marked* release tag on this fork via cargo-dist, and that
  doubles as the git-workflow maintenance checklist (remotes, `gh auth setup-git`
  credential helper, branch tracking).
- **Fork-marking convention (local-only):** fork releases use a prerelease
  version suffix `-mistral-fork.<N>` (e.g. `1.1.0-rc.1-mistral-fork.1`).
  **Changed 2026-08-10** from the earlier generic `-fork.<N>` (used for
  `0.29.1-fork.1`/`.2`/`.3`) to brand the suffix after the fork's flagship
  divergence (the custom Mistral `reasoning_effort=high` provider, above) —
  chosen deliberately over the literal `-mistral-fork`/`-mistral-fork-v2` the
  user first proposed, to keep the dot-incrementing `.<N>` pattern that's easy
  to bump and unambiguous to parse. cargo-dist requires the `ironclaw`
  `[package]` version to equal the tag version, so a fork release bumps that
  version line — **this diverges from upstream and will conflict on `git
  rebase upstream/main`** (or need re-applying on a reset-and-reapply
  catch-up). Resolution: take upstream's base version, re-apply the
  `-mistral-fork.<N>` suffix, reset `N` to 1 on a new base.
- **Release targeting repointed to the fork (local-only):** upstream hardcodes
  `nearai/ironclaw` in release generation. We repoint `repository`/`homepage`
  → `k-l-sorensen/ironclaw` on the sole dist-able package,
  **`crates/app/ironclaw_cli/Cargo.toml`** (cargo-dist bakes this into the
  generated installers, including the WiX MSI's ARPHELPLINK). **Correction
  (2026-08-10):** `crates/app/ironclaw_cli/wix/main.wxs` *is* a committed,
  generated file — cargo-dist 0.31's `plan` job (`dist host --steps=create`)
  diffs it against what it would regenerate from package metadata and fails
  the whole release build if they've drifted. The first
  `1.1.0-rc.1-mistral-fork.1` build failed exactly this way: the metadata
  repoint above landed, but the committed `main.wxs` still had `ARPHELPLINK`
  pointing at `nearai/ironclaw`. Fixed with a direct one-line text patch (no
  `dist`/`cargo-dist` CLI installed locally to run `dist init`); the
  fork-release skill's preflight now greps for stale `nearai/ironclaw`
  references in that directory before tagging. The release workflow itself is
  `.github/workflows/ironclaw-release.yml` (renamed from `release.yml`
  upstream at some point past our old base). `authors` and the license are
  deliberately left as NEAR AI. Upstream's new `cut-ironclaw-release.yml` is
  their own App-token-gated release tooling — not usable by, or relevant to,
  the fork; a direct annotated-tag push still triggers `ironclaw-release.yml`
  as before.
- **MSI installer dropped (local-only, 2026-08-10):** root `Cargo.toml`
  `[workspace.metadata.dist] installers` is `["shell", "powershell"]`, not
  upstream's `["shell", "powershell", "msi"]`. WiX's MSI-version mapper only
  handles a version with one dotted prerelease group (`name.N`, e.g.
  `1.2.3-prerelease.4` → `1.2.3.4`). A fork release built on an upstream
  `-rc.N` base plus our `-mistral-fork.N` suffix produces *two*
  (`1.1.0-rc.1-mistral-fork.1` → prerelease `rc.1-mistral-fork.1`, three
  dot-separated identifiers), which WiX rejects — and since `dist build` runs
  archive + shell + powershell + msi as one command per target, the MSI
  failure aborted the *entire* Windows job (no `.zip`, no install scripts
  either), which in turn blocked `host`/`announce` and lost the release for
  all 8 platforms, not just Windows. Hit on the first
  `1.1.0-rc.1-mistral-fork.1` build attempt; fixed by dropping `msi` rather
  than reworking the version scheme. **Revisit once releases are cut from a
  clean (non-`-rc.N`) base** per the catch-up-cadence policy above — a version
  like `1.1.0-mistral-fork.1` has only one dotted prerelease group and should
  satisfy WiX, so `msi` may be safe to re-add then; verify with a real build
  before assuming it, not by inspection alone.
- **2026-08-10 catch-up:** `ironclaw_cli` moved to `crates/app/ironclaw_cli`
  under upstream's WS7 family-directory reorg; every path reference in the
  skill and in this section was updated accordingly. No behavior change.
- **Hard rule:** tags/branches/releases go to `origin` (the fork) only; never
  `git push upstream`, never `git push --tags`.

### GitHub Actions disable list

- **What:** on a personal single-user fork with zero Actions secrets, upstream's
  scheduled/publish workflows either spam failure emails or can never succeed.
  Disabled via `gh workflow disable <file> -R k-l-sorensen/ironclaw`
  (GitHub-level toggle, **not** a file edit — keeps the workflow YAML
  unmodified and rebase/reset-and-reapply-safe). Reverse with
  `gh workflow enable`.
- **Disable (cron/push-to-main, will auto-fire and fail without secrets):**
  `docker.yml`* (Docker Hub push, no `DOCKER_REGISTRY_*`), `live-canary.yml`
  (3h schedule), `nightly-deep-ci.yml` (daily schedule), `coverage.yml`
  (Codecov, no token), `nightly-watchdog.yml` (daily schedule — **new as of
  the 2026-08-05 catch-up**, same rationale as `nightly-deep-ci.yml`), and
  `main-ci-slack-alerts.yml` (**new as of the 2026-08-05 catch-up** —
  `workflow_run`-triggered off our own CI failing, and the job itself
  hard-fails, not skips, when `MAIN_CI_SLACK_WEBHOOK_URLS`/`SLACK_WEBHOOK_URL`
  are unset — the exact "fires precisely when something already failed, then
  fails again" pattern that motivated disabling the others).
  \* `docker.yml` no longer has a standalone trigger on this tree — it's
  `workflow_call`-only, invoked by `ironclaw-release.yml` during a release;
  nothing to disable directly, the inertness (no Docker secrets) carries
  through unchanged.
- **Self-gated, no action needed:** `release-plz.yml`
  (`if: github.repository_owner == 'nearai'`), `codebase-graph-refresh.yml`
  (`if: github.repository == 'nearai/ironclaw'`, **new as of the 2026-08-05
  catch-up**).
- **Dormant by trigger, no action needed:** `sccache-dist-smoke.yml`,
  `rebuild-release-image.yml`, `cut-ironclaw-release.yml` (upstream's own
  App-token-gated release tooling, see above), `live-canary-command.yml` /
  `nearai-bench.yml` (`issue_comment`), `nearai-bench-tests.yml`
  (path-scoped to the bench workflow files themselves).
- **Keep enabled:** PR-triggered CI (`code_style`, `platform-and-compat`,
  `reborn-*`, `history-check.yml` — **new as of the 2026-08-05 catch-up**, a
  PR-triggered repo-hygiene check with no external secrets — `pr-label-*`)
  and `ironclaw-release.yml` (the intentional tag-driven fork release).
- **2026-08-10 catch-up:** workflow file set is unchanged in this range (all
  12 touched `.github/workflows/*` files in the 68-commit diff are
  modifications, zero adds/removes/renames) — this disable list's content
  stays accurate as-is.
- **Action item:** the disable list above reflects the workflow set as of the
  2026-08-05 catch-up; the actual `gh workflow disable` calls need re-running
  once this branch's workflow files are live on `origin` (GitHub only lets you
  disable a workflow once it has run/registered on that repo).

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

**Status (2026-08-05): re-landed** on the 608-commit upstream catch-up (which
branched fresh off `upstream/main` rather than rebasing — see "Maintenance
workflow" above — so the provider was re-created against the new tree, not
carried; the old integration points, `src/config/llm.rs` and
`src/cli/models.rs`, no longer exist, and `providers.json` moved to
`crates/ironclaw_llm/assets/providers.json`). See the **"Mistral
`reasoning_effort=high` on upstream's native `reasoning_details`"** entry
under *Active local changes* above for the current shape and the specific
re-landing deviations.

**Status (2026-08-10): re-applied**, path-only, onto upstream's WS7
family-directory reorg (`crates/ironclaw_llm` → `crates/domains/ironclaw_llm`).
No logic changes; see the "2026-08-10 catch-up" note under *Active local
changes* above. This same catch-up also relocated the reference docs below:
upstream's `docs/ publication boundary` work (commit `50311eab4` in this
range, enforced by `scripts/ci/docs_publication_boundary.py`) retired
`docs/plans/` entirely in favor of `docs/internal/plans/`, and flagged our
`docs/providers/mistral-reasoning.md` as an unfenced page (neither public-nav'd
nor under `internal/`) — moved to `docs/internal/research/mistral-reasoning.md`.

**Reference material retained in-tree to seed the re-architecture** (so it need
not be reinvented):

- `docs/internal/research/mistral-reasoning.md` — provider-agnostic API
  research + the rig-core parse blocker (still valid).
- `docs/internal/plans/2026-07-05-mistral-reasoning-native-arch.md` — the
  **current approved** C4 L3 architecture (native `reasoning_details` path,
  model catalog, components, rule-compliance, acceptance criteria).
- `docs/internal/plans/2026-06-24-mistral-reasoning-provider-architecture.md`
  — the superseded design (bannered), **plus the acceptance criteria** the
  re-arch must re-satisfy (clean round-trip, multi-turn replay).
- `docs/internal/plans/2026-06-24-mistral-reasoning-impl.md` — superseded
  work breakdown, kept for the edge cases it enumerates.
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

### Advisory ignore: `RUSTSEC-2026-0187` (lopdf DoS)

**Resolved in the 2026-08-10 catch-up cleanup.** `deny.toml` carried an
`ignore` entry for `RUSTSEC-2026-0187` (lopdf stack-overflow DoS via deeply
nested PDF objects, reachable through `pdf-extract` document text extraction
in `crates/domains/ironclaw_extractors`), added on 2026-06-26 to unblock
`cargo deny` while `lopdf` was still on the vulnerable `0.34.0`. Fork-tracked
in [issue #2](https://github.com/k-l-sorensen/ironclaw/issues/2).

By the 2026-08-05 and 2026-08-10 catch-ups, ordinary upstream dependency
movement had already carried `lopdf` to `0.42.0` — the advisory's patched
version (affected range `<= 0.41.0`, fixed `>= 0.42.0`; the fix adds a
max-nesting-depth check so the parser now returns an `Err` instead of
aborting). Both catch-ups reapplied the `ignore` entry anyway, unchanged,
without checking the pinned version against the advisory's fixed threshold —
the reapply commit messages note "lopdf is still pinned at 0.42.0" as if that
meant still-vulnerable, when 0.42.0 was already the fix. The entry sat dead
for two catch-ups before this cleanup removed it and confirmed `cargo deny
check advisories` passes clean without it.

**Fork-carry pitfall:** an `ignore` entry surviving a rebase/re-carry
unchanged is not evidence it's still needed — a later catch-up can bump the
offending dependency past an advisory's fix as a side effect of unrelated
upstream work. Before reapplying an advisory ignore during a catch-up, check
the ignored advisory's patched-version threshold against the dependency's
*current* `Cargo.lock` pin, not just whether the version number looks
unchanged from the last carry.

The residual reachability point from the original issue still holds
structurally — attachment bytes reach `extract_pdf`
(`crates/domains/ironclaw_extractors/src/lib.rs`) with only an
attachment-level `max_bytes` cap upstream of it, no PDF-specific
timeout/sandbox in `ironclaw_extractors` itself — but the actual crash vector
(unbounded recursion aborting the process) is now handled inside `lopdf`
itself, which is what the advisory's fix does. No further app-level guard
was judged necessary to close #2.

<!-- Add new local changes above the "Retired" section, newest first. -->
