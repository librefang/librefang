# PLAN-MT-001: Phase 1 — Account Data Model & Foundation (v2 — post-BHR)

**SPEC:** SPEC-MT-001 (Account Data Model & Storage)
**ADR:** ADR-MT-001 (Account Model)
**Date:** 2026-04-06 (v2: post-BHR review)

---

## BHR Fixes Applied (v2)

| # | Issue | Severity | Fix |
|---|-------|----------|-----|
| 1 | Wrong State type (`Arc<Kernel>` vs `Arc<AppState>`) | CRITICAL | Corrected all handler templates to `State(state): State<Arc<AppState>>` |
| 2 | spawn_agent has 6 variants, plan said 1 | CRITICAL | Listed all 6 variants; clarified only `spawn_agent_inner()` needs core change |
| 3 | `get_agent()` doesn't exist in kernel | CRITICAL | Removed phantom reference; scoped to registry.get() + filter |
| 4 | AgentEntry in 5 crates, plan covered 3 | SCOPE GAP | Added Round 3.5 for librefang-cli fixup |
| 5 | Rounds 5b/5c missing clippy+test gates | QUALITY | Added full gates to all sub-rounds |
| 6 | Round 4 TDD was test-last | QUALITY | Restructured into 4 RED-GREEN micro-cycles |
| 7 | Test failure attribution misleading | MINOR | Clarified runtime vs source distinction |

---

## Verified Baseline Facts

```bash
# Handler counts in Phase 1 scope (2026-04-06):
$ grep -c "pub async fn" crates/librefang-api/src/routes/{agents,channels,config}.rs
crates/librefang-api/src/routes/agents.rs:50
crates/librefang-api/src/routes/channels.rs:11
crates/librefang-api/src/routes/config.rs:15
# Total: 76 handlers, 0 scoped

# Actual handler signature pattern (VERIFIED):
# State(state): State<Arc<AppState>>, NOT State<Arc<Kernel>>
$ head -5 <(grep -A2 "pub async fn" crates/librefang-api/src/routes/agents.rs)
# pub async fn spawn_agent(
#     State(state): State<Arc<AppState>>,
#     lang: Option<axum::Extension<RequestLanguage>>,

# New files to create (all confirmed non-existent):
$ ls crates/librefang-types/src/account.rs       # No such file
$ ls crates/librefang-api/src/extractors.rs      # No such file
$ ls crates/librefang-api/src/macros.rs          # No such file
$ ls crates/librefang-api/src/routes/shared.rs   # No such file
$ ls crates/librefang-api/tests/account_tests.rs # No such file

# AgentEntry constructors across ALL crates (14 total, 5 crates):
$ grep -rn "AgentEntry {" crates/ --include="*.rs"
# librefang-types/src/agent.rs:        2 locations (tests)
# librefang-memory/src/structured.rs:  3 locations (load_agent, list_agents, test)
# librefang-kernel/src/kernel.rs:      4 locations (spawn_agent_inner + 3 tests)
# librefang-kernel/src/heartbeat.rs:   2 locations (tests)
# librefang-kernel/src/registry.rs:    1 location  (test_entry helper)
# librefang-cli/src/tui/event.rs:      2 locations (JSON→AgentEntry mapping)

# Kernel spawn_agent variants (6 total):
# 1. pub fn spawn_agent(&self, manifest) → KernelResult<AgentId>
# 2. pub fn spawn_agent_with_source(&self, manifest, source_toml_path) → KernelResult<AgentId>
# 3. pub fn spawn_agent_with_parent(&self, manifest, parent) → KernelResult<AgentId>
# 4. fn spawn_agent_with_parent_and_source(&self, manifest, parent, source_toml_path) [private]
# 5. fn spawn_agent_inner(&self, manifest, parent, source_toml_path, predetermined_id) [private]
# 6. async fn spawn_agent(&self, manifest_toml, parent_id) [KernelHandle trait]

# Registry interface (DashMap-based):
# pub fn get(&self, id: AgentId) -> Option<AgentEntry>  ← NO account filter
# pub fn list(&self) -> Vec<AgentEntry>                  ← NO account filter
# NOTE: No kernel-level get_agent() exists. Only registry.get().

# Current schema version: 17 (from migration.rs)
# Pre-existing test failures: 2 in multi_agent_test.rs
#   - test_deactivate_kills_agent: "duplicate column name: title" (runtime migration error)
#   - test_default_provider_resolved_to_kernel_default: "duplicate column name: agent_id"
#   These are migration idempotency bugs — ALTER TABLE ADD COLUMN without existence check.
#   Round 2 MUST guard against this same class of error.
```

---

## Implementation Rounds

### Round 1: Types Crate — AccountId & AgentEntry (librefang-types)

No dependencies. Pure data structures. Must compile cleanly before any other round.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Create AccountId type | `crates/librefang-types/src/account.rs` | `AccountId(Option<String>)`, `Account`, `AccountStatus` — exact code from SPEC §Exact Type Definitions | AC-1, AC-2 |
| Register module | `crates/librefang-types/src/lib.rs` | Add `pub mod account;` | — |
| Add account_id to AgentEntry | `crates/librefang-types/src/agent.rs` | Add `pub account_id: Option<String>` field to `AgentEntry` | AC-3 |
| Fix struct literals in types | `crates/librefang-types/src/agent.rs` (2 locations: lines ~1201, ~1260) | Add `account_id: None` to test AgentEntry literals | — |

**TDD micro-cycle:**
1. **RED:** `test_account_id_system_default` — `AccountId::default()` returns `AccountId(None)`, `as_str_or_system()` returns `"system"`
2. **GREEN:** Implement `AccountId` struct, `Default`, `as_str_or_system()`
3. **RED:** `test_account_id_scoped` — `AccountId(Some("tenant-1".into()))` is scoped, returns `"tenant-1"`
4. **GREEN:** Implement `is_scoped()` + complete `as_str_or_system()` match
5. **RED:** `test_account_id_equality` — `AccountId(Some("a".into())) != AccountId(Some("b".into()))`
6. **GREEN:** Already works via `#[derive(PartialEq, Eq)]`
7. **REFACTOR:** Clippy clean, add `Account` + `AccountStatus` structs

**Round 1 gate:**
```bash
cargo clippy -p librefang-types --all-targets -- -D warnings && \
cargo test -p librefang-types && \
echo "Round 1 PASS"
```

**⚠️ Cascade warning:** After this round, `cargo build --workspace` will FAIL.
14 AgentEntry constructors across 4 other crates now need `account_id` field.
Expected failures in: librefang-memory, librefang-kernel, librefang-cli.
Rounds 2, 3, and 3.5 fix these in dependency order.

```bash
# Verify the full cascade list before proceeding:
grep -rn "AgentEntry {" crates/ --include="*.rs" | grep -v librefang-types | grep -v "^Binary"
# Expected: 12 locations in memory (3), kernel (7), cli (2)
```

---

### Round 2: Memory Crate — Migration v18 + AgentEntry Fix (librefang-memory)

Depends on: Round 1 (types compile).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Add migration v18 | `crates/librefang-memory/src/migration.rs` | Column-existence-guarded `ALTER TABLE` (see SQL below) | AC-4 |
| Bump schema version | `crates/librefang-memory/src/migration.rs` | 17 → 18 | — |
| Fix AgentEntry in load_agent | `crates/librefang-memory/src/structured.rs` (line ~285) | Read `account_id` from row: `account_id: row.get("account_id").ok()` | AC-4 |
| Fix AgentEntry in list_agents | `crates/librefang-memory/src/structured.rs` (line ~523) | Same pattern | — |
| Fix AgentEntry in test | `crates/librefang-memory/src/structured.rs` (line ~638) | Add `account_id: None` | — |

**Migration SQL — with existence guard (learned from pre-existing duplicate-column bugs):**
```sql
-- v18: Multi-tenant account isolation
-- Guard: only add column if it doesn't exist (prevents the duplicate-column
-- class of bug that already broke test_deactivate_kills_agent)
SELECT CASE
  WHEN COUNT(*) = 0 THEN 'ALTER TABLE agents ADD COLUMN account_id TEXT NOT NULL DEFAULT ''system'''
  ELSE 'SELECT 1'
END
FROM pragma_table_info('agents') WHERE name = 'account_id';

-- Fallback if the above CTE approach doesn't work in rusqlite:
-- Use a try/catch pattern: attempt ALTER, ignore "duplicate column" error.

-- Index (always safe — CREATE INDEX IF NOT EXISTS):
CREATE INDEX IF NOT EXISTS idx_agents_account_id ON agents(account_id);
```

**TDD micro-cycle:**
1. **RED:** `test_migration_v18_adds_account_id` — fresh DB → migrate → `PRAGMA table_info(agents)` includes `account_id`
2. **GREEN:** Implement migration with column-existence guard
3. **RED:** `test_migration_v18_default_system` — existing agents get `account_id = 'system'`
4. **GREEN:** DEFAULT clause handles this
5. **RED:** `test_migration_v18_idempotent` — running migrate twice doesn't fail
6. **GREEN:** Column-existence guard ensures idempotency

**Rollback SQL (document, don't auto-run):**
```sql
-- SQLite ≥ 3.35.0:
ALTER TABLE agents DROP COLUMN account_id;
-- SQLite < 3.35.0:
-- CREATE TABLE agents_backup AS SELECT [all cols except account_id] FROM agents;
-- DROP TABLE agents; ALTER TABLE agents_backup RENAME TO agents;
-- Recreate indexes.
```

**Round 2 gate:**
```bash
cargo clippy -p librefang-memory --all-targets -- -D warnings && \
cargo test -p librefang-memory && \
echo "Round 2 PASS"
```

---

### Round 3: Kernel Crate — AccountId Threading (librefang-kernel)

Depends on: Round 1 (types) + Round 2 (memory compiles).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Add AccountId to `spawn_agent_inner` | `kernel.rs` line ~2802 | Core funnel — all 5 public/private variants call this. Add `account_id: Option<String>` param, store in AgentEntry | AC-5 |
| Thread through public spawn variants | `kernel.rs` lines ~2773–2795 | 3 public + 1 private wrapper: add `account_id` param, pass through to `spawn_agent_inner` | AC-5 |
| Thread through KernelHandle trait | `kernel.rs` line ~9832 | `async fn spawn_agent()` trait impl: extract account from context or default None | AC-5 |
| Add account filter to registry.list() | `registry.rs` | New method: `pub fn list_by_account(&self, account: &AccountId) -> Vec<AgentEntry>` | AC-6 |
| Add account filter to registry.get() | `registry.rs` | New method: `pub fn get_scoped(&self, id: AgentId, account: &AccountId) -> Option<AgentEntry>` | AC-7 |
| Update kernel list_agents | `kernel.rs` line ~9887 | Accept `AccountId`, delegate to `registry.list_by_account()` | AC-6 |
| Fix AgentEntry in spawn_agent_inner | `kernel.rs` line ~2888 | Set `account_id` from param | — |
| Fix AgentEntry in kernel tests | `kernel.rs` lines ~11928, ~11967, ~11992 | Add `account_id: None` to 3 test literals | — |
| Fix AgentEntry in heartbeat tests | `heartbeat.rs` lines ~249, ~274 | Add `account_id: None` to 2 test literals | — |
| Fix AgentEntry in registry test | `registry.rs` line ~433 | Add `account_id: None` to `test_entry()` helper | — |

**Why NOT modify `get()` and `list()` directly:**
Existing callers (background agents, heartbeat, internal kernel ops) need unfiltered access.
Adding new `get_scoped()` / `list_by_account()` preserves backward compat.
API layer calls the scoped versions; internal kernel calls the unscoped originals.

**TDD micro-cycle:**
1. **RED:** `test_spawn_agent_stores_account_id` — spawn with `AccountId(Some("t1"))` → entry has `account_id = Some("t1")`
2. **GREEN:** Modify `spawn_agent_inner` to accept and store `account_id`
3. **RED:** `test_list_by_account_filters` — spawn 2 agents (t1, t2) → `list_by_account(t1)` returns only t1's
4. **GREEN:** Implement `registry.list_by_account()`
5. **RED:** `test_list_by_account_none_returns_all` — `AccountId(None)` returns all (legacy mode)
6. **GREEN:** Add None-means-all logic
7. **RED:** `test_get_scoped_cross_tenant_returns_none` — `get_scoped(t1_agent, t2_account)` → None
8. **GREEN:** Implement `registry.get_scoped()`

**Round 3 gate:**
```bash
cargo clippy -p librefang-kernel --all-targets -- -D warnings && \
cargo test -p librefang-kernel && \
echo "Round 3 PASS"
```

---

### Round 3.5: CLI Crate — AgentEntry Constructor Fix (librefang-cli)

Depends on: Round 1 (types). Can run in parallel with Rounds 2-3 if convenient.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Fix AgentEntry in tui/event.rs | `crates/librefang-cli/src/tui/event.rs` line ~1324 | Add `account_id: None` to JSON→AgentEntry mapping (memory agents) | — |
| Fix AgentEntry in tui/event.rs | `crates/librefang-cli/src/tui/event.rs` line ~1340 | Add `account_id: None` to registry list mapping | — |

**Round 3.5 gate:**
```bash
cargo clippy -p librefang-cli --all-targets -- -D warnings && \
cargo build -p librefang-cli && \
echo "Round 3.5 PASS"
```

**Workspace compile check (MANDATORY after Round 3.5):**
```bash
# ALL AgentEntry cascade sites should now compile:
cargo build --workspace 2>&1 | grep -c "error" | xargs -I{} test {} -eq 0
echo "Workspace compiles clean"
```

---

### Round 4: API Foundation — Extractors, Macros, Guard, HMAC (librefang-api)

Depends on: Rounds 1-3.5 (full workspace compiles). Creates infrastructure BEFORE touching handlers.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Create AccountId extractor | `crates/librefang-api/src/extractors.rs` | `impl FromRequestParts for AccountId` — infallible, `X-Account-Id` header | AC-8 |
| Create validate_account! macro | `crates/librefang-api/src/macros.rs` | Returns 400 if `AccountId(None)` | AC-9 |
| Create account_or_system! macro | `crates/librefang-api/src/macros.rs` | Defaults to `"system"` if None | AC-10 |
| Create check_account guard | `crates/librefang-api/src/routes/shared.rs` | Returns 404 (not 403) on cross-tenant | AC-11 |
| Add HMAC verification | `crates/librefang-api/src/middleware.rs` | `HMAC-SHA256(secret, account_id)` constant-time | AC-12 |
| Wire into router | `crates/librefang-api/src/server.rs` | Add HMAC middleware layer to middleware stack | — |
| Add Cargo deps | `crates/librefang-api/Cargo.toml` | `hmac = "0.12"`, `sha2 = "0.10"`, `hex = "0.4"` | — |
| Register modules | `crates/librefang-api/src/lib.rs` | Add `pub mod extractors;` and `pub mod macros;` | — |

**TDD micro-cycles (4 batches, not 18-then-implement):**

**Batch A: Extractor (RED→GREEN×6)**
1. RED: `test_extract_account_id_from_header` → GREEN: implement `FromRequestParts`
2. RED: `test_extract_account_id_missing_header` → GREEN: return `AccountId(None)`
3. RED: `test_extract_account_id_empty_header` → GREEN: treat empty as None
4. RED: `test_extract_account_id_whitespace_trimmed` → GREEN: `.trim()`
5. RED: `test_extract_account_id_case_sensitive` → GREEN: no `.to_lowercase()`
6. RED: `test_extract_is_infallible` → GREEN: `Rejection = Infallible`

**Batch B: Macros (RED→GREEN×4)**
7. RED: `test_validate_account_rejects_none` → GREEN: implement `validate_account!`
8. RED: `test_validate_account_accepts_some` → GREEN: pass-through on Some
9. RED: `test_account_or_system_returns_system` → GREEN: implement `account_or_system!`
10. RED: `test_account_or_system_returns_value` → GREEN: return inner value

**Batch C: Guard (RED→GREEN×4)**
11. RED: `test_check_account_owner_passes` → GREEN: implement `check_account()`
12. RED: `test_check_account_non_owner_gets_404` → GREEN: return 404 NOT_FOUND
13. RED: `test_check_account_none_passes_all` → GREEN: None = legacy bypass
14. RED: `test_error_body_is_generic` → GREEN: generic "Agent not found" message

**Batch D: HMAC (RED→GREEN×4)**
15. RED: `test_hmac_valid_signature_passes` → GREEN: implement HMAC verify
16. RED: `test_hmac_invalid_signature_rejected` → GREEN: return 401
17. RED: `test_hmac_missing_signature_allowed` → GREEN: no secret = dev mode passthrough
18. RED: `test_hmac_constant_time` → GREEN: use `verify_slice()` not `==`

**REFACTOR after each batch.** Not after all 18.

**Round 4 gate:**
```bash
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4 PASS"
```

**Critical: Do NOT touch any handler in this round.** Only infrastructure.

---

### Round 5: API Handlers — Mechanical Sweep (librefang-api, 76 handlers)

Depends on: Round 4 (extractors, macros, guard all compile and pass).

Every handler gets the same pattern:

```rust
// BEFORE (actual signature from codebase):
pub async fn handler_name(
    State(state): State<Arc<AppState>>,
    lang: Option<axum::Extension<RequestLanguage>>,
    // ... existing params
) -> impl IntoResponse {
    // ... body
}

// AFTER:
pub async fn handler_name(
    State(state): State<Arc<AppState>>,
    account: AccountId,  // NEW — after State, before other extractors
    lang: Option<axum::Extension<RequestLanguage>>,
    // ... existing params
) -> impl IntoResponse {
    // ... check_account() or validate_account! or account_or_system! as appropriate
    // ... body with account-scoped kernel calls
}
```

#### Round 5a: agents.rs (50 handlers)

| # | Handler Pattern | Macro/Guard | AC |
|---|----------------|-------------|-----|
| 1-50 | All 50 `pub async fn` in agents.rs | `check_account()` on reads after fetch, `validate_account!` on writes | AC-13–AC-17 |

**Approach:**
- GET/LIST: call `state.kernel.registry.list_by_account(&account)` or `get_scoped(id, &account)`
- POST/PUT: `validate_account!(account)` then `state.kernel.spawn_agent(manifest)` with account threaded
- DELETE: fetch via `get_scoped()`, then delete — returns 404 if cross-tenant

**Round 5a gate:**
```bash
TOTAL=$(grep -c "pub async fn" crates/librefang-api/src/routes/agents.rs)
SCOPED=$(grep -c "account: AccountId" crates/librefang-api/src/routes/agents.rs)
echo "agents.rs: $SCOPED/$TOTAL scoped"
[ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $((TOTAL - SCOPED)) unscoped"; exit 1; }
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 5a PASS"
```

#### Round 5b: channels.rs (11 handlers)

| # | Handler Pattern | Macro/Guard | AC |
|---|----------------|-------------|-----|
| 1-11 | All 11 `pub async fn` | `account: AccountId` extractor only; full channel scoping deferred to Phase 4 | AC-18 |

**Note:** Phase 1 adds the extractor param so the type signature is future-proof. The handler body does NOT enforce account scoping on channels yet — that requires channel-to-account routing (Phase 4).

**Round 5b gate:**
```bash
TOTAL=$(grep -c "pub async fn" crates/librefang-api/src/routes/channels.rs)
SCOPED=$(grep -c "account: AccountId" crates/librefang-api/src/routes/channels.rs)
echo "channels.rs: $SCOPED/$TOTAL scoped"
[ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $((TOTAL - SCOPED)) unscoped"; exit 1; }
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 5b PASS"
```

#### Round 5c: config.rs (15 handlers)

| # | Handler Pattern | Macro/Guard | AC |
|---|----------------|-------------|-----|
| 1-15 | All 15 `pub async fn` | `account_or_system!` for reads, `validate_account!` for writes | AC-19 |

**Round 5c gate:**
```bash
TOTAL=$(grep -c "pub async fn" crates/librefang-api/src/routes/config.rs)
SCOPED=$(grep -c "account: AccountId" crates/librefang-api/src/routes/config.rs)
echo "config.rs: $SCOPED/$TOTAL scoped"
[ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $((TOTAL - SCOPED)) unscoped"; exit 1; }
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 5c PASS"
```

---

### Round 6: Test Suite & Integration Wiring

Depends on: Rounds 1-5 (everything compiles and all round gates pass).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Create test file | `crates/librefang-api/tests/account_tests.rs` | All 33 account tests from SPEC Claims table | AC-20–AC-33 |
| Integration tests | same file | TestServer harness with `post_agent`, `get_agents`, `delete_agent` | AC-25–AC-33 |

**TDD micro-cycles:**

**Batch A: Agent CRUD scoping (RED→GREEN×4)**
1. RED: `test_create_agent_stores_account_id` → GREEN: POST /agents with X-Account-Id stores it
2. RED: `test_list_agents_returns_only_owned` → GREEN: tenant-1 sees only own agents
3. RED: `test_get_agent_cross_tenant_returns_404` → GREEN: tenant-2 can't see tenant-1's agent
4. RED: `test_delete_agent_cross_tenant_returns_404` → GREEN: tenant-2 can't delete tenant-1's agent

**Batch B: Legacy + system mode (RED→GREEN×2)**
5. RED: `test_legacy_mode_sees_all` → GREEN: no X-Account-Id → sees everything
6. RED: `test_system_agents_visible_to_all` → GREEN: account_id="system" visible to all

**Batch C: Security (RED→GREEN×2)**
7. RED: `test_error_body_never_leaks_account_id` → GREEN: 404 body = generic message
8. RED: `test_migration_preserves_existing_agents` → GREEN: pre-existing → account_id="system"

**Round 6 gate:**
```bash
cargo test -p librefang-api --test account_tests && \
cargo test --workspace && \
echo "Round 6 PASS"
```

---

## Pattern Coverage Gate (MANDATORY)

```bash
#!/bin/bash
set -e
echo "=== Pattern Coverage Gate ==="

FAIL=0
for f in agents.rs channels.rs config.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  UNSCOPED=$((TOTAL - SCOPED))
  if [ "$UNSCOPED" -gt 0 ]; then
    echo "FAIL: $f: $SCOPED/$TOTAL scoped ($UNSCOPED remaining)"
    FAIL=1
  else
    echo "PASS: $f: $SCOPED/$TOTAL scoped"
  fi
done

# Also verify AgentEntry cascade is fully resolved:
BROKEN=$(cargo build --workspace 2>&1 | grep -c "error\[" || echo 0)
if [ "$BROKEN" -gt 0 ]; then
  echo "FAIL: $BROKEN compile errors remain"
  FAIL=1
else
  echo "PASS: workspace compiles clean"
fi

if [ "$FAIL" -eq 1 ]; then
  echo "=== GATE FAILED ==="
  exit 1
fi
echo "=== Pattern Coverage: ALL CLEAR ==="
```

**Expected output:**
```
PASS: agents.rs: 50/50 scoped
PASS: channels.rs: 11/11 scoped
PASS: config.rs: 15/15 scoped
PASS: workspace compiles clean
=== Pattern Coverage: ALL CLEAR ===
```

---

## Pre-BHR Checklist

- [ ] Pattern coverage gate passes (0 unscoped handlers)
- [ ] Workspace compiles clean (`cargo build --workspace`)
- [ ] All 33 SPEC tests pass (`cargo test -p librefang-api --test account_tests`)
- [ ] All SPEC claims have cited test names
- [ ] Clippy clean: `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] No new warnings introduced
- [ ] Full workspace tests pass: `cargo test --workspace` (expect same 2 pre-existing failures only)
- [ ] Migration v18 is idempotent (test confirms — no duplicate-column errors)
- [ ] AgentEntry backward compat: all 14 constructor sites compile with `account_id` field
- [ ] librefang-cli tui/event.rs compiles (Round 3.5 verified)

---

## Day-by-Day Schedule — Sequential (Human-Driven)

| Day | Round | Deliverable | Gate |
|-----|-------|-------------|------|
| 1 AM | Round 1 | AccountId type, AgentEntry field, 2 type-crate literals | `cargo test -p librefang-types` |
| 1 PM | Round 2 | Migration v18 (with existence guard), 3 memory literals | `cargo test -p librefang-memory` |
| 2 AM | Round 3 | 6 spawn variants + registry scoped methods + 7 kernel literals | `cargo test -p librefang-kernel` |
| 2 mid | Round 3.5 | 2 CLI literals | `cargo build -p librefang-cli` + workspace compile |
| 2 PM | Round 4 | Extractors (Batch A), macros (B), guard (C), HMAC (D) | `cargo test -p librefang-api` |
| 3 | Round 5a | agents.rs 50 handlers | Pattern gate: 50/50 + clippy + test |
| 4 AM | Round 5b | channels.rs 11 handlers | Pattern gate: 11/11 + clippy + test |
| 4 mid | Round 5c | config.rs 15 handlers | Pattern gate: 15/15 + clippy + test |
| 4 PM | Round 6 | 33 tests (3 batches), workspace green | Full gate pass |
| 5 | BHR | Pre-BHR checklist → submit | Confirmation only |

**Timeline:** 5-7 days (sequential human execution)

---

## Swarm Execution Model — Parallel (Agent-Driven) ⚡

**For claude-flow multi-agent swarm execution (3x speedup):**

### Parallelization Strategy

Rounds 2-4 and Rounds 5a/5b/5c are **independent** and can execute concurrently with proper workspace coordination:

```
PHASE 1: Round 1 (agent-types)
  └─ Agent-1: librefang-types (AccountId, AgentEntry.account_id)
     └─ HANDOFF GATE: cargo test -p librefang-types ✅

     ↓ (unblocks Phase 2)

PHASE 2: Rounds 2+3+3.5 PARALLEL (consume types, no inter-crate dependencies)
  ├─ Agent-2: librefang-memory (Round 2: v18 migration + 3 literals)
  ├─ Agent-3: librefang-kernel (Round 3: 6 spawn variants + registry)
  └─ Agent-4: librefang-cli (Round 3.5: 2 literals in tui/event.rs)
     
     Each agent runs independently: cargo test -p [crate]
     ↓ COORDINATION GATE (Coordinator waits for all 3):
     └─ Coordinator verifies: cargo build --workspace compiles clean
        (catches any cross-crate breakage)

     ↓ (unblocks Phase 3)

PHASE 3: Round 4 (agent-api-foundation)
  └─ Agent-5: librefang-api (extractors, macros, guard, HMAC)
     └─ HANDOFF GATE: cargo test -p librefang-api ✅

     ↓ (unblocks Phase 4)

PHASE 4: Rounds 5a+5b+5c PARALLEL (independent route files, no dependencies)
  ├─ Agent-6: agents.rs (50 handlers: add account:AccountId + check_account())
  ├─ Agent-7: channels.rs (11 handlers: add account:AccountId extractor)
  └─ Agent-8: config.rs (15 handlers: add account:AccountId + macros)
     
     Each agent works independently on separate file
     ↓ COORDINATION GATE (Coordinator collects pattern counts):
     └─ Coordinator verifies:
        agents.rs: grep -c "account: AccountId" = 50
        channels.rs: grep -c "account: AccountId" = 11
        config.rs: grep -c "account: AccountId" = 15
     └─ All three: cargo clippy -p librefang-api --all-targets
     └─ All three: cargo test -p librefang-api

     ↓ (unblocks Phase 5)

PHASE 5: Round 6 (agent-tests)
  └─ Agent-9: Write 33 tests + integration suite
     └─ HANDOFF GATE: cargo test --workspace ✅

PHASE 6: Pre-BHR (agent-validation)
  └─ Agent-10: Run full checklist
     └─ FINAL GATE: All exit criteria ✅
```

### Swarm Configuration (Ruflo)

```yaml
# mcp__claude-flow__swarm_init + agent_spawn
topology: hierarchical              # Coordinator prevents drift
maxAgents: 10                       # Peak: 3 agents in Phase 2 + 3 in Phase 4
strategy: specialized              # Each agent specializes in one crate/file

coordinator:
  name: phase-coordinator
  role: verify gates, unblock phases, enforce dependency order
  
agents:
  - agent-types:
      crate: librefang-types
      rounds: [1]
      gate: "cargo test -p librefang-types"
      
  - agent-memory:
      crate: librefang-memory
      rounds: [2]
      gate: "cargo test -p librefang-memory"
      parallel: true
      unblock_on: [Phase 2 coordination gate]
      
  - agent-kernel:
      crate: librefang-kernel
      rounds: [3]
      gate: "cargo test -p librefang-kernel"
      parallel: true
      unblock_on: [Phase 2 coordination gate]
      
  - agent-cli:
      crate: librefang-cli
      rounds: [3.5]
      gate: "cargo build -p librefang-cli"
      parallel: true
      unblock_on: [Phase 2 coordination gate]
      
  - agent-api-foundation:
      crate: librefang-api
      rounds: [4]
      gate: "cargo test -p librefang-api"
      unblock_on: [Phase 2 coordination gate]
      
  - agent-handlers-agents:
      file: crates/librefang-api/src/routes/agents.rs
      rounds: [5a]
      gate: "grep -c 'account: AccountId' = 50"
      parallel: true
      unblock_on: [Phase 4 coordination gate]
      
  - agent-handlers-channels:
      file: crates/librefang-api/src/routes/channels.rs
      rounds: [5b]
      gate: "grep -c 'account: AccountId' = 11"
      parallel: true
      unblock_on: [Phase 4 coordination gate]
      
  - agent-handlers-config:
      file: crates/librefang-api/src/routes/config.rs
      rounds: [5c]
      gate: "grep -c 'account: AccountId' = 15"
      parallel: true
      unblock_on: [Phase 4 coordination gate]
      
  - agent-tests:
      crate: librefang-api
      rounds: [6]
      gate: "cargo test --workspace"
      unblock_on: [Phase 4 coordination gate]
      
  - agent-validation:
      role: pre-bhr-checklist
      rounds: [final]
      gate: all exit criteria
      unblock_on: [Phase 5 gate]
```

### Coordination Handoff Gates

Each phase waits for all agents in that phase to report:

```bash
# Phase 1 → Phase 2 handoff
Phase 1 Coordinator Gate: agent-types report "✅ cargo test -p librefang-types"

# Phase 2 → Phase 3 handoff (coordination point)
Phase 2 Coordination Gate:
  ✅ agent-memory report "✅ cargo test -p librefang-memory"
  ✅ agent-kernel report "✅ cargo test -p librefang-kernel"
  ✅ agent-cli report "✅ cargo build -p librefang-cli"
  Coordinator verifies: "✅ cargo build --workspace" (cross-crate check)
  → Unblock Phase 3

# Phase 3 → Phase 4 handoff
Phase 3 Coordinator Gate: agent-api-foundation report "✅ cargo test -p librefang-api"

# Phase 4 → Phase 5 handoff (coordination point)
Phase 4 Coordination Gate:
  ✅ agent-handlers-agents report "✅ 50/50 handlers scoped"
  ✅ agent-handlers-channels report "✅ 11/11 handlers scoped"
  ✅ agent-handlers-config report "✅ 15/15 handlers scoped"
  Coordinator verifies: "✅ cargo clippy -p librefang-api"
  Coordinator verifies: "✅ cargo test -p librefang-api"
  → Unblock Phase 5

# Phase 5 → Phase 6 handoff
Phase 5 Coordinator Gate: agent-tests report "✅ cargo test --workspace"
  
# Final gate
Phase 6 Coordinator Gate: agent-validation report "✅ All pre-BHR criteria"
```

### Timeline — Swarm Execution

| Phase | Rounds | Agents | Duration | Critical Path |
|-------|--------|--------|----------|-------|
| 1 | 1 | 1 | 1-2h | Sequential |
| 2 | 2,3,3.5 | 3 parallel | 2-3h | Max(2h, 3h, 30m) = 3h |
| Coordination | — | Coordinator | 15m | Verify workspace compile |
| 3 | 4 | 1 | 3-4h | Sequential |
| 4 | 5a,5b,5c | 3 parallel | 2-3h | Max(2-3h, 2-3h, 2-3h) = 2-3h |
| Coordination | — | Coordinator | 15m | Pattern gates + clippy |
| 5 | 6 | 1 | 2-3h | Sequential |
| 6 | final | 1 | 1-2h | Sequential |
| **TOTAL** | — | 10 max | **~14-19h** | **2 wall-clock days** |

**Speedup:** 5-7 days → 1.5-2 days = **3.5x faster** ⚡

### Swarm Readiness Checklist

Before spawning swarm, ensure:
- [ ] All 6 ADRs (MT-001 through RV-001) verified against BHR-AUDIT ✅
- [ ] PLAN-MT-001 v2 (this file) with phase gates documented ✅
- [ ] All handler counts verified (50+11+15=76) ✅
- [ ] Migration v18 idempotency tested in isolation ✅
- [ ] HMAC verification gates written + testable ✅
- [ ] Pattern coverage script ready: `grep -c "account: AccountId"` ✅
- [ ] Agents have clear, separate files to work on (no merge conflicts) ✅
- [ ] Coordinator agent has handoff gate script ✅

---

## Rollback Plan

```bash
# 1. Revert all Phase 1 commits
git revert --no-commit HEAD~N  # N = Phase 1 commit count

# 2. Rollback migration (SQLite ≥ 3.35.0)
sqlite3 librefang.db "ALTER TABLE agents DROP COLUMN account_id;"
sqlite3 librefang.db "DROP INDEX IF EXISTS idx_agents_account_id;"

# 3. Rollback migration (SQLite < 3.35.0 — table rebuild)
sqlite3 librefang.db << 'SQL'
CREATE TABLE agents_backup AS
  SELECT id, name, manifest, state, mode /* enumerate ALL cols except account_id */
  FROM agents;
DROP TABLE agents;
ALTER TABLE agents_backup RENAME TO agents;
DROP INDEX IF EXISTS idx_agents_account_id;
-- Recreate original indexes from migration.rs
SQL

# 4. Verify
cargo test --workspace
```

---

## Exit Criteria

```bash
# ALL must exit 0:
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p librefang-api --test account_tests

# Pattern coverage:
for f in agents.rs channels.rs config.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  [ "$TOTAL" -eq "$SCOPED" ] || exit 1
done

# AgentEntry cascade fully resolved:
cargo build --workspace 2>&1 | grep -c "error\[" | xargs -I{} test {} -eq 0

echo "ALL EXIT CRITERIA PASS"
```

---

## Dependency Graph

```
Round 1: librefang-types ─────────────────────────────────────┐
         (AccountId, AgentEntry.account_id)                   │
                │                                             │
Round 2: librefang-memory ──────────────────────────┐         │
         (migration v18, structured.rs 3 literals)  │         │
                │                                   │         │
Round 3: librefang-kernel ──────────────────┐       │         │
         (6 spawn variants, registry scoped,│       │         │
          7 literals in kernel/heartbeat/   │       │         │
          registry)                         │       │         │
                │                           │       │         │
Round 3.5: librefang-cli ──────────────┐    │       │         │
           (2 literals in tui/event.rs)│    │       │         │
                │                      │    │       │         │
         ┌──── WORKSPACE COMPILES ─────┘────┘───────┘─────────┘
         │
Round 4: librefang-api (foundation) ────┐
         (extractors, macros, guard,    │
          HMAC, server wiring)          │
                │                       │
Round 5: librefang-api (76 handlers) ───┘
         5a: agents.rs (50)
         5b: channels.rs (11)
         5c: config.rs (15)
                │
Round 6: test suite + integration
         (33 tests, workspace green)
```

---

## Anti-Patterns to Avoid

| Anti-Pattern | Mitigation in This Plan |
|-------------|------------------------|
| Instance-based gates | Pattern coverage gate counts ALL handlers mechanically |
| BHR as discovery | Pre-BHR checklist catches everything first |
| Prose exit criteria | Every gate is `bash` commands that exit 0 |
| Missing round gates | Every round (including 5b, 5c) has clippy + test gate |
| Mixed-crate rounds | Strict dependency order: types → memory → kernel → cli → api |
| Wrong round order | Dependency graph, each round names prerequisites |
| Handlers before infrastructure | Round 4 complete before Round 5 |
| Test-last disguised as TDD | Micro-cycles: RED→GREEN per batch, REFACTOR between batches |
| Duplicate-column migration bugs | Column-existence guard in v18 SQL |
| Missing cascade sites | All 14 AgentEntry locations enumerated across 5 crates |
| Phantom API references | Verified: no `get_agent()` in kernel; uses `registry.get_scoped()` |
