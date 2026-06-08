# Change C-005b — provider-default → store + HTTP route tests

**Phase:** phase-9-config-store-migration
**Status:** DONE (2026-06-08)
**Gap:** G-5 (deferred slice) · **Effort:** M · **Depends on:** C-004, C-005 · **Agent:** claude

## Scope (user decision)
Deferred from C-005's "MCP-only" scope: migrate the **provider-default**
(`default_model`) write to the store, AND add **route-level HTTP TestServer**
coverage for the MCP CRUD endpoints. Generic `config_set` is deferred again to
**C-005c** (it needs a new kernel-side config-override merge layer — larger and
security-sensitive; no in-memory path-setter exists today).

## What landed
### Provider-default → config store
- `config_store_overlay.rs`: `DEFAULT_MODEL_KEY`, `write_default_model`,
  `overlay_default_model` (reads the store → sets the kernel's
  `default_model_override_ref()` RwLock — the same runtime-override pattern as
  `effective_mcp_servers`), `seed_default_model`. Extracted shared
  `open_config_store` + generic `seed_value` helpers; `seed_config_store` now
  seeds BOTH `mcp_servers` and `default_model`.
- `providers.rs`: `persist_default_model_durable(state, &dm)` — surreal writes
  the store (`source=runtime`); sqlite-only falls back to the legacy
  `persist_default_model` (now `#[cfg(not(surreal-backend))]`). Both default-model
  writers (`set_default_provider`, `set_provider_key` auto-switch) route through
  it. The in-memory override-set + agent-sync are unchanged, so the switch still
  takes effect immediately; the store makes it survive a restart under a
  read-only config.toml.
- `server.rs` + `routes/config/manage.rs`: `overlay_default_model` added to the
  boot pipeline and the reload re-resolve (after `overlay_mcp_servers`).

### Test-harness isolation
- `librefang-testing/mock_kernel.rs`: `MockKernelBuilder::build()` now points
  `config.storage` at a tempdir-embedded path. The default `StorageConfig` is
  CWD-relative `.librefang/`; without this, tests that touch the config store
  would pollute the repo working directory and contend on the process-global
  embedded lock.

### HTTP route coverage
- `tests/mcp_http_crud_test.rs` (new): full-router (`build_router`) `oneshot`
  POST → GET → DELETE → GET for `/api/mcp/servers`, asserting the add is listed,
  config.toml is never created, and the server is gone after delete.

## Files
- `crates/librefang-api/src/config_store_overlay.rs`, `src/server.rs`,
  `src/routes/config/manage.rs`, `src/routes/providers.rs`
- `crates/librefang-testing/src/mock_kernel.rs`
- `crates/librefang-api/tests/{config_store_overlay_test,mcp_http_crud_test,providers_routes_test}.rs`

## Verification (green)
- `config_store_overlay_test` → 10 passed (2 new default_model: overlay-into-override;
  seed-then-runtime-protected).
- `mcp_http_crud_test` → 1 passed.
- `providers_routes_test` → 37 passed (the `#5116` config.toml-rewrite test gated
  to sqlite-only; the "absent" test now asserts store/override behavior under
  surreal).
- `cargo check --workspace --lib`; clippy `-p librefang-api -p librefang-testing --lib`;
  brand audit — all clean.

## Notes / deferred
- **C-005c**: generic `config_set` store migration (needs a kernel config-override
  merge layer applied at load/reload; security review for section-wholesale writes).
- Pre-existing, unrelated: `api_integration_test` does not compile on the base
  (`registry_sync::sync_registry` 3-vs-4 arg mismatch) — untouched by this change.