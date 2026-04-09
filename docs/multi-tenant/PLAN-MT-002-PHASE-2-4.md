# PLAN-MT-002: Phase 2-4 Historical Convergence Summary

**Status:** Historical plan, normalized 2026-04-08
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-003-RESOURCE-ISOLATION.md`, `ADR-MT-004-DATA-MEMORY-ISOLATION.md`

---

## Purpose

Record what Phase 2-4 should now be understood as, without leaving old plan text
that reads like current unresolved architecture.

---

## What Phase 2-4 Delivered

### 1. Route Policy Reconciliation

- converged the main tenant-owned slices
- classified mixed modules by endpoint class instead of one blanket policy
- kept intentionally global/admin infrastructure explicitly global/admin

### 2. Store/Data Convergence

- persisted concrete ownership across the core tenant-owned paths
- reduced fallback ownership semantics to residual compatibility debt
- moved the branch toward explicit query/store scoping

### 3. Event Bus / Background Flows

- propagate account identity through event, trigger, queue, and background paths

### 4. Test Backfill

- locked major route-policy expectations into tests
- expanded cross-tenant denial coverage for converged slices
- identified remaining fallback-oriented tests as debt, not target behavior

## What Is Still Actually Open

- channel QR/session ownership modeling
- shared integration user/chat/thread binding beyond integration-instance scope
- broader tenant-owned skill content beyond hands, only if product requires it
- residual `AccountId(None)` compatibility debt

---

## Completion State

- the historical phase plan should now be read alongside
  `CURRENT-CODE-AUDIT.md` and `PENDING-WORK.md`, which describe the live branch
  state
- this file is historical context, not the current backlog
