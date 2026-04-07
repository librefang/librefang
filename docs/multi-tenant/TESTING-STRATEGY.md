# Multi-Tenant Testing Strategy

**Date:** 2026-04-06
**Author:** Engineering
**Related:** ADR-MT-001 through ADR-MT-005, ADR-RV-001, SPEC-MT-001 through SPEC-MT-004, SPEC-RV-001/002, PLAN-MT-001, PLAN-MT-002, PLAN-RV-001
**Epic:** Multi-Tenant Architecture
**Status:** PROPOSED

---

## 1. Test Strategy Overview

### Current State (verified 2026-04-06)

| Metric | Value |
|--------|-------|
| Total `#[test]` + `#[tokio::test]` functions in librefang | 3,458 |
| Source files with `#[cfg(test)]` modules (in-crate) | 241 |
| Integration test files (`crates/*/tests/`) | 9 |
| Multi-tenant tests | **0** |
| Test framework | Rust native `#[test]` + `tokio::test`, `tempfile`, `uuid` |

### Existing Integration Test Files

| Crate | File | Focus |
|-------|------|-------|
| librefang-api | `tests/api_integration_test.rs` | API route smoke tests |
| librefang-api | `tests/daemon_lifecycle_test.rs` | Daemon start/stop |
| librefang-api | `tests/load_test.rs` | Load/performance |
| librefang-api | `tests/openapi_spec_test.rs` | OpenAPI schema validation |
| librefang-kernel | `tests/integration_test.rs` | Kernel lifecycle |
| librefang-kernel | `tests/multi_agent_test.rs` | Multi-agent orchestration |
| librefang-kernel | `tests/wasm_agent_integration_test.rs` | WASM agent runtime |
| librefang-kernel | `tests/workflow_integration_test.rs` | Workflow execution |
| librefang-channels | `tests/bridge_integration_test.rs` | Channel bridge routing |

### Known Pre-Existing Failures (2)

From `multi_agent_test.rs` -- migration idempotency bugs unrelated to multi-tenant:
- `test_deactivate_kills_agent`: "duplicate column name: title"
- `test_default_provider_resolved_to_kernel_default`: "duplicate column name: agent_id"

These are documented in PLAN-MT-001 and must not regress further.

### Reference Implementations

| Repo | Tests | Files | Key Patterns |
|------|-------|-------|-------------|
| openfang-ai | ~5,322 test functions | 54 test/spec files | Account extraction, HMAC validation, ownership guard, info disclosure prevention |
| qwntik | 37 `.spec.ts` E2E | 37 Playwright specs | Supabase auth, agent CRUD isolation, memory isolation, session isolation |

### Three-Level Testing Model

```
Level 3: E2E (Playwright)          ~15 specs (ported from qwntik)
         Supabase auth -> account context -> agent CRUD -> isolation assertion

Level 2: Integration (multi-crate)  ~45 tests
         TestServer harness -> HTTP requests with X-Account-Id -> response assertions

Level 1: Unit (Rust #[test])        ~80 tests
         Type construction, HMAC math, extractor parsing, guard logic, SQL filtering
```

### Phase-Gated Approach

Each implementation phase (1-4) has its own test suite. A phase gate cannot pass
until all tests for that phase and all prior phases are green. This prevents
forward progress on leaky foundations.

```
Phase 1 gate: 33 tests (SPEC-MT-001 claims) + 3,458 existing tests pass
Phase 2 gate: Phase 1 + ~35 new tests (SPEC-MT-002 claims) + all existing pass
Phase 3 gate: Phase 1+2 + ~25 new tests (SPEC-MT-003 claims) + all existing pass
Phase 4 gate: Phase 1+2+3 + ~20 new tests (hardening) + all existing pass
E2E gate:     All unit+integration + ~15 Playwright specs pass
```

---

## 2. Per-Phase Test Plan

### Phase 1: Foundation (ADR-MT-001, ADR-MT-002, SPEC-MT-001, PLAN-MT-001)

**New test file:** `crates/librefang-api/tests/account_tests.rs`
**Total new tests:** 33

All 33 tests are mapped 1:1 to SPEC-MT-001 claims table (C-1 through C-33).

#### Group A: AccountId Type Construction (6 unit tests)

| Test Name | SPEC Claim | Assertion |
|-----------|-----------|-----------|
| `test_account_id_present` | C-1 | `AccountId(Some("user-abc-123"))` from valid `X-Account-Id` header |
| `test_account_id_absent` | C-2 | `AccountId(None)` when no `X-Account-Id` header present |
| `test_account_id_whitespace_only_treated_as_absent` | C-3 | `AccountId(None)` for `X-Account-Id: "   "` |
| `test_account_id_empty_string` | C-4 | `AccountId(None)` for `X-Account-Id: ""` |
| `test_account_id_extraction_is_infallible` | C-5 | `Rejection = Infallible` -- extractor never returns `Err` |
| `test_account_id_uuid_style` | C-6 | `AccountId(Some("550e8400-e29b-41d4-a716-446655440000"))` parses correctly |

**Pattern under test:** `impl FromRequestParts for AccountId` (infallible extractor)

#### Group B: HMAC Signature Validation (5 unit tests)

| Test Name | SPEC Claim | Assertion |
|-----------|-----------|-----------|
| `test_verify_account_sig_valid` | C-7 | `verify_account_sig("test-secret", "acc-123", &valid_hex)` returns `true` |
| `test_verify_account_sig_wrong_account_id` | C-8 | Wrong account_id returns `false` |
| `test_verify_account_sig_wrong_secret` | C-9 | Wrong secret returns `false` |
| `test_verify_account_sig_malformed_hex` | C-10 | Non-hex string returns `false` |
| `test_verify_account_sig_empty_sig` | C-11 | Empty signature returns `false` |

**Pattern under test:** `HMAC-SHA256(secret, account_id)` with `verify_slice()` (constant-time)

#### Group C: HMAC Policy Matrix (5 unit tests)

| Test Name | SPEC Claim | Input: (secret, account_id, sig) | Expected |
|-----------|-----------|----------------------------------|----------|
| `test_policy_no_secret_passes_through` | C-12 | `(None, Some("acc"), Some("sig"))` | `None` (pass) |
| `test_policy_no_account_id_passes_through` | C-13 | `(Some("secret"), None, Some("sig"))` | `None` (pass) |
| `test_policy_sig_absent_returns_error` | C-14 | `(Some("secret"), Some("acc"), None)` | `Some("Missing X-Account-Sig header")` |
| `test_policy_sig_invalid_returns_error` | C-15 | `(Some("secret"), Some("acc"), Some("bad"))` | `Some("Invalid account signature")` |
| `test_policy_valid_sig_passes_through` | C-16 | `(Some("secret"), Some("acc"), Some(valid))` | `None` (pass) |

**Pattern under test:** `account_sig_policy()` 5-case matrix from SPEC-MT-001

#### Group D: check_account Guard (6 unit tests)

| Test Name | SPEC Claim | Assertion |
|-----------|-----------|-----------|
| `test_check_account_matching_owner` | C-17 | `AgentEntry{account_id: Some("u1")}` + `AccountId(Some("u1"))` returns `Ok(())` |
| `test_check_account_mismatching_owner_returns_404` | C-18 | Different owner returns `Err(404)` -- NOT 403 |
| `test_check_account_no_header_allows_all` | C-19 | `AccountId(None)` returns `Ok(())` for any entry (admin/legacy) |
| `test_check_account_no_header_allows_unowned` | C-20 | `AccountId(None)` + `AgentEntry{account_id: None}` returns `Ok(())` |
| `test_check_account_scoped_request_vs_unowned_agent_returns_404` | C-21 | `AccountId(Some("u1"))` + `AgentEntry{account_id: None}` returns `Err(404)` |
| `test_check_account_error_body_is_generic` | C-22 | Error JSON contains `"Agent not found"` only -- never leaks real `account_id` |

**Pattern under test:** `check_account()` returns 404 (not 403) on cross-tenant, generic error body

#### Group E: Migration v18 (2 tests)

| Test Name | SPEC Claim | Assertion |
|-----------|-----------|-----------|
| `test_migration_v18_agents_account_id` | C-23 | `PRAGMA table_info(agents)` includes `account_id TEXT NOT NULL DEFAULT 'system'` |
| `test_migration_v18_default_system` | C-24 | Pre-existing agent rows have `account_id = 'system'` after migration |

**Additional migration tests (from PLAN-MT-001 TDD cycles):**

| Test Name | Round | Assertion |
|-----------|-------|-----------|
| `test_migration_v18_adds_account_id` | Round 2 | Fresh DB migrated, column exists |
| `test_migration_v18_idempotent` | Round 2 | Running migration twice does not produce "duplicate column" error |

**Pattern under test:** Column-existence-guarded `ALTER TABLE` (learned from pre-existing migration bugs)

#### Group F: Agent CRUD Scoping (5 integration tests)

| Test Name | SPEC Claim | Assertion |
|-----------|-----------|-----------|
| `test_spawn_agent_stores_account_id` | C-25 | POST `/api/agents` with `X-Account-Id: user-1` stores `account_id = "user-1"` |
| `test_list_agents_filters_by_account` | C-26 | 3 agents (u1, u2, system); GET with `X-Account-Id: user-1` returns only u1's agent |
| `test_get_agent_cross_tenant_404` | C-27 | GET `/api/agents/{id}` with wrong account returns 404 |
| `test_delete_agent_cross_tenant_404` | C-28 | DELETE `/api/agents/{id}` with wrong account returns 404, agent NOT deleted |
| `test_no_header_admin_sees_all_agents` | C-29 | GET `/api/agents` without `X-Account-Id` returns all agents (backward compat) |

**Pattern under test:** `registry.list_by_account()`, `registry.get_scoped()`, cross-tenant returns 404

#### Group G: Macros and Remaining Handlers (4 tests)

| Test Name | SPEC Claim | Assertion |
|-----------|-----------|-----------|
| `test_account_or_system_defaults_to_system` | C-30 | `account_or_system!(AccountId(None))` returns `"system"` |
| `test_channels_accept_account_id_extractor` | C-31 | Channel handlers accept `AccountId` param without error |
| `test_config_reads_default_to_system` | C-32 | GET `/api/config/status` without header returns 200 (not 400) |
| `test_config_mutation_requires_account` | C-33 | POST `/api/config/...` without header returns 400 |

#### Phase 1 Kernel Tests (from PLAN-MT-001 Round 3)

| Test Name | Round | Assertion |
|-----------|-------|-----------|
| `test_spawn_agent_stores_account_id` | Round 3 | `spawn_agent_inner` with `AccountId(Some("t1"))` stores in `AgentEntry` |
| `test_list_by_account_filters` | Round 3 | Spawn agents for t1, t2; `list_by_account(t1)` returns only t1's |
| `test_list_by_account_none_returns_all` | Round 3 | `AccountId(None)` returns all agents (legacy mode) |
| `test_get_scoped_cross_tenant_returns_none` | Round 3 | `get_scoped(t1_agent, t2_account)` returns `None` |

#### Phase 1 Regression Gate

```bash
#!/bin/bash
set -euo pipefail

echo "=== Phase 1 Test Gate ==="

# 1. All 33 SPEC-MT-001 tests pass
cargo test -p librefang-api --test account_tests
echo "PASS: 33 account tests"

# 2. All 3,458 existing tests still pass
cargo test --workspace
echo "PASS: workspace tests (3,458 existing + 33 new)"

# 3. Pattern coverage: all Phase 1 handlers scoped
for f in agents.rs channels.rs config.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  [ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $f has unscoped handlers"; exit 1; }
  echo "PASS: $f: $SCOPED/$TOTAL scoped"
done

# 4. Clippy clean
cargo clippy --workspace --all-targets -- -D warnings
echo "PASS: clippy clean"

echo "=== Phase 1 Gate: ALL PASSED ==="
```

---

### Phase 2: Resource Isolation (ADR-MT-003, ADR-MT-005, SPEC-MT-002)

**New test file:** `crates/librefang-api/tests/resource_isolation_tests.rs`
**Total new tests:** ~35

All tests mapped to SPEC-MT-002 acceptance criteria (AC-1 through AC-6) and claims table.

#### Tier 1 -- Full Ownership Tests (per-route cross-account access returns 404)

| Test Name | SPEC-MT-002 Claim | Route File | Assertion |
|-----------|-------------------|------------|-----------|
| `test_cross_tenant_skill_404` | `test_cross_tenant_skill_404` | skills.rs (53 handlers) | Account-B GET of account-A's skill returns 404 |
| `test_cross_tenant_workflow_404` | `test_cross_tenant_workflow_404` | workflows.rs (30 handlers) | Account-B GET of account-A's workflow returns 404 |
| `test_cross_tenant_goal_404` | (AC-1) | goals.rs (7 handlers) | Account-B GET of account-A's goal returns 404 |
| `test_cross_tenant_media_404` | (AC-1) | media.rs (6 handlers) | Account-B GET of account-A's media returns 404 |
| `test_cross_tenant_inbox_404` | (AC-1) | inbox.rs (1 handler) | Account-B access to account-A's inbox returns 404 |
| `test_tier1_all_check_account` | `test_tier1_all_check_account` | All Tier 1 files | Pattern grep: every Tier 1 handler calls `check_account()` |

#### Tier 2 -- Account-Filtered Tests (system visible, writes require account)

| Test Name | SPEC-MT-002 Claim | Route File | Assertion |
|-----------|-------------------|------------|-----------|
| `test_system_provider_visible_all` | `test_system_provider_visible_all` | providers.rs (19 handlers) | System provider "openai" visible to all accounts |
| `test_tenant_provider_invisible_cross_account` | (AC-2) | providers.rs | Account-A's custom provider not visible to account-B |
| `test_provider_write_requires_account` | (AC-3) | providers.rs | POST without `X-Account-Id` returns 400 |
| `test_budget_write_requires_account` | (AC-3) | budget.rs (10 handlers) | POST without account returns 400 |
| `test_plugin_system_visible_all` | (AC-2) | plugins.rs (8 handlers) | System plugins visible to all, tenant plugins isolated |
| `test_tier2_reads_system_fallback` | `test_tier2_reads_system_fallback` | All Tier 2 files | Pattern grep: reads use `account_or_system!` |
| `test_tier2_writes_require_account` | `test_tier2_writes_require_account` | All Tier 2 files | Pattern grep: writes use `validate_account!` |

#### Tier 3 -- Shared + Overlay Tests

| Test Name | SPEC-MT-002 Claim | Route File | Assertion |
|-----------|-------------------|------------|-----------|
| `test_memory_recall_scoped` | `test_memory_recall_scoped` | memory.rs (25 handlers) | Account-A recall returns only A's memories + system |
| `test_memory_system_visible` | (AC-4) | memory.rs | System memories visible to scoped accounts |
| `test_network_account_or_system` | (AC-4) | network.rs (19 handlers) | Network peers shared across accounts |
| `test_channel_listing_scoped` | `test_channel_listing_scoped` | channels.rs (11 handlers) | Channel configured by A not visible to B |
| `test_channel_message_routing_scoped` | (AC-6) | channels.rs | Messages from A's channel not routed to B's agents |

#### Tier 4 -- Public Endpoints

| Test Name | SPEC-MT-002 Claim | Route File | Assertion |
|-----------|-------------------|------------|-----------|
| `test_health_no_account` | `test_health_no_account` | system.rs (63 handlers) | GET `/health` returns 200 without `X-Account-Id` |
| `test_version_no_account` | (AC-5) | system.rs | GET `/version` returns 200 without header |
| `test_ready_no_account` | (AC-5) | system.rs | GET `/ready` returns 200 without header |
| `test_wellknown_no_account` | (AC-5) | system.rs | GET `/.well-known/agent.json` returns 200 without header |

#### Skill Allowlist Tests

| Test Name | ADR-MT-003 | Assertion |
|-----------|-----------|-----------|
| `test_skill_allowlist_enforced` | Resource Isolation | Account with allowlist `["web_search"]` cannot invoke `code_exec` |
| `test_empty_allowlist_all_skills` | Resource Isolation | Empty allowlist = all skills available (backward compat) |

#### Event Bus Filtering Tests

| Test Name | ADR-MT-003 | Assertion |
|-----------|-----------|-----------|
| `test_event_bus_filters_by_account` | Event Bus Isolation | Account-A subscriber receives only account-A events |
| `test_event_bus_system_events_broadcast` | Event Bus Isolation | System-account events visible to all subscribers |

#### Phase 2 Pattern Coverage Gate

```bash
#!/bin/bash
set -euo pipefail

echo "=== Phase 2 Test Gate ==="

# 1. All Phase 1 + Phase 2 tests pass
cargo test -p librefang-api --test account_tests
cargo test -p librefang-api --test resource_isolation_tests
echo "PASS: account + isolation tests"

# 2. Pattern coverage: ALL route files scoped
for f in agents.rs channels.rs config.rs skills.rs workflows.rs \
         memory.rs network.rs providers.rs budget.rs plugins.rs \
         goals.rs media.rs inbox.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  UNSCOPED=$((TOTAL - SCOPED))
  [ "$UNSCOPED" -eq 0 ] || { echo "FAIL: $f: $SCOPED/$TOTAL ($UNSCOPED remaining)"; exit 1; }
  echo "PASS: $f: $SCOPED/$TOTAL scoped"
done

# 3. system.rs: non-public handlers scoped
TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/system.rs")
PUBLIC=$(grep -c "// PUBLIC" "crates/librefang-api/src/routes/system.rs" || echo 0)
SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/system.rs" || echo 0)
EXPECTED=$((TOTAL - PUBLIC))
[ "$SCOPED" -ge "$EXPECTED" ] || { echo "FAIL: system.rs non-public"; exit 1; }
echo "PASS: system.rs: $SCOPED/$EXPECTED non-public scoped ($PUBLIC public)"

# 4. Full workspace green
cargo test --workspace
echo "PASS: workspace tests"

echo "=== Phase 2 Gate: ALL PASSED ==="
```

---

### Phase 3: Data & Memory Isolation (ADR-MT-004, SPEC-MT-003, SPEC-MT-004)

**New test file:** `crates/librefang-memory/tests/data_isolation_tests.rs`
**Total new tests:** ~25

All tests mapped to SPEC-MT-003 acceptance criteria (AC-1 through AC-10) and claims table.

#### Migration v19 Tests

| Test Name | SPEC-MT-003 Claim | Assertion |
|-----------|-------------------|-----------|
| `test_migration_v19_all_tables` | `test_migration_v19_all_tables` | All 14 tables have `account_id TEXT NOT NULL DEFAULT 'system'` after v19 |
| `test_migration_v19_idempotent` | `test_migration_v19_idempotent` | Running v19 migration twice produces no errors |
| `test_migration_v19_default_system` | `test_migration_v19_default_system` | Pre-existing rows in all 14 tables have `account_id = 'system'` |
| `test_migration_v19_fts5_rebuilt` | `test_migration_v19_fts5_rebuilt` | `sessions_fts` recreated with `account_id` column, row count matches `sessions` |

**Idempotency detail:** The column-existence guard (`PRAGMA table_info` check) must prevent
the "duplicate column name" class of bug that already broke 2 existing tests. This is the
single most important migration test.

**Tables verified per test (14 of 19 total):**

```
sessions, events, kv_store, memories, entities, relations, task_queue,
usage_events, canonical_sessions, paired_devices, audit_entries,
prompt_versions, prompt_experiments, approval_audit
```

Not scoped (by design): `migrations` (system), `sessions_fts` (rebuilt), `experiment_variants` (FK-scoped), `experiment_metrics` (FK-scoped).

#### Per-Store Cross-Account Invisibility Tests

6 store files, 106 total methods, ~76 account-sensitive methods (per ADR-MT-004).
Each store gets at least one positive and one negative isolation test.

| Test Name | SPEC-MT-003 Claim | Store File (methods) | Assertion |
|-----------|-------------------|---------------------|-----------|
| `test_kv_filtered_by_account` | `test_kv_filtered_by_account` | structured.rs (11) | `list_kv("agent-1", "account-A")` returns only A's entries |
| `test_kv_cross_account_invisible` | (AC-5) | structured.rs | Account-B's KV entries not visible to account-A |
| `test_recall_filtered_by_account` | `test_recall_filtered_by_account` | semantic.rs (24) | Semantic recall with account-A filter returns only A's memories |
| `test_recall_cross_account_invisible` | (AC-6) | semantic.rs | "secret-B" not leaked to account-A |
| `test_system_sees_all` | `test_system_sees_all` | semantic.rs | `account_id = "system"` or `AccountId(None)` returns all data |
| `test_session_scoped` | `test_session_scoped` | session.rs (22) | Account-B cannot list/compact/delete account-A's sessions |
| `test_session_lifecycle_cross_account` | (AC-8) | session.rs | Full lifecycle (create, list, compact, delete) scoped by account |
| `test_knowledge_entities_scoped` | (AC-5) | knowledge.rs (6) | Entity CRUD scoped by account |
| `test_knowledge_relations_scoped` | (AC-5) | knowledge.rs | Relation queries filter by account |
| `test_usage_scoped_billing` | `test_usage_scoped_billing` | usage.rs (17) | Billing query for A returns only A's token usage/costs |
| `test_proactive_memory_scoped` | (AC-6) | proactive.rs (26) | Proactive memory hooks scoped by account |
| `test_audit_merkle_preserved` | `test_audit_merkle_preserved` | (audit_entries) | Existing Merkle hash chain integrity preserved after migration |

#### FTS5 Rebuild Verification

| Test Name | SPEC-MT-003 Claim | Assertion |
|-----------|-------------------|-----------|
| `test_fts5_search_scoped_by_account` | (AC-4) | Full-text search returns only matching account's session content |
| `test_fts5_rebuild_row_count_matches` | `test_migration_v19_fts5_rebuilt` | `SELECT COUNT(*) FROM sessions_fts` equals `SELECT COUNT(*) FROM sessions` |

#### Supabase Vector Store Account Scoping

| Test Name | SPEC-RV-002 | Assertion |
|-----------|-------------|-----------|
| `test_supabase_vector_insert_with_account_id` | (RLS) | Vector insert includes `account_id` in payload metadata |
| `test_supabase_vector_search_rls_filtered` | (RLS) | Search results filtered by Supabase RLS policy per JWT claim |
| `test_supabase_rls_cross_account_invisible` | (RLS) | Account-A cannot retrieve account-B's vectors |

**Note:** Supabase RLS tests require a running Supabase instance. They run in CI with
`docker-compose` from `docker/docker-compose.yml` but are skipped in local `cargo test`
unless `SUPABASE_URL` is set.

#### Phase 3 Gate

```bash
#!/bin/bash
set -euo pipefail

echo "=== Phase 3 Test Gate ==="

# 1. Migration tests
cargo test -p librefang-memory -- migration_v19
echo "PASS: migration v19 tests"

# 2. Store isolation tests
cargo test -p librefang-memory --test data_isolation_tests
echo "PASS: data isolation tests"

# 3. Per-store method count verification
for f in structured.rs semantic.rs session.rs knowledge.rs usage.rs proactive.rs; do
  TOTAL=$(grep -c "pub fn\|pub async fn" "crates/librefang-memory/src/$f")
  SCOPED=$(grep -c "account_id\|account:" "crates/librefang-memory/src/$f" || echo 0)
  echo "$f: $SCOPED account references in $TOTAL methods"
done

# 4. All 14 tables verified via SQLite PRAGMA
cargo test -p librefang-memory -- test_migration_v19_all_tables

# 5. All prior phase tests still pass
cargo test -p librefang-api --test account_tests
cargo test -p librefang-api --test resource_isolation_tests

# 6. Full workspace green
cargo test --workspace
echo "PASS: workspace tests"

echo "=== Phase 3 Gate: ALL PASSED ==="
```

---

### Phase 4: Hardening (Security Audit + Performance)

**New test file:** `crates/librefang-api/tests/security_hardening_tests.rs`
**Total new tests:** ~20

#### Security Audit Checklist Tests

| Test Name | Assertion |
|-----------|-----------|
| `test_every_sql_query_includes_account_id` | Static analysis: `grep` every SQL string literal in `crates/librefang-memory/src/*.rs` -- each `SELECT`, `INSERT`, `UPDATE`, `DELETE` on tenant-scoped tables includes `account_id` in WHERE or column list |
| `test_no_sql_query_uses_string_interpolation` | No `format!("... {account_id} ...")` in SQL -- all use parameterized `?` |
| `test_hmac_constant_time_verification` | `verify_account_sig()` uses `verify_slice()` (constant-time) not `==` |
| `test_error_responses_never_leak_account_ids` | For every 4xx/5xx response across all handlers, response body does not contain any account_id string |
| `test_cross_account_agent_enumeration_impossible` | Sequential GET `/api/agents/{uuid}` with wrong account always returns 404 -- no timing difference reveals existence |

#### Cross-Account Penetration Suite

Each test creates two accounts (acc_test_a, acc_test_b) with pre-loaded data, then
attempts every documented cross-tenant access pattern.

| Test Name | Attack Vector | Expected Result |
|-----------|--------------|----------------|
| `test_pentest_agent_crud_cross_account` | Account-B CRUDs account-A's agents | 404 on every operation |
| `test_pentest_session_access_cross_account` | Account-B reads account-A's session messages | 404 or empty |
| `test_pentest_memory_recall_cross_account` | Account-B recalls account-A's semantic memories | Empty result set |
| `test_pentest_kv_read_cross_account` | Account-B reads account-A's KV store entries | Empty or 404 |
| `test_pentest_workflow_trigger_cross_account` | Account-B triggers account-A's workflow | 404 |
| `test_pentest_skill_invoke_cross_account` | Account-B invokes skill owned by account-A | 404 |
| `test_pentest_channel_message_cross_account` | Account-B sends message via account-A's channel binding | Rejected/404 |
| `test_pentest_config_mutation_cross_account` | Account-B mutates account-A's config | 404 |
| `test_pentest_usage_billing_cross_account` | Account-B queries account-A's usage data | Empty |
| `test_pentest_audit_trail_cross_account` | Account-B reads account-A's audit entries | Empty |

#### Timing-Safe HMAC Verification

| Test Name | Assertion |
|-----------|-----------|
| `test_hmac_timing_safe_no_early_return` | Verifying a valid vs invalid signature of the same length shows no statistically significant timing difference over 1000 iterations (< 1ms variance) |
| `test_hmac_invalid_hex_no_timing_leak` | Malformed hex input does not short-circuit in a way that leaks valid hex prefix length |

#### Performance Benchmark Tests

| Test Name | Assertion | Threshold |
|-----------|-----------|-----------|
| `test_perf_list_agents_with_account_filter` | `list_by_account()` latency vs unfiltered `list()` | < 5% overhead |
| `test_perf_sql_query_with_account_id_index` | `SELECT * FROM agents WHERE account_id = ? AND id = ?` uses index scan | No full table scan (`EXPLAIN QUERY PLAN` does not contain `SCAN TABLE`) |
| `test_perf_account_id_extraction_overhead` | `FromRequestParts` extraction adds < 1us per request | < 1us measured over 10,000 iterations |
| `test_perf_hmac_verification_overhead` | HMAC-SHA256 computation per request | < 50us per request |
| `test_perf_1000_agents_across_100_accounts` | List agents for one account in a dataset of 1,000 agents across 100 accounts | < 10ms (indexed) |

#### Phase 4 Gate

```bash
#!/bin/bash
set -euo pipefail

echo "=== Phase 4 Test Gate ==="

# 1. Security hardening tests
cargo test -p librefang-api --test security_hardening_tests
echo "PASS: security hardening"

# 2. All prior phase tests
cargo test -p librefang-api --test account_tests
cargo test -p librefang-api --test resource_isolation_tests
cargo test -p librefang-memory --test data_isolation_tests
echo "PASS: all phase tests"

# 3. Performance benchmarks (release mode)
cargo test --release -p librefang-api --test security_hardening_tests -- test_perf
echo "PASS: performance benchmarks"

# 4. Full workspace
cargo test --workspace
echo "PASS: workspace tests"

# 5. Clippy clean
cargo clippy --workspace --all-targets -- -D warnings
echo "PASS: clippy"

echo "=== Phase 4 Gate: ALL PASSED ==="
```

---

## 3. Contract Tests (from qwntik)

### Ported E2E Specs

qwntik has 37 Playwright `.spec.ts` files. Three are directly relevant to multi-tenant
isolation and should be ported as contract tests to validate that the `@kit/openfang` SDK
continues to work against the librefang backend.

#### Source Specs to Port

| qwntik File | Lines | Describe Blocks | Port Priority |
|-------------|-------|-----------------|--------------|
| `openfang-multi-tenant.spec.ts` | 245 | 2 | P0 -- core isolation contract |
| `memory-agent-isolation.spec.ts` | ~150 | 1 | P0 -- memory isolation |
| `sessions-agent-isolation.spec.ts` | ~120 | 1 | P0 -- session isolation |

#### Test Pattern (from qwntik)

Every E2E test follows this pattern:
```
1. Supabase auth: create/login user with known account_id
2. SDK call: create agent/memory/session via @kit/openfang
3. Cross-account attempt: different user tries to access resource
4. Isolation assertion: 404 or empty result
```

#### Ported Contract Test Names

| Test Name | Source Spec | Assertion |
|-----------|-----------|-----------|
| `test_e2e_agent_crud_isolation` | openfang-multi-tenant.spec.ts | User-A creates agent, User-B cannot see/modify/delete it |
| `test_e2e_agent_list_isolation` | openfang-multi-tenant.spec.ts | User-A sees only own agents in list |
| `test_e2e_memory_store_isolation` | memory-agent-isolation.spec.ts | Memory stored by User-A's agent not recallable by User-B's agent |
| `test_e2e_memory_recall_scoped` | memory-agent-isolation.spec.ts | Recall with account context returns only scoped results |
| `test_e2e_session_create_isolation` | sessions-agent-isolation.spec.ts | Session created under User-A not visible to User-B |
| `test_e2e_session_messages_isolation` | sessions-agent-isolation.spec.ts | Session message history isolated per account |
| `test_e2e_sdk_unchanged` | (parity) | SDK methods (agent.create, agent.list, memory.store, memory.recall) work without changes when account headers are present |

#### Parity-Audit Tests

qwntik has 23 parity-audit spec files that verify the `@kit/openfang` SDK works identically
against both openfang-ai and librefang. These serve as regression tests for the
multi-tenant port:

| Category | Spec Count | What They Verify |
|----------|-----------|-----------------|
| API endpoint tiers + batches | 7 | Tier 1/2/3 endpoints, complete audit, batched endpoint ranges (1-48, 49-87, 88-140) |
| Page features | 5 | Overview/agents, approvals/budget, comms/channels, knowledge/skills, remaining pages |
| Live integration + verification | 4 | Live openfang integration, live API verification, live SSE validation, tier-1 real |
| SSE + sessions | 2 | SSE live sessions page, SSE mock logs page |
| UI components | 2 | Approvals page components, workflow components |
| Client + edge cases | 2 | Client integration tests, edge cases + error handling |
| Auth + client SDK | 1 | Tier-1 authenticated endpoint parity |

---

## 4. Test Infrastructure

### Test Helpers

All multi-tenant tests share a common set of helpers in a shared test utility module.

#### File: `crates/librefang-api/tests/helpers/account_helpers.rs`

```rust
/// Create a test account with a known ID.
/// Returns the account_id string for use in X-Account-Id headers.
pub fn create_test_account(suffix: &str) -> String {
    format!("acc_test_{}", suffix)
}

/// Create an agent manifest scoped to a specific account.
/// Uses the test account's ID and a unique agent name.
pub fn create_test_agent_for_account(
    server: &TestServer,
    account_id: &str,
    agent_name: &str,
) -> AgentId {
    let response = server
        .post("/api/agents")
        .header("X-Account-Id", account_id)
        .json(&json!({ "name": agent_name, "manifest": minimal_manifest() }))
        .send();
    assert_eq!(response.status(), 200);
    response.json::<AgentResponse>().id
}

/// Assert that a cross-account access attempt returns 404.
/// Verifies both the status code and that the response body
/// does not contain the real owner's account_id.
pub fn assert_cross_account_404(
    response: &Response,
    real_owner_account: &str,
) {
    assert_eq!(response.status(), 404);
    let body = response.text();
    assert!(!body.contains(real_owner_account),
        "Response body must not leak real owner's account_id");
    assert!(body.contains("not found"),
        "Response should contain generic 'not found' message");
}

/// Generate a valid HMAC-SHA256 signature for testing.
pub fn sign_account(secret: &str, account_id: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(account_id.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}
```

### Test Fixtures

All multi-tenant integration tests use two pre-defined accounts with pre-loaded data:

| Fixture | Account ID | Pre-loaded Resources |
|---------|-----------|---------------------|
| Account A | `acc_test_a` | 3 agents, 5 memory fragments, 2 sessions, 1 workflow, 2 skills |
| Account B | `acc_test_b` | 2 agents, 3 memory fragments, 1 session, 1 workflow, 1 skill |
| System | `system` (default) | 1 agent, 2 system-level memories, global config |

Fixture setup runs before each test module via `#[ctor]` or test setup function.
Each test creates a fresh in-memory SQLite database to ensure isolation between tests.

### CI Configuration

#### Per-Phase CI Gates

```yaml
# .github/workflows/multi-tenant-tests.yml

jobs:
  phase-1-foundation:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p librefang-types
      - run: cargo test -p librefang-memory -- migration_v18
      - run: cargo test -p librefang-kernel -- account
      - run: cargo test -p librefang-api --test account_tests
      - run: cargo test --workspace

  phase-2-isolation:
    needs: phase-1-foundation
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p librefang-api --test resource_isolation_tests
      - run: cargo test --workspace

  phase-3-data:
    needs: phase-2-isolation
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p librefang-memory --test data_isolation_tests
      - run: cargo test --workspace

  phase-4-hardening:
    needs: phase-3-data
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test -p librefang-api --test security_hardening_tests
      - run: cargo test --release -p librefang-api --test security_hardening_tests -- test_perf
      - run: cargo test --workspace

  e2e-contract:
    needs: phase-4-hardening
    runs-on: ubuntu-latest
    services:
      supabase:
        image: supabase/postgres:17.6.1.095
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: 22
      - run: npx playwright install --with-deps
      - run: npx playwright test --project=multi-tenant
```

#### Test Execution Order

```
cargo test -p librefang-types          # AccountId type + unit
cargo test -p librefang-memory         # Migration + store isolation
cargo test -p librefang-kernel         # Registry scoping
cargo test -p librefang-api            # Extractors, macros, guards, handlers
cargo test --workspace                 # Full regression (3,458 existing + new)
cargo test --release -- test_perf      # Performance benchmarks
npx playwright test                    # E2E contract tests (CI only)
```

---

## 5. Coverage Targets

### Per-Level Targets

| Level | Scope | Target | Measurement |
|-------|-------|--------|-------------|
| Unit | New code (types, extractors, macros, guards, HMAC) | 80%+ branch coverage | `cargo llvm-cov --branch` |
| Integration | Every route handler | 100% of 317 handlers accept `account: AccountId` param | Pattern gate script (grep for `account: AccountId`) |
| Data isolation | Every store method | All ~76 account-sensitive methods tested with cross-account assertion | Per-store test count verification |
| E2E | User-visible flows | Agent isolation + memory isolation + session isolation | 7 ported Playwright specs |
| Security | Penetration | Zero cross-account data leaks in 10-scenario penetration suite | All `test_pentest_*` tests pass |
| Performance | Overhead | < 5% latency overhead from account filtering | Benchmark tests in release mode |

### New Test Count Summary

| Phase | New Tests | Cumulative |
|-------|----------|------------|
| Phase 1 (Foundation) | 33 | 33 |
| Phase 2 (Resource Isolation) | ~35 | ~68 |
| Phase 3 (Data Isolation) | ~25 | ~93 |
| Phase 4 (Hardening) | ~20 | ~113 |
| E2E (Contract) | ~15 | ~128 |
| **Total new multi-tenant tests** | | **~128** |

### Regression Guarantee

After all phases complete:
- 3,458 existing `#[test]` + `#[tokio::test]` functions still pass (zero regressions)
- ~128 new multi-tenant tests pass
- 2 pre-existing failures in `multi_agent_test.rs` remain (unrelated migration bugs)
- Zero new warnings from `cargo clippy --workspace --all-targets -- -D warnings`

---

## 6. Cross-Reference Index

### ADR to Test Mapping

| ADR | Key Decision | Test Coverage |
|-----|-------------|---------------|
| ADR-MT-001 | `AccountId` as first-class type | Group A (6 tests), migration tests (4 tests) |
| ADR-MT-001 | Backward-compatible `SYSTEM` default | `test_no_header_admin_sees_all_agents`, `test_system_sees_all`, `test_account_or_system_defaults_to_system` |
| ADR-MT-001 | 317 handlers scoped | Pattern coverage gates (Phase 1: 76, Phase 2: ~242, total: ~317) |
| ADR-MT-002 | HMAC-SHA256 signature | Groups B+C (10 tests), timing-safe tests (2 tests) |
| ADR-MT-002 | Infallible extractor | `test_account_id_extraction_is_infallible`, Group A |
| ADR-MT-002 | 404 not 403 on cross-tenant | `test_check_account_mismatching_owner_returns_404`, all `test_pentest_*` |
| ADR-MT-003 | Tiered scoping (4 tiers) | Phase 2 tests organized by tier |
| ADR-MT-003 | Skill allowlist | `test_skill_allowlist_enforced`, `test_empty_allowlist_all_skills` |
| ADR-MT-003 | Event bus filtering | `test_event_bus_filters_by_account` (see also ADR-MT-005) |
| ADR-MT-005 | Event bus tenant isolation | `test_event_bus_filters_by_account`, `test_event_bus_system_events_broadcast` |
| ADR-MT-004 | Migration v19 (14 tables) | `test_migration_v19_all_tables`, `test_migration_v19_idempotent` |
| ADR-MT-004 | 6 store files (106 methods, ~76 account-sensitive) | Per-store cross-account tests (12 tests) |
| ADR-MT-004 | FTS5 rebuild | `test_migration_v19_fts5_rebuilt`, `test_fts5_search_scoped_by_account` |
| ADR-MT-004 | Supabase RLS | `test_supabase_rls_cross_account_invisible` |
| ADR-RV-001 | SupabaseVectorStore | SPEC-RV-002 AC-1 through AC-12 (existing in SPEC) |

### SPEC Claim to Test Mapping

| SPEC | Claims | Test File | Coverage |
|------|--------|-----------|----------|
| SPEC-MT-001 | C-1 through C-33 | `account_tests.rs` | 33/33 (100%) |
| SPEC-MT-002 | 9 claims table entries | `resource_isolation_tests.rs` | 9/9 (100%) |
| SPEC-MT-003 | 10 claims table entries | `data_isolation_tests.rs` | 10/10 (100%) |
| SPEC-MT-004 | Supabase RLS policies | `supabase_vector_tests.rs` (shared with SPEC-RV-002) | RLS subset |
| SPEC-RV-002 | 12 acceptance criteria | `supabase_vector_tests.rs` | 12/12 (100%) |

### PLAN Round to Test Mapping

| PLAN-MT-001 Round | Tests Created | Gate Command |
|-------------------|--------------|--------------|
| Round 1 (Types) | `test_account_id_system_default`, `test_account_id_scoped`, `test_account_id_equality` | `cargo test -p librefang-types` |
| Round 2 (Migration) | `test_migration_v18_adds_account_id`, `test_migration_v18_default_system`, `test_migration_v18_idempotent` | `cargo test -p librefang-memory` |
| Round 3 (Kernel) | `test_spawn_agent_stores_account_id`, `test_list_by_account_filters`, `test_list_by_account_none_returns_all`, `test_get_scoped_cross_tenant_returns_none` | `cargo test -p librefang-kernel` |
| Round 4 (API infra) | Groups A-D (18 tests in 4 RED-GREEN batches) | `cargo test -p librefang-api` |
| Round 5 (Handlers) | Pattern coverage gate (76/76 scoped) | `grep -c` gate script |
| Round 6 (Integration) | 33 tests in `account_tests.rs` | `cargo test -p librefang-api --test account_tests` |

### PLAN-MT-002 Phase to Test Mapping

| PLAN-MT-002 Phase | Tests Created | Gate Command |
|-------------------|--------------|--------------|
| Phase 2 (Resource Isolation) | ~35 tests in `resource_isolation_tests.rs` | `cargo test -p librefang-api --test resource_isolation_tests` |
| Phase 3 (Data Isolation) | ~25 tests in `data_isolation_tests.rs` | `cargo test -p librefang-memory --test data_isolation_tests` |
| Phase 4 (Hardening) | ~20 tests in `security_hardening_tests.rs` | `cargo test -p librefang-api --test security_hardening_tests` |

---

## 7. Anti-Patterns to Avoid

| Anti-Pattern | Mitigation |
|-------------|-----------|
| Testing single-account only | Every integration test uses at least 2 accounts (A and B) |
| 403 instead of 404 on cross-tenant | `assert_cross_account_404()` helper enforces 404 status code |
| Error body leaks account_id | `assert_cross_account_404()` checks body does not contain real owner |
| Testing happy path only | Penetration suite tests 10 attack vectors |
| Timing-unsafe HMAC comparison | Explicit test for `verify_slice()` usage (not `==`) |
| Migration not idempotent | `test_migration_v19_idempotent` runs migration twice |
| FTS5 rebuild leaves empty table | `test_fts5_rebuild_row_count_matches` verifies row count |
| SQL without account_id filter | Static analysis test greps every SQL string literal |
| String interpolation in SQL | Static analysis test checks for parameterized `?` only |
| Performance regression unmeasured | Release-mode benchmark tests with documented thresholds |
| Tests depend on execution order | Each test creates fresh in-memory SQLite database |
| E2E tests not reproducible | Deterministic fixtures: `acc_test_a`, `acc_test_b` with known data |
