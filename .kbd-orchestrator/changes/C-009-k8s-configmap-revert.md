# Change C-009 — K8s: drop `init-config`, restore read-only ConfigMap (LAST)

**Phase:** phase-9-config-store-migration
**Status:** PLANNED — HARD-GATED on C-008 verified in prod
**Gap:** G-8 · **Effort:** S · **Depends on:** C-008 verified · **Agent:** claude (deploy = HUMAN)

## Goal
Now that the DB is the runtime source of truth, make `config.toml` a read-only,
GitOps-versioned bootstrap ConfigMap again and remove the `init-config` PVC-copy
workaround. The `os error 30` symptom is structurally gone (UI writes → DB).

## Files
- `k8s/base/bossfang-deployment.yaml` (remove `init-config` initContainer;
  restore read-only ConfigMap subPath mount; rename volume back to `config`)
- `k8s/overlays/production-gke/patches/bossfang-pvc-storage.yaml` (PVC keeps
  `/data` for embedded SurrealDB + npm/uv caches; size unchanged)

## Tasks
- [ ] Revert to read-only ConfigMap mount (bootstrap-defaults only).
- [ ] DO NOT MERGE until C-008 confirms prod data is in the DB.

## Done when
`kubectl apply -k k8s/overlays/production-gke` rolls out; UI MCP add succeeds
with read-only ConfigMap mounted; no `os error 30` in logs.
