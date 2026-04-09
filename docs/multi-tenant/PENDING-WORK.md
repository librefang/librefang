# PENDING-WORK: Multi-Tenant Remaining Work

**Status:** Current
**Date:** 2026-04-09
**Related:** `TENANT-INVARIANTS.md`, `CURRENT-CODE-AUDIT.md`, `ROUTE-POLICY-MATRIX.md`

---

## Purpose

Track only the real remaining work on this branch.

If a module is already converged, it should not stay here as generic “drift.”

---

## Real Open Items

### 1. Shared integration binding beyond integration-instance scope

- current model is `integration instance -> account_id`
- future work, only if product requires it:
  - shared chat/user/thread binding
  - rebinding rules
  - conflict and support workflows

### 2. Broader tenant-owned skill content model

- hands are tenant-owned at the instance/settings layer
- global catalog and install/reload lifecycle are intentionally split
- tenant-authored skill content/overlays beyond hands remain future product work if needed

### 3. Residual `AccountId(None)` migration debt

- active target behavior is explicit concrete account scope
- admin-only route guards now require concrete configured admin accounts
- remaining debt should be reduced to:
  - the temporary `require_admin()` compatibility allowance for legacy operator
    callers on documented admin/global paths
  - the small set of `system` admin handlers that still branch on `account.0`
    only behind that compatibility allowance
  - extractor-level missing-header representation
  - legacy storage round-trip helpers such as `"system"`
  - stale comments/tests that still describe compatibility behavior too casually

### 4. Knowledge-graph caller-context wiring

- tenant-facing memory relations routes are now account-scoped and the knowledge
  graph schema persists `account_id`
- the remaining gap is narrower:
  generic runtime/kernel knowledge graph APIs still lack caller account context
  and therefore remain intentionally fail-closed until they can be scoped
  correctly end to end

---

## Intentionally Not Pending

- channel QR/bootstrap/session ownership at integration-instance scope
  - implementation now exists in code for the ownership-record foundation
  - WeChat bootstrap/API wiring is owned and instance-targeted
  - WhatsApp gateway/bootstrap/API wiring is owned and instance-targeted
  - see `ADR-MT-006-CHANNEL-BOOTSTRAP-OWNERSHIP.md`,
    `SPEC-MT-005-CHANNEL-BOOTSTRAP-SESSION-OWNERSHIP.md`, and
    `PLAN-MT-003-CHANNEL-BOOTSTRAP-HARDENING.md`
- inbox operator/admin intake infrastructure, including `/api/inbox/status`
  admin-only diagnostics
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
- any future tenant inbox product model
- preserving single-tenant runtime semantics
- implicit `system` fallback as target behavior
- missing-account-as-admin behavior
