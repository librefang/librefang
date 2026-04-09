# LibreFang Multi-Tenant Architecture — Master Plan

**Status:** Current
**Date:** 2026-04-09
**Scope:** Qwntik fork runtime only

---

## Canonical Rule

`TENANT-INVARIANTS.md` is the top-level policy source.

This fork is designed for **always-multi-tenant** operation.

Not part of the target architecture:

- single-tenant product mode
- implicit `system` fallback for app traffic
- missing account header as admin behavior

---

## Current Runtime Model

The branch target is now the branch reality:

- every non-public request carries a concrete account
- tenant-owned resources persist concrete ownership
- cross-tenant tenant-owned access returns `404`
- admin-only infrastructure returns `403` to non-admin tenants
- mixed modules are split by policy class

---

## Document Set

- `TENANT-INVARIANTS.md`: canonical policy
- `ENTERPRISE-DECISIONS.md`: enterprise SaaS rationale and non-negotiable tradeoffs
- `COMP-HERMES-AGENT.md`: external reference comparison for shared-agent versus true multi-tenant runtime design
- `DESIGN-CHANNEL-TENANT-BINDING.md`: ingress model for mapping messaging identities to concrete tenant accounts
- `ADR-MT-006`: channel bootstrap/session ownership decision
- `SPEC-MT-005`: channel bootstrap/session ownership implementation model
- `PLAN-MT-003`: channel bootstrap hardening execution plan
- `CURRENT-CODE-AUDIT.md`: current implementation audit and residual gaps
- `ADR-MT-001`: account model
- `ADR-MT-002`: auth and account resolution
- `ADR-MT-003`: route/resource isolation
- `ADR-MT-004`: storage and memory isolation
- `ADR-MT-005`: event bus isolation
- `ADR-MT-006`: channel bootstrap/session ownership
- `SPEC-MT-001`: account data model
- `SPEC-MT-002`: route-policy model
- `SPEC-MT-003`: database migration
- `SPEC-MT-004`: RLS policies
- `SPEC-MT-005`: channel bootstrap/session ownership
- `PLAN-MT-001`: foundation summary
- `PLAN-MT-002`: remaining implementation work
- `PLAN-MT-003`: channel bootstrap hardening
- `TESTING-STRATEGY.md`: policy test plan
- `PENDING-WORK.md`: live gap tracker

---

## Route Policy Model

### Tenant-Owned

- agents
- memory
- prompts
- goals
- media tasks and artifacts

### Admin-Only Infrastructure

- inbox operator/admin intake infrastructure and diagnostics
- most of system
- most of config
- most of network
- plugin lifecycle

### Split-Surface

- channels
- workflows
- providers
- skills
- hands
- budget

### Public

- health
- version
- ready/openapi/auth bootstrap subset

---

## What Is Converged

- core tenant-owned slices are converged for channels, workflows, goals,
  providers, and hand-instance tenancy within skills
- route-policy cleanup is converged for budget, network, and system
- channel bootstrap/session ownership is now converged at the
  integration-instance scope for WeChat and WhatsApp
  - explicit ownership records persist concrete `account_id`
  - bootstrap lookup is keyed by owned `(channel_type, instance_key)`
  - WeChat and WhatsApp bootstrap routes are now instance-targeted and
    provider handle is stored as data, not the primary ownership key
- inbox is intentionally admin/operator infrastructure, not a tenant-owned
  product surface

## What Remains Open

- shared integration user/chat/thread binding beyond integration-instance scope
- broader tenant-owned skill content beyond hands, only if product requires it
- residual `AccountId(None)` and fallback compatibility debt

---

## Success Criteria

- no docs describe stale runtime behavior as target policy
- no non-public route depends on missing-account fallback
- tenant-owned stores persist and filter by concrete `account_id`
- tests enforce the policy model end to end
