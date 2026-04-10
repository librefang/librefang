# ADR-MT-005: Event Bus Tenant Isolation

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-003-RESOURCE-ISOLATION.md`

---

## Decision

The event bus must become account-aware. Event delivery, event history, and trigger
evaluation must not cross tenant boundaries by default.

This fork does not target a backward-compatible “unscoped event stream” mode for
normal runtime behavior.

---

## Rules

1. Tenant-originated events carry tenant identity
2. Subscribers receive only events visible to their account or explicit admin scope
3. Event history endpoints must filter by account visibility
4. Trigger evaluation must not allow account A events to activate account B automation

---

## Consequences

- event structs or metadata need tenant identity
- publish paths need ownership propagation
- history APIs need account filtering
- “subscribe all” semantics must be limited to explicit admin use, not normal tenant behavior
