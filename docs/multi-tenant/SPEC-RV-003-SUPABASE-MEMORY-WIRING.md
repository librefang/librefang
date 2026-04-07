# SPEC-RV-003: Supabase Memory Wiring — Wire RuVector to LibreFang Memory

**Status:** PLANNED
**Depends on:** Phase 0 (tasks 0.1–0.6 verified 2026-04-07; task 0.7 covered here)
**Date:** 2026-04-06 (authored by swarm agent), reviewed and promoted 2026-04-07
**Covers:** MASTER-PLAN task 0.7 (HttpVectorStore → Supabase) + SupabaseVectorStore implementation
**Supersedes:** `docker/docs/SPEC-RV-002-PHASE-1.md` (swarm artifact, deleted)

---

## Purpose

Connect librefang's memory substrate to the RuVector PostgreSQL extension
running in the `supabase-ruvector` Docker image. After Phase 1, agents can
store and recall semantic memories using vector similarity search.

---

## Architecture (Verified)

### What Already Exists in LibreFang

| Component | File | Status |
|-----------|------|--------|
| `VectorStore` trait | `crates/librefang-types/src/memory.rs` L1226 | ✅ Complete |
| `HttpVectorStore` impl | `crates/librefang-memory/src/http_vector_store.rs` | ✅ Complete (232 lines, 2 tests) |
| `SqliteVectorStore` impl | `crates/librefang-memory/src/semantic.rs` | ✅ Complete (fallback) |
| `MemorySubstrate` | `crates/librefang-memory/src/substrate.rs` | ✅ Has `attach_vector_store()` |
| `EmbeddingFn` trait | `crates/librefang-memory/src/proactive.rs` L93 | ✅ Complete |
| `SemanticStore::recall_with_embedding` | `crates/librefang-memory/src/semantic.rs` | ✅ Complete |
| `MemorySubstrate.set_vector_store()` | `crates/librefang-memory/src/substrate.rs` | ✅ Wiring point |
| Text chunker | `crates/librefang-memory/src/chunker.rs` | ✅ Overlap chunking |
| Decay engine | `crates/librefang-memory/src/decay.rs` | ✅ Time-based decay |

### What Exists in RuVector Extension (Docker)

| SQL Function | Signature | Purpose |
|-------------|-----------|----------|
| `ruvector_embed(text)` | `→ real[]` (384-dim) | Local ONNX embedding |
| `ruvector_embed_batch(text[])` | `→ real[][]` | Batch embeddings |
| `ruvector_cosine_distance(ruvector, ruvector)` | `→ float` | Cosine distance |
| `<=>` operator | `ruvector <=> ruvector → float` | Cosine distance (HNSW-indexable) |
| Assignment cast | `real[] → ruvector` | Auto-converts on INSERT |

### What Exists in Migration SQL (`docker/sql/ruvector_setup.sql`)

| RPC Function | Signature | Called By |
|-------------|-----------|----------|
| `vector_search(query_embedding, match_count, match_threshold, caller_user_id)` | `→ TABLE(id, content, metadata, distance)` | PostgREST `/rest/v1/rpc/vector_search` |
| `vector_insert(doc_content, doc_embedding, doc_metadata, doc_user_id)` | `→ BIGINT` | PostgREST `/rest/v1/rpc/vector_insert` |
| `vector_insert_batch(doc_contents[], doc_embeddings[], doc_metadatas[], doc_user_id)` | `→ BIGINT[]` | PostgREST `/rest/v1/rpc/vector_insert_batch` |
| `vector_delete(doc_id)` | `→ BOOLEAN` | PostgREST `/rest/v1/rpc/vector_delete` |
| `ruvector_version_check()` | `→ TEXT` | Health endpoint |

### Integration Options

There are **two paths** to connect librefang to the extension:

#### Option A: SupabaseVectorStore (HTTP via PostgREST) — RECOMMENDED

Build a new `SupabaseVectorStore` that implements `VectorStore` by calling
the PostgREST RPC functions over HTTP. This reuses the existing
`HttpVectorStore` pattern but adapts the request/response format to
match Supabase's PostgREST RPC convention.

```
Agent → MemorySubstrate → SupabaseVectorStore → HTTP POST
    → PostgREST /rest/v1/rpc/vector_search
    → PostgreSQL vector_search() wrapper
    → ruvector_cosine_distance() + HNSW index
    → Results back as JSON
```

**Why this path:**
- No new dependencies (already have `reqwest`)
- RLS enforced by PostgREST (account isolation for free)
- Works with Supabase Cloud or self-hosted
- Decoupled from PostgreSQL driver (no `sqlx` needed)
- The RPC functions already handle embedding format conversion

#### Option B: PostgresVectorStore (Direct SQL via sqlx)

Port the `PostgresVectorStore` from openfang-ai. This connects directly
to PostgreSQL via `sqlx::PgPool` and runs raw SQL with `::ruvector` casts.

```
Agent → MemorySubstrate → PostgresVectorStore → sqlx PgPool
    → SELECT ... ORDER BY embedding <=> $1::ruvector
    → HNSW index lookup
    → Results back as rows
```

**Why NOT this path (for now):**
- Requires adding `sqlx` + `tokio-postgres` deps to librefang-memory
- Bypasses PostgREST RLS (must enforce account isolation manually)
- Tighter coupling to PostgreSQL schema
- More code to maintain

**Verdict:** Start with Option A. Revisit Option B only if HTTP latency
becomes a bottleneck (unlikely — localhost PostgREST adds <2ms).

---

## Exact Wiring Plan

### Step 1: Create `SupabaseVectorStore`

**File:** `crates/librefang-memory/src/supabase_vector_store.rs` (NEW)

```rust
use async_trait::async_trait;
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::memory::{MemoryFilter, VectorSearchResult, VectorStore};
use reqwest::Client;
use std::collections::HashMap;

pub struct SupabaseVectorStore {
    client: Client,
    /// e.g. "http://localhost:54321/rest/v1"
    rest_url: String,
    /// Supabase anon or service_role key
    api_key: String,
    /// Embedding dimension (384 for MiniLM, 1536 for OpenAI)
    embedding_dim: usize,
}

impl SupabaseVectorStore {
    pub fn new(
        rest_url: impl Into<String>,
        api_key: impl Into<String>,
        embedding_dim: usize,
    ) -> Self {
        Self {
            client: Client::new(),
            rest_url: rest_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            embedding_dim,
        }
    }

    fn rpc_url(&self, fn_name: &str) -> String {
        format!("{}/rpc/{}", self.rest_url, fn_name)
    }

    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("apikey", self.api_key.parse().unwrap());
        h.insert("Authorization", format!("Bearer {}", self.api_key).parse().unwrap());
        h.insert("Content-Type", "application/json".parse().unwrap());
        h
    }
}
```

### Step 2: Implement `VectorStore` trait

The key mapping between the trait methods and Supabase RPCs:

| Trait method | Supabase RPC | Request body | Response |
|-------------|-------------|--------------|----------|
| `insert()` | `POST /rpc/vector_insert` | `{"doc_content": payload, "doc_embedding": "[0.1,0.2,...]", "doc_metadata": {...}}` | `BIGINT` (new ID) |
| `search()` | `POST /rpc/vector_search` | `{"query_embedding": "[0.1,0.2,...]", "match_count": limit, "match_threshold": 0.3}` | `[{id, content, metadata, distance}]` |
| `delete()` | `POST /rpc/vector_delete` | `{"doc_id": id}` | `BOOLEAN` |
| `get_embeddings()` | Not available via RPC | Return empty (or add new RPC) | `{}` |

**Critical format note:** The RPC functions expect embeddings as **TEXT** strings
(e.g. `"[0.1, 0.2, 0.3]"`) which PostgreSQL casts to `ruvector` via the
assignment cast. Do NOT send as JSON arrays — send as stringified vectors.

```rust
// Example: convert &[f32] to the text format PostgreSQL expects
fn embedding_to_pg_text(embedding: &[f32]) -> String {
    format!("[{}]", embedding.iter()
        .map(|f| f.to_string())
        .collect::<Vec<_>>()
        .join(","))
}
```

### Step 3: Register in lib.rs

**File:** `crates/librefang-memory/src/lib.rs`

Add:
```rust
pub mod supabase_vector_store;
pub use supabase_vector_store::SupabaseVectorStore;
```

### Step 4: Wire into MemorySubstrate

**File:** `crates/librefang-memory/src/substrate.rs`

The substrate already has a vector store attachment point. During kernel
initialization, create the store and attach:

```rust
// In kernel or runtime startup:
let vector_store = SupabaseVectorStore::new(
    "http://localhost:54321/rest/v1",  // from config
    std::env::var("SUPABASE_ANON_KEY").unwrap(),
    384,  // MiniLM-L6-v2 dimension
);
substrate.set_vector_store(Arc::new(vector_store));
```

### Step 5: Embedding Generation

Two options for generating embeddings before calling `vector_store.insert()`:

#### Option 5A: Server-side (ruvector_embed) — Simpler

Don't compute embeddings client-side. Instead, pass raw text to
`vector_insert` and let PostgreSQL compute the embedding:

```sql
-- The RPC already does this:
INSERT INTO documents (content, embedding)
VALUES (doc_content, ruvector_embed(doc_content));
```

But the current RPC takes a pre-computed embedding string, not raw text.
To use server-side embeddings, add a new RPC:

```sql
CREATE OR REPLACE FUNCTION vector_insert_with_embed(
    doc_content TEXT,
    doc_metadata JSONB DEFAULT '{}'
)
RETURNS BIGINT AS $$
    INSERT INTO documents (user_id, content, embedding, metadata)
    VALUES (auth.uid(), doc_content, ruvector_embed(doc_content), doc_metadata)
    RETURNING id;
$$ LANGUAGE sql SECURITY INVOKER;
```

#### Option 5B: Client-side (EmbeddingFn trait) — More flexible

Use librefang's existing `EmbeddingFn` trait to compute embeddings
client-side (OpenAI, Ollama, etc.), then send the vector to the RPC.
This is what openfang-ai does.

**Verdict:** Use 5A for simplicity in Phase 1 (no external embedding API
needed — the extension does it locally). Add 5B support later for
production (OpenAI text-embedding-3-small at 1536-dim).

### Step 6: Config

**File:** `crates/librefang-types/src/config/types.rs`

Add to the memory config section:

```rust
/// Supabase vector store configuration (connects to supabase-ruvector)
#[serde(default)]
pub vector_store_backend: Option<String>,  // "supabase" | "http" | None
#[serde(default)]
pub vector_store_url: Option<String>,      // e.g. "http://localhost:54321/rest/v1"
#[serde(default)]
pub vector_store_api_key_env: Option<String>,  // env var name, e.g. "SUPABASE_ANON_KEY"
#[serde(default)]
pub embedding_dim: Option<u16>,            // 384 (MiniLM) or 1536 (OpenAI)
```

TOML example:
```toml
[memory]
vector_store_backend = "supabase"
vector_store_url = "http://localhost:54321/rest/v1"
vector_store_api_key_env = "SUPABASE_ANON_KEY"
embedding_dim = 384
```

---

## Acceptance Criteria (12)

### Group 1: SupabaseVectorStore Implementation (4)

#### AC-1.1: Insert via RPC
- **Given:** SupabaseVectorStore configured, supabase-ruvector running
- **When:** `store.insert("id-1", &[0.1, 0.2, ...384 floats], "hello world", {})`
- **Then:** Row appears in `documents` table with correct embedding

#### AC-1.2: Search via RPC
- **Given:** 3+ documents inserted with embeddings
- **When:** `store.search(&query_embedding, 3, None)`
- **Then:** Returns results ranked by cosine similarity, scores between 0.0-1.0

#### AC-1.3: Delete via RPC
- **Given:** Document exists with known ID
- **When:** `store.delete("id-1")`
- **Then:** Document no longer returned by search

#### AC-1.4: backend_name
- **When:** `store.backend_name()`
- **Then:** Returns `"supabase"`

### Group 2: Substrate Wiring (3)

#### AC-2.1: Config Parsing
- **Given:** TOML with `vector_store_backend = "supabase"`
- **When:** Config deserialized
- **Then:** `config.memory.vector_store_backend == Some("supabase")`

#### AC-2.2: Substrate Attachment
- **Given:** SupabaseVectorStore created from config
- **When:** `substrate.set_vector_store(Arc::new(store))`
- **Then:** Subsequent `recall_with_embedding()` calls use the vector store

#### AC-2.3: Graceful Fallback
- **Given:** No vector store configured (`vector_store_backend = None`)
- **When:** `recall_with_embedding()` called
- **Then:** Falls back to SQLite LIKE matching (no error, no panic)

### Group 3: End-to-End (3)

#### AC-3.1: Agent Store + Recall
- **Given:** Agent with vector store attached, supabase-ruvector running
- **When:** Agent stores "cats are fluffy pets" then recalls "feline animals"
- **Then:** Stored memory appears as top result

#### AC-3.2: Server-Side Embedding (Option 5A)
- **Given:** `vector_insert_with_embed` RPC exists
- **When:** Insert raw text (no client-side embedding)
- **Then:** PostgreSQL computes 384-dim embedding, stores in documents table

#### AC-3.3: RLS Isolation
- **Given:** Two users (A, B) each with documents
- **When:** User A searches
- **Then:** Only user A's documents returned (PostgREST RLS enforces)

### Group 4: Health + Observability (2)

#### AC-4.1: Health Check
- **Given:** supabase-ruvector running
- **When:** `GET /rest/v1/rpc/ruvector_version_check` with apikey
- **Then:** Returns `"0.3.0"`

#### AC-4.2: Connection Error Handling
- **Given:** supabase-ruvector NOT running
- **When:** `store.search(...)` called
- **Then:** Returns `Err(LibreFangError::Internal("..."))`, no panic

---

## Files to Create/Modify

| File | Action | Lines (est.) |
|------|--------|-------------|
| `crates/librefang-memory/src/supabase_vector_store.rs` | **CREATE** | ~200 |
| `crates/librefang-memory/src/lib.rs` | ADD module + re-export | +3 |
| `crates/librefang-memory/src/substrate.rs` | Verify `set_vector_store()` works | ~0 (already exists) |
| `crates/librefang-types/src/config/types.rs` | ADD vector store config fields | +10 |
| `docker/sql/ruvector_setup.sql` | ADD `vector_insert_with_embed` RPC | +15 |
| `crates/librefang-kernel/src/lib.rs` (or runtime) | Wire store creation from config | +20 |

**Total:** ~250 new lines

---

## Reference: openfang-ai Implementation (for comparison)

openfang-ai uses **direct SQL** (Option B) not HTTP. Key files for reference:

| File (openfang-ai) | What it does | Relevant to Phase 1? |
|---------------------|-------------|----------------------|
| `crates/openfang-types/src/memory.rs` L463-511 | `VectorStore` trait (identical to librefang's) | Reference only |
| `crates/openfang-memory/src/postgres_vector_store.rs` | Direct `sqlx` SQL with `<=>` operator, 23 tests | Reference for Option B later |
| `crates/openfang-runtime/src/embedding.rs` | `EmbeddingDriver` — OpenAI-compatible client | Reference for Phase 2 |
| `crates/openfang-api/src/routes/vectors.rs` | HTTP routes wrapping VectorStore (5 endpoints, 23 tests) | Not needed — PostgREST replaces this |
| `crates/openfang-api/src/server.rs` L54-73 | Creates PgPool, injects VectorStore | Reference for kernel wiring |

### Key SQL Patterns (from openfang-ai's PostgresVectorStore)

```sql
-- Insert (direct SQL)
INSERT INTO vector_memories (id, account_id, embedding, payload, metadata)
VALUES ($1::uuid, $2::uuid, $3::ruvector, $4, $5::jsonb)

-- Search (direct SQL, cosine distance via <=> operator)
SELECT id, payload, metadata,
       embedding <=> $1::ruvector AS distance
FROM vector_memories
WHERE account_id = $2::uuid
ORDER BY embedding <=> $1::ruvector
LIMIT {limit}

-- The <=> operator uses the HNSW index automatically
-- Assignment cast: real[] → ruvector fires on INSERT
```

---

## Out of Scope (deferred to Phase 2+)

| Item | Why deferred |
|------|-------------|
| Client-side embeddings (OpenAI 1536-dim) | Phase 1 uses server-side 384-dim |
| Direct sqlx PostgresVectorStore | HTTP is sufficient for now |
| ONNX sidecar (image diet) | Optimization, not functionality |
| CI/CD for Docker image builds | Manual build acceptable |
| Multi-model embedding support | One model (MiniLM) is enough for Phase 1 |
| Batch search endpoint | Single search is enough for Phase 1 |

---

## Exit Gate

```bash
#!/usr/bin/env bash
set -euo pipefail

# Prerequisite: supabase-ruvector container running on port 54322
export PGPASSWORD=postgres
PSQL="psql -h localhost -p 54322 -U postgres -d postgres -tAc"

echo "=== Gate 1: Extension loaded ==="
$PSQL "SELECT ruvector_version()" | grep -q '0.3.0'

echo "=== Gate 2: Cargo check (librefang workspace) ==="
cargo check -p librefang-memory

echo "=== Gate 3: SupabaseVectorStore unit tests ==="
cargo test -p librefang-memory supabase_vector_store

echo "=== Gate 4: Integration test (requires running container) ==="
SUPABASE_URL=http://localhost:54321 \
SUPABASE_ANON_KEY=your-anon-key \
cargo test -p librefang-memory integration_supabase -- --ignored

echo "=== Gate 5: Health endpoint ==="
curl -sf http://localhost:54321/rest/v1/rpc/ruvector_version_check \
  -H "apikey: $SUPABASE_ANON_KEY" | grep -q '0.3.0'

echo "✅ Phase 1 exit gate: ALL PASSED"
```
