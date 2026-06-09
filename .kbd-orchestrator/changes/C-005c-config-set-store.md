# Change C-005c — Route generic `config_set` to the config store

**Phase:** phase-9-config-store-migration
**Status:** CODE DONE (2026-06-09)
**Gap:** G-5 (final slice) · **Effort:** L · **Depends on:** C-004, C-005, C-006 · **Agent:** claude

## Goal
Make `POST /api/config/set` (the ~50 allowlisted settings) persist to the DB
config store instead of writing `config.toml`, so they work under a read-only
ConfigMap — closing the last `os error 30` gap from the C-009 cutover.

## Design — config-override merge layer
Unlike MCP servers / default-model (each a single runtime-mutable override
slot), `config_set` values are spread across the whole `KernelConfig` and read
live from `ArcSwap<KernelConfig>`. So this resolves the WHOLE config:
`effective = config.toml ⊕ store_overrides`, and swaps it in.

- **Kernel** `replace_config(KernelConfig, raw_toml) -> ReloadPlan`
  (`config_reload_ops.rs` + `KernelApi`): `reload_config` minus the disk read —
  validate → `build_reload_plan` (diff → hot/restart classification unchanged) →
  ordered atomic swap. **Always swaps** (it is an explicit API-resolved config,
  not a file-watcher reload; gating on `reload.mode` would make DB overrides
  never apply under the default `Off` mode).
- **Storage** one `config_overrides` key = `{ "dotted.path": value }`,
  `source=runtime` (`config_store_overlay.rs`: read/write + `overlay_config_overrides`).
- **Resolution** `resolve_config_with_overrides(config_path, overrides)` parses
  `config.toml` into a `toml_edit` doc, applies each override via the
  `apply_toml_override` helper extracted from `config_set`, deserializes.
- **Write path** `config_set` (surreal): validate (allowlist + schema + business)
  → upsert the override map → resolve → `replace_config`. No `config.toml` write.
  sqlite-only build keeps the legacy `config.toml` + `reload_config` path.
- **Wiring** `overlay_config_overrides` runs in the boot pipeline and the reload
  re-resolve (after the MCP/default-model overlays).

## Security (defense in depth)
`is_writable_config_path` (now `pub(crate)`) gates writes AND is **re-checked at
apply time** in `resolve_config_with_overrides` — a directly-tampered store row
with a blocked path (e.g. an `_env` credential redirect) is skipped with a WARN,
not applied.

## Files
- `crates/librefang-kernel/src/kernel/config_reload_ops.rs`, `.../kernel_api.rs`
- `crates/librefang-api/src/routes/config/mod.rs` (pub(crate) allowlist +
  `json_to_toml_edit_value`; new `apply_toml_override`)
- `crates/librefang-api/src/routes/config/manage.rs` (`config_set` re-target)
- `crates/librefang-api/src/config_store_overlay.rs` (overrides read/write/resolve/overlay)
- `crates/librefang-api/src/server.rs` (boot wiring)
- `crates/librefang-api/tests/config_store_overlay_test.rs` (3 new tests)

## Verification (green)
- `cargo test -p librefang-api --test config_store_overlay_test` → 13 passed,
  incl. C-005c: resolve-applies-allowlisted-skips-blocked (**security**),
  store round-trip, overlay-applies-to-live-kernel (end-to-end via replace_config).
- `cargo check --workspace --lib`; clippy `-p librefang-kernel -p librefang-api --lib`; brand.

## Notes / deferred
- **HTTP `config_set` route test** could not be run: the api integration-test
  harness does not compile on `main` (pre-existing `sync_registry` 3-vs-4 arg
  breakage across ~12 test files — flagged as a separate task). The handler is a
  thin wrapper over the (tested) resolve/store/replace helpers.
- **Behavior change:** `config_set` now applies immediately regardless of
  `reload.mode` (explicit API edits always swap). Previously `Off`/`Restart`
  mode left edits saved-but-unapplied until restart.
- Completes the phase-9 migration: MCP servers, provider-default, AND generic
  config_set are all DB-backed and read-only-config.toml-safe.
