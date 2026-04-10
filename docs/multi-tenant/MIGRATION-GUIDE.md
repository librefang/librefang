# MIGRATION-GUIDE: Normalized Migration Checklist

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`

---

## Purpose

This is an engineering normalization guide, not an operator upgrade guide.

The main convergence is already in place on this branch.
This document remains useful as a checklist for auditing residual compatibility
debt and stale assumptions.

---

## Required Outcomes

1. No non-public route relies on missing-account fallback
2. No tenant-owned resource is created without concrete `account_id`
3. No docs or tests describe `"system"` fallback as target runtime behavior
4. Admin access is explicit and account-based
5. Split-surface modules are classified and tested by endpoint class

---

## Practical Audit Sequence

1. Start from `TENANT-INVARIANTS.md` and `ROUTE-POLICY-MATRIX.md`
2. Confirm current code/tests still match those docs
3. Remove stale fallback-oriented tests or handler branches
4. Isolate any remaining admin/global compatibility logic explicitly
5. Keep deferred product questions out of the migration-debt bucket

---

## What Counts As Migration Debt

- `AccountId(None)` as normal request behavior
- missing `X-Account-Id` on non-public routes
- implicit `"system"` ownership semantics in normal app flows
- tests that assume “no account sees all”
