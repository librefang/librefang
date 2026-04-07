# SPEC-MT-004: Supabase Row-Level Security Policies — Phase 3

**ADR:** ADR-MT-004 (Data & Memory Isolation)
**Date:** 2026-04-06
**Author:** Engineering
**Epic:** Multi-Tenant Architecture — Phase 3

---

## Purpose

Define the exact Supabase PostgreSQL Row-Level Security (RLS) policies,
account-scoped RPC function signatures, and service-role bypass rules that
enforce tenant isolation on the server side. This SPEC covers the Supabase
component of ADR-MT-004; SPEC-MT-003 covers the SQLite side.

RLS is the primary isolation mechanism for all Supabase-hosted tables. Every
authenticated request is filtered by `account_id` resolved through the
`user_agents` join table against `auth.uid()`. Application-level filtering in
`HttpVectorStore` provides defense-in-depth but is NOT the enforcement layer.

## Source of Truth

| Source | What it proves |
|--------|---------------|
| qwntik `20260405_ruvector_setup.sql` | `documents` table schema: `user_id UUID`, `embedding ruvector(384)`, HNSW index, 4 user-based RLS policies |
| qwntik `20260412000200_documents_rls_account_scoping.sql` | Account-scoped RLS replacing user-based policies via `user_agents` join |
| qwntik `20260412000250_documents_backfill_account_id.sql` | Backfill: `UPDATE documents SET account_id = user_id WHERE account_id IS NULL` |
| qwntik `20260413_vector_rpc_account_id.sql` | RPC functions with `doc_account_id` / `caller_account_id` params (NULL defaults) |
| librefang `HttpVectorStore` | HTTP client at `librefang-memory/src/http_vector_store.rs` (231 lines) |
| librefang config | `vector_backend` + `vector_store_url` at `config/types.rs:3157` |
| ADR-MT-004 | "RLS policies on the vectors table filter by account_id" (note: ADR says "claim in JWT" but actual mechanism uses `user_agents` join on `auth.uid()`, not a JWT custom claim) |
| SPEC-RV-002 | SupabaseVectorStore HTTP client spec (this SPEC covers server-side) |
| SPEC-MT-003 | SQLite-side isolation (14 tables, ~76 account-sensitive methods of 106 total) |

## Scope (from ADR-MT-004 Blast Radius — Supabase component)

### Phase 1: User-Based RLS (deployed, being replaced)

Migration `20260405_ruvector_setup.sql` created 4 user-scoped policies on
`documents`:

| Policy | Operation | USING / WITH CHECK |
|--------|-----------|-------------------|
| `documents_select_own` | SELECT | `user_id = auth.uid()` |
| `documents_insert_own` | INSERT | `user_id = auth.uid()` |
| `documents_update_own` | UPDATE | `user_id = auth.uid()` |
| `documents_delete_own` | DELETE | `user_id = auth.uid()` |

These are **dropped** by the account-scoping migration.

### Phase 2: Account-Based RLS (target state)

Migration `20260412000200_documents_rls_account_scoping.sql` replaces
user-based with account-based policies. The `account_id` column is
**nullable** `UUID REFERENCES public.accounts(id) ON DELETE CASCADE` --
rows with NULL `account_id` are invisible to authenticated users (NULL
is never IN a set) but visible to `service_role`:

| Policy | Operation | USING / WITH CHECK | Join |
|--------|-----------|-------------------|------|
| `documents_select` | SELECT | `account_id IN (SELECT account_id FROM public.user_agents WHERE user_id = auth.uid())` | `user_agents` |
| `documents_insert` | INSERT | `account_id IN (SELECT account_id FROM public.user_agents WHERE user_id = auth.uid())` | `user_agents` |
| `documents_delete` | DELETE | `account_id IN (SELECT account_id FROM public.user_agents WHERE user_id = auth.uid())` | `user_agents` |
| `documents_service_role` | ALL | `true` (service role only) | none |

### Tables Requiring Account-Scoped RLS

| # | Table | Current RLS | Target RLS | Policy Pattern |
|---|-------|-------------|------------|----------------|
| 1 | `documents` | User-based (4 policies) | Account-based (3 + service_role) | `user_agents` join |
| 2 | `agent_kv` | None | Account-based (SELECT, INSERT, UPDATE, DELETE + service_role) | `user_agents` join |
| 3 | `sessions` | None | Account-based (SELECT, INSERT, UPDATE, DELETE + service_role) | `user_agents` join |
| 4 | `usage_log` | None | Account-based (SELECT, INSERT + service_role) | `user_agents` join |
| 5 | `skill_versions` | None | Account-based (SELECT, INSERT, UPDATE, DELETE + service_role) | `user_agents` join |
| 6 | `session_labels` | None | Account-based (SELECT, INSERT, DELETE + service_role) | `user_agents` join |
| 7 | `api_audit_logs` | None | Account-based (SELECT, INSERT + service_role) | `user_agents` join |

### RPC Functions with account_id Parameters

Migration `20260413_vector_rpc_account_id.sql` adds account awareness:

| Function | New Parameters | Default | Behavior |
|----------|---------------|---------|----------|
| `vector_insert(doc_content TEXT, doc_embedding TEXT, doc_metadata JSONB, doc_user_id UUID, doc_account_id UUID)` | `doc_account_id UUID` | `NULL` | Stores account_id on document row; returns `BIGINT` (new row id) |
| `vector_insert_batch(doc_contents TEXT[], doc_embeddings TEXT[], doc_metadatas JSONB[], doc_user_id UUID, doc_account_id UUID)` | `doc_account_id UUID` | `NULL` | Batch insert with account_id; returns `BIGINT[]` |
| `vector_search(query_embedding TEXT, match_count INT, match_threshold REAL, caller_user_id UUID, caller_account_id UUID)` | `caller_account_id UUID` | `NULL` | Defense-in-depth: filters results by account_id in addition to RLS; returns `TABLE(id BIGINT, content TEXT, metadata JSONB, distance REAL)` |

Old overloads are dropped to prevent ambiguous function resolution:
`vector_search(text, integer, real, uuid)`, `vector_insert(text, text, jsonb, uuid)`,
`vector_insert_batch(text[], text[], jsonb[], uuid)`.

## RLS Policy SQL (exact from qwntik migrations)

### documents table — account-scoped policies

```sql
-- Entire migration guarded: only runs if documents table exists
DO $$
BEGIN
  IF EXISTS (SELECT 1 FROM information_schema.tables
             WHERE table_schema = 'public' AND table_name = 'documents') THEN

    -- Add account_id column (nullable for backfill, FK to accounts)
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns
                   WHERE table_schema = 'public' AND table_name = 'documents'
                   AND column_name = 'account_id') THEN
      ALTER TABLE public.documents ADD COLUMN account_id UUID
        REFERENCES public.accounts(id) ON DELETE CASCADE;
      CREATE INDEX IF NOT EXISTS idx_documents_account_id
        ON public.documents(account_id);
    END IF;

    -- Enable RLS
    ALTER TABLE public.documents ENABLE ROW LEVEL SECURITY;

    -- Revoke overly broad grants
    REVOKE ALL ON TABLE public.documents FROM authenticated;
    GRANT SELECT, INSERT, DELETE ON TABLE public.documents TO authenticated;
    GRANT ALL ON TABLE public.documents TO service_role;

    -- Drop old user-based policies
    DROP POLICY IF EXISTS "documents_select_own" ON public.documents;
    DROP POLICY IF EXISTS "documents_insert_own" ON public.documents;
    DROP POLICY IF EXISTS "documents_update_own" ON public.documents;
    DROP POLICY IF EXISTS "documents_delete_own" ON public.documents;

    -- Account-scoped SELECT
    DROP POLICY IF EXISTS "documents_select" ON public.documents;
    CREATE POLICY "documents_select" ON public.documents
      FOR SELECT TO authenticated
      USING (
        account_id IN (
          SELECT account_id FROM public.user_agents
          WHERE user_id = auth.uid()
        )
      );

    -- Account-scoped INSERT
    DROP POLICY IF EXISTS "documents_insert" ON public.documents;
    CREATE POLICY "documents_insert" ON public.documents
      FOR INSERT TO authenticated
      WITH CHECK (
        account_id IN (
          SELECT account_id FROM public.user_agents
          WHERE user_id = auth.uid()
        )
      );

    -- Account-scoped DELETE
    DROP POLICY IF EXISTS "documents_delete" ON public.documents;
    CREATE POLICY "documents_delete" ON public.documents
      FOR DELETE TO authenticated
      USING (
        account_id IN (
          SELECT account_id FROM public.user_agents
          WHERE user_id = auth.uid()
        )
      );

    -- Service role full access (no WITH CHECK — Supabase convention)
    DROP POLICY IF EXISTS "documents_service_role" ON public.documents;
    CREATE POLICY "documents_service_role" ON public.documents
      FOR ALL TO service_role
      USING (true);

  END IF;
END $$;
```

**Note:** A separate backfill migration (`20260412000250_documents_backfill_account_id.sql`)
populates `account_id` for existing rows using the personal-account pattern:
`UPDATE documents SET account_id = user_id WHERE account_id IS NULL`.

### Pattern for remaining tables

Each tenant-visible table follows the same pattern:

```sql
-- Enable RLS (idempotent)
ALTER TABLE public.<table> ENABLE ROW LEVEL SECURITY;

-- Authenticated users: scoped to their accounts via user_agents join
CREATE POLICY "<table>_select" ON public.<table>
  FOR SELECT TO authenticated
  USING (
    account_id IN (
      SELECT account_id FROM public.user_agents
      WHERE user_id = auth.uid()
    )
  );

CREATE POLICY "<table>_insert" ON public.<table>
  FOR INSERT TO authenticated
  WITH CHECK (
    account_id IN (
      SELECT account_id FROM public.user_agents
      WHERE user_id = auth.uid()
    )
  );

-- DELETE and UPDATE follow the same USING clause pattern

-- Service role: unrestricted access for admin operations
CREATE POLICY "<table>_service_role" ON public.<table>
  FOR ALL TO service_role
  USING (true);
```

## Acceptance Criteria

### AC-1: Old user-based policies dropped

- **Given:** The `documents` table has 4 user-based RLS policies (`documents_select_own`, `documents_insert_own`, `documents_update_own`, `documents_delete_own`)
- **When:** Migration `20260412000200_documents_rls_account_scoping.sql` runs
- **Then:** All 4 user-based policies are dropped
- **And NOT:** Any `_own` policy remains on the `documents` table

### AC-2: Account-scoped SELECT enforced on documents

- **Given:** User-A belongs to Account-X via `user_agents`, User-B belongs to Account-Y
- **When:** User-A queries `SELECT * FROM documents`
- **Then:** Only rows where `account_id` matches Account-X are returned
- **And NOT:** Rows belonging to Account-Y visible to User-A

### AC-3: Account-scoped INSERT enforced on documents

- **Given:** User-A belongs to Account-X via `user_agents`
- **When:** User-A inserts a document with `account_id = Account-Y` (not their account)
- **Then:** INSERT fails with RLS violation
- **And NOT:** Row created with a foreign account_id

### AC-4: Account-scoped DELETE enforced on documents

- **Given:** Document-1 belongs to Account-X, User-B belongs to Account-Y
- **When:** User-B attempts `DELETE FROM documents WHERE id = Document-1`
- **Then:** Zero rows deleted (RLS filters the row out of scope)
- **And NOT:** Document-1 deleted by a user outside its account

### AC-5: Service role bypasses RLS

- **Given:** A request authenticated with the Supabase service role key
- **When:** Service role queries `SELECT * FROM documents`
- **Then:** All rows returned regardless of account_id
- **And NOT:** Service role filtered by account — `documents_service_role` policy applies

### AC-6: user_agents join resolves multi-account membership

- **Given:** User-A belongs to both Account-X and Account-Y via `user_agents`
- **When:** User-A queries `SELECT * FROM documents`
- **Then:** Rows from both Account-X and Account-Y returned
- **And NOT:** Only the first account's rows returned

### AC-7: vector_insert stores account_id on document

- **Given:** `vector_insert(doc_content, doc_embedding, doc_metadata, doc_user_id, doc_account_id)` called with `doc_account_id = Account-X`
- **When:** The RPC executes
- **Then:** The resulting row has `account_id = Account-X`
- **And NOT:** account_id left NULL when explicitly provided

### AC-8: vector_insert backward compatible with NULL account_id

- **Given:** `vector_insert(doc_content, doc_embedding, doc_metadata, doc_user_id)` called without `doc_account_id` (5th param defaults to NULL)
- **When:** The RPC executes
- **Then:** The resulting row has `account_id = NULL` (DEFAULT NULL applied)
- **And NOT:** RPC call fails due to missing parameter (old 4-arg overloads are dropped, but DEFAULT handles omission)

### AC-9: vector_search defense-in-depth filtering

- **Given:** Documents from Account-X and Account-Y in the `documents` table
- **When:** `vector_search(query_embedding, 10, 0.5, caller_user_id, caller_account_id=Account-X)` called
- **Then:** Results filtered by both RLS (via `auth.uid()` + `user_agents` join) AND application-level `caller_account_id` WHERE clause
- **And NOT:** Account-Y documents returned even if RLS is misconfigured (defense-in-depth)

### AC-10: Old RPC overloads dropped

- **Given:** Old overloads exist: `vector_insert(text, text, jsonb, uuid)`, `vector_search(text, integer, real, uuid)`, `vector_insert_batch(text[], text[], jsonb[], uuid)`
- **When:** Migration `20260413_vector_rpc_account_id.sql` runs
- **Then:** All old overloads are dropped and replaced by 5-arg versions with `DEFAULT NULL` on `doc_account_id`/`caller_account_id`
- **And NOT:** Ambiguous function signatures from duplicate overloads

### AC-11: RLS enabled on all 7 tenant-visible tables

- **Given:** Tables: `documents`, `agent_kv`, `sessions`, `usage_log`, `skill_versions`, `session_labels`, `api_audit_logs`
- **When:** All account-scoping migrations have run
- **Then:** `ALTER TABLE <table> ENABLE ROW LEVEL SECURITY` is active on all 7 tables
- **And NOT:** Any tenant-visible table left without RLS enabled

### AC-12: No cross-tenant data leakage via RPC

- **Given:** User-A (Account-X) calls `vector_search` and results exist in Account-Y
- **When:** Search completes
- **Then:** Zero results from Account-Y in the response
- **And NOT:** Embedding similarity overriding account isolation (RLS is pre-filter, not post-filter)

## Claims Requiring Verification

| Claim | Verification Method | Test Name |
|-------|--------------------|-----------| 
| Old user-based policies dropped | SQL: `SELECT policyname FROM pg_policies WHERE tablename = 'documents'` | `test_old_policies_dropped` |
| Account-scoped SELECT on documents | Integration test: insert as Account-X, query as Account-Y | `test_documents_select_account_scoped` |
| Account-scoped INSERT rejected | Integration test: insert with foreign account_id | `test_documents_insert_foreign_account_rejected` |
| Account-scoped DELETE blocked | Integration test: delete cross-account row | `test_documents_delete_cross_account_blocked` |
| Service role bypasses RLS | Integration test: query with service role key | `test_service_role_bypass` |
| Multi-account membership works | Integration test: user in 2 accounts sees both | `test_multi_account_membership` |
| vector_insert stores account_id | Integration test: insert then raw SELECT | `test_vector_insert_stores_account_id` |
| vector_insert NULL account_id backward compat | Integration test: 5-arg call with `doc_account_id` omitted (DEFAULT NULL) | `test_vector_insert_null_account_compat` |
| vector_search defense-in-depth | Integration test: mismatched JWT vs caller_account_id | `test_vector_search_defense_in_depth` |
| Old overloads dropped (3 functions) | SQL: check function signatures | `test_old_overloads_dropped` |
| RLS enabled on all 7 tables | SQL: `SELECT tablename FROM pg_tables WHERE rowsecurity = true` | `test_rls_enabled_all_tables` |
| No cross-tenant vector search leakage | Integration test: search from wrong account returns empty | `test_no_cross_tenant_vector_leakage` |

## Storage Dependency Check (MANDATORY per SPEC-writer skill)

| ADR/SPEC Claims | Actual Supabase API | Match? |
|----------------|---------------------|--------|
| "RLS policies filter by account_id" | `CREATE POLICY ... USING (account_id IN (...))` | Yes — standard PostgreSQL RLS |
| "user_agents join resolves account" | `SELECT account_id FROM public.user_agents WHERE user_id = auth.uid()` | Yes — subquery in USING clause |
| "Service role bypasses RLS" | `TO service_role USING (true)` | Yes — Supabase `service_role` inherently bypasses RLS; explicit policy is belt-and-suspenders |
| "auth.uid() available in RLS" | Supabase injects `auth.uid()` from JWT | Yes — standard Supabase auth |
| "RPC params default NULL" | `doc_account_id UUID DEFAULT NULL` | Yes — PostgreSQL DEFAULT on function params |
| "Old overloads dropped" | `DROP FUNCTION IF EXISTS vector_insert(text, text, jsonb, uuid)` | Yes — PostgreSQL supports DROP FUNCTION with signature |
| "HNSW index for cosine similarity" | `CREATE INDEX ... USING hnsw (embedding ruvector_cosine_ops)` | Yes — ruvector extension provides operator class |
| "ruvector(384) type" | `embedding ruvector(384)` | Yes — requires ADR-RV-001 extension loaded |

## Exit Gate

```bash
#!/bin/bash
set -e

# Prerequisites: SUPABASE_URL, SUPABASE_SERVICE_KEY, SUPABASE_ANON_KEY env vars set
# Requires: psql access to Supabase project OR supabase CLI

# 1. Verify RLS is enabled on all 7 tenant-visible tables
for table in documents agent_kv sessions usage_log skill_versions session_labels api_audit_logs; do
  psql "$SUPABASE_DB_URL" -tAc \
    "SELECT rowsecurity FROM pg_tables WHERE tablename = '$table'" | grep -q "t" || \
    { echo "FAIL: RLS not enabled on $table"; exit 1; }
done
echo "PASS: RLS enabled on all 7 tables"

# 2. Verify old user-based policies are gone
OLD_COUNT=$(psql "$SUPABASE_DB_URL" -tAc \
  "SELECT count(*) FROM pg_policies WHERE tablename = 'documents' AND policyname LIKE '%_own'")
[ "$OLD_COUNT" -eq 0 ] || { echo "FAIL: $OLD_COUNT old _own policies remain"; exit 1; }
echo "PASS: Old user-based policies dropped"

# 3. Verify account-scoped policies exist on documents
for op in select insert delete; do
  psql "$SUPABASE_DB_URL" -tAc \
    "SELECT policyname FROM pg_policies WHERE tablename = 'documents' AND policyname = 'documents_$op'" | \
    grep -q "documents_$op" || { echo "FAIL: documents_$op policy missing"; exit 1; }
done
echo "PASS: Account-scoped policies on documents"

# 4. Verify service_role policy exists
psql "$SUPABASE_DB_URL" -tAc \
  "SELECT policyname FROM pg_policies WHERE tablename = 'documents' AND policyname = 'documents_service_role'" | \
  grep -q "documents_service_role" || { echo "FAIL: documents_service_role policy missing"; exit 1; }
echo "PASS: Service role policy exists"

# 5. Verify RPC function signatures have account_id params
psql "$SUPABASE_DB_URL" -tAc \
  "SELECT pg_get_function_arguments(oid) FROM pg_proc WHERE proname = 'vector_insert'" | \
  grep -q "account_id" || { echo "FAIL: vector_insert missing account_id param"; exit 1; }
psql "$SUPABASE_DB_URL" -tAc \
  "SELECT pg_get_function_arguments(oid) FROM pg_proc WHERE proname = 'vector_search'" | \
  grep -q "account_id" || { echo "FAIL: vector_search missing account_id param"; exit 1; }
echo "PASS: RPC functions have account_id params"

# 6. Verify old overloads are gone (only 5-arg versions remain)
for fn in vector_insert vector_insert_batch vector_search; do
  OVERLOAD_COUNT=$(psql "$SUPABASE_DB_URL" -tAc \
    "SELECT count(*) FROM pg_proc WHERE proname = '$fn'")
  [ "$OVERLOAD_COUNT" -eq 1 ] || { echo "FAIL: Expected 1 $fn overload, found $OVERLOAD_COUNT"; exit 1; }
done
echo "PASS: No ambiguous overloads"

# 7. Verify user_agents table exists (required for RLS join)
psql "$SUPABASE_DB_URL" -tAc \
  "SELECT count(*) FROM information_schema.tables WHERE table_name = 'user_agents'" | \
  grep -q "1" || { echo "FAIL: user_agents table missing"; exit 1; }
echo "PASS: user_agents table exists"

# 8. librefang HttpVectorStore compiles (client-side defense-in-depth)
cargo clippy -p librefang-memory --all-targets -- -D warnings

echo "SPEC-MT-004 EXIT GATE: ALL PASS"
```

## Out of Scope

| Excluded | Reason | When |
|----------|--------|------|
| Per-account encryption keys for embeddings | Adds complexity, RLS provides sufficient isolation | Phase 5 |
| JWT custom claims for account_id | Currently resolved via `user_agents` join; JWT claims are an optimization | Phase 4 |
| UPDATE policy on documents | qwntik migrations do not create an UPDATE policy (documents are immutable embeddings) | If mutability needed |
| RLS policies for system/internal tables (e.g., `migrations`, `schema_info`) | System tables are not tenant-visible | Never |
| SQLite RLS (triggers) | SQLite has no native RLS; application-level filtering in SPEC-MT-003 | N/A |
| Cross-account data sharing policies | Requires sharing model design (invitations, teams) | Phase 5+ |
| Rate limiting per account on RPC calls | Supabase edge function or API gateway concern | Phase 4 |
| `vector_insert_batch` RLS integration testing | Batch function follows same pattern; test single-row first | Phase 3b |
| Direct `sqlx::PgPool` bypass of PostgREST | Performance optimization; HTTP/PostgREST is sufficient for Phase 3 | Phase 4 (SPEC-RV-002 Phase 2) |
