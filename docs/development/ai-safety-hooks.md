# AI safety hooks

LibreFang enforces its agent contract in two independent layers.
`.claude/hooks/` runs inside Claude Code as PreToolUse / SessionStart hooks and can only see calls that tool makes.
`scripts/hooks/` runs inside `git` itself, so it catches anything that reaches a commit regardless of which tool produced it.
The summary lives in [`CLAUDE.md`](../../CLAUDE.md); this page is the full enumeration.

## Claude Code hooks (`.claude/hooks/`)

### `forbid-main-worktree.sh` (PreToolUse)

Blocks edits and mutating git commands aimed at the main worktree.
It decides main-vs-linked with `test -d "$(git rev-parse --show-toplevel)/.git"`: git stores the main worktree's `.git` as a directory and a linked worktree's `.git` as a small text file pointing at `<main>/.git/worktrees/<name>`, so the directory test is true exactly in the main worktree.

Do not substitute `git rev-parse --git-dir` — its output is path-shaped and varies with cwd.
Do not path-match against `pwd` either; every developer's clone lives somewhere different.

The hook is a safety net, not a plan.
Run the check yourself as the first action of any task that will edit files.

### `guard-bash-safety.sh` (PreToolUse on Bash)

Blocks:

- Force-push to `main` / `master`, including the `+main` refspec form.
  Requires explicit user OK.
- `--no-verify` / `--no-gpg-sign` on `commit` / `push` / `rebase` / `merge` / `am` / `cherry-pick` / `pull`.
- Staging known-sensitive files: `.env*`, `*.pem`, `*.p12`, `id_rsa`, `id_ed25519`, `credentials*`, `secrets*`, `vault_*.key`.
  Also blocks broad `git add -A` / `git add .` — stage specific paths.
- Commit messages carrying Claude attribution (`Co-Authored-By: Claude`, `🤖 Generated with [Claude Code]`, and similar).
- `rm -rf` against dangerous targets: `/`, `~`, `$HOME`, `target`, `.git`, `/Users`, `/usr`, `/etc`, `/var`, `/opt`, and others.
- Daemon launches: `librefang start`, `target/{debug,release}/librefang start|daemon`.
  The daemon contends with the user's own session on port 4545, and live integration testing is human-only.

### `session-start-worktree-check.sh` (SessionStart)

Emits a banner telling the model whether the session started in the main tree or a linked worktree.
Also warns when `core.hooksPath` has not been pointed at `scripts/hooks/` yet.

## Git-side hooks (`scripts/hooks/`)

These are version-controlled, so `git pull` keeps them current.

### `pre-commit`

Target: under 2 seconds.

- `cargo fmt --check` on staged Rust files.
- CHANGELOG duplicate-`[Unreleased]` guard.
- `(@user)` attribution check on staged additions to `[Unreleased]` **and** on staged `changelog.d/` fragments.
  A fragment in an unrecognised section directory is rejected (#3400), because assembly has no heading to render it under and would drop it silently.
- `gitleaks protect --staged` against `.gitleaks.toml`, soft-warning when gitleaks is not installed.

### `pre-push`

Refuses direct pushes to `main` / `master` and exits in under 100 ms.
Heavy verification (clippy, openapi / SDK drift) deliberately lives in CI rather than gating every push — see #4532 for the rationale.
A maintainer hotfix can skip the branch guard with `LIBREFANG_PREPUSH_SKIP=1`.

### `commit-msg`

Rejects commit messages containing Claude / Anthropic attribution.
This catches heredocs and `git commit -F file`, which the PreToolUse Bash hook cannot see.

Separately rejects a commit whose *author identity* resolves to Claude / Anthropic even when the message itself is clean.
It reads `git var GIT_AUTHOR_IDENT`, so the `GIT_AUTHOR_NAME` / `GIT_AUTHOR_EMAIL` environment overrides are covered too.

## Enabling the git-side hooks

Once per clone:

```bash
just setup        # or: cargo xtask setup
```

This runs `git config core.hooksPath scripts/hooks`.
The `session-start-worktree-check.sh` banner reminds you when a clone has not done it yet.
