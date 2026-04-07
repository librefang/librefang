-- RuVector Extension Setup for Librefang
-- Creates documents table, HNSW index, RLS policies, and RPC functions.
-- Copied from qwntik/apps/web/supabase/migrations/20260405_ruvector_setup.sql
--
-- Embedding dimensions: 384 (all-MiniLM-L6-v2 via fastembed/ONNX)
-- If ruvector extension is not installed, this is a graceful no-op.
-- ============================================================================

-- 1. Enable the RuVector extension (gracefully skip if not available)
DO $$
BEGIN
  CREATE EXTENSION IF NOT EXISTS ruvector;
EXCEPTION WHEN OTHERS THEN
  RAISE NOTICE 'ruvector extension not available — skipping setup. Build supabase-ruvector:latest to enable.';
END $$;

-- 2-8. Only runs if ruvector is available
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'ruvector') THEN
    RAISE NOTICE 'ruvector not installed — skipping documents table, indexes, RLS, and RPCs.';
    RETURN;
  END IF;

  -- 2. Documents table with per-user ownership
  EXECUTE '
    CREATE TABLE IF NOT EXISTS documents (
      id BIGSERIAL PRIMARY KEY,
      user_id UUID NOT NULL,
      content TEXT NOT NULL,
      embedding ruvector(384) NOT NULL,
      metadata JSONB DEFAULT ''{}''::jsonb,
      created_at TIMESTAMPTZ DEFAULT NOW(),
      updated_at TIMESTAMPTZ DEFAULT NOW()
    )';

  -- 3. HNSW index for fast cosine similarity search
  EXECUTE 'CREATE INDEX IF NOT EXISTS idx_documents_embedding_hnsw
    ON documents USING hnsw (embedding ruvector_cosine_ops)';

  EXECUTE 'CREATE INDEX IF NOT EXISTS idx_documents_user_id ON documents(user_id)';

  -- 4. Row Level Security (uses auth.uid() for Supabase compatibility)
  EXECUTE 'ALTER TABLE documents ENABLE ROW LEVEL SECURITY';

  EXECUTE 'DROP POLICY IF EXISTS documents_select_own ON documents';
  EXECUTE 'CREATE POLICY documents_select_own ON documents
    FOR SELECT USING (auth.uid() = user_id)';

  EXECUTE 'DROP POLICY IF EXISTS documents_insert_own ON documents';
  EXECUTE 'CREATE POLICY documents_insert_own ON documents
    FOR INSERT WITH CHECK (auth.uid() = user_id)';

  EXECUTE 'DROP POLICY IF EXISTS documents_update_own ON documents';
  EXECUTE 'CREATE POLICY documents_update_own ON documents
    FOR UPDATE USING (auth.uid() = user_id)
    WITH CHECK (auth.uid() = user_id)';

  EXECUTE 'DROP POLICY IF EXISTS documents_delete_own ON documents';
  EXECUTE 'CREATE POLICY documents_delete_own ON documents
    FOR DELETE USING (auth.uid() = user_id)';

  -- 5. RPC: vector_search
  EXECUTE '
    CREATE OR REPLACE FUNCTION vector_search(
      query_embedding TEXT,
      match_count INT DEFAULT 10,
      match_threshold REAL DEFAULT 0.3,
      caller_user_id UUID DEFAULT NULL
    )
    RETURNS TABLE (
      id BIGINT,
      content TEXT,
      metadata JSONB,
      distance REAL
    )
    LANGUAGE plpgsql
    SECURITY INVOKER
    AS $fn$
    BEGIN
      RETURN QUERY
        SELECT sub.id, sub.content, sub.metadata, sub.dist
        FROM (
          SELECT d.id, d.content, d.metadata,
                 ruvector_cosine_distance(d.embedding, query_embedding::ruvector) AS dist
          FROM documents d
          WHERE caller_user_id IS NULL OR d.user_id = caller_user_id
        ) sub
        WHERE sub.dist < match_threshold
        ORDER BY sub.dist ASC
        LIMIT match_count;
    END;
    $fn$';

  -- 6. RPC: vector_insert
  EXECUTE '
    CREATE OR REPLACE FUNCTION vector_insert(
      doc_content TEXT DEFAULT '''',
      doc_embedding TEXT DEFAULT ''[]'',
      doc_metadata JSONB DEFAULT ''{}''::jsonb,
      doc_user_id UUID DEFAULT NULL
    )
    RETURNS BIGINT
    LANGUAGE plpgsql
    SECURITY INVOKER
    AS $fn$
    DECLARE
      new_id BIGINT;
      effective_uid UUID;
    BEGIN
      effective_uid := COALESCE(doc_user_id, auth.uid());
      IF effective_uid IS NULL THEN
        RAISE EXCEPTION ''Authentication required for vector insert'';
      END IF;

      INSERT INTO documents (user_id, content, embedding, metadata)
      VALUES (effective_uid, doc_content, doc_embedding::ruvector, doc_metadata)
      RETURNING documents.id INTO new_id;

      RETURN new_id;
    END;
    $fn$';

  -- 7. RPC: vector_insert_batch
  EXECUTE '
    CREATE OR REPLACE FUNCTION vector_insert_batch(
      doc_contents TEXT[] DEFAULT ''{}'',
      doc_embeddings TEXT[] DEFAULT ''{}'',
      doc_metadatas JSONB[] DEFAULT ''{}'',
      doc_user_id UUID DEFAULT NULL
    )
    RETURNS BIGINT[]
    LANGUAGE plpgsql
    SECURITY INVOKER
    AS $fn$
    DECLARE
      ids BIGINT[] := ''{}'';new_id BIGINT;
      effective_uid UUID;
      i INT;
    BEGIN
      IF array_length(doc_contents, 1) != array_length(doc_embeddings, 1) THEN
        RAISE EXCEPTION ''doc_contents and doc_embeddings arrays must be the same length'';
      END IF;

      effective_uid := COALESCE(doc_user_id, auth.uid());
      IF effective_uid IS NULL THEN
        RAISE EXCEPTION ''Authentication required for vector insert'';
      END IF;

      FOR i IN 1..array_length(doc_contents, 1) LOOP
        INSERT INTO documents (user_id, content, embedding, metadata)
        VALUES (effective_uid, doc_contents[i], doc_embeddings[i]::ruvector,
                COALESCE(doc_metadatas[i], ''{}''::jsonb))
        RETURNING documents.id INTO new_id;
        ids := ids || new_id;
      END LOOP;

      RETURN ids;
    END;
    $fn$';

  -- 8. RPC: vector_delete
  EXECUTE '
    CREATE OR REPLACE FUNCTION vector_delete(
      doc_id BIGINT DEFAULT 0
    )
    RETURNS BOOLEAN
    LANGUAGE plpgsql
    SECURITY INVOKER
    AS $fn$
    BEGIN
      DELETE FROM documents WHERE id = doc_id;
      RETURN FOUND;
    END;
    $fn$';

  -- 9. RPC: ruvector_version_check (called by app health endpoint)
  EXECUTE '
    CREATE OR REPLACE FUNCTION ruvector_version_check()
    RETURNS TEXT
    LANGUAGE sql
    SECURITY INVOKER
    AS $fn$
      SELECT ruvector_version();
    $fn$';

  -- 10. Grant to standard roles (if they exist)
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'authenticated') THEN
    EXECUTE 'GRANT EXECUTE ON FUNCTION vector_search TO authenticated, service_role';
    EXECUTE 'GRANT EXECUTE ON FUNCTION vector_insert TO authenticated, service_role';
    EXECUTE 'GRANT EXECUTE ON FUNCTION vector_insert_batch TO authenticated, service_role';
    EXECUTE 'GRANT EXECUTE ON FUNCTION vector_delete TO authenticated, service_role';
    EXECUTE 'GRANT EXECUTE ON FUNCTION ruvector_version_check TO authenticated, service_role, anon';
    EXECUTE 'GRANT ALL ON TABLE documents TO authenticated, service_role';
    EXECUTE 'GRANT USAGE, SELECT ON SEQUENCE documents_id_seq TO authenticated, service_role';
    RAISE NOTICE 'Grants applied to Supabase roles.';
  ELSE
    RAISE NOTICE 'Supabase roles not found (standalone mode) — skipping grants.';
  END IF;

END $$;
