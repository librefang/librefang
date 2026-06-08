# Current Waypoint

**Active phase:** phase-9-config-store-migration
**Previous phase:** phase-8-fixture-rebuilds
**Updated:** 2026-06-06 by kbd-plan

## Where we are

Phase 9 assessed and planned. Verdict: **proceed with corrected mechanism** —
move MCP server registrations and the UI-mutable config subset into a SurrealDB
`config_store` table (embedded + remote), seeded once from bootstrap config, with
**content-hash + revision + per-key provenance** conflict handling (NOT file
mtime, NOT whole-file "newer wins"). Bootstrap/secret/storage-connection config
stays in file+env. K8s ConfigMap revert is last and hard-gated on a verified
prod import.

## 9 ordered changes

| Change | Title | Depends on |
|---|---|---|
| C-001 | `config_store` SurrealDB migration | none |
| C-002 | `ConfigStore` trait + SurrealDB impl | C-001 |
| C-003 | Seed-once + content-hash/revision/provenance merge | C-002 |
| C-004 | Kernel effective-config read path (bootstrap ⊕ DB) | C-002 |
| C-005 | Re-target write endpoints to `ConfigStore` | C-002, C-004 |
| C-006 | Reload path reads DB store | C-004 |
| C-007 | Determinism `ORDER BY` + regression test | C-004 |
| C-008 | One-time prod `config.toml` → DB import + verify | C-003, C-005 |
| C-009 | K8s: drop `init-config`, restore read-only ConfigMap | C-008 verified |

## Next step

```
/kbd-execute C-001
```
