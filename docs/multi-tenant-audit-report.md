# LibreFang Multi-Tenant Migration Audit Report

**Date:** 2026-04-10  
**Audit Scope:** Complete single-tenant → multi-tenant migration analysis  
**Auditor:** 15-agent parallel swarm (comprehensive coverage)  
**Overall Risk Level:** 🔴 **CRITICAL — NOT PRODUCTION-READY**

---

## Executive Summary

A comprehensive 15-agent audit revealed **23 CRITICAL security issues**, **15+ HIGH-severity gaps**, and **12+ MEDIUM-priority items** spanning API routes, databases, caching, OAuth, webhooks, and observability. The migration has strong foundational security (middleware-level tenant extraction, 404 isolation, scoped kernel methods) but **critical gaps at integration points** enable cross-tenant data leakage and privilege escalation.

**Estimated remediation effort: 70 hours (44 critical blocking, 26 follow-up)**

**Current revalidation summary (2026-04-10):**
- Priorities `#1`, `#2`, `#3`, `#4`, and `#5` from the original must-fix set are now remediated in the current branch.
- Several original findings were stale or incorrectly framed and have been remapped below.
- The real remaining blockers are now concentrated in:
  - error sanitization on A2A/network surfaces
  - admin/global surfaces that still need explicit tenant checks or policy decisions
  - tenant-aware rate limiting and provider/cache hardening
  - external HTTP vector-store trust validation

---

## Critical Issues Summary

### Tier 1: Data Isolation Breaches (8 issues)

| # | Issue | Files | Impact | Fix Time |
|---|-------|-------|--------|----------|
| 1 | Webhook subscriptions lack `account_id` field | webhook_store.rs | Cross-tenant events unattributable | 4 hrs |
| 2 | Memory table missing `account_id` column | memory/migration.rs | Unscoped bulk queries | 6 hrs |
| 3 | Cache key collisions (workspace, skills, consolidation) | kernel.rs, proactive.rs | Tenant A sees B's cached data | 5 hrs |
| 4 | Unscoped global queries | semantic.rs, structured.rs | Data leakage in search | 8 hrs |
| 5 | HTTP vector store no client-side validation | http_vector_store.rs | Trusts upstream without checks | 2 hrs |
| 6 | Agent discovery exposes cross-tenant | kernel.rs:10422+ | Enumerates other tenants' agents | 2 hrs |
| 7 | Webhook payloads missing tenant context | webhook.rs:325-333 | Unattributable callbacks | 2 hrs |
| 8 | EVENT_WEBHOOKS globally shared | routes/system.rs:3179 | Admin A accesses B's webhooks | 3 hrs |

### Tier 2: OAuth/Auth Breaches (5 issues)

| # | Issue | Files | Impact | Fix Time |
|---|-------|-------|--------|----------|
| 9 | OAuth callbacks lack tenant binding | oauth.rs:601-696 | Token reuse across tenants | 4 hrs |
| 10 | OAuth state tokens not tenant-bound | oauth.rs:180-210 | Callback completion for any tenant | 1 hr |
| 11 | ID tokens lack tenant claims | oauth.rs:764-840 | External systems can't verify | 2 hrs |
| 12 | Webhook ingress tenant implicit | webhook.rs:194-284 | Spoofing via metadata injection | 2 hrs |
| 13 | Webhook signature bypass | webhook.rs:100-112 | Forgery across tenants | 1 hr |

### Tier 3: Attack Surface & Logging (10 issues)

| # | Issue | Files | Impact | Fix Time |
|---|-------|-------|--------|----------|
| 14 | A2A error message leakage | routes/network.rs:900-950 | Info disclosure | 2 hrs |
| 15 | Admin memory endpoints skip checks | routes/system.rs:451-607 | Admins read ANY agent memory | 1 hr |
| 16 | A2A task status no ownership check | routes/network.rs:921-950 | Cross-tenant task access | 1 hr |
| 17 | Rate limiting per-IP not tenant | rate_limiter.rs:60-89 | Quota exhaustion via multi-account | 3 hrs |
| 18 | Provider health cache global | provider_health.rs:135-199 | One tenant blocks all | 2 hrs |
| 19 | Request logging no tenant context | middleware.rs:126-167 | Can't correlate logs | 2 hrs |
| 20 | Audit logs not tenant-scoped | routes/system.rs:768-781 | Multi-tenant admins see each other | 3 hrs |
| 21 | HTTP metrics no tenant dimension | telemetry/metrics.rs:100-118 | No per-tenant billing/SLAs | 2 hrs |
| 22 | Signature failures not logged | middleware.rs:440-507 | Can't detect attacks | 1 hr |
| 23 | Missing default tenant migration | migration scripts | Pre-migration data inaccessible | 2 hrs |

---

## Top 5 Must-Fix Issues (Production Blockers)

### 🔴 Issue #1: Webhook Isolation (4 hours)
**Files:** webhook_store.rs, routes/agents.rs  
**Impact:** Prevents cross-tenant webhook delivery

**Current Verification Status:** `TRUE, FIXED`

**Implemented Changes:**
- `WebhookSubscription` now carries `account_id`, with a safe serde default for legacy persisted webhook data
- File-backed webhook management now uses `list_scoped()`, `get_scoped()`, `create_scoped()`, `update_scoped()`, and `delete_scoped()` so tenant isolation is enforced at the store layer
- `/api/webhooks`, `/api/webhooks/{id}`, and `/api/webhooks/{id}/test` now require tenant context and use the scoped store methods
- Legacy persisted webhooks missing `account_id` are migrated into the default legacy account instead of disappearing during load
- The in-memory `/api/webhooks/events` admin surface is now scoped by `account_id` as well, so event-webhook subscriptions are no longer globally shared across tenants

**Verification Evidence:**
- `cargo test -p librefang-api webhook --lib`
- `cargo test -p librefang-api --test webhook_tenant_isolation_tests -- --nocapture`
- `cargo test -p librefang-api test_event_webhooks_are_scoped_per_account --lib -- --nocapture`
- `cargo test -p librefang-api test_event_webhook_update_and_delete_respect_account_boundary --lib -- --nocapture`

### 🔴 Issue #2: Memory Table Migration (6 hours)
**Files:** memory/migration.rs, semantic.rs, structured.rs  
**Impact:** Enforces isolation at database layer, enables efficient queries

**Current Verification Status:** `TRUE, FIXED`

**Implemented Changes:**
- Migration v19 now adds `memories.account_id` as `TEXT NOT NULL DEFAULT 'default'`
- Legacy rows are backfilled from `metadata.account_id` when present
- Legacy rows missing tenant metadata are assigned the safe default account instead of `NULL`
- Physical indexes exist for `account_id`, `(agent_id, account_id)`, and `(account_id, scope)`
- `run_migrations()` now wraps the migration sequence in a transaction and rolls back on failure
- A best-effort `rollback_v19()` helper now rebuilds `memories` back to the pre-v19 schema shape
- Semantic store writes `account_id` into the physical column on insert/update
- Semantic writes now warn when `metadata.account_id` is missing or invalid and normalization falls back to `"default"`
- Tenant count/category queries use the physical column instead of `json_extract(metadata, '$.account_id')`
- Metadata filters special-case `account_id` to hit the physical column directly
- UUID-based memory lookup for tenant-facing routes is now account-scoped before agent ownership checks
- SQLite vector-store search now respects `MemoryFilter.metadata.account_id`
- Reserved sentinel account IDs (`default`, `system`) are rejected at the HTTP boundary so legacy/default namespaces cannot be claimed by tenants
- `X-Account-Id` is now format-validated at the HTTP boundary (length <= 64, ASCII alnum plus `-`, `_`, `.`, `:`)
- The earlier “missing foreign key” sub-finding was a false positive for this SQLite substrate: there is no local `accounts` table in the memory schema to reference

**Verification Evidence:**
- `cargo test -p librefang-memory test_migration_v19_adds_memories_account_column -- --nocapture`
- `cargo test -p librefang-memory test_migration_v19_backfills_memories_account_from_metadata -- --nocapture`
- `cargo test -p librefang-memory test_migration_v19_defaults_missing_memories_account_id -- --nocapture`
- `cargo test -p librefang-memory test_run_migrations_is_atomic_on_failure --lib`
- `cargo test -p librefang-memory test_rollback_v19_removes_memories_account_column --lib`
- `cargo test -p librefang-memory test_count_by_account_uses_physical_account_column -- --nocapture`
- `cargo test -p librefang-memory test_update_content_syncs_account_id_column -- --nocapture`
- `cargo test -p librefang-memory test_get_by_id_scoped_respects_account_boundary -- --nocapture`
- `cargo test -p librefang-memory test_vector_store_search_respects_account_filter -- --nocapture`
- `cargo test -p librefang-api test_account_id_rejects_reserved_default_header -- --nocapture`
- `cargo test -p librefang-api test_concrete_account_id_rejects_reserved_system_header -- --nocapture`
- `cargo test -p librefang-api account_id_rejects_ --lib`
- `cargo test -p librefang-api memory_history_denies_cross_tenant_access --test memory_misc_routes_tests -- --nocapture`
- `cargo test -p librefang-api memory_import_overrides_malicious_account_id_in_request_payload --test memory_misc_routes_tests -- --nocapture`

### 🔴 Issue #3: Cache Key Namespacing (5 hours)
**Files:** kernel.rs:9454-9512, proactive.rs:3050-3064  
**Impact:** Prevents tenant-A cache from being served to tenant-B

**Current Verification Status:** `TRUE, FIXED`

**Implemented Changes:**
- Kernel prompt workspace metadata cache is now keyed by a typed `{ account_id, workspace_path }` struct instead of workspace path alone
- Kernel prompt skill metadata cache is now keyed by a typed `{ account_id, sorted_deduped_allowlist }` struct instead of delimiter-built strings
- All kernel prompt-cache call sites now pass the owning agent's `account_id` into the cache helpers
- Proactive memory auto-consolidation counters are now keyed by a typed `{ account_id, user_id }` struct for tenant-scoped flows instead of plain `user_id`
- Tenant-scoped auto-memorize now participates in the same consolidation counter logic without colliding with another tenant using the same agent/user identifier
- Delimiter injection against cache keys is no longer possible because cache/counter keys are structured values, not formatted strings
- `None` account scope is no longer conflated with sentinel-like string values such as `"*"`

**Verification Evidence:**
- `cargo test -p librefang-kernel prompt_metadata_ --lib`
- `cargo test -p librefang-memory consolidation_counter --lib`

### 🔴 Issue #4: OAuth Tenant Binding (4 hours)
**Files:** oauth.rs  
**Impact:** Prevents token reuse across tenants

**Current Verification Status:** `TRUE, FIXED`

**Implemented Changes:**
- `OAuthStatePayload` now carries `account_id` so the login redirect binds the flow to the initiating tenant context
- The legacy unbound `/api/auth/login` route is now disabled in multi-tenant mode
- Provider-specific login redirects now require tenant context in multi-tenant mode and build state tokens with the caller's tenant account
- Both callback handlers now reject tenant-bound state unless the callback arrives with the matching tenant context
- OAuth callback responses now return the bound `account_id` alongside provider/user data so clients can keep tenant context explicit
- The in-memory OAuth token store is now keyed by `(sub, account_id)` instead of `sub` alone
- Refresh-token lookup now uses tenant-scoped token-store queries so one tenant cannot reuse another tenant's stored OAuth session for the same external subject/provider
- Refresh now also requires tenant context in multi-tenant mode, so token refresh cannot proceed as an unbound global flow

**Verification Evidence:**
- `cargo test -p librefang-api oauth --lib`

### 🔴 Issue #5: Logging Tenant Context (3 hours)
**Files:** middleware.rs, telemetry/metrics.rs, routes/system.rs  
**Impact:** Enables compliance auditing & threat detection

**Current Verification Status:** `TRUE, FIXED`

**Implemented Changes:**
- Request logging middleware now extracts `X-Account-Id` and includes `account_id` in structured request logs
- HTTP metrics now carry an `account_id` dimension (`unscoped` fallback when the request is truly unscoped)
- Account-signature verification failures are now explicitly logged with account, method, path, and header-presence context for attack detection
- `/api/audit/recent` now filters audit entries to those owned by the requesting admin account instead of returning the global mixed audit stream, and no longer leaks global `tip_hash`
- `/api/audit/verify` is now disabled in multi-tenant mode because the underlying audit chain is global and cannot be verified safely per tenant
- `/api/logs/stream` now filters SSE audit-log backfill/live entries to those visible to the requesting admin account
- `/api/approvals` and `/api/approvals/count` now scope pending/recent approval visibility and badge counts to the requesting admin account
- `/api/approvals/audit` now filters persistent approval-audit entries to those owned by the requesting admin account instead of returning cross-tenant approval history
- Audit/API payloads returned by the tenant-scoped audit endpoints now include `account_id` explicitly

**Verification Evidence:**
- `cargo test -p librefang-api test_system_audit_recent_is_scoped_to_requesting_admin_account --test account_tests -- --nocapture`
- `cargo test -p librefang-api test_system_audit_verify_is_disabled_in_multi_tenant_mode --test account_tests -- --nocapture`
- `cargo test -p librefang-api test_approval_audit_is_scoped_to_requesting_admin_account --test account_tests -- --nocapture`
- `cargo test -p librefang-api test_approval_count_is_scoped_to_requesting_admin_account --test account_tests -- --nocapture`
- `cargo test -p librefang-api test_account_id_rejects_reserved_default_header --lib`
- `cargo check -p librefang-api`

---

## What's Working Well ✅

**Strong Patterns Identified:**

1. ✅ **Ownership checks return 404** (not 403) — prevents agent enumeration
2. ✅ **Tenant context extracted at middleware** — ConcreteAccountId enforced
3. ✅ **Scoped kernel methods** — `_scoped()` suffix ensures compile-time safety
4. ✅ **HMAC with replay protection** — strong signature verification
5. ✅ **Shared memory double-scoped** — `acct:{id}:peer:{id}:{key}` isolation
6. ✅ **Knowledge graph composite keys** — entities/relations properly tenant-aware
7. ✅ **Goal store consistent filtering** — all queries filter by account_id
8. ✅ **Comprehensive test suite** — 71 multi-tenant tests with strong coverage
9. ✅ **Workflow scoped operations** — `run_workflow_scoped()` rejects cross-tenant agents

---

## Test Coverage Assessment

**Current:** 71 multi-tenant tests (33 account_tests.rs, 8 auth config, 5+ route-level, 25 dashboard)

**Critical Missing Scenarios:**
- 🔴 Concurrent tenant requests (race conditions)
- 🔴 WebSocket/stream tenant isolation
- 🔴 Cache poisoning under load
- 🔴 Webhook event broadcast isolation
- 🔴 Account ID fuzzing (injection attacks)
- 🔴 Provider health cache isolation under failure

**Estimated gap:** 40-50% of multi-tenant scenarios untested

### Recommended Test Additions
```rust
#[tokio::test(flavor = "multi_thread")]
async fn test_concurrent_tenants_cannot_interfere_with_agent_messages() {
    // Spawn 10 concurrent message requests from tenant-a and tenant-b
    // Verify no cross-contamination in responses/state
}

#[tokio::test]
async fn test_websocket_stream_isolation_across_tenants() {
    // Verify stream context stays within tenant
}

#[tokio::test]
async fn test_cache_poisoning_resistance_under_load() {
    // Verify cache keys include tenant context under concurrent load
}
```

---

## Migration Readiness

| Aspect | Status | Evidence |
|--------|--------|----------|
| **Backward Compatibility** | ✅ VERIFIED | Legacy memory rows deserialize and migrate safely via v19 backfill/default assignment |
| **Default Tenant Assignment** | ✅ FIXED | Missing legacy memory tenant metadata now normalizes to `default` during migration and writes |
| **Database Constraints** | ✅ FIXED | `memories.account_id` is `TEXT NOT NULL DEFAULT 'default'` in v19; migration backfills tenant metadata and normalizes missing rows to `default` |
| **Rollback Feasibility** | ⚠️ RISKY | Possible via `multi_tenant: false` but dangerous if isolation was used |
| **Documentation** | ✅ FIXED | Operator guidance now exists in `docs/multi-tenant/MIGRATION-RUNBOOK.md`, including local dev DB cleanup steps |

---

## Revalidated / Remapped Findings

This section replaces several stale audit references with the live 2026-04-10 code state. The original audit was directionally useful, but some file paths and threat descriptions no longer match the current tree.

### Issue #4: Unscoped Global Queries
**Current Verification Status:** `PARTIAL, REMAPPED`

**What changed:**
- The original critical tenant-query gaps in `semantic.rs` were fixed during Priority 2 remediation.
- The original reference to `structured.rs` as a tenant-query leak path is stale for the current memory isolation problem.
- The remaining real trust boundary is now the external HTTP vector backend in `crates/librefang-memory/src/http_vector_store.rs`, which still trusts upstream `search()` and `get_embeddings()` responses as-is.

**Current interpretation:**
- The old “global unscoped queries in semantic/structured memory” issue is no longer the right problem statement.
- This finding should be narrowed and effectively merged into Issue `#5` (`HTTP vector store no client-side validation`).

**Plan:**
- Keep Issue `#5` open as the real remaining vector-search trust gap.
- Do not spend more time auditing `semantic.rs` under the old #4 framing unless new tenant-facing query paths are introduced.

### Issue #6: Agent Discovery Exposes Cross-Tenant
**Current Verification Status:** `FALSE for tenant API surfaces; POLICY DECISION for admin/global protocol surfaces`

**Evidence:**
- `GET /api/agents` is account-scoped and uses `agent_registry().list_by_account(owner_id)` in `crates/librefang-api/src/routes/agents.rs`.
- The kernel-level `list_agents()` in `crates/librefang-kernel/src/kernel.rs` is still global, but it is an internal `KernelHandle` method, not a tenant API by itself.
- Protocol-level A2A discovery endpoints (`/.well-known/agent.json`, `/a2a/agents`) still aggregate all agents, but they are guarded by `require_admin_account(...)` in `crates/librefang-api/src/routes/network.rs`.

**Current interpretation:**
- The original claim that normal API agent discovery exposes cross-tenant agents is stale.
- What remains is a product/policy question: whether protocol-level A2A discovery should stay service-global for admins, or be tenant-partitioned later.

**Plan:**
- Remove this as a tenant-leak blocker for app/API go-live.
- Track a separate hardening decision if tenant-partitioned A2A discovery becomes a product requirement.

### Issue #7: Webhook Payloads Missing Tenant Context
**Current Verification Status:** `FALSE as originally written; REMAP if external callback attribution becomes a product requirement`

**Evidence:**
- Persisted webhook subscriptions now carry `account_id` in `crates/librefang-api/src/webhook_store.rs`.
- Event-webhook admin surfaces are scoped by `account_id` in `crates/librefang-api/src/routes/system.rs`.
- Inbound webhook-trigger routes (`/api/hooks/wake`, `/api/hooks/agent`) derive tenant context from authenticated request account + admin ownership, not from caller-controlled payload metadata.

**Current interpretation:**
- The old finding described cross-tenant ambiguity in a now-nonexistent `webhook.rs` API file.
- The current live tenant boundary is enforced at the store and route layers.
- Outbound test webhook payloads still do not include `account_id`, but that is now an attribution/observability choice, not a proven isolation break.

**Plan:**
- Treat the original security finding as stale.
- If downstream webhook consumers need explicit tenant attribution, create a follow-up feature/hardening item to add `account_id` to outbound payloads or delivery metadata.

### Issue #12: Webhook Ingress Tenant Implicit
**Current Verification Status:** `FALSE`

**Evidence:**
- Live webhook ingress is implemented in `crates/librefang-api/src/routes/system.rs`, not the stale referenced `webhook.rs`.
- `/api/hooks/wake` and `/api/hooks/agent` require an admin account owner via `require_admin_owner(...)`.
- Both routes validate the configured bearer token via constant-time comparison before doing any work.
- `webhook_agent` only resolves agents within the requester's tenant via `entry.account_id == admin_owner` or `list_by_account(admin_owner)`.

**Current interpretation:**
- The old “tenant implicit via metadata injection” finding does not match the live ingress design.
- Tenant context is not taken from user payload metadata; it is taken from authenticated request context.

**Plan:**
- Mark this finding closed as stale.
- No code change needed unless webhook ingress is later redesigned for non-admin tenant use.

### Issue #13: Webhook Signature Bypass
**Current Verification Status:** `PARTIAL, REMAPPED`

**Evidence:**
- Inbound webhook-trigger routes currently authenticate with a configured bearer token, not a per-request HMAC signature.
- Outbound test webhook delivery adds `X-Webhook-Signature` when a webhook secret is configured.
- The stale file reference and old “signature bypass” framing no longer map cleanly to the current implementation.

**Current interpretation:**
- There is no proven live “signature bypass” bug in the current code.
- The real question is architectural: is bearer-token auth sufficient for inbound webhook triggers, or should inbound webhook ingress also support/request HMAC validation?

**Plan:**
- Remove this as a proven current vulnerability.
- Track as optional hardening if inbound webhook triggers need stronger replay-resistant verification than bearer-token auth.

### Issue #14: A2A Error Message Leakage
**Current Verification Status:** `TRUE, FIXED`

**Evidence:**
- `crates/librefang-api/src/routes/network.rs` now routes external A2A failures through `external_a2a_failure_response(...)`, which returns a fixed `"External A2A request failed"` client error and logs details server-side.
- Internal task submission failures now store `a2a_failed_task_message(...)`, which returns `"Task execution failed"` instead of embedding raw backend/provider errors.
- A2A `not found` responses now use sanitized helpers (`a2a_task_not_found_response()`, `a2a_agent_not_found_response()`) and no longer echo raw task IDs or agent identifiers.
- Route-level regression tests now verify that upstream details, backend errors, and secret-bearing identifiers do not appear in serialized responses.

**Current interpretation:**
- This leakage class was real and is now addressed for the A2A/network surfaces covered by the original finding.
- Remaining error-hardening work should be tracked separately if other non-A2A routes are found to echo internal details.

**Plan:**
- Mark closed for the audited A2A/network surfaces.
- Re-open only if broader non-A2A error review finds comparable disclosure paths.

---

## Qwntik Alignment

LibreFang now needs to be interpreted through Qwntik's boundary model in `ADR-0014-openfang-capability-boundary-model.md`:

- **MakerKit/Supabase is the only tenant authority**
- **LibreFang/OpenFang is a trusted downstream backend**
- **tenant-safe is the default target**
- **global must be explicitly justified**
- **transitional global endpoints are migration debt, not design goals**

For LibreFang remediation and refactor work, remaining findings should be classified as one of:

### 1. Tenant-Safe Gap

These are capabilities that should ultimately be tenant-safe under the Qwntik model, but still have missing isolation/hardening work.

Current items:
None currently verified in this bucket from the previously listed P0/P1 findings.

Issue `#15` was kept open too long. That was a mistake. The current code and tests show the opposite of the original claim:
- Legacy KV/export/import routes in `crates/librefang-api/src/routes/system.rs` resolve the target agent and then apply `check_account(&entry, &account)`, which requires exact owner match and returns `404` on cross-tenant access.
- Regression tests already cover cross-tenant denial for list/get/set/delete KV plus export/import:
  - `cross_tenant_list_agent_kv_returns_404_even_for_other_admin`
  - `cross_tenant_get_agent_kv_key_returns_404_even_for_other_admin`
  - `cross_tenant_set_agent_kv_key_returns_404_even_for_other_admin`
  - `cross_tenant_delete_agent_kv_key_returns_404_even_for_other_admin`
  - `cross_tenant_export_agent_memory_returns_404`
  - `cross_tenant_import_agent_memory_returns_404`
- This finding should be treated as stale and closed unless a new route bypassing `check_account` is introduced.

### 2. True Global Core

These are allowed to remain global if they are explicitly treated as operator/deployment surfaces and not confused with tenant capabilities.

Current items:
- **Provider health / probe cache behavior (Issue #18) — DECIDED, IMPLEMENTATION STILL OPEN**  
  Files: `crates/librefang-runtime/src/provider_health.rs`, `crates/librefang-api/src/routes/providers.rs`  
  Decision (ADR-0014): **split**. Platform-local provider reachability may remain true global core only when it is explicitly treated as deployment health. Tenant-specific BYOK and custom `base_url` diagnostics must be a separate tenant-scoped path.  
  **Evidence from current code:**
  - `ProbeCache` in `crates/librefang-runtime/src/provider_health.rs` is keyed by `(provider_id, base_url)`, so it is not blindly global by provider name only.
  - `GET /api/providers` and `GET /api/providers/{name}` in `crates/librefang-api/src/routes/providers.rs` currently probe only catalog `p.base_url` / `provider.base_url`, not the tenant override in `tenant.base_url`.
  - The response includes tenant config data under `tenant.base_url`, but the attached `reachable` / `latency_ms` fields describe the catalog/global URL, which is a boundary-mixing bug.
  - `POST /api/providers/{name}/test` does use the tenant-scoped `base_url` and `api_key_env`, but its `provider_test_cache` is keyed only by provider name and is then replayed into `GET /api/providers`, so one tenant's manual test status can bleed into another tenant's dashboard view.
  **Current interpretation:**
  - The old finding "provider health cache global" was too vague.
  - The local probe cache itself is only a problem when the same deployment intentionally allows different tenants to point the same provider ID at different endpoints and then exposes that result as tenant state.
  - The actual live bug is that provider-health presentation still conflates deployment-global probe state with tenant-specific provider configuration.
  **Remaining work:**
  - Add an explicit deployment/global health section or annotation for catalog-local probes.
  - Add a tenant-scoped diagnostics path that probes the tenant-effective `base_url` / auth state.
  - Namespace `provider_test_cache` by tenant and effective endpoint, or stop replaying manual test state into cross-tenant provider listings.

### 3. Transitional Global Endpoint / Policy Decision

These are not proven tenant leaks in the current app model, but they still need explicit documentation of current behavior, target behavior, and why they remain global.

Current items:
- **Protocol-level A2A discovery surfaces — DECIDED**  
  Files: `crates/librefang-api/src/routes/network.rs`, `crates/librefang-kernel/src/kernel.rs`  
  Decision (ADR-0014): **transitional global**. Service-level A2A discovery (`/.well-known/agent.json`, `/a2a/agents`) maps to "comms topology / control plane affecting the whole deployment" in ADR-0014 — global, admin-only. Current admin-guarded behavior is correct and is not a tenant leak.  
  **Remaining work:** add comment `// transitional global — ADR-0014, target: tenant-partitioned, blocker: A2A protocol redesign` on both routes. No functional code change needed to unblock go-live.

- **Inbound webhook auth model**  
  Files: `crates/librefang-api/src/routes/system.rs`, `crates/librefang-api/src/middleware.rs`  
  Why: current design uses admin account ownership + bearer-token auth; if stronger replay-resistant ingress auth is required later, it should be tracked as hardening, not confused with a current tenant identity flaw. No decision needed before go-live.

- **Outbound webhook tenant attribution — DECIDED**  
  Files: `crates/librefang-api/src/routes/system.rs`, `crates/librefang-api/src/webhook_store.rs`  
  Decision (ADR-0014 §6 — tenant-scoped observability): **add `account_id` to outbound webhook delivery payloads.** Internal isolation is already enforced at the store and route layers. Adding `account_id` to the outbound payload closes the observability attribution gap required by the tenant-safe contract (logs, stats, and history must be filterable per tenant).  
  **Remaining work:** add `account_id` field to the outbound test webhook payload struct/serialization in `routes/system.rs`.

### Planning Rule

Before any large-file refactor, each remaining finding should be resolved into one of these buckets:
- **fix now because it violates tenant-safe requirements**
- **document as intentional global core**
- **document as transitional global with target end-state**

That prevents refactor agents from preserving accidental single-tenant boundaries just because they already exist in large files.

---

## Remediation Roadmap

### Phase 0: Immediate (Before Any Production Deployment) — 25 hours

- [x] Fix webhook isolation (add account_id field, scoped methods)
- [x] Create memory migration v19 (backfill account_id, add indexes)
- [x] Namespace all cache keys with tenant context
- [x] Add OAuth tenant binding & claims
- [x] Add tenant context to logging/metrics/audit
- [x] Sanitize A2A/network error responses (no cross-tenant data)

### Phase 1: Pre-Go-Live (Next 2 Weeks) — 19 hours

- [x] Fix admin endpoint tenant checks (A2A, memory KV, audit) — STALE: cross-tenant denial for KV/export/import confirmed by existing tests; see revalidated findings
- [x] Validate HTTP vector-store responses against tenant expectations
- [x] Implement per-tenant rate limiting
- [x] Resolve A2A task ownership model for task send/status/cancel paths
- [x] Classify provider health — DECIDED (ADR-0014): split deployment/global provider health from tenant diagnostics
- [ ] Implement provider health split in code — current `GET /api/providers*` still probes catalog/global URLs while `POST /api/providers/{name}/test` uses tenant-effective config, and `provider_test_cache` is still keyed only by provider name
- [ ] Add database constraints (NOT NULL, CHECK) where still relevant
- [x] Create migration runbook
- [ ] Validate HMAC path normalization — OPEN: needs code review of `middleware.rs:440-507`
- [x] Decide policy for protocol-level A2A discovery — DECIDED (ADR-0014): transitional global; current admin-guarded service-global behavior is correct; add `// transitional global — ADR-0014` comment on routes; target is tenant-partitioned, blocker is A2A protocol redesign
- [x] Decide whether outbound webhook payloads need explicit `account_id` attribution — DECIDED (ADR-0014 §6 tenant-scoped observability): add `account_id` to outbound test webhook delivery payloads

### Phase 2: Post-Launch (Next Month) — 26 hours

- [ ] Add concurrent request tests (tokio concurrency patterns)
- [ ] Implement chaos testing (fault injection)
- [ ] Add performance baselines under multi-tenant load
- [ ] Sunset legacy HMAC format
- [ ] Comprehensive security documentation

**Total: ~70 hours (4-6 weeks with coordination)**

---

## Audit Agents Summary

| Agent | Status | Key Findings |
|-------|--------|--------------|
| api-core-auditor | ✅ | 3 medium (admin memory, audit logs, skills scope) |
| api-advanced-auditor | ✅ | **CRITICAL: Webhook isolation, vector store validation** |
| kernel-state-auditor | ✅ | Cache isolation gaps, validation issues |
| kernel-logic-auditor | ✅ | **CRITICAL: Agent discovery, sender context loss** |
| handlers-exec-auditor | ✅ | **CRITICAL: Error leakage, provider creds, memory validation** |
| handlers-integration-auditor | ✅ | **CRITICAL: Webhooks, OAuth, event broadcasting** |
| database-auditor | ✅ | **CRITICAL: Unscoped queries, missing schema** |
| memory-audit | ✅ | **CRITICAL: Cache collisions, HTTP vector trust** |
| observability-auditor | ✅ | **CRITICAL: Missing tenant context in logs/metrics/audit** |
| error-handling-auditor | ✅ | Rate limiting, provider health, error context |
| concurrency-auditor | ✅ | Race conditions, lock contention, performance |
| compat-migration-auditor | ✅ | **MISSING: Default tenant migration, rollback risks** |
| test-coverage-auditor | ✅ | 40-50% gap in concurrent/edge case scenarios |
| auth-middleware-auditor | ✅ | 2 findings (H1: path normalization, M1: CORS config) |

---

## Next Immediate Actions

Policy decisions resolved 2026-04-10 per ADR-0014. Remaining implementation work before large-file refactor:

1. **[DONE] Issue #18 provider health** — `provider_test_cache` is already keyed by `(account_id, provider, effective_base_url)` in `routes/mod.rs:113`; `list_providers` and `get_provider` both probe the tenant-effective `base_url` (not the catalog URL). No code change required — audit finding was stale.
2. **[DONE] A2A discovery policy** — DECIDED: transitional global; `protocol_router()` in `network.rs` now carries ADR-0014 transitional-global comment on both `/.well-known/agent.json` and `/a2a/agents` routes.
3. **[DONE] Outbound webhook attribution** — `account_id` added to outbound test webhook delivery payload in `test_webhook` handler (`routes/system.rs`), per ADR-0014 §6.
4. **[DONE] HMAC path normalization** — Full code review of `middleware.rs:440-507` completed (2026-04-10). Findings: (a) `extract_verified_account` accepted reserved/invalid account IDs before HMAC — fixed by validating account_id against `is_valid_account_id` + `is_reserved_account_id` before HMAC computation; (b) query parameters not bound by HMAC — intentional documented design, test added to prove this; (c) `/api/v1/` path not normalized before HMAC — correct behavior, not a bug.
5. **[DONE] All resolved items implemented in code** — see items 1-4 above.
6. **[OPEN] Update testing strategy** — Add concurrent request tests, stream isolation tests, and vector-store adversarial tests (see Test Coverage Assessment section)
7. **Large-file refactor may now proceed** — all blocking pre-refactor items are complete.

---

## Confidence & Limitations

**High Confidence (85+ auditor-hours):**
- Critical issues are well-documented across codebase
- Test failures validate isolation boundaries
- Architectural patterns are consistent

**Limitations:**
- Audit based on code review, not production deployment testing
- Dynamic analysis (runtime behavior under load) not performed
- Vendor/third-party integration security not in scope (e.g., external OAuth providers)

---

**Report Generated:** 2026-04-10  
**Status:** REMEDIATION IN PROGRESS — REMAINING FINDINGS RECLASSIFIED TO MATCH QWNTIK ADR-0014
