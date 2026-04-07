# PLAN-MT-002: Phases 2-4 — Resource Isolation, Data & Memory Isolation, Hardening

**SPEC:** SPEC-MT-002 (API Route Changes), SPEC-MT-003 (Database Migration), SPEC-MT-004 (Supabase RLS), SPEC-RV-002 (Supabase Vector Store)
**ADR:** ADR-MT-003 (Resource Isolation), ADR-MT-004 (Data & Memory Isolation), ADR-MT-005 (Event Bus Isolation)
**Predecessor:** PLAN-MT-001 (Phase 1 — Account Data Model & Foundation)
**Date:** 2026-04-06

---

## Verified Baseline Facts

```bash
# Handler counts in Phase 2 scope (2026-04-06):
$ grep -c "pub async fn" crates/librefang-api/src/routes/{skills,system,workflows,memory,network,providers,budget,plugins,goals,media,inbox,channels}.rs
crates/librefang-api/src/routes/skills.rs:53
crates/librefang-api/src/routes/system.rs:63
crates/librefang-api/src/routes/workflows.rs:30
crates/librefang-api/src/routes/memory.rs:25
crates/librefang-api/src/routes/network.rs:19
crates/librefang-api/src/routes/providers.rs:19
crates/librefang-api/src/routes/budget.rs:10
crates/librefang-api/src/routes/plugins.rs:8
crates/librefang-api/src/routes/goals.rs:7
crates/librefang-api/src/routes/media.rs:6
crates/librefang-api/src/routes/inbox.rs:1
crates/librefang-api/src/routes/channels.rs:11
# Total Phase 2: ~252 handlers (~10 system.rs public endpoints exempt)

# Tier breakdown (from ADR-MT-003 / SPEC-MT-002):
# Tier 1 (Full ownership — check_account):      97 (skills 53 + workflows 30 + goals 7 + inbox 1 + media 6)
# Tier 2 (Account-filtered — validate_account!): 37 (providers 19 + budget 10 + plugins 8)
# Tier 3 (Shared+overlay — account_or_system!):  55 (network 19 + memory 25 + channels 11)
# Tier 4 (system.rs — account_or_system!):       ~53 non-public scoped + ~10 public (no guard)
# Total scoped: ~242 | Public: ~10 | Grand total: 252

# Phase 1 exit state (prerequisite — MUST pass before Phase 2 starts):
$ grep -c "account: AccountId" crates/librefang-api/src/routes/{agents,channels,config}.rs
# agents.rs:    50/50  (fully scoped)
# channels.rs:  11/11  (extractor added, full guard logic deferred to Phase 2)
# config.rs:    15/15  (fully scoped)

# Memory store method counts (verified 2026-04-06):
$ grep -c "pub fn\|pub async fn" crates/librefang-memory/src/{structured,semantic,session,knowledge,usage,proactive}.rs
# structured.rs:  11 methods  (~8 account-sensitive, ~3 exempt)
# semantic.rs:    24 methods  (~18 account-sensitive, ~6 exempt)
# session.rs:     22 methods  (~16 account-sensitive, ~6 exempt)
# knowledge.rs:    6 methods  (~4 account-sensitive, ~2 exempt)
# usage.rs:       17 methods  (~12 account-sensitive, ~5 exempt)
# proactive.rs:   26 methods  (~18 account-sensitive, ~8 exempt)
# TOTAL: 106 methods, ~76 need account_id, ~30 exempt

# SQLite table count (all 19 objects from migration.rs):
$ grep -c "CREATE TABLE\|CREATE VIRTUAL TABLE" crates/librefang-memory/src/migration.rs
# 19
# Tables needing account_id in Phase 3: 14 (of 19)
#   Already scoped: 1 (agents — Phase 1 v18)
#   FK-scoped:      2 (experiment_variants, experiment_metrics)
#   System tables:  2 (migrations, sessions_fts rebuild-only)

# Current schema version (from migration.rs):
# v18 (post Phase 1)

# Supabase tables needing RLS (from SPEC-MT-004):
# 7 tables: documents, agent_kv, sessions, usage_log, skill_versions,
#           session_labels, api_audit_logs
```

---

## Phase 2: Resource Isolation (5-7 days, 11 tasks)

**Prerequisite:** PLAN-MT-001 Phase 1 exit criteria ALL PASS.

### Task Index

| Task | Description | Round | Depends On |
|------|-------------|-------|------------|
| 2.1 | AgentEntry `account_id` wired through `spawn_agent` | Round 1 | Phase 1 complete |
| 2.2 | `spawn_agent` callers pass account from extractor | Round 3 | 2.1 |
| 2.3 | Agent route handlers use scoped registry calls | Round 4 | 2.1, 2.2 |
| 2.4 | Integration registry gains `account_id` | Round 2 | Phase 1 complete |
| 2.5 | Integration route handlers scoped | Round 4 | 2.4 |
| 2.6 | Channel loading filters by account config | Round 3 | Phase 1 channels extractor |
| 2.7 | Channel route handlers fully scoped (guard logic) | Round 4 | 2.6 |
| 2.8 | Skill allowlist per account | Round 2 | Phase 1 complete |
| 2.9 | Remaining route files scoped (mechanical sweep) | Round 4 | 2.1-2.8 |
| 2.10 | Event bus account tagging | Round 3 | 2.1 |
| 2.11 | Filesystem partitioning (`accounts/{id}/`) | Round 2 | Phase 1 complete |

---

### Round 1: Types & Kernel — Account Propagation (no new deps)

Extends Phase 1 foundation. The `account_id` field on `AgentEntry` already exists; this round wires it through kernel entry points that Phase 1 left as `None` passthrough.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Wire account through `spawn_agent` public API | `crates/librefang-kernel/src/kernel.rs` | All 3 public spawn variants accept `AccountId` from caller (Phase 1 added param but callers passed `None`) | SPEC-MT-002 AC-1 |
| Registry `list_by_account` enforces filtering | `crates/librefang-kernel/src/registry.rs` | Verify Phase 1's `list_by_account()` is called by all list endpoints (not raw `list()`) | SPEC-MT-002 AC-1 |

**TDD micro-cycle:**
1. **RED:** `test_spawn_agent_with_explicit_account` -- spawn with `AccountId(Some("t1"))`, verify `registry.get(id).account_id == Some("t1")`
2. **GREEN:** Callers pass `AccountId` from extractor, not hardcoded `None`
3. **RED:** `test_list_by_account_excludes_other_tenants` -- spawn t1 + t2, `list_by_account(t1)` excludes t2
4. **GREEN:** Already implemented in Phase 1; test confirms wiring

**Round 1 gate:**
```bash
cargo clippy -p librefang-kernel --all-targets -- -D warnings && \
cargo test -p librefang-kernel && \
echo "Phase 2 Round 1 PASS"
```

---

### Round 2: Supporting Crates — Integration Registry, Skill Allowlist, Filesystem (librefang-types, librefang-kernel, librefang-channels)

No dependency on Round 1 output. Can run in parallel with Round 1 if convenient.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Integration registry `account_id` | `crates/librefang-kernel/src/kernel.rs` or relevant integration module | Add `account_id` to integration entries, filter in `list_integrations` | SPEC-MT-002 AC-2 |
| Skill allowlist config | `crates/librefang-types/src/config/types.rs` | Add `skill_allowlist: Option<Vec<String>>` to account config | ADR-MT-003 Skills |
| Skill allowlist check at invocation | `crates/librefang-kernel/src/kernel.rs` | If allowlist set and skill not in list, reject with 403 | ADR-MT-003 Skills |
| Filesystem partitioning | `crates/librefang-kernel/src/kernel.rs` | Account-partitioned dirs: `~/.librefang/accounts/{id}/agents/`, `/channels/`, `/workflows/`, `/prompts/` | ADR-MT-003 |
| Channel config loading by account | `crates/librefang-channels/` (if exists) or kernel channel loading | Filter channel configs to requesting account's directory | SPEC-MT-002 AC-6 |

**TDD micro-cycle:**
1. **RED:** `test_integration_registry_scoped` -- register integration for t1, `list_integrations(t2)` returns empty
2. **GREEN:** Add `account_id` to integration entries
3. **RED:** `test_skill_allowlist_blocks_disallowed` -- allowlist = ["web_search"], invoke "code_exec" -> error
4. **GREEN:** Check allowlist in skill invocation path
5. **RED:** `test_skill_allowlist_empty_allows_all` -- empty allowlist -> all skills available (backward compat)
6. **GREEN:** Empty = allow all
7. **RED:** `test_filesystem_partitioned_by_account` -- create account dir, verify agents/channels subdirs exist
8. **GREEN:** Implement `ensure_account_dirs(account_id)`

**Round 2 gate:**
```bash
cargo clippy -p librefang-kernel --all-targets -- -D warnings && \
cargo clippy -p librefang-types --all-targets -- -D warnings && \
cargo test -p librefang-kernel && \
cargo test -p librefang-types && \
echo "Phase 2 Round 2 PASS"
```

---

### Round 3: Kernel Wiring — Event Bus, Channel Loading, Spawn Callers (librefang-kernel)

Depends on: Rounds 1 + 2.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Event bus account tagging | `crates/librefang-kernel/src/kernel.rs` | All emitted events carry `account_id` from the originating agent | ADR-MT-003 |
| Channel loading filters by account | kernel channel init | Load only channels from `accounts/{id}/channels/` for the requesting account | SPEC-MT-002 AC-6 |
| Spawn callers pass account | All API route files that call `kernel.spawn_agent()` | Replace `None` with extracted `AccountId` | SPEC-MT-002 AC-1 |

**TDD micro-cycle:**
1. **RED:** `test_event_bus_tags_account` -- spawn agent for t1, trigger event, verify `event.account_id == "t1"`
2. **GREEN:** Thread `account_id` through event emission
3. **RED:** `test_channel_loading_scoped` -- t1 has slack channel, t2 has discord channel, `load_channels(t1)` returns only slack
4. **GREEN:** Filter channel dir listing by account

**Round 3 gate:**
```bash
cargo clippy -p librefang-kernel --all-targets -- -D warnings && \
cargo test -p librefang-kernel && \
echo "Phase 2 Round 3 PASS"
```

---

### Round 4: API Handlers — Mechanical Sweep (librefang-api, ~252 handlers)

Depends on: Rounds 1-3 (all kernel-level wiring complete).

Every handler gets the same pattern established in Phase 1:

```rust
pub async fn handler_name(
    State(state): State<Arc<AppState>>,
    account: AccountId,  // NEW
    // ... existing params
) -> impl IntoResponse {
    // Tier 1: check_account(&entry, &account)?;
    // Tier 2: validate_account!(account); on writes, account_or_system!(account); on reads
    // Tier 3: account_or_system!(account);
    // Tier 4: (no guard — public)
}
```

#### Round 4a: Tier 1 files (97 handlers)

| File | Handlers | Guard | AC |
|------|----------|-------|-----|
| `routes/skills.rs` | 53 | `check_account()` on all ops | SPEC-MT-002 AC-1 |
| `routes/workflows.rs` | 30 | `check_account()` on all ops | SPEC-MT-002 AC-1 |
| `routes/goals.rs` | 7 | `check_account()` on all ops | SPEC-MT-002 AC-1 |
| `routes/inbox.rs` | 1 | `check_account()` | SPEC-MT-002 AC-1 |
| `routes/media.rs` | 6 | `check_account()` on all ops | SPEC-MT-002 AC-1 |

**Round 4a gate:**
```bash
for f in skills.rs workflows.rs goals.rs inbox.rs media.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  echo "$f: $SCOPED/$TOTAL"
  [ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $f not fully scoped"; exit 1; }
done
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4a PASS"
```

#### Round 4b: Tier 2 files (37 handlers)

| File | Handlers | Guard | AC |
|------|----------|-------|-----|
| `routes/providers.rs` | 19 | Reads: `account_or_system!`, Writes: `validate_account!` | SPEC-MT-002 AC-2, AC-3 |
| `routes/budget.rs` | 10 | Reads: `account_or_system!`, Writes: `validate_account!` | SPEC-MT-002 AC-2, AC-3 |
| `routes/plugins.rs` | 8 | Reads: `account_or_system!`, Writes: `validate_account!` | SPEC-MT-002 AC-2, AC-3 |

**Round 4b gate:**
```bash
for f in providers.rs budget.rs plugins.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  echo "$f: $SCOPED/$TOTAL"
  [ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $f not fully scoped"; exit 1; }
done
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4b PASS"
```

#### Round 4c: Tier 3 files (55 handlers)

| File | Handlers | Guard | AC |
|------|----------|-------|-----|
| `routes/network.rs` | 19 | `account_or_system!` | SPEC-MT-002 AC-4 |
| `routes/memory.rs` | 25 | `account_or_system!` | SPEC-MT-002 AC-4 |
| `routes/channels.rs` | 11 | Full guard logic (extractor from Phase 1, `check_account` this phase) | SPEC-MT-002 AC-6 |

**Round 4c gate:**
```bash
for f in network.rs memory.rs channels.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  echo "$f: $SCOPED/$TOTAL"
  [ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $f not fully scoped"; exit 1; }
done
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4c PASS"
```

#### Round 4d: Tier 4 — system.rs (63 handlers, ~53 scoped + ~10 public)

| File | Handlers | Guard | AC |
|------|----------|-------|-----|
| `routes/system.rs` | 63 | ~53 get `account_or_system!`; ~10 public endpoints (health/version/ready) explicitly marked `// PUBLIC` with no guard | SPEC-MT-002 AC-5 |

**Round 4d gate:**
```bash
TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/system.rs")
PUBLIC=$(grep -c "// PUBLIC" "crates/librefang-api/src/routes/system.rs" || echo 0)
SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/system.rs" || echo 0)
EXPECTED=$((TOTAL - PUBLIC))
echo "system.rs: $SCOPED/$EXPECTED non-public scoped ($PUBLIC public)"
[ "$SCOPED" -ge "$EXPECTED" ] || { echo "FAIL: system.rs not fully scoped"; exit 1; }
cargo clippy -p librefang-api --all-targets -- -D warnings && \
cargo test -p librefang-api && \
echo "Round 4d PASS"
```

---

### Round 5: Phase 2 Integration Tests

Depends on: Rounds 1-4 (all handlers scoped).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Tier 1 cross-tenant tests | `crates/librefang-api/tests/phase2_isolation_tests.rs` | Cross-tenant skill/workflow/goal/media access returns 404 | SPEC-MT-002 AC-1 |
| Tier 2 system fallback tests | same file | System providers visible to all accounts; writes require account | SPEC-MT-002 AC-2, AC-3 |
| Tier 3 memory scoping tests | same file | Memory recall scoped by account; system memories visible | SPEC-MT-002 AC-4 |
| Tier 4 public endpoint tests | same file | Health/version work without X-Account-Id | SPEC-MT-002 AC-5 |
| Channel scoping tests | same file | Channel listing returns only own account's channels | SPEC-MT-002 AC-6 |

**TDD micro-cycles:**

**Batch A: Tier 1 cross-tenant (RED->GREEN x4)**
1. RED: `test_cross_tenant_skill_404` -> GREEN: skill owned by t1, GET by t2 -> 404
2. RED: `test_cross_tenant_workflow_404` -> GREEN: workflow owned by t1, GET by t2 -> 404
3. RED: `test_cross_tenant_goal_404` -> GREEN: goal owned by t1, GET by t2 -> 404
4. RED: `test_cross_tenant_media_404` -> GREEN: media owned by t1, GET by t2 -> 404

**Batch B: Tier 2 system defaults (RED->GREEN x3)**
5. RED: `test_system_provider_visible_all` -> GREEN: system provider listed for t1 and t2
6. RED: `test_tenant_provider_invisible_cross` -> GREEN: t1's custom provider not in t2's list
7. RED: `test_tier2_write_requires_account` -> GREEN: POST provider without X-Account-Id -> 400

**Batch C: Tier 3 + Tier 4 (RED->GREEN x3)**
8. RED: `test_memory_recall_scoped` -> GREEN: t1 recall excludes t2 memories
9. RED: `test_health_no_account` -> GREEN: GET /health without header -> 200
10. RED: `test_channel_listing_scoped` -> GREEN: t1 channels not visible to t2

**Round 5 gate:**
```bash
cargo test -p librefang-api --test phase2_isolation_tests && \
cargo test --workspace && \
echo "Phase 2 Round 5 PASS"
```

---

## Phase 3: Data & Memory Isolation (3-5 days, 11 tasks)

**Prerequisite:** Phase 2 exit criteria ALL PASS.

### Task Index

| Task | Description | Round | Depends On |
|------|-------------|-------|------------|
| 3.1 | Migration v19 — 14 tables + indexes + FTS5 rebuild | Round 1 | Phase 2 complete |
| 3.2 | `semantic.rs` — 18 methods get `account_id` param | Round 2a | 3.1 |
| 3.3 | `session.rs` — 16 methods get `account_id` param | Round 2b | 3.1 |
| 3.4 | `structured.rs` — 8 methods get `account_id` param | Round 2c | 3.1 |
| 3.5 | `proactive.rs` — 18 methods get `account_id` param | Round 2d | 3.1 |
| 3.6 | `knowledge.rs` — 4 methods get `account_id` param | Round 2d | 3.1 |
| 3.7 | `usage.rs` — 12 methods get `account_id` param | Round 2d | 3.1 |
| 3.8 | Context engine threads account through recall | Round 3 | 3.2-3.7 |
| 3.9 | `http_vector_store.rs` passes account to Supabase | Round 3 | 3.2 |
| 3.10 | Supabase RLS policies deployed (SPEC-MT-004) | Round 3 | 3.9 |
| 3.11 | Memory route handlers call scoped store methods | Round 4 | 3.2-3.10 |

---

### Round 1: Migration v19 (librefang-memory)

No dependency on other Phase 3 rounds. Must complete before any store file changes.

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Add migration v19 | `crates/librefang-memory/src/migration.rs` | Column-existence-guarded ALTER TABLE on 14 tables + indexes + FTS5 rebuild | SPEC-MT-003 AC-1 |
| Bump schema version | `crates/librefang-memory/src/migration.rs` | 18 -> 19 | -- |

**Migration SQL (column-existence-guarded, learned from Phase 1):**

Each ALTER is wrapped in a Rust-side column-existence check to prevent the
duplicate-column class of bug that already broke `test_deactivate_kills_agent`
in multi_agent_test.rs. The Rust implementation pattern:

```rust
// Rust wrapper for each ALTER (in migration.rs):
fn add_column_if_missing(conn: &Connection, table: &str, column: &str, typedef: &str) {
    let exists: bool = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM pragma_table_info('{}') WHERE name = ?", table),
            [column],
            |row| row.get(0),
        )
        .unwrap_or(false);
    if !exists {
        conn.execute_batch(&format!(
            "ALTER TABLE {} ADD COLUMN {} {};", table, column, typedef
        )).expect("ALTER TABLE failed");
    }
}

// Called for each of the 14 tables:
let tables = [
    "sessions", "events", "kv_store", "memories", "entities", "relations",
    "task_queue", "usage_events", "canonical_sessions", "paired_devices",
    "audit_entries", "prompt_versions", "prompt_experiments", "approval_audit",
];
for table in &tables {
    add_column_if_missing(conn, table, "account_id", "TEXT NOT NULL DEFAULT 'system'");
}
```

Equivalent SQL (for documentation / manual rollback reference):
```sql
-- v19: Multi-tenant data isolation
-- NOTE: In production, each ALTER is guarded by column-existence check in Rust.
-- The raw SQL below is for reference only — do NOT run without guards.

-- Core tables
ALTER TABLE sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE events ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE kv_store ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE memories ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE entities ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE relations ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE task_queue ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Usage & billing
ALTER TABLE usage_events ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Sessions & devices
ALTER TABLE canonical_sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE paired_devices ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Audit (Merkle chain — existing prev_hash chain preserved because DEFAULT does not change hashes)
ALTER TABLE audit_entries ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Prompts
ALTER TABLE prompt_versions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE prompt_experiments ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Approval
ALTER TABLE approval_audit ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Indexes (all idempotent)
CREATE INDEX IF NOT EXISTS idx_sessions_account ON sessions(account_id);
CREATE INDEX IF NOT EXISTS idx_events_account ON events(account_id);
CREATE INDEX IF NOT EXISTS idx_kv_store_account ON kv_store(account_id);
CREATE INDEX IF NOT EXISTS idx_memories_account ON memories(account_id);
CREATE INDEX IF NOT EXISTS idx_usage_events_account ON usage_events(account_id);

-- FTS5 rebuild (cannot ALTER virtual tables)
DROP TABLE IF EXISTS sessions_fts;
CREATE VIRTUAL TABLE sessions_fts USING fts5(
  session_id, content, account_id,
  content='sessions', content_rowid='rowid'
);
INSERT INTO sessions_fts(session_id, content, account_id)
  SELECT id, messages, account_id FROM sessions;
```

**TDD micro-cycle:**
1. **RED:** `test_migration_v19_all_tables` -- fresh DB -> migrate -> all 14 tables have `account_id` via `PRAGMA table_info`
2. **GREEN:** Implement migration with all 14 ALTERs
3. **RED:** `test_migration_v19_default_system` -- pre-existing rows get `account_id = 'system'`
4. **GREEN:** `DEFAULT 'system'` clause handles this
5. **RED:** `test_migration_v19_idempotent` -- run migrate twice, no duplicate-column error
6. **GREEN:** Column-existence guard wrapping each ALTER
7. **RED:** `test_migration_v19_fts5_rebuilt` -- FTS5 table has `account_id` column, row count matches sessions
8. **GREEN:** DROP + RECREATE + INSERT from sessions

**Rollback SQL (document, don't auto-run):**
```sql
-- SQLite >= 3.35.0: DROP COLUMN per table
ALTER TABLE sessions DROP COLUMN account_id;
ALTER TABLE events DROP COLUMN account_id;
ALTER TABLE kv_store DROP COLUMN account_id;
ALTER TABLE memories DROP COLUMN account_id;
ALTER TABLE entities DROP COLUMN account_id;
ALTER TABLE relations DROP COLUMN account_id;
ALTER TABLE task_queue DROP COLUMN account_id;
ALTER TABLE usage_events DROP COLUMN account_id;
ALTER TABLE canonical_sessions DROP COLUMN account_id;
ALTER TABLE paired_devices DROP COLUMN account_id;
ALTER TABLE audit_entries DROP COLUMN account_id;
ALTER TABLE prompt_versions DROP COLUMN account_id;
ALTER TABLE prompt_experiments DROP COLUMN account_id;
ALTER TABLE approval_audit DROP COLUMN account_id;

-- FTS5: rebuild without account_id
DROP TABLE IF EXISTS sessions_fts;
CREATE VIRTUAL TABLE sessions_fts USING fts5(
  session_id, content,
  content='sessions', content_rowid='rowid'
);
```

**Round 1 gate:**
```bash
cargo clippy -p librefang-memory --all-targets -- -D warnings && \
cargo test -p librefang-memory -- migration_v19 && \
echo "Phase 3 Round 1 PASS"
```

---

### Round 2: Memory Store Files — Account Parameter Threading (librefang-memory)

Depends on: Round 1 (migration compiles, schema at v19).

Every account-sensitive store method gains an `account_id: &str` parameter:

```rust
// BEFORE:
pub fn list_sessions(&self, agent_id: &str) -> Vec<Session>

// AFTER:
pub fn list_sessions(&self, agent_id: &str, account_id: &str) -> Vec<Session>
// SQL: WHERE agent_id = ? AND account_id = ?
// Exception: if account_id == "system" → no account_id filter (legacy mode)
```

#### Round 2a: semantic.rs (24 methods, ~18 account-sensitive)

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Add `account_id` to ~18 methods | `crates/librefang-memory/src/semantic.rs` | recall, store, search, delete, list — all filtered by account | SPEC-MT-003 AC-6 |
| System account sees all | same | `if account_id == "system" { /* no filter */ }` | SPEC-MT-003 AC-7 |

**TDD micro-cycle:**
1. **RED:** `test_semantic_recall_scoped` -- store memory for t1 and t2, recall(t1) returns only t1's
2. **GREEN:** Add `WHERE account_id = ?` to recall SQL
3. **RED:** `test_semantic_system_sees_all` -- recall("system") returns all memories
4. **GREEN:** Skip account filter for "system"

**Round 2a gate:**
```bash
cargo clippy -p librefang-memory --all-targets -- -D warnings && \
cargo test -p librefang-memory -- semantic && \
echo "Round 2a PASS"
```

#### Round 2b: session.rs (22 methods, ~16 account-sensitive)

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Add `account_id` to ~16 methods | `crates/librefang-memory/src/session.rs` | list, get, compact, delete — all filtered by account | SPEC-MT-003 AC-8 |

**TDD micro-cycle:**
1. **RED:** `test_session_list_scoped` -- sessions for t1 and t2, `list_sessions(t1)` returns only t1's
2. **GREEN:** Add `WHERE account_id = ?`
3. **RED:** `test_session_delete_cross_tenant` -- t2 cannot delete t1's session
4. **GREEN:** Delete includes account_id in WHERE

**Round 2b gate:**
```bash
cargo clippy -p librefang-memory --all-targets -- -D warnings && \
cargo test -p librefang-memory -- session && \
echo "Round 2b PASS"
```

#### Round 2c: structured.rs (11 methods, ~8 account-sensitive)

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Add `account_id` to ~8 methods | `crates/librefang-memory/src/structured.rs` | KV CRUD, agent load/list, task queue — all filtered by account | SPEC-MT-003 AC-5 |

**TDD micro-cycle:**
1. **RED:** `test_kv_filtered_by_account` -- t1 KV entry not visible to t2
2. **GREEN:** Add `WHERE account_id = ?` to KV queries

**Round 2c gate:**
```bash
cargo clippy -p librefang-memory --all-targets -- -D warnings && \
cargo test -p librefang-memory -- structured && \
echo "Round 2c PASS"
```

#### Round 2d: proactive.rs (26 methods, ~18 account-sensitive) + knowledge.rs (6 methods, ~4) + usage.rs (17 methods, ~12)

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Add `account_id` to ~18 methods | `crates/librefang-memory/src/proactive.rs` | add, search, list, hooks — all filtered | SPEC-MT-003 AC-6 |
| Add `account_id` to ~4 methods | `crates/librefang-memory/src/knowledge.rs` | entity/relation CRUD filtered | SPEC-MT-003 AC-5 |
| Add `account_id` to ~12 methods | `crates/librefang-memory/src/usage.rs` | log, query, aggregate filtered for billing | SPEC-MT-003 AC-10 |

**TDD micro-cycle:**
1. **RED:** `test_usage_scoped_billing` -- t1 usage not counted in t2's billing query
2. **GREEN:** Add `WHERE account_id = ?` to usage aggregation
3. **RED:** `test_audit_merkle_preserved` -- existing Merkle chain hashes unchanged after migration
4. **GREEN:** DEFAULT 'system' preserves all existing rows; chain integrity verified

**Round 2d gate:**
```bash
cargo clippy -p librefang-memory --all-targets -- -D warnings && \
cargo test -p librefang-memory && \
echo "Round 2d PASS (full memory crate)"
```

---

### Round 3: Kernel + Vector Store — Account Wiring (librefang-kernel, librefang-memory)

Depends on: Round 2 (all store methods accept `account_id`).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Context engine threads account | `crates/librefang-kernel/src/kernel.rs` | `build_context()` passes agent's `account_id` to memory recall | SPEC-MT-003 AC-6 |
| HttpVectorStore passes account | `crates/librefang-memory/src/http_vector_store.rs` | Insert: tag vectors with `account_id`. Search: filter param. | SPEC-MT-004 AC-7 |
| SupabaseVectorStore account awareness | `crates/librefang-memory/src/supabase_vector_store.rs` | RPC calls include `doc_account_id` / `caller_account_id` | SPEC-MT-004 AC-7, AC-9 |
| Supabase RLS policies deployed | Supabase migration SQL | 7 tables: account-scoped SELECT/INSERT/DELETE + service bypass | SPEC-MT-004 AC-1 through AC-12 |

**TDD micro-cycle:**
1. **RED:** `test_context_engine_scoped_recall` -- agent with t1 account, recall uses t1 filter
2. **GREEN:** Thread `account_id` from agent entry through context engine to store
3. **RED:** `test_http_vector_store_tags_account` -- insert includes account_id param
4. **GREEN:** Add `account_id` to HTTP request body
5. **RED:** `test_supabase_search_filters_account` -- search includes `caller_account_id`
6. **GREEN:** Add param to RPC call

**Round 3 gate:**
```bash
cargo clippy -p librefang-kernel --all-targets -- -D warnings && \
cargo clippy -p librefang-memory --all-targets -- -D warnings && \
cargo test -p librefang-kernel && \
cargo test -p librefang-memory && \
echo "Phase 3 Round 3 PASS"
```

---

### Round 4: API Memory Routes — Scoped Calls (librefang-api)

Depends on: Round 3 (kernel and stores are account-aware).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Memory route handlers call scoped methods | `crates/librefang-api/src/routes/memory.rs` | All 25 handlers pass `account_id` from extractor to store methods | SPEC-MT-002 AC-4 |
| Integration test: memory isolation | `crates/librefang-api/tests/phase3_memory_tests.rs` | End-to-end: store memory for t1, recall as t2 returns empty | SPEC-MT-003 AC-6 |

**TDD micro-cycles:**

**Batch A: Memory isolation (RED->GREEN x4)**
1. RED: `test_memory_store_scoped` -> GREEN: store with account, retrieve only with matching account
2. RED: `test_memory_recall_cross_tenant_empty` -> GREEN: t2 recall excludes t1 memories
3. RED: `test_session_api_scoped` -> GREEN: t1 sessions not visible to t2 via API
4. RED: `test_system_sees_all_memories` -> GREEN: system account retrieves all memories

**Round 4 gate:**
```bash
cargo test -p librefang-api --test phase3_memory_tests && \
cargo test --workspace && \
echo "Phase 3 Round 4 PASS"
```

---

## Phase 4: Hardening & Integration (3-5 days, 9 tasks)

**Prerequisite:** Phase 3 exit criteria ALL PASS.

### Task Index

| Task | Description | Round | Depends On |
|------|-------------|-------|------------|
| 4.1 | Security audit — cross-tenant access review | Round 1 | Phase 3 complete |
| 4.2 | HMAC timing attack review | Round 1 | Phase 1 HMAC impl |
| 4.3 | Pentest — API cross-tenant enumeration | Round 2 | 4.1 |
| 4.4 | Pentest — RLS bypass attempts | Round 2 | 4.1 |
| 4.5 | Performance baseline — multi-tenant overhead | Round 3 | 4.3, 4.4 |
| 4.6 | Monitoring dashboard — per-account metrics | Round 3 | 4.5 |
| 4.7 | Migration guide update | Round 4 | 4.1-4.6 |
| 4.8 | API documentation — multi-tenant endpoints | Round 4 | 4.1-4.6 |
| 4.9 | CONTRIBUTING.md — multi-tenant dev guide | Round 4 | 4.1-4.6 |

---

### Round 1: Security Audit (no code changes unless findings)

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Cross-tenant access audit | All route files | Manual review: every handler uses correct tier guard | -- |
| HMAC timing review | `crates/librefang-api/src/middleware.rs` | Verify `verify_slice()` (constant-time), NOT `==` | Phase 1 AC-12 |
| Error body audit | All route files | Verify 404 bodies never leak account_id or resource details | SPEC-MT-002 AC-1 |

**Audit checklist (executable):**
```bash
# 1. No handler returns 403 (always 404 for cross-tenant)
grep -rn "StatusCode::FORBIDDEN\|FORBIDDEN\|403" crates/librefang-api/src/routes/ && \
  echo "WARNING: Found 403 references — verify these are NOT cross-tenant paths" || \
  echo "PASS: No 403 references in route handlers"

# 2. No error body contains "account" string
grep -rn '"account' crates/librefang-api/src/routes/ | grep -i "error\|response\|json\|body" && \
  echo "WARNING: Possible account leak in error response" || \
  echo "PASS: No account references in error bodies"

# 3. HMAC uses constant-time comparison
grep -n "verify_slice\|constant_time" crates/librefang-api/src/middleware.rs || \
  echo "FAIL: No constant-time HMAC verification found"
```

**Round 1 gate:**
```bash
echo "Security audit: manual review + automated checks complete"
# No cargo gate — this round produces findings, not code
```

---

### Round 2: Penetration Testing

Depends on: Round 1 (audit complete, critical findings fixed).

| Test | Method | Expected Result |
|------|--------|----------------|
| Header spoofing | Send `X-Account-Id: victim` without valid HMAC sig | 401 Unauthorized (if HMAC enabled) or scoped to attacker's resources |
| ID enumeration | Iterate agent IDs with wrong account | 404 for every attempt, no timing difference |
| SQL injection via account_id | Send `'; DROP TABLE agents;--` as X-Account-Id | Parameterized query prevents injection |
| FTS5 injection | Search with malicious FTS5 syntax via account_id | No SQL execution; account_id is parameterized |
| RLS bypass via direct Supabase | Use anon key to query documents table directly | RLS filters to authenticated user's accounts only |
| Service role leak | Verify service role key never exposed in API responses | Key only in server-side env, never in response headers/body |

**Round 2 gate:**
```bash
# Pentest findings documented; all critical/high findings have fixes
echo "Pentest round complete — see findings report"
```

---

### Round 3: Performance & Monitoring

Depends on: Round 2 (no outstanding critical security findings).

| Change | File | Description | AC |
|--------|------|-------------|-----|
| Performance baseline | `tests/bench/` or script | Measure: list agents (10 accounts x 100 agents each), memory recall latency, migration time on 100k rows | -- |
| Per-account usage dashboard | `crates/librefang-api/src/routes/system.rs` | New endpoint: `GET /system/accounts/:id/usage` (admin only) | -- |

**Performance targets:**
- `list_agents` with account filter: < 5ms for 100 agents (index on `account_id`)
- `memory_recall` with account filter: < 50ms for 1000 memories per account
- Migration v19 on 100k rows: < 30s (ALTER TABLE + index creation)
- FTS5 rebuild on 10k sessions: < 10s

**Round 3 gate:**
```bash
cargo test --workspace && \
echo "Phase 4 Round 3 PASS"
```

---

### Round 4: Documentation

Depends on: Rounds 1-3.

| Change | File | Description |
|--------|------|-------------|
| Migration guide update | `docs/multi-tenant/MIGRATION-GUIDE.md` | Add Phase 2-4 migration steps, rollback procedures, Supabase RLS setup |
| API docs | `docs/api/` | Document X-Account-Id header, HMAC signing, tier behavior for all endpoints |
| CONTRIBUTING.md update | `CONTRIBUTING.md` | Multi-tenant development guide: how to add new handlers with account scoping |

**Round 4 gate:**
```bash
# Documentation review — no cargo gate
echo "Documentation complete"
```

---

## Pattern Coverage Gate (MANDATORY — spans Phases 1-2)

```bash
#!/bin/bash
set -e
echo "=== Full Pattern Coverage Gate (Phase 1 + Phase 2) ==="

FAIL=0
for f in skills.rs workflows.rs memory.rs network.rs providers.rs \
         budget.rs plugins.rs goals.rs media.rs inbox.rs channels.rs; do
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

# system.rs: check non-public handlers separately (NOT in the loop above — ~10 public handlers are exempt)
TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/system.rs")
PUBLIC=$(grep -c "// PUBLIC" "crates/librefang-api/src/routes/system.rs" || echo 0)
SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/system.rs" || echo 0)
EXPECTED=$((TOTAL - PUBLIC))
if [ "$SCOPED" -lt "$EXPECTED" ]; then
  echo "FAIL: system.rs: $SCOPED/$EXPECTED non-public scoped"
  FAIL=1
else
  echo "PASS: system.rs: $SCOPED/$EXPECTED non-public scoped ($PUBLIC public)"
fi

# Also include Phase 1 files to verify no regression:
for f in agents.rs config.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  [ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $f REGRESSION: $SCOPED/$TOTAL"; FAIL=1; }
  echo "PASS: $f: $SCOPED/$TOTAL scoped (Phase 1 — no regression)"
done

# Workspace compiles clean:
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

**Expected output after Phase 2:**
```
PASS: skills.rs: 53/53 scoped
PASS: workflows.rs: 30/30 scoped
PASS: memory.rs: 25/25 scoped
PASS: network.rs: 19/19 scoped
PASS: providers.rs: 19/19 scoped
PASS: budget.rs: 10/10 scoped
PASS: plugins.rs: 8/8 scoped
PASS: goals.rs: 7/7 scoped
PASS: media.rs: 6/6 scoped
PASS: inbox.rs: 1/1 scoped
PASS: channels.rs: 11/11 scoped
PASS: system.rs: 53/53 non-public scoped (10 public)
PASS: agents.rs: 50/50 scoped (Phase 1 — no regression)
PASS: config.rs: 15/15 scoped (Phase 1 — no regression)
PASS: workspace compiles clean
=== Pattern Coverage: ALL CLEAR ===
```

---

## Memory Store Coverage Gate (MANDATORY — Phase 3)

```bash
#!/bin/bash
set -e
echo "=== Memory Store Coverage Gate ==="

FAIL=0

# Verify all 14 tables have account_id column
for table in sessions events kv_store memories entities relations \
             task_queue usage_events canonical_sessions paired_devices \
             audit_entries prompt_versions prompt_experiments approval_audit; do
  # Run migration test that checks PRAGMA table_info
  echo "Table $table: verified via test_migration_v19_all_tables"
done

# Method count verification
for f in structured.rs semantic.rs session.rs knowledge.rs usage.rs proactive.rs; do
  TOTAL=$(grep -c "pub fn\|pub async fn" "crates/librefang-memory/src/$f")
  SCOPED=$(grep -c "account_id\|account:" "crates/librefang-memory/src/$f" || echo 0)
  if [ "$SCOPED" -lt 2 ]; then
    echo "FAIL: $f: only $SCOPED account references in $TOTAL methods"
    FAIL=1
  else
    echo "PASS: $f: $SCOPED account references in $TOTAL methods"
  fi
done

# Full test suite
cargo test -p librefang-memory || FAIL=1

if [ "$FAIL" -eq 1 ]; then
  echo "=== MEMORY GATE FAILED ==="
  exit 1
fi
echo "=== Memory Store Coverage: ALL CLEAR ==="
```

---

## Pre-BHR Checklist

### Phase 2
- [ ] Pattern coverage gate passes: 11 Phase 2 route files fully scoped + system.rs 53/53 non-public scoped
- [ ] Phase 1 regression check: agents.rs 50/50, channels.rs 11/11, config.rs 15/15
- [ ] Workspace compiles clean: `cargo build --workspace`
- [ ] All SPEC-MT-002 tests pass: `cargo test -p librefang-api --test phase2_isolation_tests`
- [ ] All SPEC-MT-002 claims have cited test names
- [ ] Clippy clean: `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] No new warnings introduced
- [ ] Full workspace tests pass: `cargo test --workspace`
- [ ] Cross-tenant access returns 404 (never 403)
- [ ] Error bodies never contain account_id or cross-tenant resource details
- [ ] System health endpoints work without X-Account-Id header

### Phase 3
- [ ] Memory store coverage gate passes: all 6 store files have account references
- [ ] Migration v19 adds account_id to all 14 tables
- [ ] Migration v19 is idempotent (no duplicate-column errors)
- [ ] FTS5 rebuilt with account_id column, row count matches sessions table
- [ ] Existing data defaults to `account_id = 'system'`
- [ ] Audit trail Merkle chain integrity preserved
- [ ] System account ("system") sees all data (backward compat)
- [ ] All SPEC-MT-003 tests pass: `cargo test -p librefang-memory -- migration_v19`
- [ ] All SPEC-MT-003 claims have cited test names
- [ ] Supabase RLS policies deployed and verified (SPEC-MT-004)

### Phase 4
- [ ] Security audit complete: no cross-tenant access paths found
- [ ] HMAC uses constant-time comparison (`verify_slice()`)
- [ ] Pentest complete: no critical/high findings outstanding
- [ ] RLS pentest: no bypass vectors found
- [ ] Performance baseline documented and within targets
- [ ] Migration guide updated with Phase 2-4 steps
- [ ] API docs include multi-tenant header requirements
- [ ] CONTRIBUTING.md includes multi-tenant development guide

---

## Day-by-Day Schedule

### Phase 2 (5-7 days)

| Day | Round | Deliverable | Gate |
|-----|-------|-------------|------|
| 1 AM | Round 1 | Kernel spawn wiring, registry enforcement | `cargo test -p librefang-kernel` |
| 1 PM | Round 2 | Integration registry, skill allowlist, filesystem partitioning | `cargo test -p librefang-kernel` + `cargo test -p librefang-types` |
| 2 | Round 3 | Event bus tagging, channel loading, spawn caller wiring | `cargo test -p librefang-kernel` |
| 3 | Round 4a | Tier 1: skills.rs (53), workflows.rs (30), goals.rs (7), inbox.rs (1), media.rs (6) | Pattern gate: 97/97 + clippy + test |
| 4 AM | Round 4b | Tier 2: providers.rs (19), budget.rs (10), plugins.rs (8) | Pattern gate: 37/37 + clippy + test |
| 4 PM | Round 4c | Tier 3: network.rs (19), memory.rs (25), channels.rs (11) | Pattern gate: 55/55 + clippy + test |
| 5 AM | Round 4d | Tier 4: system.rs (63, ~53 scoped + ~10 public) | Pattern gate: 53/53 non-public + clippy + test |
| 5 PM | Round 5 | Integration tests (10 tests, 3 batches), workspace green | Full gate pass |
| 6 | BHR | Pre-BHR checklist -> submit | Confirmation only |

### Phase 3 (3-5 days)

| Day | Round | Deliverable | Gate |
|-----|-------|-------------|------|
| 7 AM | Round 1 | Migration v19 (14 tables, FTS5 rebuild) | `cargo test -p librefang-memory -- migration_v19` |
| 7 PM | Round 2a | semantic.rs (~18 methods scoped) | `cargo test -p librefang-memory -- semantic` |
| 8 AM | Round 2b | session.rs (~16 methods scoped) | `cargo test -p librefang-memory -- session` |
| 8 mid | Round 2c | structured.rs (~8 methods scoped) | `cargo test -p librefang-memory -- structured` |
| 8 PM | Round 2d | proactive.rs + knowledge.rs + usage.rs (~34 methods scoped) | `cargo test -p librefang-memory` |
| 9 AM | Round 3 | Context engine, HttpVectorStore, Supabase RLS | `cargo test -p librefang-kernel` + `cargo test -p librefang-memory` |
| 9 PM | Round 4 | Memory route handlers, integration tests | `cargo test --workspace` |
| 10 | BHR | Pre-BHR checklist -> submit | Confirmation only |

### Phase 4 (3-5 days)

| Day | Round | Deliverable | Gate |
|-----|-------|-------------|------|
| 11 | Round 1 | Security audit + HMAC timing review | Findings documented |
| 12 | Round 2 | Pentest: API cross-tenant + RLS bypass | All critical/high fixed |
| 13 AM | Round 3 | Performance baseline + dashboard endpoint | Targets met |
| 13 PM | Round 4 | Migration guide, API docs, CONTRIBUTING.md | Documentation reviewed |
| 14 | BHR | Pre-BHR checklist -> submit | Confirmation only |

---

## Rollback Plan

### Phase 2 Rollback (API handlers only — no schema changes)

```bash
# Revert all Phase 2 commits (handler signature changes)
git revert --no-commit HEAD~N  # N = Phase 2 commit count

# Verify Phase 1 still works
cargo test --workspace
```

### Phase 3 Rollback (schema + store changes)

```bash
# 1. Revert Phase 3 commits
git revert --no-commit HEAD~N  # N = Phase 3 commit count

# 2. Rollback migration v19 (SQLite >= 3.35.0)
sqlite3 librefang.db << 'SQL'
ALTER TABLE sessions DROP COLUMN account_id;
ALTER TABLE events DROP COLUMN account_id;
ALTER TABLE kv_store DROP COLUMN account_id;
ALTER TABLE memories DROP COLUMN account_id;
ALTER TABLE entities DROP COLUMN account_id;
ALTER TABLE relations DROP COLUMN account_id;
ALTER TABLE task_queue DROP COLUMN account_id;
ALTER TABLE usage_events DROP COLUMN account_id;
ALTER TABLE canonical_sessions DROP COLUMN account_id;
ALTER TABLE paired_devices DROP COLUMN account_id;
ALTER TABLE audit_entries DROP COLUMN account_id;
ALTER TABLE prompt_versions DROP COLUMN account_id;
ALTER TABLE prompt_experiments DROP COLUMN account_id;
ALTER TABLE approval_audit DROP COLUMN account_id;

DROP TABLE IF EXISTS sessions_fts;
CREATE VIRTUAL TABLE sessions_fts USING fts5(
  session_id, content,
  content='sessions', content_rowid='rowid'
);
SQL

# 3. Rollback Supabase RLS (run in Supabase SQL editor)
# Revert to user-based policies — see SPEC-MT-004 for exact SQL

# 4. Verify
cargo test --workspace
```

---

## Exit Criteria

```bash
#!/bin/bash
set -e

echo "=== PLAN-MT-002 Exit Criteria ==="

# === Phase 2: Resource Isolation ===

# 1. Clippy + tests
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 2. Phase 2 integration tests
cargo test -p librefang-api --test phase2_isolation_tests

# 3. Pattern coverage — ALL route files (Phase 1 + Phase 2)
for f in agents.rs channels.rs config.rs skills.rs workflows.rs \
         memory.rs network.rs providers.rs budget.rs plugins.rs \
         goals.rs media.rs inbox.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  [ "$TOTAL" -eq "$SCOPED" ] || { echo "FAIL: $f: $SCOPED/$TOTAL"; exit 1; }
done

# system.rs non-public check
TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/system.rs")
PUBLIC=$(grep -c "// PUBLIC" "crates/librefang-api/src/routes/system.rs" || echo 0)
SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/system.rs" || echo 0)
EXPECTED=$((TOTAL - PUBLIC))
[ "$SCOPED" -ge "$EXPECTED" ] || { echo "FAIL: system.rs: $SCOPED/$EXPECTED"; exit 1; }

# === Phase 3: Data & Memory Isolation ===

# 4. Migration v19 tests
cargo test -p librefang-memory -- migration_v19

# 5. Memory store coverage
for f in structured.rs semantic.rs session.rs knowledge.rs usage.rs proactive.rs; do
  SCOPED=$(grep -c "account_id\|account:" "crates/librefang-memory/src/$f" || echo 0)
  [ "$SCOPED" -ge 2 ] || { echo "FAIL: $f: $SCOPED account references"; exit 1; }
done

# 6. Phase 3 memory isolation tests
cargo test -p librefang-api --test phase3_memory_tests

# === Phase 4: Hardening ===

# 7. HMAC constant-time check
grep -q "verify_slice\|constant_time" crates/librefang-api/src/middleware.rs || \
  { echo "FAIL: No constant-time HMAC"; exit 1; }

# 8. No 403 in cross-tenant paths
FORBIDDEN=$(grep -rn "StatusCode::FORBIDDEN" crates/librefang-api/src/routes/ | \
  grep -v "// INTENTIONAL" | wc -l)
[ "$FORBIDDEN" -eq 0 ] || echo "WARNING: $FORBIDDEN potential 403 returns — verify intent"

# 9. Workspace green
cargo build --workspace 2>&1 | grep -c "error\[" | xargs -I{} test {} -eq 0

echo "=== ALL EXIT CRITERIA PASS ==="
```

---

## Dependency Graph

```
PLAN-MT-001 (Phase 1) ──────────────────────────────────────────────────────┐
  AccountId type, AgentEntry.account_id, v18 migration, 76 handlers,       │
  extractors, macros, guard, HMAC                                          │
                                                                            │
═══════════════════════════════════════════════════════════════════════════  │
PHASE 2: Resource Isolation                                                 │
═══════════════════════════════════════════════════════════════════════════  │
                                                                            │
Round 1: Kernel spawn wiring ──────────────────────────────────────────┐    │
         (account propagation to spawn_agent callers)                  │    │
                │                                                      │    │
Round 2: Supporting crates ─────────────────────────────────────┐      │    │
         (integration registry, skill allowlist, filesystem)    │      │    │
                │                                               │      │    │
Round 3: Kernel wiring ────────────────────────────────┐        │      │    │
         (event bus, channel loading, spawn callers)   │        │      │    │
                │                                      │        │      │    │
Round 4: API handler sweep (~252 handlers) ────────────┘────────┘──────┘    │
         4a: Tier 1 (skills, workflows, goals, inbox, media — 97)          │
         4b: Tier 2 (providers, budget, plugins — 37)                      │
         4c: Tier 3 (network, memory, channels — 55)                       │
         4d: Tier 4 (system — ~53 scoped + ~10 public)                     │
                │                                                           │
Round 5: Integration tests (10 tests) ─────────────────────────────────────┘
                │
═══════════════════════════════════════════════════════════════════════════
PHASE 3: Data & Memory Isolation
═══════════════════════════════════════════════════════════════════════════
                │
Round 1: Migration v19 ────────────────────────────────────────────────┐
         (14 tables + indexes + FTS5 rebuild)                          │
                │                                                      │
Round 2: Memory store files ───────────────────────────────────────┐   │
         2a: semantic.rs (18 methods)                              │   │
         2b: session.rs (16 methods)                               │   │
         2c: structured.rs (8 methods)                             │   │
         2d: proactive.rs + knowledge.rs + usage.rs (34 methods)   │   │
                │                                                  │   │
Round 3: Kernel + vector store ────────────────────────────────────┘───┘
         (context engine, HttpVectorStore, Supabase RLS)
                │
Round 4: API memory routes + integration tests ────────────────────────┐
                │                                                      │
═══════════════════════════════════════════════════════════════════════════
PHASE 4: Hardening & Integration                                       │
═══════════════════════════════════════════════════════════════════════════
                │                                                      │
Round 1: Security audit ──────────────┐                                │
Round 2: Penetration testing ─────────┤                                │
Round 3: Performance + monitoring ────┤                                │
Round 4: Documentation ───────────────┘                                │
                │                                                      │
         ┌──── ALL PHASES COMPLETE ────────────────────────────────────┘
         │
    Multi-tenant production-ready
```

---

## Cross-Reference Index

| Document | Phase | Purpose |
|----------|-------|---------|
| ADR-MT-003 (Resource Isolation) | Phase 2 | Tiered scoping strategy, blast radius scan |
| ADR-MT-004 (Data & Memory Isolation) | Phase 3 | Schema changes, store-by-store impact |
| ADR-MT-005 (Event Bus Isolation) | Phase 2 | Event bus account tagging, cross-tenant event filtering |
| SPEC-MT-002 (API Route Changes) | Phase 2 | Handler-level spec, acceptance criteria, exit gate |
| SPEC-MT-003 (Database Migration) | Phase 3 | Migration SQL, store method spec, exit gate |
| SPEC-MT-004 (Supabase RLS) | Phase 3 | RLS policies, RPC changes, exit gate |
| SPEC-RV-002 (Supabase Vector Store) | Phase 3 | Vector store account awareness |
| PLAN-MT-001 (Phase 1) | Predecessor | Foundation this plan builds on |

---

## Anti-Patterns to Avoid

| Anti-Pattern | Mitigation in This Plan |
|-------------|------------------------|
| Instance-based gates | Pattern coverage gate counts ALL handlers mechanically |
| BHR as discovery | Pre-BHR checklist catches everything first |
| Prose exit criteria | Every gate is `bash` commands that exit 0 |
| Missing round gates | Every round (including 4a/4b/4c/4d) has clippy + test gate |
| Mixed-crate rounds | Strict dependency order: types -> memory -> kernel -> api |
| Wrong round order | Dependency graph, each round names prerequisites |
| Handlers before infrastructure | Kernel wiring (Rounds 1-3) complete before handler sweep (Round 4) |
| Test-last disguised as TDD | Micro-cycles: RED->GREEN per batch, REFACTOR between batches |
| Duplicate-column migration bugs | Column-existence guard in v19 SQL (same pattern as v18) |
| Phase 1 regression | Pattern coverage gate includes agents.rs + config.rs re-check |
| System endpoints broken | Tier 4 explicitly marks `// PUBLIC` handlers; gate verifies |
| Memory store partial scoping | Memory store coverage gate verifies all 6 files |
| RLS bypass undetected | Phase 4 pentest specifically targets RLS policies |
| Performance regression hidden | Phase 4 establishes baseline with explicit targets |
| 403 leaking account existence | All cross-tenant returns 404; audit grep confirms no 403 |
