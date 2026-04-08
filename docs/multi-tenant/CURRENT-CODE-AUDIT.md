# CURRENT-CODE-AUDIT: Multi-Tenant Branch State

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ROUTE-POLICY-MATRIX.md`, `ENTERPRISE-DECISIONS.md`, `PENDING-WORK.md`

---

## Purpose

Describe the branch as it exists now after the active multi-tenant convergence work.

This is no longer primarily a "drift log" for the converged slices. It is the
current engineering truth for what is done, what is intentionally split, and
what is still deferred.

---

## Branch Summary

The core tenant-owned product slices are materially converged:

- channels: tenant-owned config/secrets, account-scoped runtime registration,
  ingress binding v1, multiplicity enforcement, and reload rollback protection
- workflows: tenant-owned definitions, runs, schedules, triggers, runtime
  execution, and channel-command scoping
- goals: tenant-owned records, tenant-scoped CRUD, prompt enrichment, and
  internal goal update tooling
- providers: tenant-owned keys, URLs, defaults, and fail-closed tenant runtime
  resolution

The remaining work is mostly split-surface cleanup and deferred product
modeling, not missing `AccountId` plumbing.

---

## Converged Tenant-Owned Slices

### Channels

**Files:** `crates/librefang-api/src/routes/channels.rs`, `crates/librefang-api/src/channel_bridge.rs`, `crates/librefang-channels/src/bridge.rs`, `crates/librefang-channels/src/router.rs`

Current behavior:

- tenant-facing channel read/write/test operations require concrete account scope
- channel config entries persist per account
- secret env var references persist per account and fail closed
- partial updates preserve scoped secret bindings
- runtime adapter registration is tenant-qualified for multi-instance-safe families
- ingress binding requires concrete `metadata.account_id` for non-CLI traffic
- account-bound router resolution no longer falls back to daemon-global defaults
- multiplicity enforcement rejects unsupported duplicate daemon instances
- reload rollback preserves the previous working bridge when a conflicting
  single-instance family cannot be activated

Still intentionally not complete:

- QR/session ownership modeling
- shared integration user/chat/thread binding beyond integration-instance binding
- multi-tenant coexistence for fixed-route/shared-port adapter families

### Workflows

**Files:** `crates/librefang-api/src/routes/workflows.rs`, `crates/librefang-kernel/src/workflow.rs`, `crates/librefang-kernel/src/cron.rs`, `crates/librefang-kernel/src/triggers.rs`, `crates/librefang-kernel/src/kernel.rs`

Current behavior:

- workflow definitions persist concrete `account_id`
- workflow runs persist concrete `account_id`
- schedules persist concrete `account_id`
- triggers persist concrete `account_id`
- cross-tenant tenant-owned workflow resources resolve as `404`
- workflow execution, cron firing, and tenant-bound command paths stay within the
  owning tenant namespace
- no tenant-bound workflow path falls back to daemon-global workflow lookup

Still intentionally global/admin-only:

- workflow templates
- raw `/cron/jobs` infrastructure endpoints

### Goals

**Files:** `crates/librefang-api/src/routes/goals.rs`, `crates/librefang-kernel/src/goals.rs`, `crates/librefang-kernel/src/kernel.rs`, `crates/librefang-runtime/src/tool_runner.rs`, `crates/librefang-runtime/src/kernel_handle.rs`

Current behavior:

- goal records persist concrete `account_id`
- goal CRUD is tenant-scoped
- cross-tenant goal access resolves as `404`
- prompt enrichment reads tenant-owned active goals
- internal `goal_update` tooling now uses caller tenant context instead of
  opaque goal ID only

### Providers

**Files:** `crates/librefang-api/src/routes/providers.rs`, `crates/librefang-kernel/src/provider_accounts.rs`, `crates/librefang-kernel/src/kernel.rs`

Current behavior:

- tenant-owned provider records persist concrete `account_id`
- tenant provider keys, URLs, and defaults are no longer daemon-global
- tenant runtime provider resolution fails closed instead of falling back to
  daemon-global defaults or env conventions
- provider list/detail routes merge shared catalog metadata with caller-scoped
  tenant state

Still intentionally global/admin-only:

- catalog refresh/status
- alias/custom-model mutation
- daemon-global lifecycle/discovery operations

---

## Explicit Split-Surface Modules

These modules are not "unfinished by accident." They now have explicit mixed
classifications and should be audited by endpoint class, not file name.

### Budget

- tenant-owned: tenant usage summaries and agent-owned budget views
- admin-only: daemon-global reporting and daemon-wide budget control

### Network

- tenant-derived: comms overlays derived from owned agents
- admin-only: peer registry, daemon network status, protocol-level A2A, MCP HTTP,
  and other daemon-global infrastructure

### System

- public: small enumerated infra subset only
- tenant-derived: agent-owned memory export/import
- admin-only: template inventory, audit, logs, sessions, queue, bindings,
  pairing, registry, tools, webhooks, backup/restore, and other daemon-global
  operations

### Skills / Hands / Media

- skills remain split across shared catalog, admin lifecycle, and tenant-owned
  hand instances/settings
- media remains split across tenant-owned generation/tasks/artifacts and
  admin/shared provider registry surfaces

---

## Intentionally Admin-Only or Global

- inbox diagnostics in `inbox.rs`
- workflow templates
- raw cron infrastructure
- plugin lifecycle
- daemon-global provider catalog mutation
- daemon-global network/A2A/MCP operational surfaces
- daemon-global system operational surfaces

These are not current bugs unless they expose tenant-owned state without the
right policy.

---

## Remaining Real Gaps

### 1. Shared integration identity modeling

- current ingress binding is integration-instance -> tenant
- shared user/chat/thread ownership beyond that is not modeled yet

### 2. Channel session / QR ownership

- config/runtime ownership is converged
- QR/bootstrap/session ownership rules are still product work

### 3. Inbox product decision

- current inbox endpoint is still daemon diagnostics
- there is no tenant-owned inbox product model yet

### 4. Residual `AccountId(None)` migration debt

- active product paths now target explicit account scope
- any remaining `AccountId(None)` behavior should be treated as migration debt or
  explicitly admin/global-only compatibility logic

### 5. Broader tenant-owned skill content model

- hands are tenant-owned at the instance/settings layer
- broader tenant-authored skill content or overlays remain future design work

---

## Bottom Line

This branch is no longer in "major tenant-ownership missing" territory for the
active slices. The main remaining work is:

- deferred product modeling
- explicit split-surface boundaries
- cleanup of residual compatibility debt

It should be reviewed as a branch with converged core slices and a short,
explicit deferred-work list, not as an early refactor with unknown ownership.
