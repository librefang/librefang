# Change C-005d.1 — Trusted-section apply path in config-override resolve

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-d1/G-d4 · **Effort:** S · **Depends on:** C-005c · **Agent:** claude

## Goal
Let dedicated typed handlers persist a whole section to the DB config store
without going through the generic `config_set` allowlist, while keeping
`config_set` itself unable to write those sections.

## Design
`resolve_config_with_overrides` currently applies an override only when its key
passes `is_writable_config_path` (the generic `config_set` gate). Add a
`TRUSTED_SECTION_KEYS: &[&str] = &["budget", "memory", "proactive_memory",
"sidecar_channels"]`. An override applies when its key is allowlisted **OR** in
`TRUSTED_SECTION_KEYS`. The final `deserialize → validate_config_for_reload` at
resolve/replace time remains the guard.

This UNIFIES the budget special-case (C-005d-budget shipped via the exact
allowlist in PR #84) into the trusted set: **remove the `"budget"` entry from
`is_writable_config_path`'s EXACT list** and add it to `TRUSTED_SECTION_KEYS`, so
`config_set` can no longer write `budget` either (only the typed budget handler
can). `memory` / `channels` / `sidecar_channels` are deliberately NOT added to
`is_writable_config_path` — the generic `config_set` surface stays locked
(`_env` / depth-2 protections intact for the untrusted caller).

## Files
- `crates/librefang-api/src/config_store_overlay.rs`
  (`resolve_config_with_overrides` + `TRUSTED_SECTION_KEYS`)
- `crates/librefang-api/src/routes/config/mod.rs` (remove the `"budget"` EXACT
  entry added in PR #84)

## Tasks
- [ ] Add `TRUSTED_SECTION_KEYS` and the OR-gate in `resolve_config_with_overrides`.
- [ ] Remove `"budget"` from `is_writable_config_path` EXACT; confirm
  `config_set("budget", …)` now returns 403 (test).
- [ ] Tests: a trusted key (`budget`) applies via resolve; a hand-planted blocked
  path (`channels.x.token_env`) is still skipped; a non-trusted, non-allowlisted
  key (`vault`) is still skipped.

## Done when
`cargo test -p librefang-api --test config_store_overlay_test` green incl. the
trusted-apply + still-skips-blocked cases; budget round-trip still passes via the
trusted path (not the allowlist).
