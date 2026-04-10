# PLAN-MT-001: Phase 1 Foundation

**Status:** Historical foundation, normalized 2026-04-08
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `SPEC-MT-001-ACCOUNT-DATA-MODEL.md`

---

## Purpose

Record the Phase 1 foundation work in terms that still match the current target
architecture.

Phase 1 established the building blocks:

- `AccountId`
- account extraction
- HMAC verification
- initial route scoping
- registry/account ownership wiring

---

## What Phase 1 Should Be Understood As

Phase 1 was about introducing account-aware plumbing, not preserving a permanent
single-tenant/system runtime model.

The important surviving outcomes are:

- non-public routes can be required to carry account identity
- account ownership can be attached to resources
- route handlers can enforce tenant ownership
- middleware can verify tenant identity

---

## What Should Be Considered Obsolete

The following should not be carried forward as target behavior:

- `AccountId(None)` as normal runtime admin behavior
- `account_or_system!` style policy as a general route design
- tests whose success condition is “missing account sees all”
- “system account” semantics as part of the Qwntik app model

---

## Output of Phase 1 That Still Matters

- extractor and middleware foundation
- account-aware registry methods
- scoped agent ownership
- first integration tests for account behavior

The next phases should build on those pieces while removing any remaining fallback assumptions.
