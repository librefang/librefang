# ADR-RV-001: RuVector PostgreSQL Extension Port

**Status:** Proposed
**Date:** 2026-04-06
**Author:** Engineering
**Related:** MASTER-PLAN.md, SPEC-RV-001 (Acceptance Criteria), SPEC-RV-002 (Supabase Vector Store), PLAN-RV-001 (Phase 0 Tasks)
**Epic:** Multi-Tenant Architecture — Phase 0

---

## Problem Statement

LibreFang's memory system supports vector search via the `VectorStore` trait and
`HttpVectorStore` HTTP client, but has **no self-hosted vector backend**. Users
must either:

1. Use the built-in `SqliteVectorStore` (local, no RLS, no multi-tenant isolation)
2. Point `HttpVectorStore` at an external service they provision themselves

The openfang-ai fork already ported 7 Rust crates from the ruvector upstream into
its workspace. These crates compile into a PostgreSQL extension (`ruvector.so`)
providing 161+ live SQL functions for vector operations, HNSW indexing, local
embeddings (384-dim all-MiniLM-L6-v2), attention mechanisms, and learning systems.

LibreFang needs this extension to:
- Provide a production-grade self-hosted vector backend (Supabase PostgreSQL 17)
- Enable multi-tenant memory isolation via Supabase RLS policies keyed on `account_id`
- Eliminate external API dependency for embeddings (all local, zero cost)
- Support the `HttpVectorStore` → Supabase RPC integration path (SPEC-RV-002)

### Source files verified (2026-04-06)

| Component | Location | Key Finding |
|-----------|----------|-------------|
| `VectorStore` trait | `librefang-types/src/memory.rs:1228-1348` | 5-method interface: insert, search, delete, get_embeddings, health |
| `HttpVectorStore` | `librefang-memory/src/http_vector_store.rs:1-259` | Working HTTP client, ready for Supabase RPC |
| Kernel backend match | `librefang-kernel/src/kernel.rs:1668-1685` | `vector_backend` config selects SqliteVectorStore or HttpVectorStore |
| Config fields | `librefang-types/src/config/types.rs:3145-3157` | `vector_backend` + `vector_store_url` already exist |
| `MemorySubstrate` | `librefang-memory/src/substrate.rs:110-114` | `set_vector_store()` — hot-swappable backend |
| openfang 7 crates | `openfang-ai/crates/ruvector-*` | 3 files have unstaged changes — commit before port |
| Dockerfile | `qwntik/docker/Dockerfile.supabase-ruvector` | Builds PG extension from Rust 1.88 + pgrx 0.12.6 |
| Migration SQL | `qwntik/apps/web/supabase/migrations/20260405_ruvector_setup.sql` | documents table + HNSW + RLS |

---

## Blast Radius Scan

This ADR is **additive** — it introduces new workspace members and a Docker image.
No existing LibreFang binary code changes. The blast radius is the workspace
configuration and Docker infrastructure.

### New Files (7 crates + Docker)

| Component | Files | LOC (approx) |
|-----------|-------|---------------|
| `crates/ruvector-postgres/` | ~40 files | ~8,000 |
| `crates/ruvector-solver/` | ~15 files | ~2,500 |
| `crates/ruvector-math/` | ~12 files | ~1,800 |
| `crates/ruvector-attention/` | ~10 files | ~1,500 |
| `crates/ruvector-sona/` | ~8 files | ~1,200 |
| `crates/ruvector-domain-expansion/` | ~6 files | ~800 |
| `crates/ruvector-mincut-gated-transformer/` | ~4 files | ~600 |
| `docker/Dockerfile.supabase-ruvector` | 1 file | ~80 |
| `docker/docker-compose.yml` | 1 file (modify) | +30 lines |
| **Total** | ~97 files | ~16,500 |

### Existing Files Modified

| File | Change | Risk |
|------|--------|------|
| `Cargo.toml` (workspace root) | Add 7 workspace members (commented by default) | LOW — no effect unless uncommented |
| `docker/docker-compose.yml` | Add `supabase-ruvector` service | LOW — additive service |
| `.gitignore` | Add `target/` patterns for pgrx build artifacts | NONE |

**Scope decision:** Zero existing Rust source files touched. The extension runs
*inside PostgreSQL*, not inside the LibreFang binary. Integration is via HTTP
(existing `HttpVectorStore`).

---

## Decision

**Port the 7 ruvector crates into the librefang workspace as optional workspace
members. Build the PG extension via Docker. Connect at runtime via the existing
`HttpVectorStore`.**

### Crates to Port

| Crate | Version | Purpose | Dependencies |
|-------|---------|---------|-------------|
| `ruvector-postgres` | 0.3.0 | PG extension hub — 161+ live SQL functions via pgrx 0.12.6 | All below (optional) |
| `ruvector-solver` | 2.0.4 | Sublinear sparse linear system solvers | None |
| `ruvector-math` | 2.0.4 | Optimal transport, information geometry, manifolds | None |
| `ruvector-attention` | 2.0.4 | 39 attention mechanisms | ruvector-math (optional) |
| `ruvector-sona` | 0.1.9 | SONA engine — LoRA, EWC++, ReasoningBank | None |
| `ruvector-domain-expansion` | 2.0.4 | Cross-domain transfer learning | None |
| `ruvector-mincut-gated-transformer` | 0.1.0 | Ultra-low latency transformer | None |

### Integration Architecture

```
┌─────────────────────────┐     HTTP      ┌──────────────────────────────┐
│   LibreFang Daemon      │ ◄──────────► │  Supabase PostgreSQL 17      │
│                         │              │  + ruvector extension v0.3   │
│  HttpVectorStore ───────┤  /insert     │  + RLS policies (account_id) │
│  (existing, unchanged)  │  /search     │  + HNSW indexes              │
│                         │  /delete     │  + 161+ SQL functions        │
│                         │  /embeddings │  + Local embeddings (384-dim)│
└─────────────────────────┘              └──────────────────────────────┘
```

The ruvector crates are **not compiled into the LibreFang binary**. They exist
in the workspace solely for building the PostgreSQL extension (`cargo pgrx package`).
The extension ships as a Docker image (~1.86 GB with ONNX Runtime).

### Feature Flags

```toml
# Workspace Cargo.toml — ruvector crates are optional
[workspace]
members = [
    "crates/librefang-*",
    # Uncomment to build PG extension locally:
    # "crates/ruvector-*",
]
```

### Docker Image

```dockerfile
# docker/Dockerfile.supabase-ruvector
ARG RUST_VERSION=1.88
ARG PG_VERSION=17
FROM rust:${RUST_VERSION}-bookworm AS builder
# ... Rust toolchain, pgrx install, compile extension
# Build: cargo pgrx package --features pg17,index-all,quant-all,all-features-v3
FROM supabase/postgres:17.6.1.095 AS runtime
# ... Copy .so + .control + .sql + ONNX models
# Result: ~1.86 GB image with ruvector.so + all-MiniLM-L6-v2
```

---

## Pattern Definition

All ruvector workspace crates MUST follow this structural pattern:

```
crates/ruvector-{name}/
├── Cargo.toml           # version matches upstream, path deps to siblings
├── src/
│   └── lib.rs           # No references to "openfang" anywhere
└── sql/                 # (ruvector-postgres only) Extension SQL
    └── ruvector--0.3.0.sql
```

Naming rules:
- SQL function prefix: `ruvector_` (not `openfang_`)
- Extension name: `ruvector` (in `pg_extension`)
- GUC prefix: `ruvector.` (e.g., `ruvector.ef_search`)
- Zero references to "openfang" in any ruvector crate file

---

## Implementation Scope

See PLAN-RV-001 for task breakdown. Summary:

| Task | Description | Verification |
|------|-------------|-------------|
| 0.1 | Copy 7 crates from openfang-ai workspace | `cargo check -p ruvector-postgres` |
| 0.2 | Scrub openfang references | `grep -r openfang crates/ruvector-*` returns empty |
| 0.3 | Add as workspace members (commented) | Workspace resolves with members uncommented |
| 0.4 | Port Dockerfile.supabase-ruvector | `docker build` succeeds |
| 0.5 | Add to docker-compose.yml | `docker compose up` starts PG with extension |
| 0.6 | Verify extension: functions, SIMD, embeddings | SPEC-RV-001 Groups 3-5 pass |
| 0.7 | Configure HttpVectorStore → Supabase | Semantic search round-trip via HTTP |

---

## Verification Gate

```bash
#!/bin/bash
# Gate: ADR-RV-001 fully implemented when ALL pass
set -e

# 1. No openfang references in ruvector crates
[ $(grep -rl openfang crates/ruvector-* 2>/dev/null | wc -l) -eq 0 ] || exit 1

# 2. All 7 crates present
for crate in postgres solver math attention sona domain-expansion mincut-gated-transformer; do
  [ -f "crates/ruvector-${crate}/Cargo.toml" ] || exit 1
done

# 3. Workspace compiles with crates uncommented
cargo check -p ruvector-postgres --features all-features-v3 || exit 1

# 4. Docker image builds
docker build -f docker/Dockerfile.supabase-ruvector . || exit 1

# 5. Extension loads and functions exist
docker compose exec db psql -U postgres -c "CREATE EXTENSION IF NOT EXISTS ruvector" || exit 1
FN_COUNT=$(docker compose exec db psql -U postgres -t -c \
  "SELECT count(*) FROM pg_proc WHERE proname LIKE 'ruvector_%'")
[ "$FN_COUNT" -ge 100 ] || exit 1

# 6. Embeddings work
RESULT=$(docker compose exec db psql -U postgres -t -c \
  "SELECT array_length(ruvector_embed('hello world'), 1)")
[ "$RESULT" -eq 384 ] || exit 1

echo "ADR-RV-001: ALL GATES PASS"
```

---

## Alternatives Considered

### Alternative 1: Use pgvector instead of ruvector

**Pros:**
- pgvector is widely adopted, smaller footprint
- Available as Supabase default extension

**Cons:**
- No local embeddings — requires external API (OpenAI, Cohere)
- No attention mechanisms, SONA engine, or learning systems
- No sublinear solvers for advanced operations
- Would lose openfang-ai's proven 7-crate ecosystem

**Rejected:** ruvector provides local embeddings (zero cost, zero latency) and
advanced ML primitives that pgvector lacks. The crate port is mechanical since
openfang already proved it.

### Alternative 2: Compile ruvector into LibreFang binary (direct PG driver)

**Pros:**
- Single binary, no Docker dependency
- Direct `libpq` connection, lower latency

**Cons:**
- Adds ~16K LOC + ONNX Runtime to the main binary
- Breaks the clean `VectorStore` trait abstraction
- Requires `libpq` system dependency on every platform
- Can't use Supabase RLS (needs to run inside PostgreSQL)

**Rejected:** The HTTP bridge architecture is proven, simpler, and leverages
Supabase's existing RLS infrastructure for multi-tenant isolation.

### Alternative 3: Ship ruvector as a separate crate on crates.io

**Pros:**
- Clean separation, independent release cycle
- Upstream-friendly (LibreFang doesn't carry the code)

**Cons:**
- Coordination overhead between repos
- Version alignment complexity
- Docker build still needs workspace context for pgrx

**Deferred:** This is the fallback if upstream LibreFang rejects the workspace
members PR. See Upstream Contribution Strategy in MASTER-PLAN.

---

## Consequences

### Positive
- Self-hosted vector search with zero external API dependency
- Local embeddings (384-dim all-MiniLM-L6-v2) eliminate per-query costs
- Multi-tenant memory isolation enforced at PostgreSQL RLS level
- 161+ SQL functions for vector ops, HNSW indexing, attention, learning
- `HttpVectorStore` works unchanged — zero LibreFang binary changes
- Proven in openfang-ai production (46 commits ahead of upstream)

### Negative
- Workspace gains ~16.5K LOC in 7 crates (but they don't affect the main binary)
- Docker image is ~1.86 GB (ONNX Runtime is large)
- Requires Rust 1.88 + pgrx 0.12.6 toolchain for extension builds
- Port requires committing 3 unstaged files in openfang-ai first

### Known Limitations (from openfang PLAN-033)
1. `ruvector_embed()` returns `real[]` not `ruvector` type — requires manual cast
2. `ruvector_embed_batch()` SQL stub missing
3. HNSW bitmap scan warning (cosmetic, does not affect results)
4. Image size 1.86 GB (ONNX Runtime dominates)

### Phase 3 Debt
- `ruvector_embed_batch()` implementation (blocked on upstream pgrx SRF support)
- Image size optimization (multi-stage build with ONNX static linking)
- Automated SPEC-RV-001 acceptance test suite in CI (currently manual verification)

---

## Affected Files

```
Cargo.toml                                    → Add ruvector workspace members (commented)
crates/ruvector-postgres/                     → Port from openfang-ai (hub crate)
crates/ruvector-solver/                       → Port (sublinear solvers)
crates/ruvector-math/                         → Port (optimal transport, geometry)
crates/ruvector-attention/                    → Port (39 attention mechanisms)
crates/ruvector-sona/                         → Port (SONA engine, LoRA, EWC++)
crates/ruvector-domain-expansion/             → Port (cross-domain transfer)
crates/ruvector-mincut-gated-transformer/     → Port (ultra-low latency transformer)
docker/Dockerfile.supabase-ruvector           → New: PG extension Docker build
docker/docker-compose.yml                     → Add supabase-ruvector service
.gitignore                                    → Add pgrx build artifact patterns
```

---

## Cross-References

| Document | Relationship |
|----------|-------------|
| SPEC-RV-001 | 26 acceptance criteria for this ADR's implementation |
| SPEC-RV-002 | Supabase-specific `HttpVectorStore` configuration and RLS policies |
| PLAN-RV-001 | Phase 0 task breakdown (7 tasks, 2-3 day estimate) |
| ADR-MT-004 | Data & Memory Isolation depends on RLS policies from this extension |
| MASTER-PLAN | This ADR is Phase 0 (independent, no multi-tenant deps) |
