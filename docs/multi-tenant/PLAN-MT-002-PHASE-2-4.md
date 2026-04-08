# PLAN-MT-002: Remaining Multi-Tenant Convergence

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-003-RESOURCE-ISOLATION.md`, `ADR-MT-004-DATA-MEMORY-ISOLATION.md`

---

## Objective

Finish converging the fork onto the invariant multi-tenant model.

---

## Workstreams

### 1. Route Policy Reconciliation

- convert tenant-owned modules from temporary admin clamps to real tenant scoping where required
- split mixed modules by policy class
- keep admin-only surfaces explicitly admin-only

### 2. Store/Data Convergence

- add concrete ownership to remaining stores
- eliminate runtime dependency on fallback ownership semantics
- make query-layer filtering explicit

### 3. Event Bus / Background Flows

- propagate account identity through event, trigger, queue, and background paths

### 4. Test Backfill

- lock route policy into integration tests
- add missing cross-tenant denial coverage
- remove tests that encode obsolete fallback behavior

---

## Completion Criteria

- docs describe one runtime model
- routes match the approved policy matrix
- stores persist and filter by concrete account identity
- tests enforce the model end to end
