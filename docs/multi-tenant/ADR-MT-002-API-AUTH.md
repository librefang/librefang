# ADR-MT-002: API Authentication & Account Resolution

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ADR-MT-001-ACCOUNT-MODEL.md`

---

## Decision

This fork uses explicit account-bearing request auth for all non-public runtime traffic.

### Rules

1. Non-public requests must carry `X-Account-Id`
2. Account-bearing requests must pass HMAC signature verification
3. Missing account headers are rejected on non-public routes
4. Admin behavior requires a concrete admin account, not a missing account header

---

## Request Model

### Public/Auth Bootstrap Routes

These may be accessed without `X-Account-Id`:

- health/version/openapi style public endpoints
- auth bootstrap endpoints

### Non-Public Routes

These require:

- `X-Account-Id`
- `X-Account-Sig`
- `X-Account-Timestamp` where the replay-protected format is in use

---

## Extractor Model

The extractor may still be infallible and produce `AccountId(None)` when the header
is absent. That is acceptable as an implementation detail.

It is **not** acceptable as runtime policy.

The policy is enforced by middleware and route classification:

- public/auth bootstrap routes can omit account identity
- all other app-facing routes reject missing account identity

---

## Outstanding Work

- finish removing legacy HMAC acceptance
- add nonce-cache replay protection if still missing
- ensure docs and tests do not encode missing-account fallback behavior
