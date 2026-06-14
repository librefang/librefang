# Current Waypoint

**Active phase:** phase-9-config-store-migration
**Previous phase:** phase-8-fixture-rebuilds
**Updated:** 2026-06-14 by kbd-plan

## Where we are

Phase 9 production cutover is **complete and deployed** (C-001..C-009 merged via
PR #74; root-signin auth fix #78 unblocked the prod config store; budget settings
migrated #84). New sub-phase **C-005d** planned: move the last two file-writing
settings handlers — memory and channels — into the SurrealDB config store, using
the same load-file-first → write-DB → run-from-DB method as budget.

**Security invariant:** no secret VALUE enters the DB. Channel secrets stay in
`~/.librefang/secrets.env` (already separated). `*_env` fields are env-var
pointers (names), which are config, not secrets. The generic `config_set`
endpoint stays unable to write `memory` / `channels` / `sidecar_channels`.

## C-005d — 3 ordered changes

| Change | Title | Depends on |
|---|---|---|
| C-005d.1 | Trusted-section apply path (`TRUSTED_SECTION_KEYS`) + fold budget off the `config_set` allowlist | C-005c |
| C-005d.2 | `PATCH /api/memory/config` → store `memory` + `proactive_memory` overrides | C-005d.1 |
| C-005d.3 | `configure_sidecar_channel` → store `sidecar_channels` override; secrets.env untouched; include-shadow guard kept | C-005d.1 |

## Next step

```
/kbd-execute C-005d.1
```
