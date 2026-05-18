# [High] Git hooks & CI hygiene — rustfmt paths, openapi sha256, missing pre-push, unconsumed `.secrets.baseline`

**Severity:** High · **Domain:** CI / hooks
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `pre-commit` uses unquoted `$STAGED_RS` + `grep`; files with spaces or glob characters get word-split | `scripts/hooks/pre-commit:25-30` |
| sha256sum fallback | The openapi-sha check in `pre-commit` only calls `shasum`; no fallback on `sha256sum`-only environments (typical Linux dev boxes) | `scripts/hooks/pre-commit` (openapi-sha section) |
| missing pre-push file | `CLAUDE.md` and `scripts/hooks/` describe a `pre-push` hook, but no such file exists in the repo — docs and reality disagree | `CLAUDE.md`, `scripts/hooks/` |
| detect-secrets unconsumed | `.secrets.baseline` exists but CI never calls `detect-secrets`; only `pre-commit` soft-warns | `.github/workflows/`, `.secrets.baseline` |

## Why merged

All four are git-hook / CI hygiene items concentrated in `scripts/hooks/` + `.github/workflows/`; a single sweep is more efficient than 4 separate PRs.

## Combined fix plan

1. **(this) NUL-delimited xargs**:
   ```bash
   git diff --cached --name-only --diff-filter=ACM -z \
     | grep -z '\.rs$' \
     | xargs -0 -r rustfmt --check --edition 2021
   ```
2. **(sha256sum fallback) shasum / sha256sum interop**:
   ```bash
   sha256() { command -v sha256sum >/dev/null && sha256sum "$@" || shasum -a 256 "$@"; }
   ```
3. **(missing pre-push file) choose one**: either actually implement `pre-push` and commit it to `scripts/hooks/pre-push`, or delete its description from `CLAUDE.md`. The former is preferred (pre-push runs `cargo clippy --workspace --all-targets -- -D warnings` + openapi/SDK drift).
4. **(detect-secrets unconsumed) wire detect-secrets into CI**: add `.github/workflows/secrets.yml` running `detect-secrets scan --baseline .secrets.baseline`; fail the PR on diff.

## Tests

- `scripts/tests/pre-commit-spaces.sh`: stage `with space.rs` containing bad formatting; the hook rejects.
- `scripts/tests/pre-commit-sha-fallback.sh`: mock `shasum` out of `PATH` → openapi check still runs.
- pre-push file exists + CI assertion.
- A deliberate secret-bearing commit makes the secrets workflow fail.
