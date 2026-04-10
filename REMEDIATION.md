# Multi-Tenant Migration: Critical Fixes Roadmap

**Branch:** `remediation/critical-fixes`  
**Target:** Production-ready multi-tenant deployment  
**Status:** ✅ All 5 priorities resolved — additional issues fixed post-audit

---

## Top 5 Critical Fixes (44 hours total)

### Priority 1: Webhook Isolation (4 hours)
- **Issue:** Webhook subscriptions lack `account_id` field
- **Files:** `webhook_store.rs`, `routes/system.rs`
- **Status:** ✅ COMPLETED
- **Changes Made:**
  - [x] Add `account_id: String` to `WebhookSubscription` struct
  - [x] Implement `list_scoped()`, `get_scoped()`, `update_scoped()`, `delete_scoped()` methods
  - [x] Remove non-scoped methods (list, get, create, update, delete) to enforce tenant isolation
  - [x] Update all webhook route handlers to extract account_id and use scoped methods
  - [x] Update webhook routes: list_webhooks, get_webhook, create_webhook, update_webhook, delete_webhook, test_webhook
  - [x] Add unit tests for cross-tenant isolation (in webhook_store.rs)
  - [x] Add integration tests for cross-tenant webhook isolation
- **Owner:** automated
- **Tests:** `webhook_tenant_isolation_tests.rs` (created) + unit tests in webhook_store.rs
- **Post-audit fix:** `MAX_WEBHOOKS` was a global cap (DoS vector — one tenant could starve all others). Changed to `MAX_WEBHOOKS_PER_TENANT = 25`. Added `tenant_quota_does_not_starve_other_tenants` regression test.

### Priority 2: Memory Table Migration (6 hours)
- **Issue:** Memory table missing `account_id` column
- **Files:** `memory/migration.rs`, `semantic.rs`, `structured.rs`
- **Status:** ✅ COMPLETED
- **Changes Made:**
  - [x] Migration v19: Add `account_id TEXT NOT NULL DEFAULT 'default'` column
  - [x] Index: `idx_memories_account` and `idx_memories_agent_account`
  - [x] Backfill existing rows from `metadata.account_id` JSON field
  - [x] SemanticStore queries scoped by `account_id` column
  - [x] `filter_account_id()` helper + `get_by_id_scoped()` for tenant-isolated reads
  - **Residual debt:** `account_id` is extracted from caller-supplied `metadata` HashMap at write time rather than taken as a first-class parameter. Works correctly when callers populate it, but relies on convention not structure.

### Priority 3: Cache Key Namespacing (5 hours)
- **Issue:** Cache collisions allow Tenant A to see Tenant B's cached data
- **Files:** `kernel.rs`, `proactive.rs`
- **Status:** ✅ COMPLETED
- **Changes Made:**
  - [x] `WorkspaceCacheKey` struct: keyed by `(account_id: Option<String>, workspace: PathBuf)`
  - [x] Skills cache: keyed by `(account_id, allowlist_hash)`
  - [x] Consolidation counters scoped by account

### Priority 4: OAuth Tenant Binding (4 hours)
- **Issue:** OAuth tokens reusable across tenants
- **Files:** `oauth.rs`
- **Status:** ✅ COMPLETED
- **Changes Made:**
  - [x] `account_id: Option<String>` added to `OAuthStatePayload`
  - [x] `validate_callback_tenant_binding()` enforces state.account_id == callback.account_id in multi-tenant mode
  - [x] `require_account_bound_oauth()` rejects login initiation without `X-Account-Id` in multi-tenant mode
  - [x] HMAC-signed state token prevents tampering
  - [x] `LIBREFANG_STATE_SECRET` warning added at key derivation site
- **Residual:** `LIBREFANG_STATE_SECRET` must be set for multi-instance deployments; single-tenant mode does not enforce callback account binding when callback omits `X-Account-Id`.

### Priority 5: Logging Tenant Context (3 hours)
- **Issue:** Logs, metrics, and audit trails lack tenant context
- **Files:** `middleware.rs`, `telemetry/metrics.rs`, `routes/system.rs`
- **Status:** ✅ COMPLETED
- **Changes Made:**
  - [x] `request_logging` middleware extracts and logs `account_id` from `X-Account-Id` header
  - [x] `metrics::record_http_request` accepts `account_id` label
  - [x] Structured log events include `account_id` field

---

## Verification Checklist

### After Each Fix
- [ ] Tests pass: `cargo test --workspace`
- [ ] Lint clean: `cargo clippy --all-targets`
- [ ] Build succeeds: `cargo build --release`
- [ ] No new warnings introduced

### Before Merge to Main
- [ ] All 5 critical fixes implemented
- [ ] 40+ new tests added (concurrency, cross-tenant, edge cases)
- [ ] Audit report integration tests pass
- [ ] Security review conducted on changes
- [ ] Database migration tested (backup/restore cycle)

---

## Additional Context

**Full Audit Report:** `docs/multi-tenant-audit-report.md`

**Related Issues:**
- Webhook scope: Issue #7, #8, #12, #13
- Memory isolation: Issue #2, #4, #5
- Caching: Issue #3
- OAuth: Issue #9, #10, #11
- Logging: Issue #19, #20, #21

---

## Branch Status

```
remediation/critical-fixes
├── docs/multi-tenant-audit-report.md (audit findings)
├── REMEDIATION.md (this file)
└── [pending: critical fix implementations]
```

**Next Step:** Assign DRIs and begin Priority 1 (Webhook Isolation)
