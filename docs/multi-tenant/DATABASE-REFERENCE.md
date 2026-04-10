# Database Reference — Supabase + RuVector

**Source of truth:** Live database at `localhost:54322` (PostgreSQL 17)  
**Last verified:** 2026-04-07  
**Image:** `supabase-ruvector:latest` (~1.86 GB)  
**Extension:** ruvector 0.3.0 (161 native SQL functions)

---

## Architecture Overview

```
┌─────────────────────────────┐     PostgREST      ┌──────────────────────────────────────┐
│   LibreFang Daemon          │  ◄──── HTTP ────►  │  Kong → PostgREST → PostgreSQL 17    │
│                             │   :54321            │  :54322                               │
│  SupabaseVectorStore ───────┤  /rpc/vector_*     │  ┌─────────────────────────────────┐  │
│  (reqwest + retry)          │                     │  │  ruvector extension v0.3.0      │  │
│                             │                     │  │  • 161 native SQL functions     │  │
│  Config:                    │                     │  │  • HNSW cosine index            │  │
│    vector_backend="supabase"│                     │  │  • all-MiniLM-L6-v2 (384-dim)  │  │
│    vector_store_url=...     │                     │  │  • NEON SIMD acceleration       │  │
│    vector_dimensions=384    │                     │  └─────────────────────────────────┘  │
│                             │                     │  RLS policies enforce tenant isolation│
└─────────────────────────────┘                     └──────────────────────────────────────┘
```

**Two function namespaces:**
- `ruvector_*` (161 functions) — Extension-native: embeddings, distances, HNSW, graphs, SONA, attention, solvers
- `vector_*` (4 RPC wrappers) — Application-level: insert, search, delete, insert_batch — called via PostgREST

---

## 1. Extensions Installed

| Extension | Version | Purpose |
|---|---|---|
| **ruvector** | **0.3.0** | Vector type, HNSW index, 161 SQL functions, local embeddings, SONA engine |
| pg_cron | 1.6.4 | Scheduled jobs |
| pg_graphql | 1.5.11 | GraphQL API |
| pg_net | 0.20.0 | HTTP requests from SQL |
| pg_stat_statements | 1.11 | Query performance |
| pgcrypto | 1.3 | Cryptographic functions |
| supabase_vault | 0.3.1 | Secret management |
| unaccent | 1.1 | Text search normalization |
| uuid-ossp | 1.1 | UUID generation |

---

## 2. The `documents` Table

Core vector storage table. Every embedding lives here.

```sql
CREATE TABLE public.documents (
    id          BIGINT       NOT NULL DEFAULT nextval('documents_id_seq'),
    user_id     UUID         NOT NULL,          -- FK → auth.users(id) ON DELETE CASCADE
    content     TEXT         NOT NULL,          -- original document text
    embedding   ruvector     NOT NULL,          -- 384-dim vector (auto-normalized on insert)
    metadata    JSONB        DEFAULT '{}',      -- arbitrary JSON (librefang_id, source, etc.)
    created_at  TIMESTAMPTZ  DEFAULT now(),
    updated_at  TIMESTAMPTZ  DEFAULT now(),
    account_id  UUID,                           -- FK → accounts(id) ON DELETE CASCADE

    CONSTRAINT documents_pkey PRIMARY KEY (id)
);
```

### Indexes

| Index | Type | Definition |
|---|---|---|
| `documents_pkey` | B-tree | `PRIMARY KEY (id)` |
| `idx_documents_embedding_hnsw` | **HNSW** | `USING hnsw (embedding ruvector_cosine_ops)` |
| `idx_documents_user_id` | B-tree | `(user_id)` |
| `idx_documents_account_id` | B-tree | `(account_id)` |

### Foreign Keys

| Constraint | Column | References | On Delete |
|---|---|---|---|
| `documents_user_id_fkey` | `user_id` | `auth.users(id)` | CASCADE |
| `documents_account_id_fkey` | `account_id` | `public.accounts(id)` | CASCADE |

### RLS Policies

| Policy | Command | Rule |
|---|---|---|
| `documents_select` | SELECT | `account_id IN (SELECT account_id FROM user_agents WHERE user_id = auth.uid())` |
| `documents_insert` | INSERT | Same — `WITH CHECK` |
| `documents_update` | UPDATE | Same — both `USING` and `WITH CHECK` |
| `documents_delete` | DELETE | Same |
| `documents_service_role` | ALL | `true` — service_role bypasses RLS |

**Tenant isolation:** Users can only see documents belonging to accounts they have agents in (`user_agents` join). Service role sees everything.

---

## 3. Application RPC Functions (PostgREST API)

These are the 4 functions called by `SupabaseVectorStore` via HTTP.

### `vector_insert`

```sql
CREATE FUNCTION vector_insert(
    doc_content    TEXT    DEFAULT '',
    doc_embedding  TEXT    DEFAULT '[]',    -- "[0.1,0.2,...]" NOT JSON array
    doc_metadata   JSONB   DEFAULT '{}',
    doc_user_id    UUID    DEFAULT NULL,    -- falls back to auth.uid()
    doc_account_id UUID    DEFAULT NULL
) RETURNS BIGINT
```

**Behavior:**
1. `effective_uid = COALESCE(doc_user_id, auth.uid())` — must not be NULL
2. Embedding auto-normalized: `ruvector_normalize(doc_embedding::ruvector)`
3. Returns new row `BIGINT` ID
4. Raises exception if no authentication

**Grants:** `authenticated`, `service_role`, `postgres` only (NOT `anon`)

### `vector_insert_batch`

```sql
CREATE FUNCTION vector_insert_batch(
    doc_contents   TEXT[]  DEFAULT '{}',
    doc_embeddings TEXT[]  DEFAULT '{}',
    doc_metadatas  JSONB[] DEFAULT '{}',
    doc_user_id    UUID    DEFAULT NULL,
    doc_account_id UUID    DEFAULT NULL
) RETURNS BIGINT[]
```

**Behavior:**
1. Validates `doc_contents` and `doc_embeddings` have same length
2. Single `effective_uid` and `account_id` applied to ALL rows
3. Each embedding individually normalized
4. Returns array of new row IDs
5. Raises exception on array length mismatch or no auth

**Grants:** `authenticated`, `service_role`, `postgres` only

### `vector_search`

```sql
CREATE FUNCTION vector_search(
    query_embedding    TEXT    ,           -- "[0.1,0.2,...]" NOT JSON array
    match_count        INTEGER DEFAULT 10,
    match_threshold    REAL    DEFAULT 0.3, -- cosine DISTANCE threshold
    caller_user_id     UUID    DEFAULT NULL,
    caller_account_id  UUID    DEFAULT NULL
) RETURNS TABLE(id BIGINT, content TEXT, metadata JSONB, distance REAL)
```

**Behavior:**
1. Computes `ruvector_cosine_distance(d.embedding, query_embedding::ruvector)` for each document
2. Filters: `distance < match_threshold` (lower = more similar)
3. Optional tenant filter: `d.user_id = caller_user_id AND/OR d.account_id = caller_account_id`
4. Returns results ordered by distance ASC, limited to `match_count`
5. Uses HNSW index for approximate nearest neighbor lookup

**Score conversion in Rust:** `score = (1.0 - distance).clamp(0.0, 1.0)`

**Grants:** `authenticated`, `service_role`, `postgres` only

### `vector_delete`

```sql
CREATE FUNCTION vector_delete(
    doc_id BIGINT DEFAULT 0
) RETURNS BOOLEAN
```

**Behavior:**
1. Looks up document owner (`user_id`, `account_id`)
2. Authorization: caller must own via `user_id = auth.uid()` OR `has_role_on_account(account_id)`
3. Returns `true` if deleted, `false` if not found
4. Raises exception if caller doesn't own the document
5. **Idempotent:** non-existent IDs return `false`, not an error

**Grants:** `authenticated`, `service_role`, `postgres` only

---

## 4. Security Model

### Grant Matrix

| Function | `anon` | `authenticated` | `service_role` | `postgres` |
|---|---|---|---|---|
| `vector_insert` | ❌ | ✅ | ✅ | ✅ |
| `vector_insert_batch` | ❌ | ✅ | ✅ | ✅ |
| `vector_search` | ❌ | ✅ | ✅ | ✅ |
| `vector_delete` | ❌ | ✅ | ✅ | ✅ |
| `vector_add`, `vector_norm`, etc. | ✅ | ✅ | ✅ | ✅ |
| All `ruvector_*` | ✅ | ✅ | ✅ | ✅ |

**Key security properties:**
- The 4 application RPCs are **NOT** callable by `anon` — requires authentication
- Pure math functions (`vector_add`, `vector_norm`, etc.) are public — no data access
- `vector_delete` has **authorization check inside the function** — prevents deleting other users' documents
- RLS on `documents` table provides defense-in-depth for direct table access

### Embedding Format (Critical)

⚠️ **Embeddings MUST be sent as TEXT strings, not JSON arrays.**

```
✅ Correct: "[0.1,0.2,0.3]"     ← TEXT string, PG casts to ruvector
❌ Wrong:   [0.1, 0.2, 0.3]     ← JSON array, will fail type cast
```

The `::ruvector` cast in PostgreSQL expects the `[val,val,val]` TEXT format.

---

## 5. RuVector Extension — 161 Native Functions

Grouped by capability:

### Embeddings (6 functions)

Local model — zero API calls, zero cost.

| Function | Purpose |
|---|---|
| `ruvector_embed(text, model_name)` | Generate 384-dim embedding from text |
| `ruvector_embed_batch(text[], model_name)` | Batch embedding generation |
| `ruvector_embedding_dims(model_name)` | Get dimensions for a model |
| `ruvector_embedding_models()` | List available models |
| `ruvector_embedding_stats()` | Model usage statistics |
| `ruvector_embedding_drift(old, new)` | Measure embedding drift between sets |

**Model:** `all-MiniLM-L6-v2` (384 dimensions, ONNX Runtime, NEON SIMD on ARM)

### Distance Functions (2)

| Function | Purpose |
|---|---|
| `ruvector_cosine_distance(a, b)` | Cosine distance [0, 2] |
| `ruvector_l2_distance(a, b)` | Euclidean distance |

### Graph Database (11 functions)

Full property graph + RDF/SPARQL support inside PostgreSQL.

| Function | Purpose |
|---|---|
| `ruvector_create_graph(name)` | Create named graph |
| `ruvector_add_node(graph, labels, props)` | Add node with labels |
| `ruvector_add_edge(graph, src, tgt, type, props)` | Add typed edge |
| `ruvector_cypher(graph, query, params)` | Execute Cypher queries |
| `ruvector_sparql(store, query)` | Execute SPARQL queries |
| `ruvector_sparql_json(store, query)` | SPARQL → JSON results |
| `ruvector_sparql_update(store, update)` | SPARQL UPDATE |
| `ruvector_graph_pagerank(graph)` | PageRank computation |
| `ruvector_graph_centrality(graph)` | Centrality metrics |
| `ruvector_graph_diffusion(graph)` | Diffusion on graph |
| `ruvector_graph_stats(graph)` | Graph statistics |

### SONA Engine (4 functions)

Self-Organizing Neural Architecture — in-database learning.

| Function | Purpose |
|---|---|
| `ruvector_sona_learn(...)` | Train on new data |
| `ruvector_sona_apply(...)` | Apply learned model |
| `ruvector_sona_ewc_status(...)` | EWC++ catastrophic forgetting status |
| `ruvector_sona_stats(...)` | SONA engine statistics |

### Learning System (3 functions)

| Function | Purpose |
|---|---|
| `ruvector_enable_learning(table, config)` | Enable feedback learning on a table |
| `ruvector_clear_learning(table)` | Reset learning data |
| `ruvector_learning_stats(...)` | Learning metrics |

### RDF Knowledge Store (3 functions)

| Function | Purpose |
|---|---|
| `ruvector_create_rdf_store(name)` | Create triple store |
| `ruvector_delete_rdf_store(name)` | Delete triple store |
| `ruvector_rdf_stats(...)` | Triple store statistics |

### Attention Mechanisms (6+ functions)

| Function | Purpose |
|---|---|
| `ruvector_cross_attention(q, keys, values)` | Cross-attention |
| `ruvector_linear_attention(...)` | O(n) linear attention |
| `ruvector_sparse_attention(...)` | Sparse attention |
| `ruvector_sliding_window_attention(...)` | Local window attention |
| `ruvector_hyperbolic_attention(...)` | Hyperbolic space attention |
| `ruvector_moe_attention(...)` | Mixture-of-experts attention |
| `ruvector_attention_benchmark(dim, seq, type)` | Benchmark attention types |

### Sublinear Solvers (5+ functions)

| Function | Purpose |
|---|---|
| `ruvector_solve_sparse(...)` | Sparse linear system solver |
| `ruvector_solve_laplacian(...)` | Graph Laplacian solver |
| `ruvector_conjugate_gradient(...)` | CG iterative solver |
| `ruvector_matrix_analyze(...)` | Matrix property analysis |
| `ruvector_solver_info(...)` | Solver capabilities |

### Topological Data Analysis (5 functions)

| Function | Purpose |
|---|---|
| `ruvector_persistent_homology(...)` | Compute persistence diagrams |
| `ruvector_betti_numbers(...)` | Betti numbers |
| `ruvector_vietoris_rips(...)` | Vietoris-Rips complex |
| `ruvector_bottleneck_distance(...)` | Bottleneck distance between diagrams |
| `ruvector_topological_summary(...)` | TDA summary statistics |

### Optimal Transport (4 functions)

| Function | Purpose |
|---|---|
| `ruvector_wasserstein_distance(...)` | Wasserstein (earth mover's) distance |
| `ruvector_sinkhorn_distance(...)` | Sinkhorn divergence |
| `ruvector_sliced_wasserstein(...)` | Sliced Wasserstein |
| `ruvector_gromov_wasserstein(...)` | Gromov-Wasserstein |

### Hyperbolic / Manifold Geometry (8+ functions)

| Function | Purpose |
|---|---|
| `ruvector_poincare_distance(...)` | Poincaré ball distance |
| `ruvector_lorentz_distance(...)` | Lorentz model distance |
| `ruvector_poincare_to_lorentz(...)` | Model conversion |
| `ruvector_lorentz_to_poincare(...)` | Model conversion |
| `ruvector_mobius_add(...)` | Möbius addition |
| `ruvector_exp_map(...)` | Exponential map |
| `ruvector_log_map(...)` | Logarithmic map |
| `ruvector_spherical_distance(...)` | Spherical distance |
| `ruvector_product_manifold_distance(...)` | Product manifold |

### GNN / Message Passing (4 functions)

| Function | Purpose |
|---|---|
| `ruvector_gnn_aggregate(...)` | GNN aggregation |
| `ruvector_gnn_batch_forward(...)` | Batch GNN forward pass |
| `ruvector_gcn_forward(...)` | GCN forward pass |
| `ruvector_graphsage_forward(...)` | GraphSAGE forward |
| `ruvector_message_pass(...)` | Generic message passing |

### Multi-Tenant Support (12 functions)

| Function | Purpose |
|---|---|
| `ruvector_tenant_create(...)` | Create tenant |
| `ruvector_tenant_delete(...)` | Delete tenant |
| `ruvector_tenant_set(...)` | Set active tenant |
| `ruvector_tenant_stats(...)` | Tenant statistics |
| `ruvector_tenant_isolate(...)` | Isolate tenant data |
| `ruvector_tenant_migrate(...)` | Migrate tenant |
| `ruvector_tenant_migration_status(...)` | Migration progress |
| `ruvector_tenant_quota_check(...)` | Check quotas |
| `ruvector_tenant_update_quota(...)` | Update quotas |
| `ruvector_tenant_suspend(...)` | Suspend tenant |
| `ruvector_tenant_resume(...)` | Resume tenant |
| `ruvector_tenants()` | List all tenants |
| `ruvector_enable_tenant_rls(table, col)` | Generate RLS for tenant column |
| `ruvector_generate_rls_sql(...)` | Generate RLS SQL |
| `ruvector_generate_roles_sql(...)` | Generate role SQL |

### Agent Routing (6 functions)

| Function | Purpose |
|---|---|
| `ruvector_register_agent(...)` | Register agent with capabilities |
| `ruvector_register_agent_full(...)` | Full agent registration |
| `ruvector_route(...)` | Route query to best agent |
| `ruvector_find_agents_by_capability(...)` | Find agents by capability |
| `ruvector_list_agents()` | List all agents |
| `ruvector_get_agent(...)` | Get agent details |
| `ruvector_routing_stats()` | Routing statistics |

### Self-Healing (12 functions)

| Function | Purpose |
|---|---|
| `ruvector_healing_enable(...)` | Enable self-healing |
| `ruvector_healing_execute(...)` | Execute healing |
| `ruvector_healing_configure(...)` | Configure thresholds |
| `ruvector_healing_stats(...)` | Healing statistics |
| `ruvector_healing_history(...)` | Healing event history |
| `ruvector_healing_strategies(...)` | Available strategies |
| `ruvector_healing_effectiveness(...)` | Effectiveness metrics |
| `ruvector_is_healthy(...)` | Health check |
| `ruvector_health_status(...)` | Detailed health status |

### Hybrid Search (6 functions)

| Function | Purpose |
|---|---|
| `ruvector_hybrid_search(...)` | Combined keyword + vector search |
| `ruvector_hybrid_configure(...)` | Configure fusion parameters |
| `ruvector_hybrid_score(...)` | Score fusion |
| `ruvector_hybrid_list(...)` | List configurations |
| `ruvector_hybrid_stats(...)` | Hybrid search stats |
| `ruvector_register_hybrid(...)` | Register hybrid index |

### Utility

| Function | Purpose |
|---|---|
| `ruvector_version()` | Extension version |
| `ruvector_simd_info()` | SIMD acceleration status |
| `ruvector_system_metrics()` | System performance metrics |
| `ruvector_memory_stats()` | Memory usage |
| `ruvector_auto_tune(table, optimize_for, queries)` | Auto-tune index parameters |
| `ruvector_get_search_params()` | Current search parameters |

---

## 6. GUC Settings (Runtime Tuning)

| Setting | Default | Description |
|---|---|---|
| `ruvector.ef_search` | `100` | HNSW ef parameter — higher = more accurate, slower |
| `ruvector.probes` | `1` | IVFFlat probes (if IVF index used) |
| `ruvector.hybrid_alpha` | `0.5` | Hybrid search: 0=keyword, 1=vector |
| `ruvector.hybrid_prefetch_k` | `100` | Prefetch K results per branch |
| `ruvector.hybrid_rrf_k` | `60` | Reciprocal Rank Fusion constant |

```sql
-- Tune for higher recall at search time
SET ruvector.ef_search = 200;
```

---

## 7. Full Table Inventory (36 tables)

### Core Platform

| Table | RLS | FK → accounts | Purpose |
|---|---|---|---|
| `accounts` | ✅ | — | Organizations / tenants |
| `accounts_memberships` | ✅ | ✅ | User ↔ account membership |
| `roles` | ✅ | — | Role definitions |
| `role_permissions` | ✅ | — | Permission matrix |
| `invitations` | ✅ | ✅ | Pending invites |

### AI / Agent System

| Table | RLS | FK → accounts | Purpose |
|---|---|---|---|
| **`documents`** | ✅ | ✅ | **Vector embeddings** (ruvector HNSW) |
| `user_agents` | ✅ | ✅ | Agent instances per user |
| `agent_kv` | ✅ | ✅ | Agent key-value store |
| `agent_allowlist` | ✅ | ✅ | Agent permission allowlist |
| `models` | ✅ | ✅ | LLM model configurations |
| `skills` | ✅ | ✅ | Agent skills |
| `skill_versions` | ✅ | — | Skill version history |
| `knowledge` | ✅ | ✅ | Knowledge base entries |
| `sessions` | ✅ | ✅ | Chat sessions |
| `user_llm_keys` | ✅ | ✅ | User API keys for LLMs |
| `usage_log` | ✅ | ✅ | Token / API usage tracking |
| `api_audit_logs` | ✅ | ✅ | API audit trail |

### Brain System

| Table | RLS | Purpose |
|---|---|---|
| `brain_memories` | — | Long-term memory store |
| `brain_nodes` | — | Knowledge graph nodes |
| `brain_lora` | — | LoRA adapter metadata |
| `brain_contributors` | — | Contributor tracking |
| `brain_votes` | — | Feedback votes |
| `brain_page_status` | — | Page processing status |

### Billing

| Table | RLS | FK → accounts | Purpose |
|---|---|---|---|
| `billing_customers` | ✅ | ✅ | Stripe customer records |
| `subscriptions` | ✅ | ✅ | Active subscriptions |
| `subscription_items` | ✅ | — | Line items |
| `orders` | ✅ | ✅ | One-time purchases |
| `order_items` | ✅ | — | Order line items |

### Other

| Table | RLS | Purpose |
|---|---|---|
| `config` | ✅ | App configuration |
| `notifications` | ✅ | User notifications |
| `onboarding` | ✅ | Onboarding state |
| `nonces` | ✅ | Auth nonces |
| `creator_libraries` | ✅ | Creator marketplace |
| `access_grants` | ✅ | Library access grants |
| `account_features` | ✅ | Feature flags per account |
| `feature_entitlements` | ✅ | Global feature entitlements |

---

## 8. LibreFang ↔ Database Integration

### Rust Client Configuration

```toml
# librefang config.toml
[memory]
vector_backend = "supabase"
vector_store_url = "http://localhost:54321/rest/v1"  # Kong/PostgREST
vector_store_api_key_env = "SUPABASE_ANON_KEY"       # env var name
vector_dimensions = 384                                # validates before HTTP
```

### Data Flow: Insert

```
Rust: store.insert(id, embedding, payload, metadata)
  ├─ validate_embedding(384 dims?)
  ├─ embedding_to_text → "[0.1,0.2,...]"
  ├─ stash id in metadata["librefang_id"]
  ├─ extract user_id, account_id from metadata
  └─ POST /rest/v1/rpc/vector_insert
       ├─ Headers: apikey + Bearer token
       └─ Body: {doc_content, doc_embedding, doc_metadata, doc_user_id, doc_account_id}

PostgreSQL:
  ├─ COALESCE(doc_user_id, auth.uid()) → effective_uid
  ├─ ruvector_normalize(doc_embedding::ruvector)
  ├─ INSERT INTO documents
  └─ RETURN new BIGINT id
```

### Data Flow: Search

```
Rust: store.search(query_embedding, limit, filter)
  ├─ validate_embedding(384 dims?)
  ├─ cap limit to i32::MAX
  ├─ extract caller_user_id, caller_account_id from filter
  └─ POST /rest/v1/rpc/vector_search

PostgreSQL:
  ├─ HNSW index scan (ruvector_cosine_ops)
  ├─ WHERE distance < match_threshold
  ├─ AND (user_id = caller_user_id if provided)
  ├─ AND (account_id = caller_account_id if provided)
  └─ RETURN {id, content, metadata, distance}

Rust:
  ├─ score = (1.0 - distance).clamp(0.0, 1.0)
  ├─ original_id = metadata["librefang_id"]
  └─ Vec<VectorSearchResult>
```

### Data Flow: Delete

```
Rust: store.delete("42")  ← Supabase BIGINT row ID
  ├─ parse "42" → i64 (InvalidInput if not a number)
  └─ POST /rest/v1/rpc/vector_delete {doc_id: 42}

PostgreSQL:
  ├─ Look up document owner
  ├─ Check: user_id = auth.uid() OR has_role_on_account(account_id)
  ├─ DELETE FROM documents WHERE id = 42
  └─ RETURN true/false

Rust:
  ├─ false → log warning (idempotent)
  └─ Ok(())
```

### Retry Logic

- 1 retry on 5xx, timeout, or connection error
- 500ms delay between attempts
- Immediate failure on 4xx (client error, not transient)
- All RPCs use `send_with_retry`

### ID Semantics

| Context | ID Type | Example |
|---|---|---|
| `VectorStore::insert(id, ...)` | Application string | `"mem-uuid-42"` |
| `metadata["librefang_id"]` | Stashed app ID | `"mem-uuid-42"` |
| `documents.id` (DB) | Auto-increment BIGINT | `42` |
| `VectorSearchResult.id` | DB row ID as string | `"42"` |
| `VectorStore::delete(id)` | DB row ID as string | `"42"` |

---

## 9. Docker Infrastructure

### Ownership Split

| Component | Repo | Responsibility |
|---|---|---|
| `docker/Dockerfile.supabase-ruvector` | **librefang** | Builds PG image with ruvector.so |
| `docker/docker-compose.yml` | **librefang** | Local dev compose |
| `migrations/` (all SQL) | **Qwntik** | Tables, RPCs, RLS, indexes |
| `zz-ruvector-init.sql` (baked in image) | **librefang** | `CREATE EXTENSION ruvector` only |

### Build & Run

```bash
# Build the custom image
docker build -f docker/Dockerfile.supabase-ruvector -t supabase-ruvector:latest .

# Start the database
docker compose -f docker/docker-compose.yml up -d

# Connect
psql -h localhost -p 54322 -U postgres -d postgres

# Verify extension
SELECT ruvector_version();    -- 0.3.0
SELECT ruvector_simd_info();  -- NEON SIMD status
SELECT ruvector_dims(ruvector_embed('hello world'));  -- 384
```

---

## 10. Quick Reference: Common Operations

```sql
-- Generate an embedding (in-database, zero API cost)
SELECT ruvector_embed('What is machine learning?');

-- Insert a document with embedding
SELECT vector_insert(
    'Machine learning is a branch of AI',
    ruvector_embed('Machine learning is a branch of AI')::text,
    '{"source": "wiki"}'::jsonb,
    '00000000-0000-0000-0000-000000000001'::uuid,  -- user_id
    NULL  -- account_id
);

-- Semantic search
SELECT * FROM vector_search(
    ruvector_embed('What is AI?')::text,
    5,     -- top 5
    0.5,   -- distance threshold
    NULL,  -- any user
    NULL   -- any account
);

-- Graph operations
SELECT ruvector_create_graph('knowledge');
SELECT ruvector_add_node('knowledge', ARRAY['Concept'], '{"name": "AI"}'::jsonb);
SELECT ruvector_cypher('knowledge', 'MATCH (n) RETURN n LIMIT 10', '{}');

-- SPARQL query
SELECT ruvector_sparql('mystore', 'SELECT ?s ?p ?o WHERE { ?s ?p ?o } LIMIT 10');

-- Tune HNSW for higher recall
SET ruvector.ef_search = 200;

-- Auto-tune for a table
SELECT ruvector_auto_tune('documents', 'recall', ARRAY[0.1, 0.2, 0.3]::real[]);

-- Check system health
SELECT ruvector_health_status();
SELECT ruvector_system_metrics();
SELECT ruvector_memory_stats();
```

---

## Cross-References

| Document | What |
|---|---|
| `ADR-RV-001-RUVECTOR-EXTENSION-PORT.md` | Decision to port ruvector into workspace |
| `SPEC-RV-001-PHASE-0.md` | 26 acceptance criteria for extension port |
| `SPEC-RV-002-SUPABASE-VECTOR-STORE.md` | SupabaseVectorStore implementation spec |
| `SPEC-MT-004-SUPABASE-RLS-POLICIES.md` | RLS policy design |
| `docker/sql/README.md` | Schema ownership split (librefang vs Qwntik) |
| `crates/librefang-memory/src/supabase_vector_store.rs` | Rust implementation (1370 lines, 53 tests) |
