# ENTERPRISE-DECISIONS: Security, Safety, Scalability, and Soundness

**Status:** Canonical
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ROUTE-POLICY-MATRIX.md`, `MASTER-PLAN.md`

---

## Purpose

Document the explicit architecture decisions for this Qwntik fork from an
enterprise SaaS perspective.

These decisions are chosen to optimize for:

- security isolation
- operational safety
- future scalability
- architectural soundness

---

## Decision 1: One Runtime Model Only

This fork supports one product runtime model:

- always multi-tenant
- every non-public request carries a concrete account

### Why

- removes ambiguous permission branches
- reduces cross-tenant escape risk
- simplifies testing and incident response
- avoids designing around compatibility modes the product will not use

### Consequence

- missing account headers are invalid on non-public routes
- `AccountId(None)` is migration debt, not target behavior

---

## Decision 2: Concrete Ownership Everywhere

Tenant-owned resources must persist concrete `account_id` in storage.

### Why

- route-level checks alone are not enough
- background jobs, event flows, and internal APIs need ownership too
- storage-layer ownership is required for correct scaling and forensic clarity

### Consequence

- workflows, channels, goals, prompts, queues, and related stores need ownership convergence

---

## Decision 3: Prefer Invisible Denial for Tenant-Owned Resources

Cross-tenant access to tenant-owned resources returns `404`.

### Why

- prevents resource enumeration
- aligns with least-information disclosure
- preserves strong tenant boundary semantics

### Consequence

- tenant-owned routes must use scoped lookup or ownership checks

---

## Decision 4: Use Explicit `403` for Admin-Only Infrastructure

Endpoint classes that are daemon-global remain admin-only and return `403` to
non-admin tenants.

### Why

- makes privileged infrastructure behavior explicit
- separates tenant ownership problems from operator privilege problems
- keeps dangerous surfaces easy to reason about

### Consequence

- most of `system`, `config`, `network`, and plugin lifecycle remain admin-only

---

## Decision 5: Reject Global Shared Config for Tenant-Owned Product Features

If a feature is tenant-owned, it cannot rely on implicit daemon-global singleton
semantics.

### Why

- global config turns route-level tenant checks into security theater
- one tenant must never be able to overwrite another tenant’s operational state

### Consequence

- if shared files such as `config.toml` / `secrets.env` are still used, they
  must persist explicit account-owned records and scoped secret references
- `goals` cannot remain backed by a single shared synthetic-agent bucket
- workflows cannot remain globally registered if they are product-facing tenant assets

---

## Decision 6: Split Mixed Modules by Endpoint Class

Modules that mix catalog, tenant, and infrastructure behavior must be treated as
split-surface modules, not forced into one policy.

Primary examples:

- providers
- skills
- hands
- budget
- system
- network
- media

### Why

- mixed modules are where accidental privilege leaks and product confusion happen
- per-module blanket policy causes either overexposure or overrestriction

### Consequence

- tests and audits must classify handlers by endpoint class, not only by file

---

## Decision 7: Temporary Admin Clamps Are Acceptable but Must Be Called Out

If a module is unsafe to expose as tenant-owned today, temporary admin-guarding is
acceptable as a short-term containment measure.

### Why

- safer to overrestrict than to leak data
- lets the team ship protective controls before store refactors are complete

### Constraint

Temporary clamps must be documented as drift, not mistaken for final product policy.

### Consequence

Current examples:

- narrowly scoped containment while a route family is being decomposed to its
  documented endpoint classes
- short-lived protection for residual compatibility paths that are not intended
  as branch end-state behavior

Inbox is not one of those temporary clamps in the current architecture.
It is currently classified as daemon-level operator/admin intake
infrastructure, and `/api/inbox/status` is admin-only diagnostics.

---

## Decision 8: Public Surface Must Stay Small

Public endpoints should be rare, explicit, and infrastructure-only.

### Why

- reduces attack surface
- reduces auth bypass ambiguity
- keeps monitoring compatibility without weakening tenant boundaries

### Consequence

Public should remain limited to:

- health/version/readiness/openapi/auth bootstrap style endpoints
- a very small explicit catalog/metadata subset where no tenant-owned or
  daemon-sensitive state is exposed

---

## Decision 9: Test the Policy, Not the Plumbing

Tests should assert product policy outcomes:

- missing account rejected
- own-account success
- cross-tenant `404`
- admin-only `403`

### Why

- policy tests survive implementation refactors
- plumbing tests alone do not prove tenant safety

### Consequence

- remove tests that enshrine obsolete fallback behavior
- add integration coverage per route family and endpoint class

---

## Decision 10: Favor Simpler Security Invariants Over Compatibility Tricks

When forced to choose between a compatibility shortcut and a cleaner security model,
choose the cleaner security model for this fork.

### Why

- this is a Qwntik-specific fork, not a general upstream distribution target
- simpler invariants are easier to audit, reason about, and scale safely

### Consequence

- no product requirement to preserve single-tenant semantics
- no product reason to keep `system` as a normal runtime identity

---

## Operational Implications

### Highest-Risk Architectural Gaps

1. tenant-owned route families backed by global storage
2. singleton/shared storage used for product-facing tenant data
3. mixed modules without endpoint-class separation
4. legacy/fallback behavior still present in tests and helper paths

### Recommended Implementation Order

1. channel QR/session ownership modeling
2. shared integration user/chat/thread binding beyond integration-instance ownership
3. broader tenant-owned skill content beyond hands, only if product requires it
4. removal of residual `AccountId(None)` compatibility debt outside explicit admin/global paths

At the current branch state, this primarily means cleanup of extractor/storage
compatibility semantics and stale comments/tests, not continued admin-route
fallback behavior.

---

## Bottom Line

The enterprise-safe design for this fork is:

- strict tenant identity
- explicit ownership in storage
- invisible denial for tenant-owned resources
- explicit privilege boundaries for infrastructure
- no compatibility-driven fallback in normal runtime behavior
