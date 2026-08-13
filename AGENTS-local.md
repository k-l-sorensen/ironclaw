# Fork-Specific Agent Rules (k-l-sorensen/ironclaw)

## Fork Identity

This is the **k-l-sorensen/ironclaw** fork of nearai/ironclaw.
**NEVER work on upstream nearai/ironclaw.**

All commits, tags, branches, and releases must go to `origin` (github.com/k-l-sorensen/ironclaw), never to `upstream` (github.com/nearai/ironclaw).

## Active Local Divergences

- **Mistral provider**: Custom `reasoning_effort=high` configuration for Mistral models
- **Release convention**: Tag-driven cargo-dist with `-mistral-fork.N` suffix (see fork-release skill)
- **Branch tracking**: `main` follows fork (`origin/main`), not upstream
- **WiX files**: Committed `wix/main.wxs` must reference fork, not upstream (checked in fork-release skill)
- **PR workflow**: Use feature branches and PRs to trigger GitHub Actions checks, even on this fork
- **gh CLI default**: Continuously verify `gh repo view` shows k-l-sorensen/ironclaw; set with `gh repo set-default k-l-sorensen/ironclaw` if needed

## Git Workflow Maintenance

Use this checklist when asked to "fix git settings" or a push misbehaves:

```bash
git remote -v                                   # origin=fork, upstream=nearai
git remote get-url origin | grep k-l-sorensen   # origin must be the fork
git config --get-all credential.'https://github.com'.helper  # should be 'gh auth git-credential'
gh auth status                                  # gh logged in, scope includes 'repo'/'workflow'
git branch -vv                                  # see what each branch tracks
```

Common repairs:
- **Push hangs / asks for a password** → `gh auth setup-git` (routes HTTPS auth through gh)
- **Wrong origin** → `git remote set-url origin https://github.com/k-l-sorensen/ironclaw.git`
- **Missing upstream** → `git remote add upstream https://github.com/nearai/ironclaw.git`
- **`main` should follow the fork** → `git branch --set-upstream-to=origin/main main`
- **Accidental tag created** → delete locally and on the fork: `git tag -d "$TAG" && git push origin :refs/tags/"$TAG"`

## Skills

Skills in `.claude/skills/` are automatically loaded by OpenCode. Available skills:
- `architecture-video` - Generate/update architecture overview video
- `fork-release` - Guide cutting marked release tags on this fork
- `ironclaw-reborn-architecture-review` - Boundary/abstraction changes
- `ironclaw-reborn-orientation` - Starting work, tracing request flows
- `ironclaw-reborn-skill-maintainer` - Editing guidance files
- `ironclaw-reborn-testing` - Test tiers and harness
- `mintlify-docs` - Build/maintain Mintlify documentation
- `railway-test` - Test PRs on Railway
- `reborn-extension-surfaces` - Adding/changing extensions
- `reborn-feature` - Build user-facing features
- `thermo-nuclear-code-quality-review` - Strict maintainability review

## Path-Scoped Rules (Lazy Loading)

Load these rule files **only when working with matching file patterns**:

| When working with | Load rule file |
|-------------------|----------------|
| Any `Cargo.toml` | `.claude/rules/cargo-features.md` |
| Rust files in `crates/**` | `.claude/rules/error-handling.md`, `.claude/rules/testing.md`, `.claude/rules/database.md`, `.claude/rules/architecture.md`, `.claude/rules/type-placement.md`, `.claude/rules/types.md`, `.claude/rules/review-discipline.md` |
| Event system (`crates/events/...`) | `.claude/rules/gateway-events.md` |
| Extension registry/support | `.claude/rules/lifecycle.md` |
| Network/secrets/safety | `.claude/rules/safety-and-sandbox.md` |
| Skills domain | `.claude/rules/skills.md` |
| Capability/host/runtime | `.claude/rules/tools.md`, `.claude/rules/tool-evidence.md` |

**Instruction for OpenCode**: When you begin work on files matching the patterns above, use the Read tool to load the corresponding rule file(s) from `.claude/rules/` and follow them as mandatory instructions.
