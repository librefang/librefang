# `LIBREFANG_VAULT_KEY` environment variable silently overrides the master key in the OS keyring

**Severity:** Low
**Category:** Secrets & credential handling
**Labels:** `security`, `secrets`, `defense-in-depth`, `low`

## Affected files
- `crates/librefang-extensions/src/vault.rs:705-714` (`resolve_master_key`), comment at `:700-706`

## Description

The comment justifies env-over-keyring as "for test stability." In production:

- Any sibling process under the same UID can read `/proc/<pid>/environ` and recover the base64 master key;
- When both env and keyring are set and disagree, env silently wins — an operator who intends keyring to take priority gets no warning;
- There's no audit signal indicating which source was used.

## Recommendation

At startup, resolve both keys:

- If both are present and **disagree** → `WARN` log, and depending on a config switch, fail closed;
- If only one is present or both agree → emit `info!` naming the chosen source (with the value redacted).

Test stability can be addressed by a `cfg(test)`-only override rather than hard-coding env precedence on the production path.
