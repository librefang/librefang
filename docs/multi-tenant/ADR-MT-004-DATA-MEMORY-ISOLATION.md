# ADR-MT-004: Data & Memory Isolation

**Status:** Proposed
**Date:** 2026-04-06 (v2: post-BHR)
**Author:** Engineering
**Related:** ADR-MT-001, ADR-MT-003, SPEC-MT-001
**Epic:** Multi-Tenant Architecture — Phase 3

---

## BHR Fixes Applied (v2)

| # | Issue | Severity | Fix |
|---|-------|----------|-----|
| 1 | Table count said 17, actual 19 | CRITICAL | Corrected to 19 with full enumeration |
| 2 | structured.rs said ~40 methods, actual 11 | CRITICAL | All 6 store files counted with exact numbers |
| 3 | semantic.rs said ~15, actual 24 | CRITICAL | Corrected |
| 4 | session.rs said ~12, actual 22 | CRITICAL | Corrected |

---

## Problem Statement

Phase 1 adds `account_id` to the `agents` table. Phase 2 scopes API handlers.
But 18 other SQLite tables still have no tenant isolation — any query against
sessions, events, kv_store, memories, entities, relations, prompt_versions, etc.
returns data from ALL accounts.

Additionally, the Supabase vector store (SPEC-RV-002) currently has no
account-scoped namespacing — all embeddings live in a single collection.

Phase 3 must add `account_id` columns to all tenant-visible tables AND wire
memory recall/insert to filter by account.

## Blast Radius Scan

```bash
# All 19 SQLite tables/objects (from migration.rs, verified 2026-04-06):
$ grep -c "CREATE TABLE\|CREATE VIRTUAL TABLE" crates/librefang-memory/src/migration.rs
# 19

# Full enumeration:
#  1. agents              ← account_id added in Phase 1 (v18)
#  2. sessions            ← NEEDS account_id
#  3. events              ← NEEDS account_id
#  4. kv_store            ← NEEDS account_id
#  5. task_queue          ← NEEDS account_id
#  6. memories            ← NEEDS account_id
#  7. entities            ← NEEDS account_id
#  8. relations           ← NEEDS account_id
#  9. migrations          ← system table, no scoping needed
# 10. usage_events        ← NEEDS account_id (per-tenant billing)
# 11. canonical_sessions  ← NEEDS account_id
# 12. paired_devices      ← NEEDS account_id
# 13. audit_entries       ← NEEDS account_id (Merkle chain per account)
# 14. sessions_fts        ← FTS5 virtual — cannot ALTER, must DROP+RECREATE
# 15. prompt_versions     ← NEEDS account_id
# 16. prompt_experiments  ← NEEDS account_id
# 17. experiment_variants ← scoped via prompt_experiments FK — no column needed
# 18. experiment_metrics  ← scoped via experiment_variants FK — no column needed
# 19. approval_audit      ← NEEDS account_id
#
# Tables needing account_id: 14 (of 19)
# Already scoped:            1  (agents — Phase 1)
# FK-scoped (no change):     2  (experiment_variants, experiment_metrics)
# System tables (no scope):  2  (migrations, sessions_fts rebuild-only)

# Memory store method counts (verified 2026-04-06):
$ grep -c "pub fn\|pub async fn" crates/librefang-memory/src/{structured,semantic,session,knowledge,usage,proactive}.rs
# structured.rs:  11 methods
# semantic.rs:    24 methods
# session.rs:     22 methods
# knowledge.rs:    6 methods
# usage.rs:       17 methods
# proactive.rs:   26 methods
# TOTAL: 106 methods across 6 store files
```

**Scope decision:** Add `account_id TEXT NOT NULL DEFAULT 'system'` to 14 tables
in a single migration (v19). Wire filtering through all 6 store files (~106 methods).
FTS5 table requires rebuild (DROP + CREATE).

## Decision

### Migration v19: Batch column addition

All 14 tables get the same migration pattern:
```sql
-- v19: Multi-tenant data isolation
-- Each ALTER guarded against duplicate-column errors

-- Core tables (from v1)
ALTER TABLE sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE events ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE kv_store ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE memories ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE entities ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE relations ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE task_queue ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Usage & billing (from v4)
ALTER TABLE usage_events ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Sessions & devices (from v5-7)
ALTER TABLE canonical_sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE paired_devices ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Audit (from v8 — Merkle chain)
ALTER TABLE audit_entries ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Prompts (from v13)
ALTER TABLE prompt_versions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE prompt_experiments ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Approval (from v17)
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

### Memory store filtering strategy

Every store method that queries SQLite gets an `account_id` parameter:

```rust
// BEFORE:
pub fn list_sessions(&self, agent_id: &str) -> Vec<Session>

// AFTER:
pub fn list_sessions(&self, agent_id: &str, account_id: &str) -> Vec<Session>
// SQL: WHERE agent_id = ? AND account_id = ?
```

Backward compatibility — "system" account sees all:
```rust
if account_id == AccountId::SYSTEM {
    // No account_id filter — legacy mode
} else {
    // WHERE account_id = ?
}
```

### Store-by-store impact

| Store | File | Methods | Account-sensitive | Exempt (internal) |
|-------|------|---------|-------------------|-------------------|
| StructuredStore | structured.rs | 11 | ~8 (CRUD on agents, KV, tasks) | ~3 (open, migrate, internal) |
| SemanticStore | semantic.rs | 24 | ~18 (recall, store, search) | ~6 (init, config, helpers) |
| SessionStore | session.rs | 22 | ~16 (list, get, compact, delete) | ~6 (internal management) |
| KnowledgeStore | knowledge.rs | 6 | ~4 (entity/relation CRUD) | ~2 (init) |
| UsageStore | usage.rs | 17 | ~12 (log, query, aggregate) | ~5 (init, internal) |
| ProactiveMemoryStore | proactive.rs | 26 | ~18 (add, search, list, hooks) | ~8 (config, embedding) |
| **Total** | | **106** | **~76 need account_id** | **~30 exempt** |

### Supabase vector store scoping

The `SupabaseVectorStore` (SPEC-RV-002) gains account awareness in Phase 3:
- RLS policies on the vectors table filter by `account_id` claim in JWT
- Insert operations tag vectors with the requesting account's ID
- Search operations automatically filtered by RLS — zero code change in librefang

## Verification Gate

```bash
# Gate: all 14 tables have account_id column after v19
for table in sessions events kv_store memories entities relations \
             task_queue usage_events canonical_sessions paired_devices \
             audit_entries prompt_versions prompt_experiments approval_audit; do
  sqlite3 test.db "PRAGMA table_info($table)" | grep -q "account_id" || \
    { echo "FAIL: $table missing account_id"; exit 1; }
done
echo "All 14 tables have account_id"

# Gate: method count verification
for f in structured.rs semantic.rs session.rs knowledge.rs usage.rs proactive.rs; do
  TOTAL=$(grep -c "pub fn\|pub async fn" "crates/librefang-memory/src/$f")
  SCOPED=$(grep -c "account_id\|account:" "crates/librefang-memory/src/$f" || echo 0)
  echo "$f: $SCOPED account references in $TOTAL methods"
done
```

## Alternatives Considered

### Alt 1: Separate SQLite file per account
**Rejected.** SQLite doesn't support cross-database JOINs well. Would require
a database-per-tenant architecture that breaks the shared-daemon model.

### Alt 2: Add account_id incrementally (per table as needed)
**Rejected.** Leads to inconsistent state where some tables are scoped and others
aren't. Batch migration is safer — one version bump, one rollback point.

### Alt 3: Row-level security in SQLite (via triggers)
**Rejected.** SQLite has no native RLS. Trigger-based enforcement is fragile and
bypassed by direct SQL access. Application-level filtering is the correct approach.

## Consequences

- **Positive:** Complete tenant isolation at the data layer. Supabase RLS handles vector isolation.
- **Negative:** ~76 store methods need account_id parameter. FTS5 rebuild requires temporary search downtime.
- **Phase 4 debt:** Audit trail per-account Merkle chains. Cross-account data migration tooling.
