# Change C-005d.2 — Persist memory settings to the DB store

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-d1/G-d2 · **Effort:** S · **Depends on:** C-005d.1 · **Agent:** claude

## Goal
`PATCH /api/memory/config` persists the `[memory]` + `[proactive_memory]` settings
to the DB config store instead of writing `config.toml`, so they work under the
read-only ConfigMap.

## Design
`memory_config_patch` (`crates/librefang-api/src/routes/memory.rs:1592`) reads
`config.toml`, edits the `[memory]` (`embedding_provider`, `embedding_model`,
`embedding_api_key_env`, `decay_rate`) and `[proactive_memory]` tables, writes
the whole file, and reloads. Under `surreal-backend`, instead:

1. Build the resulting `[memory]` and `[proactive_memory]` tables (config.toml ⊕
   patch) — keep the existing merge logic.
2. Store them as two `config_overrides` entries `memory` + `proactive_memory`
   (`write_config_overrides`).
3. `resolve_config_with_overrides` (which now applies the trusted sections from
   C-005d.1) → `replace_config`.

sqlite-only build keeps the legacy `config.toml` + `reload_config` path.

`embedding_api_key_env` is an env-var POINTER (not a secret value), so storing it
is safe; the actual key stays in the env var. No secret VALUE enters the store.

`GET /api/memory/config` reads `kernel.config_ref()` (the live overlay-resolved
config) so reads are correct after `replace_config` with no further change.

## Files
- `crates/librefang-api/src/routes/memory.rs` (`memory_config_patch`)
- `crates/librefang-api/tests/config_store_overlay_test.rs` (tests)

## Tasks
- [ ] cfg-branch `memory_config_patch`: surreal → store `memory` +
  `proactive_memory` overrides + resolve + `replace_config`; sqlite → existing.
- [ ] Test: PATCH-equivalent override → store → live config reflects it →
  survives restart via `overlay_config_overrides`; `config.toml` untouched.

## Done when
`cargo test -p librefang-api --test config_store_overlay_test` green; under
surreal `PATCH /api/memory/config` no longer writes `config.toml`.
