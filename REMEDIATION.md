# Multi-Tenant Migration: Critical Fixes Roadmap

**Branch:** `remediation/critical-fixes`  
**Target:** Production-ready multi-tenant deployment  
**Status:** 🔴 BLOCKED (23 critical issues require fixes)

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

### Priority 2: Memory Table Migration (6 hours)
- **Issue:** Memory table missing `account_id` column
- **Files:** `memory/migration.rs`, `semantic.rs`, `structured.rs`
- **Status:** 🔴 BLOCKED
- **Required Changes:**
  - [ ] Create migration v19: Add `account_id` column
  - [ ] Create index: `idx_memories_account_id`
  - [ ] Backfill existing rows from metadata JSON
  - [ ] Update SemanticStore queries (20+ locations)
  - [ ] Update StructuredStore queries (10+ locations)
  - [ ] Add constraints: `NOT NULL`, composite key enforcement
- **Owner:** _assign_
- **Tests:** Backfill validation, query isolation tests

### Priority 3: Cache Key Namespacing (5 hours)
- **Issue:** Cache collisions allow Tenant A to see Tenant B's cached data
- **Files:** `kernel.rs`, `proactive.rs`
- **Status:** 🔴 BLOCKED
- **Required Changes:**
  - [ ] Workspace cache: `(account_id, workspace_path)` key
  - [ ] Skills cache: `{account_id}:{skill_allowlist_hash}` key
  - [ ] Consolidation counters: `{account_id}:{user_id}` key
  - [ ] Add tests for concurrent cache access across tenants
- **Owner:** _assign_
- **Tests:** Cache poisoning under load tests

### Priority 4: OAuth Tenant Binding (4 hours)
- **Issue:** OAuth tokens reusable across tenants
- **Files:** `oauth.rs`
- **Status:** 🔴 BLOCKED
- **Required Changes:**
  - [ ] Add `account_id` field to `OAuthStatePayload`
  - [ ] Validate state.account_id == request.account_id in callback
  - [ ] Add `account_id` claim to ID token payload
  - [ ] Validate token claims include matching tenant on validation
  - [ ] Add tests for state token tampering
- **Owner:** _assign_
- **Tests:** OAuth token reuse, state tampering tests

### Priority 5: Logging Tenant Context (3 hours)
- **Issue:** Logs, metrics, and audit trails lack tenant context
- **Files:** `middleware.rs`, `telemetry/metrics.rs`, `routes/system.rs`
- **Status:** 🔴 BLOCKED
- **Required Changes:**
  - [ ] Extract `AccountId` in `request_logging` middleware
  - [ ] Add `account_id` to all structured log events
  - [ ] Add tenant label to Prometheus metrics
  - [ ] Ensure audit records include `account_id`
  - [ ] Add tests verifying tenant context in logs
- **Owner:** _assign_
- **Tests:** Log context validation, metrics dimension tests

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
