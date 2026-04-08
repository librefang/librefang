# PENDING-WORK: Multi-Tenant Remaining Work

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `CURRENT-CODE-AUDIT.md`, `ROUTE-POLICY-MATRIX.md`

---

## Purpose

Track only the real remaining work on this branch.

If a module is already converged, it should not stay here as generic “drift.”

---

## Real Open Items

### 1. Inbox product decision

- keep `/api/inbox/status` as daemon diagnostics permanently
- or design a real tenant-owned inbox data model and API surface

### 2. Channel QR / session ownership modeling

- channel config/secrets/runtime ownership is converged
- ingress binding v1 is converged at integration-instance scope
- QR/bootstrap/session ownership still needs explicit product and storage rules

### 3. Shared integration binding beyond integration-instance scope

- current model is `integration instance -> account_id`
- future work, only if product requires it:
  - shared chat/user/thread binding
  - rebinding rules
  - conflict and support workflows

### 4. Broader tenant-owned skill content model

- hands are tenant-owned at the instance/settings layer
- global catalog and install/reload lifecycle are intentionally split
- tenant-authored skill content/overlays beyond hands remain future product work if needed

### 5. Residual `AccountId(None)` migration debt

- active target behavior is explicit concrete account scope
- any remaining `AccountId(None)` behavior should be reduced to:
  - explicit admin/global compatibility paths
  - or removed entirely from tenant-facing logic

---

## Intentionally Not Pending

- channels config/secrets tenant ownership
- channels ingress binding v1
- channels runtime adapter keying / reload cleanup / multiplicity enforcement
- workflow tenant-owned definitions/runs/schedules/triggers
- goal tenant-owned store and tool/runtime hardening
- provider tenant-owned keys/URLs/defaults and fail-closed tenant runtime resolution
- budget split-surface enforcement
- network split-surface enforcement
- system public/admin/tenant-derived classification
- workflow templates remaining admin/global
- preserving single-tenant runtime semantics
- implicit `system` fallback as target behavior
- missing-account-as-admin behavior
