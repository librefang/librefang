# SPEC-MT-001: Account Data Model

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-001-ACCOUNT-MODEL.md`, `ADR-MT-002-API-AUTH.md`

---

## Purpose

Define the concrete account model for this Qwntik fork.

This fork is **always multi-tenant**. The account model is not designed to preserve
single-tenant runtime behavior or implicit `"system"` fallback semantics.

---

## Core Model

### `AccountId`

`AccountId` is the request-scoped tenant identity.

Rules:

- normal runtime traffic uses `AccountId(Some(<account-id>))`
- non-public routes require a concrete account
- `AccountId(None)` may still exist in transitional code paths, but it is not a valid
  application/runtime identity for the intended architecture

### Resource Ownership

Every tenant-owned resource must persist a concrete `account_id`.

Required examples:

- agents
- memories
- prompts
- channels
- workflows
- goals
- inbox records
- media tasks and media artifacts

### Admin Accounts

Admin behavior is not represented by “missing account”.

Admin behavior means:

- the request still carries a concrete `X-Account-Id`
- that account is in `admin_accounts`
- admin-only endpoints return `403` to non-admin tenants

---

## Extraction Rules

The request extractor may still return `AccountId(None)` when the header is missing.
That is an extractor implementation detail, not a permission model.

Enforcement rules are:

1. Public/auth bootstrap endpoints may be accessed without `X-Account-Id`
2. All other app-facing endpoints reject missing `X-Account-Id`
3. Handlers assume concrete account identity for tenant-owned flows

---

## Guard Rules

### Tenant-Owned Resources

Use scoped lookup or ownership checks.

Expected behavior:

- own-account access succeeds
- cross-tenant access returns `404`
- resource creation persists caller `account_id`

### Admin-Only Endpoints

Use explicit admin-account guards.

Expected behavior:

- configured admin account succeeds
- non-admin tenant receives `403`
- missing account header is rejected before handler logic

### Split-Surface Modules

Do not use one fallback macro for mixed modules such as:

- providers
- skills
- hands
- budget

These must be split by endpoint class:

- shared catalog/discovery
- tenant-owned content or overrides
- daemon-global admin operations

---

## Data Model Constraints

1. New tenant-owned records must never be written without `account_id`
2. Store APIs should take account context explicitly where ownership matters
3. Query-layer filtering is preferred over caller-side filtering where possible
4. Cross-tenant invisibility is part of the model, not just an API convention

---

## Acceptance Criteria

### AC-1: Non-public routes require concrete account identity

- Given no `X-Account-Id`
- When calling a non-public tenant-facing route
- Then the request is rejected

### AC-2: Tenant-owned resources store account ownership

- Given a create request from account A
- When a tenant-owned resource is persisted
- Then the record stores account A's `account_id`

### AC-3: Cross-tenant lookups are invisible

- Given a tenant-owned resource owned by account A
- When account B requests it
- Then the API returns `404`

### AC-4: Admin is explicit

- Given an admin-only endpoint
- When a non-admin tenant calls it
- Then the API returns `403`
- And not `200`

### AC-5: Missing account is not admin

- Given a non-public route
- When the request omits `X-Account-Id`
- Then the request is rejected
- And not treated as admin/system access

---

## Implementation Direction

- Prefer explicit helper functions over `validate_account!` / `account_or_system!` style fallback macros
- Remove or isolate any remaining `"system"` or `AccountId(None)` runtime semantics from normal request flows
- Keep the extractor simple; keep policy strict
