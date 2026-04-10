-- 002_security_hardening.sql
-- ============================================================
-- Hardens the ruvector document store for production.
--
-- Apply with:
--   psql -h localhost -p 54322 -U postgres -f docker/sql/002_security_hardening.sql
-- ============================================================

-- ┌──────────────────────────────────────────────────────────┐
-- │ 1. Revoke excessive anon table grants                    │
-- │    anon should NOT have TRUNCATE, UPDATE, REFERENCES,    │
-- │    or TRIGGER on the documents table.                    │
-- └──────────────────────────────────────────────────────────┘
REVOKE TRUNCATE ON documents FROM anon;
REVOKE UPDATE ON documents FROM anon;
REVOKE REFERENCES ON documents FROM anon;
REVOKE TRIGGER ON documents FROM anon;
-- anon retains: SELECT (PostgREST introspection), INSERT, DELETE (via RPCs)

-- ┌──────────────────────────────────────────────────────────┐
-- │ 2. Make vector RPCs SECURITY DEFINER                     │
-- │    Lets RPCs bypass RLS — access control is handled in   │
-- │    the function logic (user_id / account_id params).     │
-- │    Without this, anon callers hit RLS with no policy     │
-- │    and get 0 results / silent failures.                  │
-- └──────────────────────────────────────────────────────────┘
ALTER FUNCTION vector_insert(text, text, jsonb, uuid, uuid)
  SECURITY DEFINER SET search_path = public;

ALTER FUNCTION vector_insert_batch(text[], text[], jsonb[], uuid, uuid)
  SECURITY DEFINER SET search_path = public;

ALTER FUNCTION vector_search(text, integer, real, uuid, uuid)
  SECURITY DEFINER SET search_path = public;

ALTER FUNCTION vector_delete(bigint)
  SECURITY DEFINER SET search_path = public;

-- search_path = public prevents schema-injection attacks on
-- SECURITY DEFINER functions (CWE-89 / CVE-2018-1058 pattern).

-- ┌──────────────────────────────────────────────────────────┐
-- │ 3. After SECURITY DEFINER, revoke direct INSERT/DELETE   │
-- │    from anon — all DML now goes through the RPCs.        │
-- └──────────────────────────────────────────────────────────┘
REVOKE INSERT ON documents FROM anon;
REVOKE DELETE ON documents FROM anon;
-- anon retains: SELECT (for PostgREST schema discovery only)

-- ┌──────────────────────────────────────────────────────────┐
-- │ 4. Revoke EXECUTE on internal ruvector helper functions   │
-- │    from anon — they don't need vector_add, vector_mul,   │
-- │    etc. directly.                                        │
-- └──────────────────────────────────────────────────────────┘
-- Note: helper functions (vector_add, vector_sub, etc.) use real[] signatures
-- and inherit EXECUTE from the PUBLIC role. They're pure math operations with
-- no data access, so revoking is optional. If desired:
--   REVOKE EXECUTE ON FUNCTION vector_add(real[], real[]) FROM PUBLIC;
--   (repeat for vector_sub, vector_mul_scalar, vector_norm, vector_normalize,
--    vector_dims, vector_sum, vector_sum_state, vector_avg2, vector_avg_final)

-- ┌──────────────────────────────────────────────────────────┐
-- │ 5. Notify PostgREST to reload schema cache               │
-- └──────────────────────────────────────────────────────────┘
NOTIFY pgrst, 'reload schema';

-- ┌──────────────────────────────────────────────────────────┐
-- │ Verification queries (run manually after applying):      │
-- └──────────────────────────────────────────────────────────┘
-- SELECT grantee, privilege_type FROM information_schema.table_privileges
--   WHERE table_name = 'documents' AND grantee = 'anon';
-- Expected: SELECT only
--
-- SELECT proname, prosecdef FROM pg_proc
--   WHERE proname IN ('vector_insert','vector_insert_batch','vector_search','vector_delete');
-- Expected: prosecdef = true for all four
