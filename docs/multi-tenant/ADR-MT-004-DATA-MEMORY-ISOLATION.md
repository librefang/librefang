# ADR-MT-004: Data & Memory Isolation

**Status:** Proposed
**Date:** 2026-04-06
**Author:** Engineering
**Related:** ADR-MT-001, ADR-MT-003, SPEC-MT-001
**Epic:** Multi-Tenant Architecture — Phase 3

---

## Problem Statement

Phase 1 adds `account_id` to the `agents` table. Phase 2 scopes API handlers.
But 16 other SQLite tables still have no tenant isolation — any query against
sessions, events, kv_store, memories, entities, relations, prompt_versions, etc.
returns data from ALL accounts.

Additionally, the Supabase vector store (SPEC-RV-002) currently has no
account-scoped namespacing — all embeddings live in a single collection.

Phase 3 must add `account_id` columns to all tenant-visible tables AND wire
memory recall/insert to filter by account.

## Blast Radius Scan

```bash
# All 17 SQLite tables (from migration.rs, schema v17):
# Core:
#   agents              ← account_id added in Phase 1 (v18)
#   sessions            ← NEEDS account_id
#   events              ← NEEDS account_id
#   kv_store            ← NEEDS account_id
#   memories            ← NEEDS account_id
#   entities            ← NEEDS account_id
#   relations           ← NEEDS account_id
#   migrations          ← system table, no scoping needed
#
# Usage:
#   usage_events        ← NEEDS account_id (for per-tenant billing)
#   task_queue          ← NEEDS account_id
#
# Sessions:
#   canonical_sessions  ← NEEDS account_id
#
# Devices:
#   paired_devices      ← NEEDS account_id
#
# Audit:
#   audit_entries       ← NEEDS account_id (Merkle chain per account)
#
# Search:
#   sessions_fts        ← FTS5 virtual table — cannot ADD COLUMN, rebuild needed
#
# Prompts:
#   prompt_versions     ← NEEDS account_id
#   prompt_experiments  ← NEEDS account_id
#   experiment_variants ← scoped via prompt_experiments FK
#   experiment_metrics  ← scoped via experiment_variants FK
#
# Approval:
#   approval_audit      ← NEEDS account_id

# Tables needing account_id: 14 (of 17)
# Tables already scoped:     1  (agents, Phase 1)
# System tables (no scope):  2  (migrations, sessions_fts rebuild)

# Memory layer touchpoints:
$ grep -c "fn.*(&self" crates/librefang-memory/src/structured.rs
# ~40 methods that query SQLite — all need account_id filtering

$ grep -c "fn.*(&self" crates/librefang-memory/src/semantic.rs
# ~15 methods — recall and store operations need account filtering

$ grep -c "fn.*(&self" crates/librefang-memory/src/session.rs
# ~12 methods — session lifecycle needs account scoping
```

**Scope decision:** Add `account_id TEXT NOT NULL DEFAULT 'system'` to all 14
tables in a single migration (v19). Wire filtering through StructuredStore,
SemanticStore, and SessionStore. FTS5 table requires rebuild (DROP + CREATE).

## Decision

### Migration v19: Batch column addition

All 14 tables get the same migration pattern:
```sql
-- v19: Multi-tenant data isolation
-- Each ALTER guarded against duplicate-column errors (learned from Phase 1)

-- Core tables
ALTER TABLE sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE events ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE kv_store ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE memories ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE entities ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE relations ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Usage & task
ALTER TABLE usage_events ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE task_queue ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Sessions & devices
ALTER TABLE canonical_sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
ALTER TABLE paired_devices ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';

-- Audit (Merkle chain restarts per account)
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
-- Repopulate from sessions table
INSERT INTO sessions_fts(session_id, content, account_id)
  SELECT id, content, account_id FROM sessions;
```

### Memory store filtering strategy

Every `StructuredStore`, `SemanticStore`, and `SessionStore` method that queries
SQLite gets an `account_id` parameter:

```rust
// BEFORE:
pub fn list_sessions(&self, agent_id: &str) -> Vec<Session>

// AFTER:
pub fn list_sessions(&self, agent_id: &str, account_id: &str) -> Vec<Session>
// SQL: WHERE agent_id = ? AND account_id = ?
```

For backward compatibility, "system" account sees all:
```rust
if account_id == AccountId::SYSTEM {
    // No account_id filter — legacy mode
} else {
    // WHERE account_id = ?
}
```

### Supabase vector store scoping

The `SupabaseVectorStore` (SPEC-RV-002) gains account awareness in Phase 3:
- RLS policies on the vectors table filter by `account_id` claim in JWT
- Insert operations tag vectors with the requesting account's ID
- Search operations automatically filtered by RLS — zero code change in librefang

## Verification Gate

```bash
# Gate: all 14 tables have account_id column
for table in sessions events kv_store memories entities relations \
             usage_events task_queue canonical_sessions paired_devices \
             audit_entries prompt_versions prompt_experiments approval_audit; do
  sqlite3 test.db "PRAGMA table_info($table)" | grep -q "account_id" || \
    { echo "FAIL: $table missing account_id"; exit 1; }
done
echo "All 14 tables have account_id"
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
- **Negative:** ~67 memory store methods need account_id parameter. FTS5 rebuild requires temporary search downtime.
- **Phase 4 debt:** Audit trail per-account Merkle chains. Cross-account data migration tooling.
