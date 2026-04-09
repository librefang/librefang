# ROUTE-POLICY-MATRIX: API Surface Classification

**Status:** Current
**Date:** 2026-04-08
**Related:** `TENANT-INVARIANTS.md`, `ENTERPRISE-DECISIONS.md`, `SPEC-MT-002-API-ROUTE-CHANGES.md`

---

## Purpose

This is the implementation-facing route policy matrix for the Qwntik multi-tenant fork.

Use this document to audit code and tests.
If code behavior differs from this matrix, the code should be treated as drift unless
there is a deliberate documented decision to change the policy.

---

## Policy Keys

- `tenant-owned`: concrete account required, own-account access only, cross-tenant `404`
- `admin-only`: concrete account required, admin account required, non-admin `403`
- `public`: no account required
- `split-surface`: module contains multiple endpoint classes and must not be treated as one policy

---

## Route Families

| File | Target Policy | Notes |
|------|---------------|-------|
| `agents.rs` | `tenant-owned` | Includes agent CRUD, sessions, files, uploads, messaging, traces, tools/skills assignment |
| `memory.rs` | `tenant-owned` | Account-scoped memory and agent-owned memory operations |
| `prompts.rs` | `tenant-owned` | Currently enforced through owned-agent access; standalone prompt-store ownership only matters if prompts become independent tenant assets |
| `channels.rs` | `split-surface` | Channel config/read/write/test is tenant-owned; reload, QR/session bootstrap, and registry/debug endpoints remain admin/global |
| `workflows.rs` | `split-surface` | Workflow CRUD, runs, schedules, and triggers are tenant-owned; workflow templates and raw cron infra remain explicitly global/admin-only |
| `goals.rs` | `tenant-owned` | Goal lifecycle, prompt enrichment, and internal goal tooling are tenant-owned |
| `inbox.rs` | `admin-only` | Current inbox is daemon-level operator/admin intake infrastructure; `/api/inbox/status` is admin-only diagnostics |
| `media.rs` | `split-surface` | Media generation/task/artifact paths are tenant-owned; provider registry/capability endpoints may remain admin-only or shared |
| `config.rs` | `split-surface` | `status`/`health_detail` style scoped status views plus public health/version endpoints; global config ops are admin-only |
| `budget.rs` | `split-surface` | Agent-owned budget/usage views are tenant-owned; global reporting and daemon-wide budget management are admin-only |
| `network.rs` | `split-surface` | Some overlays derived from owned agents are tenant-visible; daemon mesh/A2A/MCP infra is admin-only |
| `providers.rs` | `split-surface` | Shared catalog/discovery, tenant overrides/secrets/defaults, and admin lifecycle/refresh must be separated |
| `skills.rs` | `split-surface` | Catalog/discovery, admin install/reload, tenant-owned content/allowlists/instances are separate classes |
| `system.rs` | `split-surface` | Small public subset, large admin-only infra subset, and a few tenant-derived views tied to owned agents |
| `plugins.rs` | `admin-only` | Plugin lifecycle is daemon-global infrastructure |

---

## Recommended Per-Module Interpretation

### `agents.rs`

- Target: `tenant-owned`
- Expected responses:
  - own account: success
  - other account: `404`
- Notes:
  - upload serving follows tenant-owned policy
  - remaining `AccountId(None)` comments/tests should be treated as cleanup targets

### `memory.rs`

- Target: `tenant-owned`
- Expected responses:
  - own account: success
  - other account: `404`
- Notes:
  - account scoping should be enforced in stores, not only route handlers

### `prompts.rs`

- Target: `tenant-owned`
- Notes:
  - current prompt access is enforced through owned agents
  - direct prompt-store ownership only matters if prompt data is separated from
    agent ownership in the product model

### `channels.rs`

- Target: `split-surface`
- Expected responses:
  - tenant-owned config/read/write/test: own account success, other account `404`
  - admin/global infra: non-admin tenant `403`
- Tenant-owned:
  - `GET /api/channels`
  - `GET /api/channels/{name}`
  - `POST|DELETE /api/channels/{name}/configure`
  - `POST /api/channels/{name}/test`
- Admin-only:
  - `POST /api/channels/reload`
  - WhatsApp / WeChat QR bootstrap and status flows
  - `GET /api/channels/registry`
- Notes:
  - config, secrets, runtime registration, ingress binding, and tenant command lookup are already account-scoped
  - shared-route/shared-port adapter families remain intentionally unsupported for multi-tenant coexistence until they gain explicit partitioning

### `workflows.rs`

- Target: `split-surface`
- Tenant-owned:
  - workflow CRUD
  - workflow runs
  - schedules
  - triggers
- Admin-only:
  - raw `/cron/jobs` infrastructure endpoints
  - workflow template catalog/management until templates get a tenant/shared catalog policy
- Notes:
  - workflow definitions, runs, trigger records, and schedule records now persist concrete `account_id`
  - cross-tenant workflow, run, trigger, and schedule access should return `404`

### `goals.rs`

- Target: `tenant-owned`
- Notes:
  - goal records persist concrete `account_id`
  - goal CRUD, child lookup, prompt enrichment, and internal goal update tooling
    are tenant-scoped
  - built-in goal templates are tenant-readable under the same concrete-account requirement
  - cross-tenant goal access returns `404`

### `inbox.rs`

- Target: `admin-only`
- Notes:
  - current `/api/inbox/status` endpoint is daemon-level operator diagnostics
    tied to global config/home-dir state
  - current inbox behavior should be treated as daemon-level operator/admin
    intake infrastructure, not as a tenant-owned product surface in the current
    architecture
  - if a real tenant inbox is ever desired, it is a separate future product
    model rather than part of the current refactor

### `media.rs`

- Target: `split-surface`
- Tenant-owned:
  - generate image/speech/music/video
  - poll tenant-owned task status
  - serve tenant-owned generated artifacts through upload routes
- Non-tenant-owned:
  - media provider registry/capability listing, depending on how much infra detail it reveals

### `config.rs`

- Target: `split-surface`
- Public:
  - `health`
  - `version`
  - `config_schema`
- Tenant-safe scoped:
  - `status`
  - `health_detail` if redacted/scoped
- Admin-only:
  - reload, set, migrate, shutdown, quick init, metrics, raw config/security internals

### `budget.rs`

- Target: `split-surface`
- Tenant-owned:
  - `GET /api/usage`
  - `GET /api/usage/summary`
  - `GET /api/budget/agents`
  - `GET /api/budget/agents/{id}`
  - `PUT /api/budget/agents/{id}`
- Admin-only:
  - `GET /api/usage/by-model`
  - `GET /api/usage/by-model/performance`
  - `GET /api/usage/daily`
  - `GET /api/budget`
  - `PUT /api/budget`
- Notes:
  - tenant-visible budget and usage routes now require concrete `X-Account-Id` and derive results only from owned agents
  - no tenant-visible budget path falls back to daemon-global usage aggregation when account context is absent

### `network.rs`

- Target: `split-surface`
- Tenant-visible:
  - `GET /api/comms/topology`
  - `GET /api/comms/events`
  - `GET /api/comms/events/stream`
  - `POST /api/comms/send`
- Admin-only:
  - `GET /api/peers`
  - `GET /api/peers/{id}`
  - `GET /api/network/status`
  - `POST /api/comms/task`
  - `GET /api/a2a/agents`
  - `GET /api/a2a/agents/{id}`
  - `POST /api/a2a/discover`
  - `POST /api/a2a/send`
  - `GET /api/a2a/tasks/{id}/status`
  - `POST /mcp`
  - `GET /.well-known/agent.json`
  - `GET /a2a/agents`
  - `POST /a2a/tasks/send`
  - `GET /a2a/tasks/{id}`
  - `POST /a2a/tasks/{id}/cancel`
- Public:
  - none in the current implementation
- Notes:
  - protocol-level A2A routes are clamped admin-only because the local service card/task surfaces are daemon-global and not clean tenant-derived views
  - comms overlays must require concrete `X-Account-Id` and filter results to owned agents only

### `providers.rs`

- Target: `split-surface`
- Shared or tenant-readable:
  - model catalog
  - provider capability metadata
  - provider list/detail routes that merge shared catalog data with caller-scoped tenant provider state
- Tenant-owned:
  - provider keys
  - provider URLs
  - tenant defaults/overrides
  - tenant-bound runtime provider resolution with no fallback to daemon-global defaults or env conventions
- Admin-only:
  - daemon-global refresh or lifecycle paths
  - model alias/custom-model mutation surfaces
- Notes:
  - the tenant-owned provider slice is converged for the active runtime path

### `skills.rs`

- Target: `split-surface`
- Shared or tenant-readable:
  - marketplace/catalog browse
  - hand definitions where they are global definitions
- Tenant-owned:
  - tenant content or tenant-level skill settings/instances
  - hand instances and hand settings with persisted concrete `account_id`
  - missing `X-Account-Id` is invalid for tenant-facing hand instance/settings operations
- Admin-only:
  - install/uninstall/reload of global skills
  - dependency management
  - extension/integration lifecycle if daemon-global

### `system.rs`

- Target: `split-surface`
- Public:
  - `GET /api/versions`
  - `GET /api/profiles`
  - `GET /api/profiles/{name}`
  - `GET /api/commands`
  - `GET /api/commands/{name}`
- Tenant-owned or tenant-derived:
  - `GET /api/agents/{id}/memory/export`
  - `POST /api/agents/{id}/memory/import`
- Admin-only:
  - local template inventory
  - audit
  - logs stream
  - tools registry
  - sessions management
  - queue internals
  - backup/restore
  - webhook registry
  - pairing/device management
  - most bindings/registry internals

### `plugins.rs`

- Target: `admin-only`
- Expected responses:
  - admin account: success or normal downstream validation
  - non-admin tenant: `403`

---

## Current High-Value Drift

These are the main intentionally deferred items that still affect route policy interpretation:

1. shared integration user/chat/thread binding beyond integration-instance ownership is still deferred
2. QR/bootstrap/session ownership for channel adapters that need it is still deferred
3. broader tenant-owned skill content beyond hands remains future product work if needed
4. residual `AccountId(None)` compatibility branches/comments/tests still need cleanup outside explicitly documented legacy/admin paths

---

## Audit Use

When auditing code:

1. classify the handler by this matrix
2. verify missing-account behavior
3. verify own-account success
4. verify cross-tenant `404` for tenant-owned resources
5. verify `403` for admin-only endpoints
6. flag modules using admin clamps where target policy says tenant-owned
