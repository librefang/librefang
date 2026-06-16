# Change C-005d.2 — Persist memory settings to the DB store

**Phase:** phase-9-config-store-migration
**Status:** CODE_DONE (2026-06-15)
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
- [x] cfg-branch `memory_config_patch`: surreal → `memory_config_patch_surreal`
  builds `memory` + `proactive_memory` from the LIVE config ⊕ patch (so PATCHes
  accumulate), stores both as trusted-section overrides, resolves + validates +
  `replace_config`; sqlite → existing config.toml path unchanged.
- [x] Test `memory_and_proactive_overrides_resolve_into_config`: both sections
  fold into the merged config; `config.toml` untouched; `embedding_api_key_env`
  (env-var pointer) round-trips.
- [x] Root-cause fix in `json_to_toml_edit_value`: a JSON `null` inside an object
  is omitted (absent key) instead of `""`, so `Option<…>` fields in a
  whole-section override round-trip (was: `embedding_dimensions = ""` → parse
  error). Fixes the whole class of Option-bearing trusted sections.

## Done when ✓
`config_store_overlay_test` green (16 passed incl. the memory test); config unit
tests 18 passed; clippy + branding clean. Under surreal `PATCH /api/memory/config`
no longer writes `config.toml`.
