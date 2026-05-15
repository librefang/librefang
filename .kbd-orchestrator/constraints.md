# Constraints for the BossFang rebrand-completion phase

## Three-layer rebrand principle (hard rule)

Every change MUST classify into one of three layers. Violating this rule
inflates the per-upstream-merge conflict surface.

| Layer | Rule | Concrete examples in this codebase |
|---|---|---|
| **Internal** | NEVER rename | Cargo crate names (`librefang-cli`, `librefang-runtime`, …), Rust modules (`librefang_runtime::tool_runner`), function names (`librefang_home()`), Python SDK module names (`librefang_sdk`), workspace member paths, test fixture filenames |
| **Boundary** | Additive aliases only | Env vars (`BOSSFANG_HOME` primary + `LIBREFANG_HOME` fallback), config keys, on-disk paths (default `~/.librefang/` stays). Both forms work forever. |
| **Surface** | Full rename OK | CLI banner strings, product name "BossFang", install script echoes, README/docs prose, content-type vendor prefix, release artifact names |

Decision rule when in doubt:
- 0-1 conflicts on rename → Surface
- A handful → Boundary (additive alias)
- Dozens → Internal (leave alone)

## Default-value migration policy

Never rename a default that's already on users' disks:

- Default home dir `~/.librefang/` STAYS (renaming orphans every existing user's config / vault / registry cache).
- Default `dashboard_user`/`dashboard_pass = "librefang"` in `init_default_config.toml` STAYS (arbitrary defaults the user is expected to change).
- `noreply@librefang.ai` git committer in `workspace_setup.rs` STAYS (we don't own `bossfang.ai`).

## Domain ownership

- BossFang-owned: `github.com/GQAdonis/librefang`, `github.com/GQAdonis/librefang-registry`
- Upstream-owned (do not claim): `librefang.ai`, `docs.librefang.ai`, `stats.librefang.ai`, `librefang.com`
- Not owned by anyone yet: `bossfang.ai` — DO NOT use as a real email or URL until registered.

## File-on-disk migration policy

When renaming a string that represents a real artifact on the user's filesystem (e.g., `/Applications/LibreFang.app`, `~/.librefang/`), use the **read-old/write-new shim** pattern:

1. Detect logic checks the new name first, falls back to the legacy name
2. Write logic writes only the new name
3. One-time auto-migration step (rename existing artifact, copy data, etc.)
4. Sunset the legacy fallback after ~2 release cycles when metrics confirm near-zero hits

## Verification gates (each change)

1. `cargo check --workspace --lib` — clean
2. `cargo check -p <changed-crate>` — clean
3. `cargo test -p <changed-crate>` for any test that asserts on changed strings — pass
4. `python3 scripts/enforce-branding.py --check` — exit 0
5. Manual smoke for affected user flow (CLI banner, init wizard run, desktop install) — screenshot in PR

## Forbidden in commit / PR

- Claude / Anthropic / AI attribution
- `--no-verify` or `--no-gpg-sign` flags
- Renaming Layer Internal symbols
- Pushing to main / master without PR review
- `cargo build` in main worktree (hook-blocked)
- Daemon launches (`librefang start`, `bossfang start`) from agent (hook-blocked)
