# PLAN-MT-001: Phase 1 — Multi-Tenant API Isolation (v3 — API-only track)

**SPEC:** SPEC-MT-001 (Account Data Model & Storage)
**ADR:** ADR-MT-001 (Account Model), ADR-026 (Account Signature Policy)
**Date:** 2026-04-07 (v3: separated from storage, focusing API infrastructure)
**Scope:** API extractors, macros, guards, handler integration. Storage handled by separate team.

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
Round 2 (CLI) fixes the 2 librefang-cli literals needed for workspace to compile.

```bash
# Verify the full cascade list before proceeding:
grep -rn "AgentEntry {" crates/ --include="*.rs" | grep -v librefang-types | grep -v "^Binary"
# Expected: 12 locations in memory (3), kernel (7), cli (2)
```

**Note:** Memory and Kernel AgentEntry fixes (10 locations) are handled by the storage team in a separate multi-tenant initiative.
This Phase 1 focuses on API isolation only.

---

### Round 2: CLI Crate — AgentEntry Constructor Fix (librefang-cli)

Depends on: Round 1 (types).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Fix AgentEntry in tui/event.rs | `crates/librefang-cli/src/tui/event.rs` line ~1324 | Add `account_id: None` to JSON→AgentEntry mapping (memory agents) | — |
| Fix AgentEntry in tui/event.rs | `crates/librefang-cli/src/tui/event.rs` line ~1340 | Add `account_id: None` to registry list mapping | — |

**Round 2 gate:**
```bash
cargo clippy -p librefang-cli --all-targets -- -D warnings && \
cargo build -p librefang-cli && \
echo "Round 2 PASS"
```

---

### Round 3: API Foundation — Extractors, Macros, Guard, HMAC (librefang-api)

Depends on: Rounds 1-2 (types compile, CLI compiles). Creates infrastructure BEFORE touching handlers.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Implement FromRequestParts | `crates/librefang-api/src/middleware.rs` | `impl FromRequestParts for AccountId` in middleware, infallible, `X-Account-Id` header | AC-8 |
| Create validate_account! macro | `crates/librefang-api/src/macros.rs` | Returns 400 if `AccountId(None)` | AC-9 |
| Create account_or_system! macro | `crates/librefang-api/src/macros.rs` | Defaults to `"system"` if None | AC-10 |
| Create check_account guard | `crates/librefang-api/src/routes/shared.rs` | Returns 404 (not 403) on cross-tenant | AC-11 |
| Add HMAC verification | `crates/librefang-api/src/middleware.rs` | `HMAC-SHA256(secret, account_id)` constant-time | AC-12 |
| Wire into router | `crates/librefang-api/src/server.rs` | Add HMAC middleware layer to middleware stack | — |
| Add Cargo deps | `crates/librefang-api/Cargo.toml` | `hmac = "0.12"`, `sha2 = "0.10"`, `hex = "0.4"` | — |
| Register macros module | `crates/librefang-api/src/lib.rs` | Add `#[macro_use] mod macros;` (middleware.rs already exists with AccountId impl) | — |

**TDD micro-cycles (4 batches, not 18-then-implement):**

**Batch A: Extractor (RED→GREEN×6)**
1. RED: `test_extract_account_id_from_header` → GREEN: implement `FromRequestParts` in middleware.rs; extract from X-Account-Id header
2. RED: `test_extract_account_id_missing_header` → GREEN: return `AccountId(None)` when header absent
3. RED: `test_extract_account_id_empty_header_filtered` → GREEN: filter out empty-only headers (use `.filter(|s| !s.trim().is_empty())`)
4. RED: `test_extract_account_id_case_sensitive` → GREEN: preserve case; no `.to_lowercase()`
5. RED: `test_extract_account_id_whitespace_preserved` → GREEN: return header value as-is (openfang-ai doesn't trim return, only filters empty)
6. RED: `test_extract_is_infallible` → GREEN: `Rejection = Infallible` trait impl (never fails, always returns AccountId)

**Batch B: Macros (RED→GREEN×4) — Test via handler usage**
7. RED: `test_validate_account_macro_requires_header` → GREEN: `validate_account!(account)` returns Err((400, JSON)) if account.0 is None
8. RED: `test_validate_account_macro_passes_some` → GREEN: `validate_account!(account)` returns Ok when Some(string)
9. RED: `test_account_or_system_macro_default_system` → GREEN: `account_or_system!(account)` returns "system" if None
10. RED: `test_account_or_system_macro_returns_inner` → GREEN: `account_or_system!(account)` returns inner string if Some

**Batch C: Guard (RED→GREEN×5) — CRITICAL SECURITY TESTS**
11. RED: `test_check_account_owner_passes` → GREEN: implement `check_account()`
12. RED: `test_check_account_non_owner_gets_404` → GREEN: return 404 NOT_FOUND
13. RED: `test_check_account_none_passes_all` → GREEN: None = legacy bypass
14. RED: `test_error_body_is_generic` → GREEN: generic "Agent not found" message (no account_id leak)
14.5. RED: `test_check_account_scoped_request_vs_unowned_agent_returns_404` → GREEN: unowned agents (account_id: None in DB) MUST return 404 to scoped requests (prevents cross-tenant data access)

**Batch D: HMAC (RED→GREEN×6) — POLICY MATRIX COVERAGE**
15. RED: `test_hmac_valid_signature_passes` → GREEN: verify_account_sig() returns true for valid HMAC-SHA256
16. RED: `test_hmac_invalid_signature_rejected` → GREEN: verify returns false; account_sig_policy returns 401
17. RED: `test_hmac_no_secret_config_allows_any` → GREEN: absent secret = dev mode, passes all (no enforcement)
18. RED: `test_hmac_no_account_id_header_allows_all` → GREEN: X-Account-Id absent, passes (legacy)
18.5. RED: `test_hmac_missing_signature_with_secret_rejected` → GREEN: secret configured but sig missing = 401
19. RED: `test_hmac_constant_time` → GREEN: use `verify_slice()` not `==` (timing attack prevention)

**REFACTOR after each batch.** Not after all 18.

**Round 3 gate:**
```bash
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 3 PASS"
```

**Critical: Do NOT touch any handler in this round.** Only infrastructure.

---

### Round 4: API Handlers — Mechanical Sweep (librefang-api, 76 handlers)

Depends on: Round 3 (extractors, macros, guard all compile and pass).

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

#### Round 4a: agents.rs (50 handlers)

| # | Handler Pattern | Macro/Guard | AC |
|---|----------------|-------------|-----|
| 1-50 | All 50 `pub async fn` in agents.rs | `check_account()` on reads after fetch, `validate_account!` on writes | AC-13–AC-17 |

**Approach:**
- GET/LIST: match on `account.0` to extract &str, then call `state.kernel.registry.list_by_account()` for Some or `.list()` for None
- Pattern (list_by_account takes &str):
  ```rust
  let raw_agents = match account.0.as_deref() {
      Some(oid) => state.kernel.registry.list_by_account(oid),  // &str
      None => state.kernel.registry.list(),
  };
  ```
- POST/PUT: `validate_account!(account)` then `state.kernel.spawn_agent(manifest)` with account. CRITICAL: immediately call `state.kernel.registry.set_account(agent_id, oid.clone())` to assign ownership.
- DELETE: Use `check_account(entry, &account)` after fetch to verify ownership — returns 404 if cross-tenant or unowned

**Registry methods (librefang-kernel):**
- `list()` — all agents (legacy/admin only)
- `list_by_account(&str)` — agents scoped to owner
- `get(id)` — single agent by id (requires check_account guard)
- `set_account(id, owner_string)` — assign agent to owner (call immediately after spawn)

**Round 4a gate:**
```bash
TOTAL=$(grep -c "pub async fn" crates/librefang-api/src/routes/agents.rs)
SCOPED=$(grep -c "account: AccountId" crates/librefang-api/src/routes/agents.rs)
echo "agents.rs: $SCOPED/$TOTAL scoped"
[ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $((TOTAL - SCOPED)) unscoped"; exit 1; }
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4a PASS"
```

#### Round 4b: channels.rs (11 handlers)

| # | Handler Pattern | Macro/Guard | AC |
|---|----------------|-------------|-----|
| 1-11 | All 11 `pub async fn` | `account: AccountId` extractor only; full channel scoping deferred to Phase 2 | AC-18 |

**Note:** Phase 1 adds the extractor param so the type signature is future-proof. The handler body does NOT enforce account scoping on channels yet — that requires channel-to-account routing (Phase 2).

**Round 4b gate:**
```bash
TOTAL=$(grep -c "pub async fn" crates/librefang-api/src/routes/channels.rs)
SCOPED=$(grep -c "account: AccountId" crates/librefang-api/src/routes/channels.rs)
echo "channels.rs: $SCOPED/$TOTAL scoped"
[ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $((TOTAL - SCOPED)) unscoped"; exit 1; }
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4b PASS"
```

#### Round 4c: config.rs (15 handlers)

| # | Handler Pattern | Macro/Guard | AC |
|---|----------------|-------------|-----|
| 1-15 | All 15 `pub async fn` | `account_or_system!` for reads, `validate_account!` for writes | AC-19 |

**Round 4c gate:**
```bash
TOTAL=$(grep -c "pub async fn" crates/librefang-api/src/routes/config.rs)
SCOPED=$(grep -c "account: AccountId" crates/librefang-api/src/routes/config.rs)
echo "config.rs: $SCOPED/$TOTAL scoped"
[ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $((TOTAL - SCOPED)) unscoped"; exit 1; }
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4c PASS"
```

**CRITICAL: set_account() in Round 4a handlers:** When spawning an agent with a scoped account, handlers MUST call `state.kernel.registry.set_account(agent_id, oid.clone())` immediately after spawn succeeds. Without this, the agent has no owner and becomes visible to all tenants (SECURITY BUG). The registry method already exists in librefang-kernel; it's not deferred.

```rust
// In spawn/create handlers:
if let Some(ref oid) = account.0 {
    state.kernel.registry.set_account(agent_id, oid.clone())
        .map_err(|e| (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to assign owner"}))
        ))?;
}
```

---

### Round 5: Test Suite & Integration Wiring

Depends on: Rounds 1-4 (everything compiles and all round gates pass).

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
8. RED: `test_legacy_agent_roundtrip` → GREEN: legacy agents (no account_id) API-accessible with AccountId(None)

**Round 5 gate:**
```bash
cargo test -p librefang-api --test account_tests && \
cargo test --workspace && \
echo "Round 5 PASS"
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

- [ ] Pattern coverage gate passes (0 unscoped handlers: 50 agents + 11 channels + 15 config)
- [ ] Workspace compiles clean (`cargo build --workspace`)
- [ ] All 33 SPEC tests pass (`cargo test -p librefang-api --test account_tests`)
- [ ] All SPEC claims have cited test names
- [ ] Clippy clean: `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] No new warnings introduced
- [ ] Full workspace tests pass: `cargo test --workspace` (expect same 2 pre-existing failures only)
- [ ] AccountId extractor, macros, guard, HMAC all have RED-GREEN-REFACTOR test cycles
- [ ] AgentEntry cascade (cli 2 sites + types 2 sites) all compile
- [ ] API extractors infallible (Rejection = Infallible)

---

## Day-by-Day Schedule

| Day | Round | Deliverable | Gate |
|-----|-------|-------------|------|
| 1 AM | Round 1 | AccountId type, AgentEntry field, 2 type-crate literals | `cargo test -p librefang-types` |
| 1 PM | Round 2 | 2 CLI literals (tui/event.rs) | `cargo build -p librefang-cli` |
| 2 AM | Round 3 | Extractors (Batch A), macros (B), guard (C), HMAC (D) | `cargo test -p librefang-api` |
| 2 PM | Round 4a | agents.rs 50 handlers | Pattern gate: 50/50 + clippy + test |
| 3 AM | Round 4b | channels.rs 11 handlers | Pattern gate: 11/11 + clippy + test |
| 3 mid | Round 4c | config.rs 15 handlers | Pattern gate: 15/15 + clippy + test |
| 3 PM | Round 5 | 33 tests (3 batches), workspace green | Full gate pass |
| 4 | BHR | Pre-BHR checklist → submit | Confirmation only |

**Storage work (Memory migration v18, Kernel threading):** Handled by separate team. Not in Phase 1 scope.

---

## Rollback Plan

```bash
# 1. Revert all Phase 1 commits
git revert --no-commit HEAD~N  # N = Phase 1 commit count

# 2. Restore original handler signatures (remove AccountId param)
# Mechanical edit: find/replace "account: AccountId," with "" in all handlers

# 3. Verify
cargo test --workspace
```

**Note:** Storage schema rollback (migration v18 reversal) is handled by separate team if needed.

---

## Exit Criteria

```bash
# ALL must exit 0:
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p librefang-api --test account_tests

# Pattern coverage: 76 handlers fully scoped
for f in agents.rs channels.rs config.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  [ "$TOTAL" -eq "$SCOPED" ] || { echo "$f: $SCOPED/$TOTAL (FAIL)"; exit 1; }
  echo "$f: $SCOPED/$TOTAL (PASS)"
done

# API extractor tests all passing
cargo test -p librefang-api --lib api::extractors -- --nocapture

# HMAC middleware tests passing
cargo test -p librefang-api --lib api::middleware -- --nocapture

# AccountId type tests passing
cargo test -p librefang-types --lib types::account -- --nocapture

echo "ALL EXIT CRITERIA PASS"
```

---

## Dependency Graph

```
Round 1: librefang-types ──────────────────────┐
         (AccountId, AgentEntry.account_id)    │
                │                             │
Round 2: librefang-cli ───────────────────┐   │
         (2 literals in tui/event.rs)      │   │
                │                         │   │
         ┌──── WORKSPACE COMPILES ────────┘───┘
         │
Round 3: librefang-api (foundation) ────┐
         (extractors, macros, guard,    │
          HMAC, server wiring)          │
                │                       │
Round 4: librefang-api (76 handlers) ───┘
         4a: agents.rs (50)
         4b: channels.rs (11)
         4c: config.rs (15)
                │
Round 5: test suite + integration
         (33 tests, workspace green)

SEPARATE TRACK (not in Phase 1 scope):
Storage: librefang-memory + librefang-kernel
         (migration v18, 6 spawn variants,
          10 AgentEntry constructors)
         → Handled by storage team
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
