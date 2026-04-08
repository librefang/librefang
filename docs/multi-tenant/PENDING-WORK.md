# PENDING-WORK: Multi-Tenant Phase 1 Follow-Up

**Date:** 2026-04-07 (v2: post-brutal-honesty audit)
**Scope:** Remaining work after Phase 1 Rounds 1-5
**Baseline:** Rounds 1-5 landed for types, extractors, shared guards, `agents.rs`, `config.rs`, registry, persistence, and channel-bridge spawn wiring.

---

## Terminology

This document distinguishes between two states:

- **Extractor plumbed** = handler signature includes `account: AccountId` or `_account: AccountId`. The header is parsed. This alone provides **zero tenant isolation**.
- **Tenant-filtered** = handler actually uses the account value to filter data via `check_account()`, `list_by_account()`, `get_scoped()`, `require_agent_access()`, or direct `account.0` checks. This is what "scoped" means.

## Current State (verified 2026-04-07)

### Genuinely tenant-filtered (data isolation working)

| File | Total handlers | With `account:` | With `_account` | No `AccountId` | Filtering mechanism | Status |
|------|---------------|-----------------|-----------------|----------------|---------------------|--------|
| `agents.rs` | 50 | 48 | 1 | 1 (helper) | 45 `get_scoped`, 2 `list_by_account` | **Done** — gold standard |
| `memory.rs` | 25 | 25 | 0 | 0 | 4 `check_account`, 1 `list_by_account`, 1 `get_scoped`, 12 `require_agent_access` calls, direct `account.0` match arms | **Done** |
| `prompts.rs` | 12 | 12 | 0 | 0 | 12 `require_agent_access` calls (agent-ownership check) | **Done** |

### Admin-guarded (tenants get 403 — cross-tenant access blocked)

| File | Total handlers | Data-filtered | Admin-guarded (`require_admin`) | Tier 4 Public (`_account`) | Filtering calls | Status |
|------|---------------|--------------|-------------------------------|---------------------------|-----------------|--------|
| `config.rs` | 15 | 2 | 10 | 3 | 2 `list_by_account`, 10 `require_admin` | **Done** — all 15 handlers have `AccountId` |
| `budget.rs` | 10 | 5 | 5 | 0 | 2 `check_account`, 3 `list_by_account`, 5 `require_admin` | **Done** — admin guards on global budget/usage |
| `network.rs` | 20 | 7 | 13 | 0 | 2 `check_account`, 5 `list_by_account`, 13 `require_admin` | **Done** — admin guards on peers/A2A/MCP |

### Partially filtered (some handlers filter, most don't)

| File | Total handlers | Filtered (`account:`) | Unfiltered (`_account`) | No `AccountId` | Filtering calls | Status |
|------|---------------|----------------------|------------------------|----------------|-----------------|--------|
| `system.rs` | 63 | 6 | 57 | 0 | 2 `check_account`, 4 `list_by_account` | **Partial** — 57 handlers ignore account |
| `skills.rs` | 53 | 2 | 51 | 0 | 2 `check_account` | **Barely started** — 51 handlers ignore account |
| `workflows.rs` | 30 | 1 | 29 | 0 | 1 `check_account`, 1 `list_by_account` | **Barely started** — 29 handlers ignore account |

### Extractor plumbed only (zero filtering — cross-tenant access possible)

| File | Total handlers | `_account` (unused) | No `AccountId` | Status |
|------|---------------|--------------------|--------------------|--------|
| `channels.rs` | 11 | 11 | 0 | **Not started** — all 11 handlers ignore account |
| `providers.rs` | 19 | 19 | 0 | **Not started** |
| `plugins.rs` | 8 | 8 | 0 | **Not started** |
| `goals.rs` | 7 | 7 | 0 | **Not started** |
| `media.rs` | 6 | 6 | 0 | **Not started** — includes upload cross-tenant vuln |
| `inbox.rs` | 1 | 1 | 0 | **Not started** |

### Summary

- **Genuinely data-scoped:** 3 files, 85 handlers (agents, memory, prompts)
- **Admin-guarded (403 for tenants):** 3 files, 28 handlers + 3 Tier 4 Public (config, budget, network)
- **Partially scoped:** 3 files, ~9 handlers filtered out of ~146 total (system, skills, workflows)
- **Not scoped (extractor only):** 6 files, 52 handlers with zero filtering (channels, providers, plugins, goals, media, inbox)
- **Handlers with no `AccountId` at all:** 1 (helper in agents.rs)
- **Total `_account` (unused) parameters across route handlers:** ~180

---

## Done

- [x] Round 1: `AccountId` type, `AgentEntry.account_id`, 19 unit tests
- [x] Round 2: CLI `AgentEntry` constructor fixes in `crates/librefang-cli/src/tui/event.rs`
- [x] Round 3: API account extractor, HMAC-SHA256 verification, `check_account`, shared route helpers, middleware/shared tests
- [x] Round 4a: `agents.rs` — 48/50 handlers tenant-filtered (1 `_account` on `serve_upload`, 1 helper without `AccountId`)
- [x] Round 4c: `config.rs` — **15/15 handlers** now have `AccountId`: 2 data-filtered (`status`, `health_detail` via `list_by_account`), 10 admin-guarded via `require_admin` (`shutdown`, `quick_init`, `config_set`, `config_reload`, `get_config`, `security_status`, `prometheus_metrics`, `migrate_detect`, `migrate_scan`, `run_migrate`), 3 Tier 4 Public (`version`, `health`, `config_schema`)
- [x] `memory.rs` — 25/25 handlers tenant-filtered via `account.0` match arms + `check_account` + `require_agent_access`
- [x] `prompts.rs` — 12/12 handlers tenant-filtered via `require_agent_access()`
- [x] Registry: `list_by_account()`, `get_scoped()`, `set_account_id()` with TDD coverage
- [x] Persistence: `account_id` persisted in save/load paths with round-trip tests
- [x] Channel bridge: `spawn_agent_by_name` now takes `account_id`
- [x] Codex review fixes: atomic spawn ownership, `list_by_account` wired into handler, infallible `finalize_spawned_agent`
- [x] `require_account_id` middleware — rejects missing `X-Account-Id` in multi-tenant mode (`14e00fef`)
- [x] HMAC replay protection — timestamp+method+path binding, 10 tests (`7637f863`)
- [x] Telemetry redaction — `/api/status` and `/api/health/detail` redacted for tenants, 6 tests
- [x] `get_scoped()` refactor — 45 call sites in `agents.rs`
- [x] Partial data-layer filtering — `system.rs` (6), `skills.rs` (2), `workflows.rs` (1)
- [x] `budget.rs` — **10/10 handlers** now use `AccountId`: 5 data-filtered (2 `check_account`, 3 `list_by_account`), 5 admin-guarded via `require_admin` (`usage_by_model`, `usage_by_model_performance`, `usage_daily`, `budget_status`, `update_budget`)
- [x] `network.rs` — **20/20 handlers** now use `AccountId`: 7 data-filtered (2 `check_account`, 5 `list_by_account`), 13 admin-guarded via `require_admin` (peers, A2A management, MCP HTTP, comms_task)
- [x] `shared.rs` — added `require_admin()` guard function for admin-only endpoints (returns 403 to scoped tenants)

## Pending Checklist

### 1. CRITICAL: Wire tenant filtering in partially-scoped and unscoped modules

The following modules have `AccountId` in handler signatures but most handlers ignore it. The `_account` parameter must be replaced with actual filtering logic. `config.rs` is worse — 13 handlers have **no `AccountId` at all**.

| File | Unfiltered handlers | Blocking issue |
|------|-------------------|----------------|
| ~~`config.rs`~~ | ~~13 of 15~~ | **DONE** — 10 admin-guarded, 3 Tier 4 Public |
| `system.rs` | 57 of 63 | Many are admin/telemetry — need tier decision (filter, redact, or deny) |
| `skills.rs` | 51 of 53 | Needs kernel-level skill-per-account allowlist or ownership model |
| `workflows.rs` | 29 of 30 | Needs store-level `account_id` on workflow records |
| ~~`network.rs`~~ | ~~12 of 19~~ | **DONE** — 13 admin-guarded via `require_admin` |
| ~~`budget.rs`~~ | ~~5 of 10~~ | **DONE** — 5 admin-guarded via `require_admin` |

Files to change:
- `system.rs`, `skills.rs`, `workflows.rs` (remaining 3)
- Downstream kernel/store entry points they call
- `crates/librefang-api/tests/account_tests.rs` (cross-tenant denial tests per module)

### 2. CRITICAL: Wire tenant filtering in unscoped modules

These modules have the extractor but zero filtering. Cross-tenant data access is possible on all endpoints.

| File | Handlers | Blocking issue |
|------|----------|----------------|
| `channels.rs` | 11 | Channel ownership model needed; QR/session flows need tenant binding |
| `providers.rs` | 19 | Provider configs may be shared (Tier 2) — need tier decision |
| `plugins.rs` | 8 | Plugin registry has no account awareness |
| `goals.rs` | 7 | Goals are in-memory, no account field |
| `media.rs` | 6 | `UploadMeta` has no `account_id` field (see item #3) |
| `inbox.rs` | 1 | Needs account scoping on inbox query |

Files to change:
- All 6 route files above
- `crates/librefang-api/src/routes/media.rs` (UploadMeta struct)

### 3. HIGH: Fix upload cross-tenant read vulnerability

- [x] Stop exempting `/api/uploads/*` from `require_account_id`
- [ ] **SECURITY:** Add `account_id: Option<String>` to `UploadMeta` (or equivalent registration struct in `media.rs`)
- [ ] Populate `account_id` on upload from the request's `AccountId`
- [ ] Check ownership in `serve_upload` handler before returning file (currently accepts `_account: AccountId` but never uses it)
- [ ] Verify generated media uploads and agent uploads cannot be fetched cross-tenant
- [ ] Add integration test: tenant A uploads, tenant B fetches by file_id → 404

Files to change:
- `crates/librefang-api/src/routes/agents.rs` (`serve_upload` handler)
- `crates/librefang-api/src/routes/media.rs` (`UploadMeta` struct — needs `account_id` field)
- `crates/librefang-api/tests/account_tests.rs` (cross-tenant upload denial test)

Notes:
- **Root cause (confirmed by Codex review 2026-04-07):** `serve_upload` accepts `AccountId` but never checks it. `UploadMeta` has no owner field. A tenant who knows a `file_id` can fetch another tenant's file.
- Tracked in ADR-MT-003 §Known Gap

### 4. HIGH: Round 4b — channels.rs needs real filtering

- [x] Add `AccountId` extractor to all 11 handlers in `channels.rs`
- [ ] Wire actual tenant filtering — currently all 11 handlers use `_account` (unused)
- [ ] Ensure channel reads, mutations, QR/session flows, and reload paths do not cross tenant boundaries
- [ ] Confirm channel-driven agent spawning and channel metadata reads stay within account scope

Files to change:
- `crates/librefang-api/src/routes/channels.rs`
- `crates/librefang-api/src/channel_bridge.rs`
- `crates/librefang-api/src/routes/shared.rs`
- `crates/librefang-api/tests/account_tests.rs`

### 5. MEDIUM: Complete replay protection and sunset legacy HMAC

- [x] Bind signatures to more than raw `account_id` — timestamp+method+path binding shipped (`7637f863`)
- [x] Reject stale and replayed signatures — ±5-min window enforced
- [ ] Add nonce cache (in-memory LRU, ~10K entries) to prevent replay within the ±5-min window
- [ ] Sunset `ValidLegacy` HMAC acceptance path — add `X-Deprecation-Warning` header, log at WARN, remove by Phase 2 end
- [ ] Document the header contract and update examples/tests
- [ ] Update Qwntik integration guide to use 3-header format

Files to change:
- `crates/librefang-api/src/middleware.rs`
- `crates/librefang-types/src/config/types.rs`
- `docs/multi-tenant/ADR-MT-002-API-AUTH.md` (updated 2026-04-07)
- `crates/librefang-api/tests/account_tests.rs`

### 6. MEDIUM: Restrict global telemetry endpoints for scoped tenants

- [x] Review `/api/status` — redacted for tenants
- [x] Review `/api/health/detail` — redacted for tenants
- [x] Add 6 unit tests for redaction
- [ ] Audit remaining 64 `_account` handlers in `system.rs` — decide tier (filter/redact/deny/public) per endpoint

### 7. LOW: Replace `get() + check_account()` with `get_scoped()`

- [x] `agents.rs` — 45 call sites migrated
- [ ] Migrate remaining modules as they gain real filtering
- [ ] Keep `check_account()` only where a scoped lookup is not the right abstraction

### 8. HIGH: Expand integration test coverage

- [x] Create `crates/librefang-api/tests/account_tests.rs` — 23 integration tests (multi_thread flavor)
- [x] Cover memory endpoints end-to-end (search, list, delete, update, admin-only)
- [ ] Add cross-tenant denial tests for: channels, uploads, goals, plugins, providers, skills, workflows, system
- [ ] Ensure tests exercise the `require_account_id` middleware with the real router stack

### 9. HIGH: Kernel-level tenant filtering for stores

- [x] Memory stores — `add_scoped`, `search_scoped`, `list_by_account`, `get_scoped`, `stats_scoped`, `delete_scoped`, `consolidate_scoped`, `auto_memorize_scoped`, `import_memories_scoped` inject `account_id` into semantic metadata via `add_with_decision_meta`; `list_by_account` pre-filters at DB level (1K cap); `delete_scoped` returns `NotFound` on mismatch; `import_memories_scoped` overrides imported `account_id`
- [ ] Workflow store — workflow records need `account_id`
- [ ] Prompt store — prompt/experiment records need `account_id` (handlers use agent-ownership proxy today)
- [ ] Channel store — channel bindings need `account_id`

Notes:
- Memory kernel is now fully tenant-scoped. Remaining gap is workflow, prompt, and channel stores.
- Covered by ADR-MT-004 (Phase 3 — v19 migration for non-memory stores).

## Recommended Priority Order

### Completed
- [x] P0: Multi-tenant config gating + `require_account_id` middleware
- [x] P0: `agents.rs` scoped — 48/50 handlers tenant-filtered
- [x] P0: `memory.rs` scoped — 25/25 handlers tenant-filtered
- [x] P0: `prompts.rs` scoped — 12/12 handlers tenant-filtered via agent ownership
- [x] P1: Insecure test expectations replaced + 23 integration tests
- [x] P2: HMAC replay protection (timestamp+method+path)
- [x] P2: Telemetry redaction for tenants (2 handlers in `config.rs`)
- [x] P2: Partial data-layer filtering in budget (5), network (7), system (6), skills (2), workflows (1)
- [x] P3: `get_scoped()` refactor in agents.rs (45 call sites)

### Remaining (in priority order)
1. [x] **P0:** ~~Add `AccountId` to remaining 13 `config.rs` handlers~~ — **DONE** (10 admin-guarded, 3 Tier 4 Public)
2. [ ] **P0:** Wire real filtering in `channels.rs` (11 handlers, 0 filtered) — item #4
3. [ ] **P1:** Fix upload cross-tenant read (`UploadMeta` + `serve_upload`) — item #3
4. [x] **P1:** ~~Complete filtering in `budget.rs` (5 remaining), `network.rs` (12)~~ — **DONE** (admin-guarded via `require_admin`)
5. [ ] **P1:** Complete filtering in `workflows.rs` (29 remaining) — item #1
6. [ ] **P1:** Complete filtering in `system.rs` (57 remaining — tier decision needed) — item #1
7. [ ] **P1:** Complete filtering in `skills.rs` (51 remaining — needs ownership model) — item #1
8. [ ] **P2:** Wire filtering in `goals.rs` (7), `plugins.rs` (8), `providers.rs` (19), `inbox.rs` (1), `media.rs` (6) — item #2
9. [ ] **P2:** Nonce cache for HMAC + sunset `ValidLegacy` — item #5
10. [ ] **P2:** Expand integration tests for all newly-filtered modules — item #8
11. [ ] **P3:** Kernel/store-level `account_id` migration (ADR-MT-004, v19) — item #9

## Fork Divergence: librefang vs openfang-ai (Codex Review 2026-04-07)

### Where librefang is ahead
- **Stronger HMAC:** method+path+timestamp binding vs upstream's simple `HMAC(secret, account_id)`
- **Explicit tenant enforcement middleware:** `require_account_id` rejects missing `X-Account-Id` globally in tenant mode (upstream has no equivalent)
- **Registry-backed scoping:** `list_by_account()`, `get_scoped()`, `set_account_id()` on registry
- **Larger test suite:** 23 integration tests + 38 middleware unit tests + 8 shared.rs tests
- **Cleaner shared.rs:** 150 lines (focused) vs upstream's 504-line kitchen sink

### Where openfang-ai is ahead
- **`validate_account!` / `account_or_system!` macros** (ADR-027) — ergonomic guard patterns. Decision: don't port as-is since `require_account_id` makes `validate_account!` redundant. Consider a small helper function for `account_or_system` semantics.
- **Richer shared helper layer:** merge/persist/sync/rollback/test-builder functions in `shared.rs`
- **More internally consolidated:** smaller middleware surface, `account_sig_policy()` pure function
- **Extra route modules:** sessions, triggers, vectors, integrations (librefang doesn't have these yet)

### Convergence recommendations
1. Keep librefang's stronger HMAC as baseline — do not regress to upstream's simple scheme
2. Fix security gaps first: upload ownership (item #3), then kernel/store-level tenant filtering (item #9)
3. Add small shared helpers (not macros) for "system fallback" semantics where needed
4. Refactor toward scoped data-access APIs (`get_scoped`) over "fetch then `check_account()`"
5. Phase out legacy HMAC with docs, metrics, and removal deadline
6. Port upstream shared.rs conveniences only where they reduce duplication

## Verification

Run after each logical chunk:

```bash
cargo fmt --all
cargo clippy -p librefang-api --all-targets -- -D warnings
cargo test -p librefang-api
```

Run before declaring Phase 1 complete:

```bash
cargo build --workspace --lib
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Suggested focused checks during implementation:

```bash
cargo test -p librefang-api account
cargo test -p librefang-api middleware
cargo test -p librefang-api routes::shared
```
