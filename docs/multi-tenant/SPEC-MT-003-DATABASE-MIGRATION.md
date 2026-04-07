# SPEC-MT-003: Database Migration — Phase 3

**ADR:** ADR-MT-004 (Data & Memory Isolation)
**Date:** 2026-04-06
**Author:** Engineering

---

## Purpose

Add `account_id` columns to all 14 remaining tenant-visible SQLite tables in a
single migration (v19), wire account filtering through the memory store layer,
and rebuild the FTS5 virtual table with account awareness.

## Scope (from ADR-MT-004 Blast Radius Scan)

### Tables to Migrate

| Table | Current Cols (key) | Phase 1 Status | Phase 3 Change |
|-------|--------------------|----------------|----------------|
| agents | id, name, manifest, state, mode | ✅ account_id added (v18) | — |
| sessions | id, agent_id, content, peer_id, label | ❌ No account_id | ADD COLUMN + INDEX |
| events | id, agent_id, type, payload | ❌ No account_id | ADD COLUMN + INDEX |
| kv_store | agent_id, key, value | ❌ No account_id | ADD COLUMN + INDEX |
| memories | id, agent_id, content, embedding | ❌ No account_id | ADD COLUMN + INDEX |
| entities | id, name, type | ❌ No account_id | ADD COLUMN |
| relations | id, subject, predicate, object | ❌ No account_id | ADD COLUMN |
| usage_events | id, agent_id, tokens, cost | ❌ No account_id | ADD COLUMN + INDEX |
| task_queue | id, agent_id, payload, status | ❌ No account_id | ADD COLUMN |
| canonical_sessions | id, agent_id | ❌ No account_id | ADD COLUMN |
| paired_devices | id, device_name, peer_id | ❌ No account_id | ADD COLUMN |
| audit_entries | id, hash, prev_hash, payload | ❌ No account_id | ADD COLUMN + INDEX |
| prompt_versions | id, agent_id, version, content | ❌ No account_id | ADD COLUMN |
| prompt_experiments | id, agent_id, name | ❌ No account_id | ADD COLUMN |
| approval_audit | id, agent_id, action | ❌ No account_id | ADD COLUMN |
| sessions_fts | session_id, content (FTS5 virtual) | ❌ No account_id | DROP + RECREATE |
| experiment_variants | id, experiment_id | Scoped via FK | — |
| experiment_metrics | id, variant_id | Scoped via FK | — |
| migrations | version, applied_at | System table | — |

**Total: 14 tables get account_id, 1 FTS5 rebuilt, 3 unchanged**

### Memory Store Methods to Modify

| Store | File | Methods (est.) | Change |
|-------|------|---------------|--------|
| StructuredStore | `structured.rs` | ~40 | Add `account_id: &str` param, WHERE clause |
| SemanticStore | `semantic.rs` | ~15 | Add account filter to recall/store |
| SessionStore | `session.rs` | ~12 | Add account filter to session lifecycle |
| KnowledgeStore | `knowledge.rs` | ~8 | Add account filter to entity/relation queries |
| UsageStore | `usage.rs` | ~5 | Add account filter to usage tracking |
| ProactiveMemoryStore | `proactive.rs` | ~10 | Thread account through add/search/list |

**Total: ~90 methods across 6 store files**

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
- **Then:** sessions_fts recreated with account_id column, populated from sessions table
- **And NOT:** Empty FTS table after rebuild

### AC-5: StructuredStore filters by account
- **Given:** Agent-1 (account-A), Agent-2 (account-B) both have KV entries
- **When:** `list_kv(agent_id, "account-A")` called
- **Then:** Only account-A's entries returned
- **And NOT:** Account-B's entries visible

### AC-6: SemanticStore recall filters by account
- **Given:** Memory "secret-A" (account-A), "secret-B" (account-B)
- **When:** Semantic recall with account-A
- **Then:** Only "secret-A" in results (+ system memories)
- **And NOT:** "secret-B" leaked to account-A

### AC-7: System account sees all (backward compat)
- **Given:** Data from accounts A, B, and system
- **When:** Query with `account_id = "system"` or `AccountId(None)`
- **Then:** All data returned (legacy/desktop mode)
- **And NOT:** System-account queries filtered

### AC-8: Session lifecycle scoped
- **Given:** Session created by agent owned by account-A
- **When:** Account-B tries to list/compact/delete that session
- **Then:** Session not visible / 404
- **And NOT:** Cross-account session access

### AC-9: Audit trail integrity per account
- **Given:** Audit entries with Merkle chain
- **When:** account_id added to audit_entries
- **Then:** Existing chain integrity preserved (all entries get 'system')
- **And NOT:** Merkle chain broken by migration

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

## Exit Gate

```bash
#!/bin/bash
set -e

# 1. Migration compiles and passes
cargo clippy -p librefang-memory --all-targets -- -D warnings
cargo test -p librefang-memory

# 2. All 14 tables verified (requires test DB)
cargo test -p librefang-memory -- migration_v19

# 3. Memory store methods accept account_id
# Pattern check: every pub fn in store files should have account_id param
for f in structured.rs semantic.rs session.rs knowledge.rs usage.rs; do
  TOTAL=$(grep -c "pub fn\|pub async fn" "crates/librefang-memory/src/$f")
  # Exempt: new(), open(), internal helpers
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
-- ... repeat for all 14 tables ...

-- FTS5: rebuild without account_id
DROP TABLE IF EXISTS sessions_fts;
CREATE VIRTUAL TABLE sessions_fts USING fts5(
  session_id, content,
  content='sessions', content_rowid='rowid'
);

-- SQLite < 3.35.0: table rebuild strategy (same as Phase 1 rollback)
```

## Out of Scope

- Vector store account namespacing (Supabase RLS handles this — no code change)
- Cross-account data migration tooling (Phase 4)
- Per-account Merkle chain restart (Phase 4 audit hardening)
- Memory garbage collection per account (future optimization)
