# [Medium] CI/hooks Medium roundup — guard-bash silent failure, forbid-main glob, trufflehog from main, `pull_request_target`, shard balance, xtask hookspath, dependabot

**Severity:** Medium · **Domain:** CI / hooks
**Status:** Merges 6 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `guard-bash-safety.sh` falls back to `|| true` when `lib/check-bash-rules.py` is missing, silently allowing through | `.claude/hooks/guard-bash-safety.sh:13-37` |
| forbid-main glob | `forbid-main-worktree.sh` matches paths via a fuzzy glob; specific aliases / symlinks bypass it | `.claude/hooks/forbid-main-worktree.sh` |
| trufflehog from main | The `trufflehog` installer fetches from the `main` branch without a pinned SHA | trufflehog install step in `.github/workflows/*` |
| pull_request_target | `pull_request_target` workflows consume fork PR body without restriction — injection risk | `pull_request_target` workflow in `.github/workflows/*` |
| shard balance | CI shard balance is hash-stable but unmeasured — actual shard wallclocks may diverge dramatically | shard config in `.github/workflows/test.yml` |
| xtask setup overwrites | `xtask setup` overwrites `core.hooksPath`, clobbering a user's existing git config | `xtask/src/main.rs:setup` |
| dependabot title classifier | Dependabot auto-merge only looks at PR title categories, mis-classifying patch / minor bumps | `.github/dependabot.yml` + auto-merge action |

## Why merged

All seven are CI/hooks hygiene; auditing `.github/workflows/` + `.claude/hooks/` + `xtask` in one pass closes them out together.

## Combined fix plan

1. **(this) Fail closed**:
   ```bash
   [ -f "$LIB" ] || { echo "missing $LIB" >&2; exit 1; }
   ```
2. **(forbid-main glob) Use `git rev-parse --show-toplevel` + `[ -d "$top/.git" ]`** rather than glob path matching.
3. **(trufflehog from main) Pin trufflehog to a specific release tag + SHA**: `uses: trufflesecurity/trufflehog@v3.X.Y` (never `@main`).
4. **(pull_request_target) Restrict `pull_request_target`**: (a) only with `permissions: read-all`; (b) treat every string sourced from the PR body as untrusted — never splice into shell commands; (c) gate privileged steps behind base-repo collaborators (`if: github.event.pull_request.author_association == 'COLLABORATOR'`).
5. **(shard balance) Measure shards**: collect each shard's elapsed time, export as a metric; fail the PR if deviation > 2×.
6. **(xtask setup overwrites) Make `xtask setup` idempotent + interactive**: if `core.hooksPath` is set to a non-default, emit `warn!` and require `--force`.
7. **(dependabot title classifier) Multi-dimensional classification**: use `dependabot.yml`'s `groups` + `update-type` constraint + check PR labels (`patch` / `minor` / `major`); fall back to title only as a last resort.

## Tests

- Each hook change verified in dry-run mode; workflows tested locally with `act`.
- (shard balance) Per-shard duration histogram uploaded as a CI artifact, diffed against threshold.
- (xtask setup overwrites) Mock home directory verifies the `xtask setup --force` vs no-force behaviour difference.
