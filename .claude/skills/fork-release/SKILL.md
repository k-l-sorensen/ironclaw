---
name: fork-release
description: Guide cutting a *marked* release tag on this fork (k-l-sorensen/ironclaw) — NOT upstream nearai/ironclaw. Use when the user wants to create a release tag, cut a release, build release binaries (incl. ARM64 Linux via cargo-dist), publish a GitHub Release, or verify/repair the fork's git-workflow settings (remotes, credential helper, branch tracking). Trigger on: "cut a release", "create a tag", "release the fork", "build ARM64 binaries", "tag a version", "fix my git remotes/credentials".
---

# Fork Release (tag-driven, cargo-dist)

> **RATIONALE:** This is a *fork* of `nearai/ironclaw`. Releases here must be
> (1) pushed only to the fork (`origin`), never `upstream`, and (2) visibly
> marked as a fork build so they are never mistaken for an official upstream
> release. Both are enforced below. Fork-specific divergences are tracked in
> `AGENTS-local.md`.

## How releases work here

The release pipeline is **[cargo-dist](https://opensource.axo.dev/cargo-dist/)
v0.31.0**, generated into `.github/workflows/ironclaw-release.yml`. It triggers
**only** on pushing a git tag matching:

```
ironclaw**[0-9]+.[0-9]+.[0-9]+*
```

On a matching tag, cargo-dist reads `[workspace.metadata.dist]` in the root
`Cargo.toml` (workspace-wide dist settings — targets, installers, tag
namespace), and `[package.metadata.dist]` / `[package.metadata.wix]` in
`crates/app/ironclaw_cli/Cargo.toml` (the sole dist-able package — since the v1
monolith was deleted upstream, the root package is now `ironclaw_integration_tests`,
`dist = false`, and only hosts the integration test suite). It builds every
target in `targets = [...]` (including `aarch64-unknown-linux-gnu` and
`aarch64-unknown-linux-musl` natively on `ubuntu-24.04-arm` runners), packages
`.tar.gz` archives + installers, and publishes a GitHub Release. **cargo-dist
requires the tag's version to equal the `ironclaw` package version in
`crates/app/ironclaw_cli/Cargo.toml`** — so the version bump and the tag must agree.
**`crates/app/ironclaw_cli/wix/main.wxs` is a committed, generated file — cargo-dist
0.31 checks it against what it *would* generate from `[package.metadata.wix]`
plus the package's `homepage`/`repository`, and fails the `plan` job
(`dist host --steps=create`) if they've drifted.** Confirmed the hard way:
the first `1.1.0-rc.1-mistral-fork.1` build failed here because the
committed file still had `ARPHELPLINK` pointing at
`https://github.com/nearai/ironclaw` after the fork's homepage/repository
repoint (below) — the metadata changed but the generated file wasn't
regenerated to match. No `dist`/`cargo-dist` CLI is installed locally on this
machine, so the practical fix is a direct text patch of the drifted line(s) —
`grep -rn 'nearai/ironclaw' crates/app/ironclaw_cli/wix/` finds them — rather
than running `dist init`. Re-check this file whenever `homepage`/`repository`/
`authors`/version-adjacent metadata on the `ironclaw` package changes.

Nothing here runs on normal pushes/PRs. On a **public** fork the runners
(`ubuntu-*`, `ubuntu-*-arm`, `macos-*`, `windows-*`) are **free**.

> **`cut-ironclaw-release.yml` is upstream's own tooling, not ours.** Upstream
> added a `workflow_dispatch`-triggered "Cut Ironclaw Release" workflow gated
> behind a protected `ironclaw-release` GitHub Environment and a GitHub App
> token (`secrets.GH_RELEASES_MANAGER_APP_ID`) that only `nearai/ironclaw` has.
> The fork has neither the environment nor the app credentials, and doesn't
> need them — a direct annotated-tag push (below) still triggers
> `ironclaw-release.yml` exactly as before. Do not try to invoke
> `cut-ironclaw-release.yml` on the fork.

## Fork marking convention

Mark every fork release with a **prerelease suffix** on the version:

```
<base-version>-mistral-fork.<N>   e.g.  1.1.0-rc.1-mistral-fork.1, 1.1.0-rc.1-mistral-fork.2, 1.1.0-mistral-fork.1
```

This does three things at once:
- Keeps the tag matching the cargo-dist trigger glob (the trailing `*` covers `-mistral-fork.N`).
- Makes the version semver-prerelease, so cargo-dist **auto-flags the GitHub
  Release as a pre-release** — distinct from upstream's clean `ironclaw-v0.29.1`.
- States the fork lineage in the version string itself, branded after the
  fork's flagship divergence (the custom Mistral `reasoning_effort=high`
  provider — see `AGENTS-local.md` "Active local changes").

`<base-version>` is whatever upstream version you forked from / are building on.
Increment `<N>` for each fork release of the same base; bump the base when you
rebase onto a newer upstream version.

> **Policy:** `AGENTS-local.md` → "Catch-up cadence: sync to upstream releases, not
> intermediate `main` commits".

---

## Procedure

Run these in order. **Stop and report if any preflight check fails** — do not
improvise around a wrong remote or a hung credential prompt.

### 1. Preflight: git workflow settings (fail closed)

```bash
# 1a. Remotes must be: origin = the fork, upstream = nearai. Refuse otherwise.
git remote -v
#   origin    -> github.com/k-l-sorensen/ironclaw   (push target)
#   upstream  -> github.com/nearai/ironclaw         (NEVER a tag push target)

# 1b. Hard guard: origin must be the fork, not upstream.
git remote get-url origin | grep -q 'k-l-sorensen/ironclaw' \
  && echo "OK: origin is the fork" \
  || { echo "ABORT: origin is not the fork — fix remotes before releasing"; exit 1; }
git remote get-url origin | grep -qi 'nearai' \
  && { echo "ABORT: origin points at upstream nearai — refusing to release"; exit 1; } \
  || echo "OK: origin is not upstream"

# 1c. Credential helper must use gh, or HTTPS pushes hang on a silent prompt.
git config --get-all credential.'https://github.com'.helper | grep -q 'gh auth' \
  && echo "OK: gh credential helper active" \
  || { echo "Fixing: routing git auth through gh"; gh auth setup-git; }

# 1d. Be on the fork's main, current, and clean.
git switch main
git fetch origin
git status -sb         # working tree must be clean before tagging

# 1e. Committed WiX file must already point at the fork, not upstream —
# this is a *build-time* cargo-dist drift check (see "How releases work
# here"), so catch it here rather than from a failed tag-push build.
grep -rn 'nearai/ironclaw' crates/app/ironclaw_cli/wix/ \
  && { echo "ABORT: wix/main.wxs still references nearai/ironclaw — patch the drifted line(s) before tagging"; exit 1; } \
  || echo "OK: wix/main.wxs has no stale nearai/ironclaw references"
```

> **Branch-tracking note:** local `main` historically tracks `upstream/main`,
> but **releases are cut from the fork's `main`** (`origin/main`). After merging
> a feature PR into the fork's main, sync with `git pull origin main`. If the
> user wants `main` to follow the fork by default, offer:
> `git branch --set-upstream-to=origin/main main` (this is a fork-workflow
> change — confirm before doing it, and record it in `AGENTS-local.md`).

### 2. Choose the fork version

Ask the user for the base version (default: current `ironclaw` package version)
and the fork increment `N`. Compute `VERSION = <base>-mistral-fork.<N>` and
`TAG = ironclaw-v<VERSION>`.

```bash
# Current ironclaw package version (the base, unless rebasing onto newer upstream):
grep -m1 -A1 '^\[package\]' crates/app/ironclaw_cli/Cargo.toml | grep '^version' # e.g. version = "1.1.0-rc.1"

# Make sure the tag doesn't already exist locally or on the fork:
git tag -l "$TAG"
git ls-remote --tags origin "$TAG"
```

### 3. Set the version + update the changelog + commit

**3a. Bump the version.** cargo-dist demands the package version equal the tag
version, so bump the `ironclaw` `[package]` version in
`crates/app/ironclaw_cli/Cargo.toml` (not the root `Cargo.toml` — that package is
`ironclaw_integration_tests`, `dist = false`, and carries no release version)
to the fork version:

```bash
# Edit crates/app/ironclaw_cli/Cargo.toml [package] version -> the fork version, e.g.:
#   version = "1.1.0-rc.1-fork.1"
# (Use the Edit tool; update only the ironclaw [package] version line.)
cargo update -w 2>/dev/null || cargo check -q   # refresh the ironclaw entry in Cargo.lock
```

**3b. Add a `CHANGELOG.md` entry — REQUIRED, do not skip.** cargo-dist's `host`
step generates the GitHub Release body from the `CHANGELOG.md` section whose
heading matches the tag version. **If no `## [<VERSION>]` heading exists, it
falls back to the nearest base-version section (e.g. `## [1.1.0-rc.1]`) and
publishes *upstream's* notes — with `nearai/ironclaw` PR links and none of the
fork's changes.** (This is exactly what went wrong on the first
`0.29.1-fork.1` build.)

Insert a section for the fork version **between `## [Unreleased]` and the base
version's section**, using the fork repo for the heading link:

```markdown
## [<VERSION>](https://github.com/k-l-sorensen/ironclaw/releases/tag/ironclaw-v<VERSION>) - <YYYY-MM-DD>

First/Nth marked release of the **k-l-sorensen/ironclaw fork** — unofficial, not
affiliated with upstream nearai/ironclaw. Built on upstream `main` past the
`<base>` tag plus the fork-only changes below.   See `AGENTS-local.md` for the

full divergence list.

### Added / Changed / CI · Release
- one bullet per fork-relevant change in this build (e.g. the Mistral
  `reasoning_effort` provider, release repointing, new skills). Do NOT try to
  enumerate the upstream commits the fork merged — link the lineage instead.
```

Derive the bullets from `git log --oneline --no-merges <base-tag>..HEAD` filtered
to fork-authored commits (`chore(fork)`, `feat`/`fix` you added) and from the
"Active local changes" list in `AGENTS-local.md`.

**3c. Commit both, push to the fork:**

```bash
git add crates/app/ironclaw_cli/Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(fork): release ironclaw-v<VERSION> (fork build, not upstream)

Marked fork release of k-l-sorensen/ironclaw. Prerelease suffix
'-mistral-fork.<N>' keeps this distinct from upstream nearai/ironclaw and flags the GitHub
Release as a pre-release. Includes the matching CHANGELOG.md section so
cargo-dist emits fork notes, not upstream's base-version notes.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
git push origin main      # fork only
```

> **Upstream-merge caveat:** the version line (in `crates/app/ironclaw_cli/Cargo.toml`)
> will conflict when you later merge a newer upstream. Resolve by taking
> upstream's base version and re-applying the `-mistral-fork.<N>` suffix. Note
> this in `AGENTS-local.md` under active local changes.

### 4. Create the marked, annotated tag

Always an **annotated** tag (`-a`) whose message states the fork lineage:

```bash
git tag -a "$TAG" -m "ironclaw v<VERSION> — FORK BUILD (k-l-sorensen/ironclaw)

Unofficial fork release. Not produced by or affiliated with upstream
nearai/ironclaw. Built locally via cargo-dist."
git tag -v "$TAG" 2>/dev/null; git show --no-patch "$TAG"   # sanity-check the tag
```

### 5. Push the tag — to the FORK only

> **RATIONALE:** A tag push is what triggers the build. It must go to `origin`.
> Never `git push upstream` and never `git push --tags` (which can fan out to
> multiple remotes). Push the one tag explicitly.

```bash
git push origin "refs/tags/$TAG"      # explicit single-tag push to the fork
```

If this hangs with no output, the credential helper regressed — re-run
`gh auth setup-git` (step 1c) and retry. Do not `kill` and leave a half-push.

### 6. Watch the release build and verify

```bash
# The release workflow should appear within ~30s of the tag push:
gh run list --repo k-l-sorensen/ironclaw --workflow ironclaw-release.yml --limit 3
gh run watch  --repo k-l-sorensen/ironclaw $(gh run list --repo k-l-sorensen/ironclaw \
  --workflow ironclaw-release.yml --limit 1 --json databaseId --jq '.[0].databaseId')

# When done, confirm the Release exists, is marked prerelease, and has ARM64 Linux assets:
gh release view "$TAG" --repo k-l-sorensen/ironclaw \
  --json tagName,isPrerelease,assets --jq \
  '{tag:.tagName, prerelease:.isPrerelease, assets:[.assets[].name]}'

# Confirm the Release Notes are FORK notes, not upstream's base-version fallback.
# This must print nothing — any hit means step 3b's changelog entry was missed:
gh release view "$TAG" --repo k-l-sorensen/ironclaw --json body --jq '.body' \
  | grep -n 'nearai/ironclaw/pull' && echo "BAD: upstream PR links in notes — fix CHANGELOG (step 3b) and re-tag"
```

Confirm to the user: `isPrerelease: true`; assets include
`ironclaw-aarch64-unknown-linux-gnu.tar.gz` and `-musl.tar.gz`; and the release
notes describe the **fork** changes (no `nearai/ironclaw/pull/...` links).

> **If the notes are wrong on an already-published release** (changelog entry was
> missed), you don't have to rebuild. Fix `CHANGELOG.md` on `main`, then patch the
> live body in place, preserving the generated Install/Download sections:
> save the current body, replace everything above `## Install ...` with the fork
> notes, and `gh release edit "$TAG" --repo k-l-sorensen/ironclaw --notes-file <file>`.

---

## Hard rules

- Never push tags, branches, or releases to `upstream` (nearai). The fork owns its releases.
- Never use `git push --tags` here; push the single intended tag by full ref.
- Every fork release version carries the `-mistral-fork.<N>` suffix and an annotated tag
  whose message states it is an unofficial fork build.
- Record any new fork-only divergence (version scheme, branch tracking change) in `AGENTS-local.md`.
