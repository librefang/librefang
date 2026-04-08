# Multi-Tenant Testing Strategy

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `SPEC-MT-001-ACCOUNT-DATA-MODEL.md`, `SPEC-MT-002-API-ROUTE-CHANGES.md`

---

## Goal

Turn the multi-tenant policy into executable guardrails.

The tests should encode the current target architecture:

- always multi-tenant
- no implicit `system` fallback
- concrete account identity on non-public routes
- `404` for cross-tenant tenant-owned access
- `403` for admin-only endpoint misuse

---

## Priority Test Classes

### 1. Request Identity

- non-public routes reject missing `X-Account-Id`
- public/auth bootstrap routes remain accessible without account header
- invalid signatures fail
- valid signatures pass
- channel-originated requests without a concrete tenant binding fail closed

### 2. Tenant-Owned Resources

For each tenant-owned route family:

- account A can access own resources
- account B gets `404`
- created resources persist account A ownership

This applies to:

- agents
- memory
- prompts
- channels
- workflows
- goals
- inbox
- media artifacts

### 3. Admin-Only Endpoints

For each admin-only route family:

- configured admin account succeeds or reaches normal downstream validation
- non-admin tenant gets `403`
- missing account header is rejected before handler logic

### 4. Split-Surface Modules

For providers, skills, hands, and budget:

- catalog/discovery behavior is tested separately from admin lifecycle behavior
- tenant-owned overlays or instances are tested separately from global catalog paths

### 5. Opaque ID Lookups

For every feature that resolves an opaque identifier into state or artifacts:

- upload IDs
- media task IDs
- workflow run IDs
- session IDs
- webhook IDs
- pairing IDs
- browser session IDs
- scheduled job IDs

Tests must prove one of two things:

- the lookup is tenant-owned and cross-tenant access returns `404`
- or the lookup is intentionally global and documented as such

### 6. Channel Ingress Binding

For messaging and integration entry points:

- platform identity must map to one concrete tenant before tenant-owned actions run
- ambiguous or missing bindings must fail closed
- session partitioning tests must be kept separate from tenant ownership tests

---

## Anti-Goals

Do not keep tests that encode obsolete behavior such as:

- missing account header means admin/system access
- `AccountId(None)` sees all tenant data
- `"system"` fallback is part of the normal runtime model

If such tests still exist, they should be removed or rewritten.

---

## Gate Criteria

The route policy is not accepted until integration tests cover:

1. missing account rejection
2. own-account success
3. cross-tenant `404`
4. admin-only `403`
5. explicit public endpoint exemptions

Tests are the real guardrail; docs explain the policy, but tests must enforce it.
