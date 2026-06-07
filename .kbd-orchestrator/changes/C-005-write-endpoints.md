# Change C-005 — Re-target write endpoints to `ConfigStore`

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-5 · **Effort:** M · **Depends on:** C-002, C-004 · **Agent:** claude

## Goal
MCP/model/config-set writes go to the DB store (`source="runtime"`), not TOML.
Validation unchanged. Out-of-scope writes keep their file/env path.

## Files
- `crates/librefang-api/src/routes/skills.rs` (`upsert_mcp_server_config`, `remove_mcp_server_config`)
- `crates/librefang-api/src/routes/providers.rs` (`persist_default_model`)
- `crates/librefang-api/src/routes/config.rs` (`config_set` allowlisted subset)

## ARCHITECTURE NOTE (revised per D9/D10, C-004)
Writes go through `SurrealConfigStore` opened on the **shared pool**
(`librefang_storage::shared_pool()`), not a fresh pool. After writing, push the
new effective list into the kernel via `replace_effective_mcp_servers` (added in
C-004) and trigger reconnect — instead of editing TOML + `reload_config`.

## Tasks
- [ ] Each write → `ConfigStore.upsert(..., source="runtime", ...)` via the
  shared pool. Keep allowlist, transport, duplicate-name validation.
- [ ] Preserve `config_write_lock` serialization (or DB transaction).
- [ ] Secrets/auth/storage writes unchanged (file/env).
- [ ] MANDATORY `#[tokio::test]` against `TestServer` (#3721): `POST /api/mcp/servers`
  → read back from DB → survives simulated reload.

## Done when
`cargo test -p librefang-api` green incl. new TestServer case;
`POST /api/mcp/servers` no longer touches `config.toml`.
