# ADR-MT-001: Account Model

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-002-API-AUTH.md`, `ADR-MT-003-RESOURCE-ISOLATION.md`

---

## Decision

This fork models account identity as a first-class isolation boundary for all
non-public app traffic.

The target model is:

- every non-public request carries a concrete account identity
- every tenant-owned resource persists a concrete `account_id`
- admin behavior is explicit and account-based
- there is no intended `system` or missing-account runtime identity for normal app traffic

---

## Why

Qwntik is the only intended deployment target for this fork.

That means the cleanest model is also the correct one:

- one daemon
- many tenant accounts
- strict ownership boundaries
- no product need for single-tenant fallback semantics

---

## Consequences

### Positive

- simpler mental model
- cleaner auth policy
- fewer accidental fallback branches
- clearer test matrix

### Required follow-on work

- route handlers must align to the approved policy classes
- stores must persist and filter by concrete ownership
- docs and tests must stop encoding fallback semantics

---

## Explicit Non-Goals

Not part of the target architecture:

- missing account means admin
- implicit `"system"` ownership for normal runtime traffic
- preserving single-tenant runtime behavior as a product mode for this fork
