# Change C-008 — One-time prod `config.toml` → DB import + verify

**Phase:** phase-9-config-store-migration
**Status:** CODE DONE (2026-06-08) · **HUMAN CLUSTER VERIFY PENDING**
**Gap:** G-9 · **Effort:** M · **Depends on:** C-003, C-005 · **Agent:** claude (verify = HUMAN)

## Goal
Safely move the existing production PVC `config.toml` (with live UI edits) into
the DB store before any K8s ConfigMap revert. Prevents data loss (R-1).

## What landed (code)
- `crates/librefang-cli/src/commands/storage.rs`: `cmd_storage_config_import`
  + testable core `import_config_values`. Reads the in-scope `config.toml`
  values (`mcp_servers`, `default_model`) and seeds them as `source=bootstrap`
  rev 0 by calling the daemon's OWN boot-seed functions (`seed_mcp_servers` /
  `seed_default_model`) — zero behaviour drift. Idempotent (re-run → `Unchanged`)
  and **non-destructive** (a `runtime`/UI row → `RuntimeProtected`, never
  overwritten). `--dry-run` previews store state; refuses while the daemon holds
  the embedded lock.
- `cli.rs`: `StorageCommands::ConfigImport { from, dry_run }`. `main.rs`: dispatch.
- Incidental: fixed a pre-existing stale unit-test call
  (`detached_daemon_args(.., None)` → 1-arg) that blocked the whole
  `librefang-cli` bin test target from compiling.

## Verification (code, green)
- `cargo test -p librefang-cli --features surreal-backend config_import` →
  `config_import_is_idempotent_and_non_destructive` (fresh→Seeded; re-run→
  Unchanged; after UI write→RuntimeProtected for both keys).
- `cargo check -p librefang-cli --features surreal-backend`;
  `cargo clippy -p librefang-cli --features surreal-backend --bins`; brand clean.

## HUMAN verification (cluster — Claude must NOT launch the daemon)
Import needs exclusive store access, so the daemon is down during it.
```bash
# 1. Free the PVC's embedded lock.
kubectl -n <ns> scale deploy/bossfang --replicas=0
kubectl -n <ns> wait --for=delete pod -l app=bossfang --timeout=120s
# 2. Dry-run, then apply, from a pod mounting the SAME PVC + config.toml
#    (or `kubectl exec` into a held pod):
librefang storage config-import --dry-run
librefang storage config-import
# 3. Bring the daemon back.
kubectl -n <ns> scale deploy/bossfang --replicas=1
# 4. Confirm pre-existing prod values now resolve from the DB store:
curl -s http://<svc>/api/mcp/servers | jq '.configured[].name'
curl -s http://<svc>/api/providers   | jq '.[] | select(.is_default)'
```

## Done when
Human confirms current prod MCP servers + provider default present in the DB
store (via the API after restart). Only then is **C-009** (ConfigMap revert) safe.
