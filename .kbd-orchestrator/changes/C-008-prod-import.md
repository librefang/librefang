# Change C-008 — One-time prod `config.toml` → DB import + verify

**Phase:** phase-9-config-store-migration
**Status:** PLANNED
**Gap:** G-9 · **Effort:** M · **Depends on:** C-003, C-005 · **Agent:** claude (verify = HUMAN)

## Goal
Safely move the existing production PVC `config.toml` (with live UI edits) into
the DB store before any K8s ConfigMap revert. Prevents data loss (R-1).

## Files
- `crates/librefang-cli/src/commands/storage.rs` (add `config-import` subcommand)
- `crates/librefang-cli/src/cli.rs` (wire subcommand)

## Tasks
- [ ] CLI `librefang storage config-import [--from <path>]`: reads on-PVC
  `config.toml`, seeds in-scope values as `source="bootstrap"` rev 0, idempotent.
- [ ] HUMAN verification (cluster, port 4545): run in a maintenance pod; confirm
  `GET /api/mcp/servers` returns pre-existing prod entries from the DB. Claude
  prepares exact commands; human executes (no daemon launch from Claude).

## Done when
Human confirms current prod MCP servers + provider default present in DB store
with `source="bootstrap"`.
