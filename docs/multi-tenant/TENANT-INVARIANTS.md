# TENANT-INVARIANTS: Qwntik Multi-Tenant Runtime Rules

**Status:** Canonical
**Date:** 2026-04-08
**Author:** Engineering
**Scope:** This fork of LibreFang is operated only as the Qwntik multi-tenant runtime

---

## Purpose

This document is the top-level policy source for the multi-tenant refactor.
ADRs, SPECS, PLANs, tests, and code must align to these invariants.

This fork is **not** designed around backward-compatible single-tenant behavior.
The target architecture is **always multi-tenant**.

---

## Non-Negotiable Invariants

1. Every non-public request MUST carry a concrete `X-Account-Id`.
2. Every non-public request carrying account context MUST pass account signature verification.
3. Missing `X-Account-Id` is always an error on tenant-facing routes.
4. `AccountId(None)` is not a valid runtime identity for normal app traffic.
5. There is no implicit `system` fallback for reads or writes in the target architecture.
6. Every tenant-owned resource MUST store a concrete `account_id`.
7. Cross-tenant access to tenant-owned resources MUST return `404 Not Found`.
8. Admin-only endpoints are privileged by endpoint class, not by resource ownership; unauthorized tenants MUST receive `403 Forbidden`.
9. Public endpoints MUST be explicitly enumerated and kept small.
10. Shared/global artifacts are allowed only when intentionally modeled as global.
11. If a route family mixes tenant content and infrastructure concerns, it MUST be split by behavior or split by endpoint family.
12. Tests are the enforcement mechanism for these rules; docs describe the target, but tests define the guardrail.

---

## Operating Model

This fork has one supported operating model:

- Qwntik issues signed requests with a concrete account identity
- LibreFang validates that identity
- Handlers operate strictly within that account namespace unless the endpoint is explicitly public or admin-only

The following are **not** part of the target architecture:

- implicit single-tenant request handling
- normal API traffic without `X-Account-Id`
- `system` account semantics for app-facing behavior
- “legacy mode” as a reason to keep permissive handler branches

If temporary compatibility branches remain in code during migration, they are migration debt, not architecture.

---

## Policy Classes

### Tenant-Owned

These resources belong to exactly one account. Reads and writes are scoped to that account.

- agents
- memory
- prompts
- channels
- workflows
- goals
- inbox
- media tasks and media artifacts

Rules:

- require concrete `account_id`
- cross-tenant access returns `404`
- resource creation persists `account_id`

### Admin-Only Infrastructure

These endpoints operate on daemon-global infrastructure and are not modeled as tenant-owned.

- most of `system`
- most of `config`
- most of `network`
- plugin lifecycle

Rules:

- require concrete `account_id`
- account must belong to configured admin accounts
- unauthorized tenants receive `403`

### Split-Surface Families

These modules mix global catalog behavior with tenant behavior and must not use one policy for all endpoints.

- providers
- skills
- hands

Rules:

- catalog/discovery endpoints may be shared or tenant-readable
- daemon-global install/reload/mutation endpoints are admin-only
- tenant-authored or tenant-configured content is tenant-owned

---

## Response Semantics

### `404 Not Found`

Use for:

- wrong-tenant access to tenant-owned resources
- resource lookups where existence must remain invisible across tenants

### `403 Forbidden`

Use for:

- admin-only endpoints accessed by non-admin tenant accounts
- endpoint classes that are privileged regardless of resource ownership

---

## Documentation Impact

All multi-tenant ADRs, SPECS, and PLANs must reflect:

- Qwntik-specific deployment assumptions
- always-multi-tenant operation
- no single-tenant/system fallback in the target design
- concrete account identity on all non-public routes

Any document that still describes `AccountId(None)` or `"system"` as normal runtime behavior is stale and must be corrected.
