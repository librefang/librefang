# SPEC-RV-001: RuVector PostgreSQL Extension Port — Phase 0

**ADR:** ADR-RV-001 (RuVector PostgreSQL Extension Port)
**Date:** 2026-04-06
**Author:** Engineering

---

## Purpose

Port 7 RuVector Rust crates from the openfang-ai workspace into the librefang workspace,
build a Supabase-compatible Docker image with the PostgreSQL extension, and verify
semantic search + local embeddings work end-to-end via HTTP RPC.

## Source of Truth

| Item | Location | Notes |
|------|----------|-------|
| 7 Rust crates | `openfang-ai/crates/ruvector-*` | 3 files have unstaged changes — commit first |
| Dockerfile | `qwntik/docker/Dockerfile.supabase-ruvector` | Build context = repo root containing `crates/` |
| docker-compose.yml | `qwntik/docker/docker-compose.yml` | Line 485: db service, full Supabase stack |
| Migration SQL | `qwntik/apps/web/supabase/migrations/20260405_ruvector_setup.sql` | documents table + HNSW + RLS |
| Target workspace | claude-flow / librefang | New `crates/ruvector-*` directories |

## Verified Baseline (from openfang-ai source, 2026-04-06)

| Fact | Value | Verified By |
|------|-------|-------------|
| SQL functions | 161 live in `pg_proc` (197 in SQL file, but 161 verified via smoke test) | `SELECT count(*) FROM pg_proc WHERE proname LIKE 'ruvector_%'` |
| SQL operators | 11 `CREATE OPERATOR` (`<->`, `<=>`, `<#>`, `+`, `-`) | `grep -c` on `ruvector--0.3.0.sql` |
| SQL casts | 2 `CREATE CAST` (real[] ↔ ruvector, bidirectional) | `grep -c` on `ruvector--0.3.0.sql` |
| pgrx version | 0.12 | `Cargo.toml` workspace dep |
| cargo-pgrx | 0.12.6 (`--locked`) | Dockerfile `cargo install` line |
| Rust version | 1.88 (stable) | Dockerfile `ARG RUST_VERSION=1.88` + `rust-toolchain.toml` |
| PostgreSQL | 17 (`supabase/postgres:17.6.1.095`) | Dockerfile runtime `FROM` |
| Feature flag | `all-features-v3` | `Cargo.toml` features section |
| Build features | `pg17,index-all,quant-all,all-features-v3` | Dockerfile `cargo pgrx package` line |
| Embedding model | all-MiniLM-L6-v2 (384-dim, ONNX via `fastembed`) | `fastembed = "5"` in `Cargo.toml` |
| MAX_DIMENSIONS | 16,000 | `src/lib.rs` constant |
| GUCs | `ef_search`, `probes`, `hybrid_alpha`, `hybrid_rrf_k`, `hybrid_prefetch_k` | `src/lib.rs` `_PG_init()` |
| `embed_batch` | Functional — returns `real[][]` | Verified in openfang-ai runtime |
| Assignment cast | `real[]` → `ruvector` auto-converts on INSERT | `operators.rs` (unstaged) |
| Unstaged files | ~~All committed~~ (ff36068a, c493cdbc in openfang-ai) | `git log --oneline -2` |

### Pre-Work Requirement

Before starting the port, the 3 unstaged files in openfang-ai **must be committed**:

```bash
cd /Users/danielalberttis/Desktop/Projects/openfang-ai
git add crates/ruvector-postgres/src/operators.rs \
        crates/ruvector-postgres/sql/ruvector--0.3.0.sql \
        crates/ruvector-postgres/sql/ruvector--2.0.0--0.3.0.sql
git commit -m "feat(ruvector-postgres): add real[]<->ruvector cast functions + operator tests"
```

These contain the bidirectional cast functions and 8 operator tests that eliminate
limitations L1 and L2 from REPORT-033.

## Scope

### Crates to Port (7)

| Crate | Version | Role | Dependencies |
|-------|---------|------|--------------|
| ruvector-postgres | 0.3.0 | Hub — pgrx extension, SQL functions, GUCs | All 6 below (optional) |
| ruvector-solver | 2.0.4 | Sublinear solvers (Neumann, CG, forward-push) | serde, thiserror, tracing |
| ruvector-math | 2.0.4 | Distance metrics, TDA, nalgebra | nalgebra, rand |
| ruvector-attention | 2.0 | Flash attention, multi-head attention | ruvector-math |
| ruvector-sona | 0.1.9 | Self-optimizing neural adaptation | ruvector-math, serde |
| ruvector-domain-expansion | 2.0 | Domain expansion algorithms | ruvector-math, ruvector-solver |
| ruvector-mincut-gated-transformer | 0.1.0 | Gated transformer with min-cut | ruvector-attention, ruvector-math |

**Total: 7 crates, 161 SQL functions (verified live), 3 operators, 2 casts**

### Files to Create/Modify in Target

| File | Action | Purpose |
|------|--------|---------|
| `crates/ruvector-*` (7 dirs) | Copy from openfang-ai | Crate source code |
| `Cargo.toml` (workspace root) | Add members (commented) | Workspace integration |
| `docker/Dockerfile.supabase-ruvector` | Copy from qwntik, adapt paths | Docker build |
| `docker/docker-compose.yml` | Add/update db service | Supabase stack |
| `docker/sql/ruvector_setup.sql` | Copy from qwntik migration | Schema + HNSW + RLS |

## Acceptance Criteria

### Group 1: Crate Port (5 criteria)

#### AC-1.1: Workspace Compilation
- **Given:** 7 crates copied to target workspace, `Cargo.toml` members updated
- **When:** `cargo check -p ruvector-postgres --features all-features-v3`
- **Then:** Exit code 0, no errors
- **And NOT:** No unresolved path dependencies, no missing crate errors

#### AC-1.2: Clean Dependency Tree
- **Given:** All 7 crates in workspace
- **When:** `cargo tree -p ruvector-postgres --features all-features-v3`
- **Then:** Clean DAG, all ruvector-* crates resolve locally
- **And NOT:** No `[cycle]` markers in tree output

#### AC-1.3: Path Dependencies Resolve
- **Given:** All crate `Cargo.toml` files with `path = "../ruvector-*"` entries
- **When:** `cargo metadata -p ruvector-postgres --format-version 1`
- **Then:** All 6 optional deps resolve to local workspace paths
- **And NOT:** No references to crates.io or external registries for ruvector-* crates

#### AC-1.4: Feature Flags Gate Correctly
- **Given:** ruvector-postgres with `all-features-v3` defined in `[features]`
- **When:** `cargo check -p ruvector-postgres --no-default-features --features pg17`
- **Then:** Compiles with base-only functionality (no solver, math, attention, etc.)
- **And NOT:** No compile errors from missing optional deps when features disabled

#### AC-1.5: Crate Versions Match Source
- **Given:** Crates copied from openfang-ai (post unstaged-commit)
- **When:** Compare `version` field in each `Cargo.toml`
- **Then:** Exact match — solver 2.0.4, math 2.0.4, sona 0.1.9, postgres 0.3.0, attention 2.0, domain-expansion 2.0, mincut 0.1.0
- **And NOT:** No version bumps or modifications from source

### Group 2: Naming Standardization (4 criteria)

#### AC-2.1: No openfang References
- **Given:** All 7 crates copied and descriptions updated
- **When:** `grep -r "openfang" crates/ruvector-*`
- **Then:** Zero matches (empty output, exit code 1)
- **And NOT:** No references in Cargo.toml, comments, docs, README, or SQL

#### AC-2.2: SQL Function Prefix
- **Given:** Extension loaded in PostgreSQL
- **When:** `SELECT proname FROM pg_proc WHERE proname LIKE 'ruvector_%'`
- **Then:** All functions use `ruvector_` prefix
- **And NOT:** No functions with `openfang_`, `pgvector_`, or other prefixes

#### AC-2.3: Extension Name
- **Given:** Extension created via `CREATE EXTENSION ruvector`
- **When:** `SELECT extname FROM pg_extension WHERE extname = 'ruvector'`
- **Then:** Returns exactly 1 row
- **And NOT:** No extension named `openfang` or `pgvector`

#### AC-2.4: GUC Prefix
- **Given:** Extension loaded
- **When:** `SHOW ruvector.ef_search`
- **Then:** Returns default value `40`
- **And NOT:** No GUCs with `openfang.` prefix

### Group 3: Docker Image (5 criteria)

#### AC-3.1: Docker Build Succeeds
- **Given:** `Dockerfile.supabase-ruvector` in `docker/`, build context = workspace root
- **When:** `docker build -f docker/Dockerfile.supabase-ruvector --tag supabase-ruvector:latest .`
- **Then:** Build completes, image created
- **And NOT:** No compilation errors, no missing crate dependencies

#### AC-3.2: Extension Loads on Startup
- **Given:** Container started from `supabase-ruvector:latest`
- **When:** `CREATE EXTENSION IF NOT EXISTS ruvector`
- **Then:** Extension created successfully
- **And NOT:** No "could not load library" or "control file not found" errors

#### AC-3.3: SQL Objects Registered
- **Given:** Extension loaded
- **When:** `SELECT count(*) FROM pg_proc WHERE proname LIKE 'ruvector_%'`
- **Then:** Returns ≥ 161 (verified baseline from live Docker smoke test)
- **And NOT:** Count does not drop below 161

#### AC-3.4: SIMD Detection
- **Given:** Extension loaded
- **When:** `SELECT ruvector_simd_info()`
- **Then:** Returns detected architecture string (`avx2`, `avx512`, `neon`, or `none`)
- **And NOT:** No runtime crash, no NULL return

#### AC-3.5: Base Image Correct
- **Given:** Dockerfile
- **When:** Inspect runtime stage `FROM` line
- **Then:** `supabase/postgres:17.6.1.095` (or compatible 17.x variant)
- **And NOT:** Not vanilla `postgres:17` — must be Supabase variant for PostgREST compatibility

### Group 4: Local Embeddings (4 criteria)

#### AC-4.1: Embed Function Returns Float Array
- **Given:** Extension loaded, model auto-downloads on first call
- **When:** `SELECT ruvector_embed('hello world')`
- **Then:** Returns `real[]` with 384 elements
- **And NOT:** No external API calls — all computation is local ONNX inference

#### AC-4.2: Correct Model
- **Given:** Extension loaded
- **When:** First call to `ruvector_embed()` triggers model download/cache
- **Then:** Model is `all-MiniLM-L6-v2` (384-dim, ONNX via fastembed)
- **And NOT:** Not `all-mpnet-base-v2` (768-dim) — that is the ruvbot model, not the PG extension

#### AC-4.3: Dimension Consistency
- **Given:** Extension loaded
- **When:** `SELECT array_length(ruvector_embed('test'), 1)`
- **Then:** Returns exactly `384`
- **And NOT:** Does not return NULL, 0, or any other dimension

#### AC-4.4: Batch Embedding Functional
- **Given:** Extension loaded with `all-features-v3` enabled at build time
- **When:** `SELECT ruvector_embed_batch(ARRAY['hello', 'world'])`
- **Then:** Returns `real[][]` with 2 rows × 384 columns
- **And NOT:** Not a stub — returns actual embeddings, not NULLs or zero vectors

### Group 5: Semantic Search (4 criteria)

#### AC-5.1: HNSW Index Creation
- **Given:** Table with `ruvector` column (or `real[]` with assignment cast)
- **When:** `CREATE INDEX idx_docs_embedding ON documents USING hnsw (embedding ruvector_cosine_ops)`
- **Then:** Index created successfully
- **And NOT:** No "access method not found" or "operator class does not exist" errors

#### AC-5.2: Cosine Similarity Search
- **Given:** 3+ documents inserted with embeddings, HNSW index exists
- **When:** `SELECT content FROM documents ORDER BY embedding <=> ruvector_embed('query') LIMIT 3`
- **Then:** Returns results ranked by semantic similarity (most relevant first)
- **And NOT:** Results are not random — repeated queries produce deterministic ranking

#### AC-5.3: Insert + Search Round-Trip
- **Given:** Empty table with HNSW index
- **When:** INSERT document with `ruvector_embed('cats are pets')`, then search for `'feline animals'`
- **Then:** Inserted document appears as top result
- **And NOT:** Search does not return empty results or irrelevant documents

#### AC-5.4: Assignment Cast on INSERT
- **Given:** Table with `ruvector` column type
- **When:** `INSERT INTO documents (embedding) VALUES (ruvector_embed('test'))`
- **Then:** Succeeds without explicit `::ruvector` cast — assignment cast auto-converts `real[]` → `ruvector`
- **And NOT:** No "cannot cast type real[] to ruvector" error

### Group 6: REST Integration (4 criteria)

#### AC-6.1: PostgREST Exposes RPC Functions
- **Given:** Supabase stack running (PostgREST + PG), `GRANT EXECUTE` on ruvector functions to `anon`/`authenticated`
- **When:** `curl http://localhost:54321/rest/v1/rpc/ruvector_embed -d '{"input":"test"}' -H "apikey: $ANON_KEY" -H "Content-Type: application/json"`
- **Then:** Returns JSON array with 384 floats
- **And NOT:** No 404 (function not exposed) or 401 (missing grant)

#### AC-6.2: Search via REST
- **Given:** Documents in table, PostgREST running, `ruvector_search` wrapper function exposed
- **When:** POST to `/rest/v1/rpc/ruvector_search` with query text and limit
- **Then:** Returns JSON array of matching documents ranked by similarity
- **And NOT:** No 500 errors or empty results for known-matching queries

#### AC-6.3: RLS Account Isolation
- **Given:** RLS enabled on documents table, policies restrict by `account_id` from JWT claim
- **When:** Query as user with `account_id = 'A'`, table has docs from accounts A and B
- **Then:** Only account A's documents returned
- **And NOT:** Account B's documents never visible — not via search, not via direct SELECT

#### AC-6.4: Health Check
- **Given:** Supabase stack running
- **When:** `curl http://localhost:54321/rest/v1/` with valid apikey
- **Then:** Returns HTTP 200
- **And NOT:** No connection refused, no timeout, no 503

## Claims Requiring Verification

| # | Claim | Method | Test Command |
|---|-------|--------|-------------|
| C-1 | 7 crates compile with `all-features-v3` | `cargo check` | `cargo check -p ruvector-postgres --features all-features-v3` |
| C-2 | ≥161 functions registered in `pg_proc` | SQL count | `SELECT count(*) FROM pg_proc WHERE proname LIKE 'ruvector_%'` |
| C-3 | Embeddings return 384-dim vectors | SQL `array_length` | `SELECT array_length(ruvector_embed('test'), 1)` |
| C-4 | `embed_batch` returns `real[][]` (not stub) | SQL query | `SELECT array_length(ruvector_embed_batch(ARRAY['a','b']), 1)` |
| C-5 | Assignment cast works (`real[]` → `ruvector`) | SQL INSERT | `INSERT INTO t (emb) VALUES (ruvector_embed('x'))` where `emb` is `ruvector` type |
| C-6 | HNSW cosine search returns ranked results | SQL `ORDER BY <=>` | `SELECT content FROM t ORDER BY emb <=> ruvector_embed('q') LIMIT 3` |
| C-7 | RLS isolates accounts | SQL cross-account query | Set JWT claim to account A, query for account B data → 0 rows |

## Exit Gate

```bash
#!/usr/bin/env bash
set -euo pipefail
trap 'docker compose -f docker/docker-compose.yml down -v 2>/dev/null' EXIT

# --- Phase 0 Exit Gate ---
# All commands must exit 0.

echo "=== Gate 1: Crate compilation ==="
cargo check -p ruvector-postgres --features all-features-v3

echo "=== Gate 2: No openfang references ==="
if grep -r "openfang" crates/ruvector-* 2>/dev/null; then
  echo "FAIL: openfang references found"
  exit 1
fi

echo "=== Gate 3: Docker build ==="
docker build -f docker/Dockerfile.supabase-ruvector --tag supabase-ruvector:latest .

echo "=== Gate 4: Start stack ==="
docker compose -f docker/docker-compose.yml up -d
sleep 15  # wait for PG + PostgREST ready

export PGPASSWORD=postgres
PSQL="psql -h localhost -p 54322 -U postgres -d postgres -tAc"

echo "=== Gate 5: Extension loads ==="
$PSQL "CREATE EXTENSION IF NOT EXISTS ruvector"

echo "=== Gate 6: Function count ≥ 161 ==="
FCOUNT=$($PSQL "SELECT count(*) FROM pg_proc WHERE proname LIKE 'ruvector_%'" | tr -d ' ')
echo "Functions: $FCOUNT"
[ "$FCOUNT" -ge 161 ]

echo "=== Gate 7: Embedding dimensions = 384 ==="
EDIM=$($PSQL "SELECT array_length(ruvector_embed('test'), 1)" | tr -d ' ')
echo "Dimensions: $EDIM"
[ "$EDIM" -eq 384 ]

echo "=== Gate 8: HNSW index + cosine search ==="
$PSQL "DROP TABLE IF EXISTS gate_test"
$PSQL "CREATE TABLE gate_test (id serial, content text, emb ruvector)"
$PSQL "INSERT INTO gate_test (content, emb) VALUES ('cats are pets', ruvector_embed('cats are pets'))"
$PSQL "INSERT INTO gate_test (content, emb) VALUES ('dogs are loyal', ruvector_embed('dogs are loyal'))"
$PSQL "INSERT INTO gate_test (content, emb) VALUES ('neural networks', ruvector_embed('neural networks learn'))"
$PSQL "CREATE INDEX idx_gate ON gate_test USING hnsw (emb ruvector_cosine_ops)"
TOP=$($PSQL "SELECT content FROM gate_test ORDER BY emb <=> ruvector_embed('feline animals') LIMIT 1")
echo "Top result for 'feline animals': $TOP"
case "$TOP" in
  *cats*) ;;
  *) echo "FAIL: expected cats, got '$TOP'"; exit 1 ;;
esac
$PSQL "DROP TABLE IF EXISTS gate_test"

echo ""
echo "✅ Phase 0 exit gate: ALL PASSED"
```

## Out of Scope

| Item | Deferred To | Reason |
|------|------------|--------|
| `HttpVectorStore` Rust client in librefang-memory | Phase 1 | Requires memory crate design decisions |
| Production multi-tenant RLS policies | Phase 1 | Requires `account_id` schema from Phase 1 |
| Docker image size optimization | Phase 2 | 368 MB acceptable for Phase 0 (symlinks already applied) |
| CI/CD pipeline for extension build | Phase 2 | Manual `docker build` acceptable for Phase 0 |
| ONNX model pre-bundling in image | Phase 2 | First-call download acceptable for Phase 0 |
| `all-mpnet-base-v2` (768-dim) model support | Phase 3+ | Phase 0 uses MiniLM-L6 (384-dim) only |
