# Change C-009 — K8s: drop `init-config`, restore read-only ConfigMap (LAST)

**Phase:** phase-9-config-store-migration
**Status:** CODE DONE (2026-06-08) — **APPLY HARD-GATED on C-008 prod import verified**
**Gap:** G-8 · **Effort:** S→M (grew: found + fixed a cutover data-loss bug) · **Depends on:** C-008 verified · **Agent:** claude (deploy = HUMAN)

## Goal
Now that the DB is the runtime source of truth, make `config.toml` a read-only,
GitOps-versioned bootstrap ConfigMap again and remove the `init-config` PVC-copy
workaround. The `os error 30` symptom is structurally gone (UI writes → DB).

## ⚠️ Critical bug found + fixed (R-1 data loss in merged C-008)
Reviewing the deployment for this revert surfaced a **data-loss bug in the
already-merged C-008**: the import wrote prod's live values as `source=bootstrap`.
The prod ConfigMap baseline has **no** `mcp_servers` / `[default_model]`, so the
first post-revert boot would read that empty baseline as bootstrap and, against a
`bootstrap` store row, take the `BootstrapUpdated` branch — **overwriting prod's
MCP servers/provider-default with empty**. The revert would have wiped prod config.

**Fix (code, part of this change):** the import now writes `source=runtime`
(`import_mcp_servers` / `import_default_model` in `config_store_overlay.rs`; the
CLI `import_config_values` uses them). A `runtime` row hits `RuntimeProtected` on
the post-revert boot-seed → prod values are **kept**. Regression test added
(`imported_values_survive_post_cutover_boot_seed`). The import hadn't been run on
prod yet, so no re-import is needed — but the **fixed image must be deployed
before C-008 is run**.

## What landed (code + manifests)
- `crates/librefang-api/src/config_store_overlay.rs`: `seed_value` gains a
  `write_source` param; `import_mcp_servers` / `import_default_model` (runtime).
- `crates/librefang-cli/src/commands/storage.rs`: `import_config_values` →
  runtime import; test asserts `source=runtime` + cutover survival.
  `crates/librefang-cli/src/cli.rs`: long_about wording.
- `k8s/base/bossfang-deployment.yaml`: removed `init-config` initContainer +
  `config-seed` volume; `config.toml` now a **read-only `subPath` ConfigMap
  mount** at `/data/config.toml`; kept `wait-for-surrealdb`; comments updated.
- `k8s/README.md`: config.toml-is-read-only narrative.

## Verification (code, green)
- `cargo test -p librefang-cli --features surreal-backend config_import` —
  idempotent/non-destructive + **post-cutover-survival** regression.
- `cargo test -p librefang-api --test config_store_overlay_test` — seed refactor intact.
- `kubectl kustomize k8s/overlays/production-gke` renders cleanly; the Deployment
  has only `wait-for-surrealdb`, a read-only `subPath` config mount, and the
  `config` ConfigMap volume (verified by parsing the render).

## APPLY runbook (HUMAN — strict order, R-1)
1. Build + push the image from this branch; pin it in
   `k8s/overlays/production-gke/kustomization.yaml`.
2. Roll out the **fixed image** (still with the old initContainer is fine, OR
   apply everything — but the import in step 3 needs the fixed binary).
3. Run the C-008 import (daemon-down) on the prod PVC; confirm
   `GET /api/mcp/servers` + provider-default resolve from the DB.
4. ONLY THEN `kubectl apply -k k8s/overlays/production-gke` (this revert).
5. Confirm UI MCP add/remove succeeds with the read-only ConfigMap mounted; no
   `os error 30`.

## Known limitation
`POST /api/config/set` (generic settings) still writes `config.toml` → it will
fail under the read-only mount until **C-005c**. MCP servers + provider-default
(the high-value paths) work. Document for operators.

## Done when
Human applies the revert after a verified import; UI MCP add succeeds with the
read-only ConfigMap mounted; no `os error 30` in logs.
