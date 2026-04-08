# SPEC-MT-002: API Route Policy Model

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-003-RESOURCE-ISOLATION.md`

---

## Purpose

Define the approved route-policy model for the Qwntik multi-tenant fork.

---

## Policy Classes

### Tenant-Owned

- agents
- memory
- prompts
- channels
- workflows
- goals
- inbox
- media tasks and artifacts

Rules:

- concrete account required
- own-account access succeeds
- cross-tenant access returns `404`

### Admin-Only Infrastructure

- most of system
- most of config
- most of network
- plugin lifecycle

Rules:

- concrete account required
- admin account required
- non-admin tenants receive `403`

### Split-Surface

- providers
- skills
- hands
- budget

Rules:

- shared catalog/discovery behavior must be separated from tenant-owned behavior
- daemon-global mutation/install/reload behavior must be admin-only

### Public

- explicit health/version/openapi/auth bootstrap subset only

---

## Acceptance Criteria

### AC-1: Tenant-owned cross-tenant access is invisible

- Given a tenant-owned resource owned by account A
- When account B requests it
- Then the route returns `404`

### AC-2: Admin-only endpoints are explicit

- Given an admin-only endpoint
- When a non-admin tenant calls it
- Then the route returns `403`

### AC-3: Missing account is rejected

- Given a non-public route
- When the request has no `X-Account-Id`
- Then the request is rejected

### AC-4: Split-surface modules are tested per endpoint class

- Given a module like providers or skills
- Then discovery/catalog behavior, tenant-owned behavior, and admin lifecycle behavior are tested separately

---

## Implementation Guidance

- prefer scoped lookup helpers over post-fetch permissive fallback
- do not encode route policy through legacy fallback macros
- if a module is hard to classify, split the module or split the endpoints
