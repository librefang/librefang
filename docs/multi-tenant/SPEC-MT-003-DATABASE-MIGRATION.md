# SPEC-MT-003: Database Migration — Phase 3 (v2: post-BHR)

**ADR:** ADR-MT-004 (Data & Memory Isolation)
**Date:** 2026-04-06 (v2: post-BHR)
**Author:** Engineering

---

## BHR Fixes Applied (v2)

| # | Issue | Severity | Fix |
|---|-------|----------|-----|
| 1 | Table count said 17, actual 19 | CRITICAL | Corrected with full enumeration from migration.rs |
| 2 | sessions column "content" → actual is "messages" | CRITICAL | Fixed column names from verified CREATE TABLE |
| 3 | Method counts wrong (40/15/12) | CRITICAL | Corrected to verified counts (11/24/22/6/17/26) |

---

## Purpose

Add `account_id` columns to all 14 remaining tenant-visible SQLite tables in a
single migration (v19), wire account filtering through 6 memory store files
(~76 account-sensitive methods), and rebuild the FTS5 virtual table with account
awareness.

## Scope (from ADR-MT-004 Blast Radius Scan — verified counts)

### Tables to Migrate (14 of 19 total)

| # | Table | Key Columns (from migration.rs) | Phase 3 Change |
|---|-------|---------------------------------|----------------|
| 1 | agents | id, name, manifest, state, mode | ✅ Done (v18) |
| 2 | sessions | id, agent_id, messages (BLOB), context_window_tokens, created_at, updated_at (+peer_id v16, +label v6) | ADD COLUMN + INDEX |
| 3 | events | id, agent_id, event_type, payload, created_at | ADD COLUMN + INDEX |
| 4 | kv_store | agent_id, key, value, updated_at | ADD COLUMN + INDEX |
| 5 | memories | id, agent_id, content, embedding, level, source, created_at (+image_url v15, +modality v15) | ADD COLUMN + INDEX |
| 6 | entities | id, name, entity_type, description | ADD COLUMN |
| 7 | relations | id, subject_id, predicate, object_id, weight | ADD COLUMN |
| 8 | task_queue | id, agent_id, payload, status, created_at | ADD COLUMN |
| 9 | migrations | version, applied_at | — (system table) |
| 10 | usage_events | id, agent_id, event_type, tokens_in, tokens_out, cost | ADD COLUMN + INDEX |
| 11 | canonical_sessions | id, agent_id, created_at | ADD COLUMN |
| 12 | paired_devices | id, device_name, peer_id, created_at | ADD COLUMN |
| 13 | audit_entries | id, hash, prev_hash, agent_id, action, payload | ADD COLUMN + INDEX |
| 14 | sessions_fts | session_id, content (FTS5 virtual) | DROP + RECREATE with account_id |
| 15 | prompt_versions | id, agent_id, version, content, is_active | ADD COLUMN |
| 16 | prompt_experiments | id, agent_id, name, status | ADD COLUMN |
| 17 | experiment_variants | id, experiment_id, name, config | — (FK-scoped) |
| 18 | experiment_metrics | id, variant_id, metric_name, value | — (FK-scoped) |
| 19 | approval_audit | id, agent_id, action, decision | ADD COLUMN |

**Summary: 14 tables get account_id, 1 FTS5 rebuilt, 2 FK-scoped, 2 unchanged**

### Memory Store Methods to Modify (verified counts)

| Store | File | Total Methods | Account-sensitive | Exempt |
|-------|------|--------------|-------------------|--------|
| StructuredStore | structured.rs | 11 | ~8 | ~3 (open, migrate) |
| SemanticStore | semantic.rs | 24 | ~18 | ~6 (init, helpers) |
| SessionStore | session.rs | 22 | ~16 | ~6 (internal) |
| KnowledgeStore | knowledge.rs | 6 | ~4 | ~2 (init) |
| UsageStore | usage.rs | 17 | ~12 | ~5 (init, internal) |
| ProactiveMemoryStore | proactive.rs | 26 | ~18 | ~8 (config, embedding) |
| **Total** | | **106** | **~76** | **~30** |

## Acceptance Criteria

### AC-1: Migration v19 adds account_id to all 14 tables
- **Given:** A database at schema v18 (post-Phase 1)
- **When:** Migration v19 runs
- **Then:** All 14 tables have `account_id TEXT NOT NULL DEFAULT 'system'`
- **And NOT:** Any table missing account_id column

### AC-2: Migration is idempotent
- **Given:** Database already at v19
- **When:** Migration v19 runs again
- **Then:** No error, no duplicate columns
- **And NOT:** "duplicate column name" error (guard with column-existence check)

### AC-3: Existing data gets system account
- **Given:** Pre-migration sessions, events, memories exist
- **When:** Migration v19 runs
- **Then:** All existing rows have `account_id = 'system'`
- **And NOT:** Any row with NULL account_id

### AC-4: FTS5 rebuilt with account_id
- **Given:** sessions_fts exists from v12
- **When:** Migration v19 runs
- **Then:** sessions_fts recreated with account_id column, populated from sessions.messages
- **And NOT:** Empty FTS table after rebuild (row count matches sessions table)

### AC-5: StructuredStore filters by account
- **Given:** Agent-1 (account-A), Agent-2 (account-B) both have KV entries
- **When:** `list_kv(agent_id, "account-A")` called
- **Then:** Only account-A's entries returned
- **And NOT:** Account-B's entries visible

### AC-6: SemanticStore recall filters by account
- **Given:** Memory "secret-A" (account-A), "secret-B" (account-B)
- **When:** Semantic recall with account-A filter
- **Then:** Only "secret-A" in results (+ system memories)
- **And NOT:** "secret-B" leaked to account-A

### AC-7: System account sees all (backward compat)
- **Given:** Data from accounts A, B, and system
- **When:** Query with `account_id = "system"` or `AccountId(None)`
- **Then:** All data returned (legacy/desktop mode)
- **And NOT:** System-account queries filtered to only system-owned data

### AC-8: Session lifecycle scoped
- **Given:** Session created by agent owned by account-A
- **When:** Account-B tries to list/compact/delete that session
- **Then:** Session not visible / operation fails
- **And NOT:** Cross-account session access succeeds

### AC-9: Audit trail integrity per account
- **Given:** Audit entries with Merkle hash chain
- **When:** account_id added to audit_entries
- **Then:** Existing chain integrity preserved (all entries get 'system')
- **And NOT:** Merkle chain broken by migration (prev_hash still valid)

### AC-10: Usage events scoped for billing
- **Given:** Usage events from accounts A and B
- **When:** Billing query for account-A
- **Then:** Only account-A's token usage / costs returned
- **And NOT:** Account-B's usage counted in account-A's bill

## Claims Requiring Verification

| Claim | Verification | Test Name |
|-------|-------------|-----------|
| All 14 tables have account_id post-v19 | Migration test | `test_migration_v19_all_tables` |
| Migration idempotent | Migration test | `test_migration_v19_idempotent` |
| Existing data = 'system' | Migration test | `test_migration_v19_default_system` |
| FTS5 rebuilt with data | Migration test | `test_migration_v19_fts5_rebuilt` |
| KV filtered by account | Unit test | `test_kv_filtered_by_account` |
| Recall filtered by account | Unit test | `test_recall_filtered_by_account` |
| System account sees all | Unit test | `test_system_sees_all` |
| Session lifecycle scoped | Unit test | `test_session_scoped` |
| Merkle chain preserved | Unit test | `test_audit_merkle_preserved` |
| Usage scoped for billing | Unit test | `test_usage_scoped_billing` |

## Storage Dependency Check (MANDATORY per SPEC-writer skill)

| ADR Claims | Actual API | Match? |
|-----------|------------|--------|
| "ALTER TABLE ADD COLUMN" | rusqlite `conn.execute()` | ✅ Yes — standard SQLite DDL |
| "column-existence guard" | `PRAGMA table_info(table)` | ✅ Yes — but need Rust wrapper |
| "FTS5 DROP + CREATE" | rusqlite supports DDL on virtual tables | ✅ Yes |
| "DEFAULT 'system'" | SQLite DEFAULT clause | ✅ Yes — applied to existing rows |
| "CREATE INDEX IF NOT EXISTS" | Standard SQLite | ✅ Yes — idempotent |

## Exit Gate

```bash
#!/bin/bash
set -e

# 1. Migration compiles and passes
cargo clippy -p librefang-memory --all-targets -- -D warnings
cargo test -p librefang-memory

# 2. All 14 tables verified
cargo test -p librefang-memory -- migration_v19

# 3. Method count verification — every store file has account references
for f in structured.rs semantic.rs session.rs knowledge.rs usage.rs proactive.rs; do
  TOTAL=$(grep -c "pub fn\|pub async fn" "crates/librefang-memory/src/$f")
  SCOPED=$(grep -c "account_id\|account:" "crates/librefang-memory/src/$f" || echo 0)
  echo "$f: $SCOPED account references in $TOTAL methods"
done

# 4. Full workspace green
cargo test --workspace

echo "SPEC-MT-003 EXIT GATE: ALL PASS"
```

## Rollback Strategy

```sql
-- SQLite >= 3.35.0: DROP COLUMN supported
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

-- SQLite < 3.35.0: table rebuild strategy per table (same as Phase 1)
```

## Out of Scope

| Excluded | Reason | When |
|----------|--------|------|
| Vector store account namespacing | Supabase RLS handles this — no librefang code change | Phase 3 (automatic) |
| Cross-account data migration tooling | Admin feature, not core isolation | Phase 4 |
| Per-account Merkle chain restart | Audit hardening | Phase 4 |
| Memory garbage collection per account | Optimization | Future |
| ProactiveMemoryStore embedding model per account | All accounts share embedding model | Future |
