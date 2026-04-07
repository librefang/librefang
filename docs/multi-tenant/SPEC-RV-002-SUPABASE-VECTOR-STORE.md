# SPEC-RV-002: Supabase Vector Store Integration

**ADR:** ADR-RV-001 (RuVector Extension Port)
**Date:** 2026-04-06
**Author:** Engineering

---

## Purpose

Connect librefang's `VectorStore` trait to Supabase PostgreSQL via PostgREST
RPC endpoints, enabling RuVector-powered vector operations (insert, search,
delete) over HTTP with zero new Rust dependencies. RLS enforces tenant isolation
for free.

## Source of Truth

| Source | What it proves |
|--------|---------------|
| librefang `VectorStore` trait | 5-method interface at `librefang-types/src/memory.rs:1228` |
| librefang `HttpVectorStore` | Working HTTP impl pattern at `librefang-memory/src/http_vector_store.rs` |
| librefang kernel init | Backend selection at `kernel.rs:1668-1685` |
| librefang config | `vector_backend` + `vector_store_url` at `config/types.rs:3145` |
| qwntik ruvector-integration.md | RPC functions deployed and verified |
| qwntik ADR-004 | Feature adoption roadmap (4 phases) |
| openfang-ai DECISION-NEEDED | Architecture: HTTP bridge recommended |

## Existing Infrastructure (verified)

| Component | File | Status |
|-----------|------|--------|
| `VectorStore` trait (5 methods) | `librefang-types/src/memory.rs:1228-1393` | ✅ Exists |
| `VectorSearchResult` struct | `librefang-types/src/memory.rs:1230-1240` | ✅ Exists |
| `HttpVectorStore` impl | `librefang-memory/src/http_vector_store.rs:1-259` | ✅ Exists |
| `SqliteVectorStore` impl | `librefang-memory/src/semantic.rs:950-1514` | ✅ Exists |
| `MemorySubstrate::set_vector_store()` | `librefang-memory/src/substrate.rs:110-114` | ✅ Exists |
| Config: `vector_backend`, `vector_store_url` | `librefang-types/src/config/types.rs:3145-3157` | ✅ Exists |
| Kernel backend match | `librefang-kernel/src/kernel.rs:1668-1685` | ✅ Exists |
| `reqwest` dependency | `librefang-memory/Cargo.toml` | ✅ Already present |
| `pub mod http_vector_store` | `librefang-memory/src/lib.rs` | ✅ Exists |
| Re-export `HttpVectorStore` | `librefang-memory/src/lib.rs` | ✅ Exists |
| **`SupabaseVectorStore`** | — | ❌ Missing |
| **Config: `supabase_url`, `supabase_anon_key`** | — | ❌ Missing |
| **Kernel: `"supabase"` backend branch** | — | ❌ Missing |

## Supabase RPC Function Signatures (deployed in qwntik)

```sql
-- Insert/update a vector with metadata
CREATE FUNCTION vector_insert(
  p_id text,
  p_embedding text,      -- TEXT format: "[0.1,0.2,...]" NOT JSON array
  p_payload text,
  p_metadata jsonb DEFAULT '{}'
) RETURNS void;

-- Search nearest vectors
CREATE FUNCTION vector_search(
  p_query_embedding text, -- TEXT format: "[0.1,0.2,...]"
  p_limit int DEFAULT 10,
  p_filter jsonb DEFAULT NULL
) RETURNS TABLE(id text, payload text, score real, metadata jsonb);

-- Delete a vector by ID
CREATE FUNCTION vector_delete(
  p_id text
) RETURNS void;

-- Get embeddings for a batch of IDs
CREATE FUNCTION vector_get_embeddings(
  p_ids text[]
) RETURNS TABLE(id text, embedding real[]);

-- Health check
CREATE FUNCTION ruvector_health()
RETURNS jsonb;
```

**⚠️ Critical gotcha:** Embeddings must be sent as TEXT `"[0.1,0.2,...]"` not as
JSON arrays. The ruvector assignment cast (`text::ruvector`) handles conversion
inside PostgreSQL. Sending a JSON array will fail with a type mismatch.

## Scope — Files to Create/Modify

### Files to Create

| File | Purpose | Lines (est.) |
|------|---------|-------------|
| `crates/librefang-memory/src/supabase_vector_store.rs` | `SupabaseVectorStore` implementing `VectorStore` trait | ~200 |

### Files to Modify

| File | Change | Lines (est.) |
|------|--------|-------------|
| `crates/librefang-memory/src/lib.rs` | Add `pub mod supabase_vector_store;` + re-export | ~3 |
| `crates/librefang-types/src/config/types.rs` | Add `supabase_url`, `supabase_anon_key` to `MemoryConfig` | ~10 |
| `crates/librefang-kernel/src/kernel.rs` | Add `"supabase"` match arm in backend init (lines 1668-1685) | ~15 |

**Total: 1 new file + 3 modified files, ~228 new lines, 0 new Cargo dependencies**

## SupabaseVectorStore Implementation Spec

```rust
// crates/librefang-memory/src/supabase_vector_store.rs

use async_trait::async_trait;
use librefang_types::error::LibreFangResult;
use librefang_types::memory::{MemoryFilter, VectorSearchResult, VectorStore};
use reqwest::Client;
use std::collections::HashMap;

/// Vector store backed by Supabase PostgREST RPC endpoints.
///
/// Calls ruvector SQL functions (vector_insert, vector_search, vector_delete,
/// vector_get_embeddings) via Supabase's auto-generated REST API.
///
/// Auth: Bearer token (anon key or service role key) in Authorization header.
/// RLS: Supabase Row Level Security enforces tenant isolation automatically.
///
/// # Configuration
/// ```toml
/// [memory]
/// vector_backend = "supabase"
/// supabase_url = "https://xxxxx.supabase.co"
/// supabase_anon_key = "eyJ..."
/// ```
#[derive(Clone)]
pub struct SupabaseVectorStore {
    client: Client,
    /// Supabase project URL (e.g., "https://xxxxx.supabase.co")
    base_url: String,
    /// Supabase anon key (or service role key for server-side)
    api_key: String,
}

impl SupabaseVectorStore {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            client: Client::new(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
        }
    }

    /// Build the full RPC URL.
    fn rpc_url(&self, function_name: &str) -> String {
        format!("{}/rest/v1/rpc/{}", self.base_url, function_name)
    }

    /// Common headers for all Supabase requests.
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert("apikey", self.api_key.parse().unwrap());
        h.insert("Authorization", format!("Bearer {}", self.api_key).parse().unwrap());
        h.insert("Content-Type", "application/json".parse().unwrap());
        h
    }

    /// Format embedding as TEXT string for ruvector cast.
    /// Input: &[f32] → Output: "[0.1,0.2,0.3]"
    fn format_embedding(embedding: &[f32]) -> String {
        let inner: Vec<String> = embedding.iter().map(|v| v.to_string()).collect();
        format!("[{}]", inner.join(","))
    }
}
```

## Acceptance Criteria

### Group 1: SupabaseVectorStore Implementation

#### AC-1: Insert delegates to RPC
- **Given:** A `SupabaseVectorStore` with valid Supabase credentials
- **When:** `insert("mem-1", &[0.1, 0.2, 0.3], "hello world", metadata)` is called
- **Then:** POST to `{base_url}/rest/v1/rpc/vector_insert` with body `{"p_id": "mem-1", "p_embedding": "[0.1,0.2,0.3]", "p_payload": "hello world", "p_metadata": {...}}`
- **And NOT:** Embedding sent as JSON array `[0.1, 0.2, 0.3]` — must be TEXT string

#### AC-2: Search delegates to RPC
- **Given:** A configured store
- **When:** `search(&[0.1, 0.2, 0.3], 5, None)` is called
- **Then:** POST to `{base_url}/rest/v1/rpc/vector_search` with body `{"p_query_embedding": "[0.1,0.2,0.3]", "p_limit": 5}`
- **And:** Response parsed into `Vec<VectorSearchResult>` with id, payload, score, metadata
- **And NOT:** Empty results treated as errors — empty vec is valid

#### AC-3: Delete delegates to RPC
- **Given:** A configured store
- **When:** `delete("mem-1")` is called
- **Then:** POST to `{base_url}/rest/v1/rpc/vector_delete` with body `{"p_id": "mem-1"}`

#### AC-4: Get embeddings delegates to RPC
- **Given:** A configured store with vectors stored
- **When:** `get_embeddings(&["mem-1", "mem-2"])` is called
- **Then:** POST to `{base_url}/rest/v1/rpc/vector_get_embeddings` with body `{"p_ids": ["mem-1", "mem-2"]}`
- **And:** Response parsed into `HashMap<String, Vec<f32>>`

#### AC-5: Backend name
- **Given:** A `SupabaseVectorStore`
- **When:** `backend_name()` is called
- **Then:** Returns `"supabase"`

#### AC-6: Embedding format is TEXT not JSON
- **Given:** An embedding `[0.1, 0.2, 0.3]`
- **When:** Formatted for Supabase RPC
- **Then:** Sent as string `"[0.1,0.2,0.3]"` — TEXT, not JSON array
- **And NOT:** Sent as `[0.1, 0.2, 0.3]` (raw JSON array) — this will fail ruvector cast

### Group 2: Configuration & Wiring

#### AC-7: Config fields added
- **Given:** `MemoryConfig` in `config/types.rs`
- **When:** TOML has `vector_backend = "supabase"`, `supabase_url = "https://..."`, `supabase_anon_key = "eyJ..."`
- **Then:** All three fields parsed into config struct
- **And NOT:** Panic on missing `supabase_url` when backend is not "supabase"

#### AC-8: Kernel init wires supabase backend
- **Given:** Config with `vector_backend = "supabase"`
- **When:** Kernel boots
- **Then:** `SupabaseVectorStore::new(url, key)` created and attached via `substrate.set_vector_store()`
- **And:** Log line: `"Vector store backend: supabase ({url})"`

#### AC-9: Missing config rejected at boot
- **Given:** Config with `vector_backend = "supabase"` but no `supabase_url`
- **When:** Kernel boots
- **Then:** `KernelError::BootFailed` with message naming the missing field
- **And NOT:** Silent fallback to sqlite — misconfiguration must be loud

#### AC-10: lib.rs exports
- **Given:** `librefang-memory/src/lib.rs`
- **When:** Compiled
- **Then:** `SupabaseVectorStore` is publicly exported alongside `HttpVectorStore` and `SqliteVectorStore`

### Group 3: End-to-End

#### AC-11: Insert then search returns result
- **Given:** A running Supabase instance with ruvector extension
- **When:** Insert a vector, then search with the same embedding
- **Then:** Search returns the inserted vector with score > 0.9
- **And NOT:** Score is exactly 1.0 (floating point — use > 0.99 threshold)

#### AC-12: Health check
- **Given:** A configured `SupabaseVectorStore`
- **When:** `ruvector_health()` RPC is called
- **Then:** Returns valid JSONB with version info
- **And NOT:** Network timeout treated as panic — graceful error return

## Claims Requiring Verification

| Claim | Verification Method | Test Name |
|-------|--------------------|-----------| 
| insert sends TEXT embedding | Unit test (mock HTTP) | `test_insert_sends_text_embedding` |
| search parses response correctly | Unit test (mock HTTP) | `test_search_parses_rpc_response` |
| delete calls correct RPC | Unit test (mock HTTP) | `test_delete_calls_rpc` |
| get_embeddings returns HashMap | Unit test (mock HTTP) | `test_get_embeddings_returns_hashmap` |
| backend_name returns "supabase" | Unit test | `test_backend_name` |
| config missing url → boot error | Unit test | `test_missing_supabase_url_fails_boot` |
| kernel wires supabase backend | Integration test | `test_kernel_init_supabase_backend` |
| insert-then-search round trip | Integration test (requires live Supabase) | `test_e2e_insert_search` |
| RLS isolation works | Integration test (requires live Supabase) | `test_rls_tenant_isolation` |

## Exit Gate

```bash
#!/bin/bash
set -e

# 1. SupabaseVectorStore compiles
cargo clippy -p librefang-memory --all-targets -- -D warnings

# 2. Unit tests pass (mock HTTP, no live Supabase needed)
cargo test -p librefang-memory -- supabase

# 3. Config fields exist
grep -q "supabase_url" crates/librefang-types/src/config/types.rs
grep -q "supabase_anon_key" crates/librefang-types/src/config/types.rs

# 4. Kernel handles "supabase" backend
grep -q '"supabase"' crates/librefang-kernel/src/kernel.rs

# 5. Public export exists
grep -q "SupabaseVectorStore" crates/librefang-memory/src/lib.rs

echo "SPEC-RV-002 EXIT GATE: ALL PASS"
```

## Out of Scope

| Excluded | Reason | When |
|----------|--------|------|
| Direct sqlx PgPool | Higher perf but more deps, more wiring | Phase 2 |
| Account-scoped vector namespaces | Needs AccountId in VectorStore trait | Phase 3 (after SPEC-MT-001) |
| Hybrid search (keyword + vector) | ruvector_register_hybrid() | Phase 4 (qwntik ADR-004) |
| Learning/feedback loops | ruvector_enable_learning() | Phase 4 |
| Graph/Cypher/SPARQL queries | Advanced ruvector features | Phase 4+ |
| Connection pooling tuning | Default reqwest pool is fine for now | When needed |

## Phase Roadmap (Supabase Vector Store)

| Phase | What | Deps |
|-------|------|------|
| **Phase 1** (this spec) | SupabaseVectorStore over HTTP/PostgREST | RPC functions deployed |
| **Phase 2** | Direct sqlx::PgPool — bypass HTTP, co-hosted perf | After account model |
| **Phase 3** | Account-scoped namespaces — RLS + account_id in VectorStore | After SPEC-MT-001 |
| **Phase 4** | Advanced ruvector — hybrid search, learning, graphs | After basic ops proven |

### ⚠️ RPC Function Naming Alignment (SPEC-MT-004)

**Important:** This SPEC uses `ruvector_insert` and `ruvector_search` as RPC endpoint names
in code examples. The actual qwntik Supabase deployment (SPEC-MT-004) uses `vector_insert`
and `vector_search` as the RPC function names.

| This SPEC (SPEC-RV-002) | Actual qwntik (SPEC-MT-004) | Reconciliation |
|--------------------------|-----------------------------|-----------------|
| `ruvector_insert(p_id, p_embedding, p_account_id)` | `vector_insert(doc_content, doc_embedding, doc_metadata, doc_user_id, doc_account_id)` | Use SPEC-MT-004 signatures — they match deployed migrations |
| `ruvector_search(query_embedding, match_limit)` | `vector_search(query_embedding, match_count, match_threshold, caller_user_id, caller_account_id)` | Use SPEC-MT-004 signatures — they include account_id for defense-in-depth |

**Resolution:** SPEC-MT-004 is the source of truth for RPC function signatures because it
reflects the actual deployed Supabase migrations in qwntik (`20260413_vector_rpc_account_id.sql`).
The `PostgresVectorStore` implementation in this SPEC should call `vector_insert` and
`vector_search` (not `ruvector_insert` / `ruvector_search`).

The `ruvector_*` prefix (e.g., `ruvector_embed()`, `ruvector_simd_info()`) is reserved
for the PostgreSQL extension's native functions (59 functions from the pgrx crate).
The `vector_*` prefix is for the PostgREST RPC wrapper functions that add account_id
parameters and defense-in-depth filtering.
