# MIGRATION-GUIDE: Converging to the Qwntik Multi-Tenant Runtime

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`

---

## Purpose

This is an engineering convergence guide, not an operator upgrade guide.

Its goal is to remove stale fallback assumptions and bring the fork fully in line
with the current multi-tenant architecture.

---

## Required Outcomes

1. No non-public route relies on missing-account fallback
2. No tenant-owned resource is created without concrete `account_id`
3. No docs or tests describe `"system"` fallback as target runtime behavior
4. Admin access is explicit and account-based
5. Split-surface modules are classified and tested by endpoint class

---

## Practical Sequence

1. Approve `TENANT-INVARIANTS.md`
2. Build the per-route policy matrix
3. Backfill or rewrite integration tests
4. Reconcile handlers
5. Reconcile stores
6. Remove remaining fallback debt

---

## What Counts As Migration Debt

- `AccountId(None)` as normal request behavior
- missing `X-Account-Id` on non-public routes
- implicit `"system"` ownership semantics in normal app flows
- tests that assume “no account sees all”
