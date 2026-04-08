# ADR-MT-004: Data & Memory Isolation

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-001-ACCOUNT-MODEL.md`, `ADR-MT-003-RESOURCE-ISOLATION.md`

---

## Decision

Tenant-owned data and memory must be isolated by explicit account ownership at the
storage layer, not only at the HTTP layer.

This fork does not target a `"system"` or `AccountId(None)` data-access mode for
normal runtime behavior.

---

## Rules

1. Tenant-owned tables persist concrete `account_id`
2. Memory search, recall, insert, delete, and aggregation paths filter by account ownership
3. Cross-tenant invisibility applies at the store/query layer, not just the route layer
4. Any shared/global memory behavior must be intentionally modeled and narrowly documented

---

## Consequences

- store APIs need explicit account-aware entry points
- prompt, workflow, channel, and memory stores must carry ownership through persistence
- route-level checks alone are insufficient

---

## Rejected Model

Rejected for target architecture:

- “system account sees all data”
- query logic that skips filtering when account is missing
- normal runtime writes without concrete tenant ownership
