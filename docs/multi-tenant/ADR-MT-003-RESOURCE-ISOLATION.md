# ADR-MT-003: Resource Isolation Strategy

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-001-ACCOUNT-MODEL.md`, `ADR-MT-002-API-AUTH.md`

---

## Decision

The API surface is classified into four policy classes.

### 1. Tenant-Owned

- agents
- memory
- prompts
- channels
- workflows
- goals
- media tasks and artifacts

Rules:

- concrete account required
- ownership persisted
- cross-tenant access returns `404`

### 2. Admin-Only Infrastructure

- inbox operator/admin intake infrastructure and diagnostics
- most of system
- most of config
- most of network
- plugin lifecycle

Rules:

- concrete account required
- caller must be an admin account
- unauthorized tenants receive `403`

### 3. Split-Surface Modules

- providers
- skills
- hands
- budget

Rules:

- separate shared catalog behavior from tenant-owned behavior
- separate daemon-global lifecycle behavior from tenant behavior

### 4. Public

- explicit health/version/openapi/auth bootstrap subset

Rules:

- explicitly enumerated
- kept small

---

## Consequences

- blanket fallback patterns are not acceptable end-state design
- “extractor present but unused” is not isolation
- temporary admin clamps are acceptable as a short-term safety move, but must
  not be mistaken for final product behavior where tenant ownership is intended
- inbox is not one of those temporary clamps in the current architecture; it is
  currently classified as daemon-level operator/admin infrastructure
