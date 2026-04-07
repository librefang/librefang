# PLAN-RV-001: Phase 0 — RuVector Crate Port

**SPEC:** SPEC-RV-001
**Date:** 2026-04-06

---

## Verified Baseline Facts

```bash
# Source: openfang-ai/crates/ruvector-postgres (verified 2026-04-06)

$ grep -c "CREATE OR REPLACE FUNCTION" crates/ruvector-postgres/sql/ruvector--0.3.0.sql
197

# Source-level count (includes feature-gated functions)
$ grep -rc '#\[pg_extern\]' crates/ --include='*.rs'
225   # 57 behind optional feature flags; 168 in default (pg17-only) build

$ grep -c "CREATE OPERATOR" crates/ruvector-postgres/sql/ruvector--0.3.0.sql
11

$ grep -c "CREATE CAST" crates/ruvector-postgres/sql/ruvector--0.3.0.sql
2

$ git status --short crates/ruvector-postgres/
 M crates/ruvector-postgres/sql/ruvector--0.3.0.sql
 M crates/ruvector-postgres/sql/ruvector--2.0.0--0.3.0.sql
 M crates/ruvector-postgres/src/operators.rs
# 3 unstaged files contain cast functions + operator tests — must commit before port

$ grep "all-features-v3" crates/ruvector-postgres/Cargo.toml
all-features-v3 = ["all-features", "analytics-complete", "ai-complete-v3", "domain-expansion"]

# Dockerfile (from qwntik):
$ head -3 qwntik/docker/Dockerfile.supabase-ruvector
ARG RUST_VERSION=1.88
ARG PG_VERSION=17
FROM rust:${RUST_VERSION}-bookworm AS builder

# Build command in Dockerfile:
cargo pgrx package \
    --pg-config /usr/lib/postgresql/${PG_VERSION}/bin/pg_config \
    --features pg${PG_VERSION},index-all,quant-all,all-features-v3

# Runtime base:
FROM supabase/postgres:17.6.1.095
```

## Pre-Work: Commit Unstaged Changes (MANDATORY)

Before any port work begins, commit the 3 unstaged files in openfang-ai:

```bash
cd /Users/danielalberttis/Desktop/Projects/openfang-ai
git add crates/ruvector-postgres/src/operators.rs \
        crates/ruvector-postgres/sql/ruvector--0.3.0.sql \
        crates/ruvector-postgres/sql/ruvector--2.0.0--0.3.0.sql
git commit -m "feat(ruvector-postgres): add real[]<->ruvector cast functions + operator tests"
```

**Why:** These files contain the bidirectional assignment cast (`real[]` ↔ `ruvector`)
and 8 operator tests. Without them, AC-5.4 (assignment cast on INSERT) fails and
the SQL file is missing cast support functions.

**Pre-work gate:**
```bash
cd /Users/danielalberttis/Desktop/Projects/openfang-ai
git diff --name-only crates/ruvector-postgres/  # should be empty (all committed)
git log -1 --oneline                             # should show the cast commit
```

---

## Implementation Rounds

### Round 1: Crate Copy + Workspace Integration

| Task | Source → Target | AC |
|------|----------------|---|
| Copy ruvector-postgres | `openfang-ai/crates/` → `crates/ruvector-postgres/` | AC-1.1 |
| Copy ruvector-solver | `openfang-ai/crates/` → `crates/ruvector-solver/` | AC-1.1 |
| Copy ruvector-math | `openfang-ai/crates/` → `crates/ruvector-math/` | AC-1.1 |
| Copy ruvector-attention | `openfang-ai/crates/` → `crates/ruvector-attention/` | AC-1.1 |
| Copy ruvector-sona | `openfang-ai/crates/` → `crates/ruvector-sona/` | AC-1.1 |
| Copy ruvector-domain-expansion | `openfang-ai/crates/` → `crates/ruvector-domain-expansion/` | AC-1.1 |
| Copy ruvector-mincut-gated-transformer | `openfang-ai/crates/` → `crates/ruvector-mincut-gated-transformer/` | AC-1.1 |
| Update workspace `Cargo.toml` | Add 7 members (commented by default) | AC-1.3 |
| Verify path deps resolve | `cargo metadata` shows local paths | AC-1.3 |
| Verify version match | Compare each `Cargo.toml` version field | AC-1.5 |

**Method:**
```bash
# Copy all 7 crates (preserving directory structure, not git history)
for crate in ruvector-postgres ruvector-solver ruvector-math ruvector-attention \
             ruvector-sona ruvector-domain-expansion ruvector-mincut-gated-transformer; do
  cp -r /Users/danielalberttis/Desktop/Projects/openfang-ai/crates/$crate crates/
done

# Add to workspace Cargo.toml (commented — uncomment to build):
# [workspace]
# members = [
#     # RuVector PG extension crates (uncomment to build):
#     # "crates/ruvector-postgres",
#     # "crates/ruvector-solver",
#     # "crates/ruvector-math",
#     # "crates/ruvector-attention",
#     # "crates/ruvector-sona",
#     # "crates/ruvector-domain-expansion",
#     # "crates/ruvector-mincut-gated-transformer",
# ]
```

**Round 1 gate:**
```bash
# Uncomment workspace members first, then:
cargo check -p ruvector-postgres --features all-features-v3
cargo tree -p ruvector-postgres --features all-features-v3 2>&1 | grep -c "\[cycle\]"
# Must output: 0 (no cycles)
```

---

### Round 2: Naming Scrub

| Task | File Pattern | Change | AC |
|------|-------------|--------|---|
| Scrub Cargo.toml descriptions | `crates/ruvector-*/Cargo.toml` | Replace openfang → librefang in metadata | AC-2.1 |
| Scrub source comments | `crates/ruvector-*/src/**/*.rs` | Remove openfang mentions in comments/docs | AC-2.1 |
| Scrub README files | `crates/ruvector-*/README.md` | Update project references | AC-2.1 |
| Verify SQL prefix | `sql/ruvector--0.3.0.sql` | Confirm all functions use `ruvector_` prefix | AC-2.2 |

**Method:**
```bash
# 1. Audit: find all openfang references
grep -rn "openfang" crates/ruvector-* --include="*.rs" --include="*.toml" --include="*.md" --include="*.sql"

# 2. Replace in Cargo.toml metadata fields only (description, repository, homepage)
for f in crates/ruvector-*/Cargo.toml; do
  sed -i '' 's|openfang|librefang|g' "$f"
done

# 3. Review and fix .rs comments manually (sed is too blunt for source code)
grep -rn "openfang" crates/ruvector-*/src/ --include="*.rs"
# Fix each occurrence by hand — do NOT blind-replace in Rust source

# 4. Verify SQL prefix is already correct (ruvector_, not openfang_)
grep -c "openfang_" crates/ruvector-postgres/sql/ruvector--0.3.0.sql
# Must output: 0
```

**Round 2 gate:**
```bash
# Zero openfang references
if grep -r "openfang" crates/ruvector-* 2>/dev/null; then
  echo "FAIL: openfang references found"
  exit 1
fi
echo "PASS: zero openfang references"

# Re-verify compilation after name changes:
cargo check -p ruvector-postgres --features all-features-v3
```

---

### Round 3: Docker Build + Extension Load

| Task | Source | Target | AC |
|------|--------|--------|---|
| Copy Dockerfile | `qwntik/docker/Dockerfile.supabase-ruvector` | `docker/Dockerfile.supabase-ruvector` | AC-3.1 |
| Adapt build context paths | — | `COPY crates/ ...` points to workspace `crates/` | AC-3.1 |
| Extract db service from compose | `qwntik/docker/docker-compose.yml` line 485 | `docker/docker-compose.yml` | AC-3.5 |
| Copy migration SQL | `qwntik/apps/web/supabase/migrations/20260405_ruvector_setup.sql` | `docker/sql/ruvector_setup.sql` | AC-5.1 |
| Build Docker image | — | `supabase-ruvector:latest` | AC-3.1 |
| Verify extension loads | `CREATE EXTENSION ruvector` | — | AC-3.2 |
| Verify function count | `pg_proc` count | ≥ 161 (live; 197 in SQL file) | AC-3.3 |
| Verify SIMD detection | `ruvector_simd_info()` | Returns arch string | AC-3.4 |

**Dockerfile adaptation notes:**
- Original build context = openfang-ai repo root (crates at `crates/ruvector-*`)
- Target build context = librefang workspace root (identical structure after Round 1)
- Build features: `pg17,index-all,quant-all,all-features-v3`
- Rust 1.88, cargo-pgrx 0.12.6 (`--locked`)
- Runtime base: `supabase/postgres:17.6.1.095`
- PG port mapping: verify in compose file (qwntik uses 54322:5432)

**Round 3 gate:**
```bash
set -euo pipefail
trap 'docker compose -f docker/docker-compose.yml down -v 2>/dev/null' EXIT

# Build image
docker build -f docker/Dockerfile.supabase-ruvector --tag supabase-ruvector:latest .

# Start stack
docker compose -f docker/docker-compose.yml up -d
sleep 15  # wait for PG + PostgREST ready

export PGPASSWORD=postgres
PSQL="psql -h localhost -p 54322 -U postgres -d postgres -tAc"

# Extension loads
$PSQL "CREATE EXTENSION IF NOT EXISTS ruvector"

# Function count ≥ 197
FCOUNT=$($PSQL "SELECT count(*) FROM pg_proc WHERE proname LIKE 'ruvector_%'" | tr -d ' ')
echo "Functions registered: $FCOUNT"
[ "$FCOUNT" -ge 161 ] || { echo "FAIL: only $FCOUNT functions (need ≥161)"; exit 1; }

# SIMD
SIMD=$($PSQL "SELECT ruvector_simd_info()" | tr -d ' ')
echo "SIMD: $SIMD"
[ -n "$SIMD" ] || { echo "FAIL: SIMD info empty"; exit 1; }

# GUC check
GUC=$($PSQL "SHOW ruvector.ef_search" | tr -d ' ')
echo "ef_search default: $GUC"
[ "$GUC" = "40" ] || { echo "FAIL: ef_search=$GUC, expected 40"; exit 1; }

echo "PASS: Round 3 gate"
```

---

### Round 4: Embeddings + Search Verification

| Task | SQL Command | AC |
|------|------------|---|
| Test `ruvector_embed()` | `SELECT ruvector_embed('hello world')` | AC-4.1 |
| Verify 384 dimensions | `SELECT array_length(ruvector_embed('test'), 1)` | AC-4.3 |
| Test `embed_batch` | `SELECT ruvector_embed_batch(ARRAY['a','b'])` | AC-4.4 |
| Test assignment cast | `INSERT INTO t (emb) VALUES (ruvector_embed('x'))` — `emb` is `ruvector` type | AC-5.4 |
| Create HNSW index | `CREATE INDEX USING hnsw (emb ruvector_cosine_ops)` | AC-5.1 |
| Cosine similarity search | `ORDER BY emb <=> ruvector_embed('query')` | AC-5.2 |
| Insert + search round-trip | Insert "cats are pets", search "feline animals" | AC-5.3 |

**Prerequisite:** Round 3 container still running (or `docker compose up -d` to restart).

**Round 4 gate:**
```bash
set -euo pipefail
export PGPASSWORD=postgres
PSQL="psql -h localhost -p 54322 -U postgres -d postgres -tAc"

# Embedding dimensions = 384
EDIM=$($PSQL "SELECT array_length(ruvector_embed('test'), 1)" | tr -d ' ')
echo "Embed dimensions: $EDIM"
[ "$EDIM" -eq 384 ] || { echo "FAIL: dim=$EDIM, expected 384"; exit 1; }

# Batch embedding functional
BCOUNT=$($PSQL "SELECT array_length(ruvector_embed_batch(ARRAY['hello','world']), 1)" | tr -d ' ')
echo "Batch rows: $BCOUNT"
[ "$BCOUNT" -eq 2 ] || { echo "FAIL: batch=$BCOUNT, expected 2"; exit 1; }

# Assignment cast + HNSW + search round-trip
$PSQL "DROP TABLE IF EXISTS r4_test"
$PSQL "CREATE TABLE r4_test (id serial PRIMARY KEY, content text, embedding ruvector)"

# Assignment cast: ruvector_embed returns real[], column is ruvector — no explicit cast needed
$PSQL "INSERT INTO r4_test (content, embedding) VALUES ('cats are pets', ruvector_embed('cats are pets'))"
$PSQL "INSERT INTO r4_test (content, embedding) VALUES ('dogs are loyal', ruvector_embed('dogs are loyal'))"
$PSQL "INSERT INTO r4_test (content, embedding) VALUES ('neural networks', ruvector_embed('neural networks learn'))"

# HNSW index
$PSQL "CREATE INDEX idx_r4 ON r4_test USING hnsw (embedding ruvector_cosine_ops)"

# Cosine search — "feline animals" should rank "cats are pets" first
TOP=$($PSQL "SELECT content FROM r4_test ORDER BY embedding <=> ruvector_embed('feline animals') LIMIT 1" | xargs)
echo "Top result for 'feline animals': $TOP"
case "$TOP" in
  *cats*) ;;
  *) echo "FAIL: expected cats, got '$TOP'"; exit 1 ;;
esac

# Cleanup
$PSQL "DROP TABLE IF EXISTS r4_test"

echo "PASS: Round 4 gate"
```

---

### Round 5: REST Integration + Account Scoping

| Task | Action | AC |
|------|--------|---|
| GRANT EXECUTE on ruvector functions | SQL `GRANT` to `anon`/`authenticated` roles | AC-6.1 |
| Create `ruvector_search` wrapper | SQL function for PostgREST RPC exposure | AC-6.2 |
| Apply RLS policies | Enable RLS, add `account_id` policies from migration | AC-6.3 |
| Test REST embed endpoint | `curl /rest/v1/rpc/ruvector_embed` | AC-6.1 |
| Test REST search endpoint | `curl /rest/v1/rpc/ruvector_search` | AC-6.2 |
| Test RLS isolation | Query as two different accounts | AC-6.3 |
| Test health check | `curl /rest/v1/` | AC-6.4 |

**Note:** The migration SQL from qwntik (`20260405_ruvector_setup.sql`) already includes
the documents table, HNSW index, and RLS policies. Round 5 verifies these work through
the PostgREST HTTP layer, and adds any missing GRANTs for RPC exposure.

**Round 5 gate:**
```bash
set -euo pipefail
export PGPASSWORD=postgres
PSQL="psql -h localhost -p 54322 -U postgres -d postgres -tAc"

# Extract anon key from compose env
ANON_KEY=$(grep 'ANON_KEY' docker/docker-compose.yml | head -1 | sed 's/.*: *//;s/["[:space:]]//g')
echo "Using anon key: ${ANON_KEY:0:20}..."

# Health check (AC-6.4)
HTTP_STATUS=$(curl -s -o /dev/null -w "%{http_code}" \
  "http://localhost:54321/rest/v1/" \
  -H "apikey: $ANON_KEY")
echo "REST health: $HTTP_STATUS"
[ "$HTTP_STATUS" -eq 200 ] || { echo "FAIL: REST health=$HTTP_STATUS"; exit 1; }

# RLS isolation (AC-6.3)
$PSQL "DELETE FROM documents WHERE account_id IN ('test-a','test-b')"
$PSQL "INSERT INTO documents (content, embedding, account_id) VALUES ('secret A', ruvector_embed('secret alpha'), 'test-a')"
$PSQL "INSERT INTO documents (content, embedding, account_id) VALUES ('secret B', ruvector_embed('secret beta'), 'test-b')"

# Query as account-a should NOT see account-b
LEAK=$($PSQL "SET request.jwt.claims.account_id = 'test-a'; SELECT count(*) FROM documents WHERE account_id = 'test-b'" | tr -d ' ')
echo "RLS leak check (should be 0): $LEAK"
[ "$LEAK" -eq 0 ] || { echo "FAIL: RLS leak — account-a sees $LEAK rows from account-b"; exit 1; }

# Cleanup
$PSQL "DELETE FROM documents WHERE account_id IN ('test-a','test-b')"

echo "PASS: Round 5 gate"
```

---

## Pattern Coverage Gate (MANDATORY)

This is the mechanical check that makes BHR a confirmation, not discovery.

```bash
#!/usr/bin/env bash
set -euo pipefail

echo "=== Pattern Coverage Gate ==="

# 1. All 7 crates present with Cargo.toml
EXPECTED="ruvector-postgres ruvector-solver ruvector-math ruvector-attention ruvector-sona ruvector-domain-expansion ruvector-mincut-gated-transformer"
COUNT=0
for crate in $EXPECTED; do
  if [ -d "crates/$crate" ] && [ -f "crates/$crate/Cargo.toml" ]; then
    COUNT=$((COUNT + 1))
  else
    echo "MISSING: crates/$crate"
  fi
done
echo "Crates: $COUNT/7"
[ "$COUNT" -eq 7 ] || { echo "FAIL: only $COUNT/7 crates"; exit 1; }

# 2. Zero openfang references across all ruvector crates
REFS=$(grep -r "openfang" crates/ruvector-* 2>/dev/null | wc -l | tr -d ' ')
echo "openfang references: $REFS"
[ "$REFS" -eq 0 ] || { echo "FAIL: $REFS openfang references remaining"; exit 1; }

# 3. Docker artifacts present
for f in docker/Dockerfile.supabase-ruvector docker/docker-compose.yml docker/sql/ruvector_setup.sql; do
  [ -f "$f" ] || { echo "FAIL: $f missing"; exit 1; }
done
echo "Docker artifacts: 3/3 present"

# 4. Workspace compilation
cargo check -p ruvector-postgres --features all-features-v3
echo "Compilation: clean"

# 5. Version match check
echo "--- Version verification ---"
for pair in \
  "ruvector-postgres:0.3.0" \
  "ruvector-solver:2.0.4" \
  "ruvector-math:2.0.4" \
  "ruvector-attention:2.0" \
  "ruvector-sona:0.1.9" \
  "ruvector-domain-expansion:2.0" \
  "ruvector-mincut-gated-transformer:0.1.0"; do
  CRATE=$(echo "$pair" | cut -d: -f1)
  EXPECTED_VER=$(echo "$pair" | cut -d: -f2)
  ACTUAL_VER=$(grep '^version' "crates/$CRATE/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
  if [ "$ACTUAL_VER" = "$EXPECTED_VER" ]; then
    echo "  $CRATE: $ACTUAL_VER ✅"
  else
    echo "  $CRATE: $ACTUAL_VER ≠ $EXPECTED_VER ❌"
    exit 1
  fi
done

echo ""
echo "=== Pattern Coverage: ALL PASSED ==="
```

## Day-by-Day Schedule

| Day | Time | Rounds | Deliverables |
|-----|------|--------|-------------|
| 1 | AM | Pre-work + Round 1 | Commit unstaged files in openfang-ai; copy 7 crates; `cargo check` passes |
| 1 | PM | Round 2 | Zero openfang references; compilation still clean |
| 2 | AM | Round 3 | Docker image builds; extension loads; ≥161 live functions (197 in SQL); SIMD detected |
| 2 | PM | Round 4 | 384-dim embeddings; embed_batch works; HNSW search returns correct ranking |
| 3 | AM | Round 5 | REST endpoints respond; RLS isolates accounts |
| 3 | PM | Exit gate | Full exit gate passes; pattern coverage gate passes; commit all changes |

## TDD Cycle

Phase 0 is infrastructure (copy + build + verify), not feature code. TDD applies to the
verification scripts, not to the crate source code (which is already tested in openfang-ai).

For each round:
1. **RED:** Write the round gate script. Run it — it should fail (crates not copied, image not built, etc.)
2. **GREEN:** Execute the round tasks until the gate passes
3. **REFACTOR:** Clean up any workarounds, ensure gate is idempotent
4. **Round gate:** Script exits 0

## Pre-BHR Checklist

Run BEFORE requesting BHR review:
- [ ] Pre-work complete: 3 files committed in openfang-ai (`git diff --name-only` empty)
- [ ] Pattern coverage gate passes: 7 crates, 0 openfang refs, Docker artifacts, compilation clean
- [ ] All 26 SPEC acceptance criteria verified (Groups 1–6)
- [ ] All 7 SPEC claims have passing gate commands
- [ ] Round gates R1–R5 all exit 0
- [ ] Full exit gate script exits 0
- [ ] No cargo warnings: `cargo check -p ruvector-postgres --features all-features-v3 2>&1 | grep -c warning` = 0

## Exit Criteria

```bash
# All commands must exit 0:

# 1. Compilation
cargo check -p ruvector-postgres --features all-features-v3

# 2. Naming
if grep -r "openfang" crates/ruvector-* 2>/dev/null; then exit 1; fi

# 3. Docker build
docker build -f docker/Dockerfile.supabase-ruvector --tag supabase-ruvector:latest .

# 4. Full exit gate (starts stack, verifies extension, embeddings, search, tears down)
./docs/multi-tenant/phase0-exit-gate.sh

# 5. Pattern coverage
./docs/multi-tenant/phase0-pattern-gate.sh
```

## Rollback Plan

If any round fails beyond reasonable fix time (>4 hours on a single round):

```bash
# 1. Stop and remove Docker resources
docker compose -f docker/docker-compose.yml down -v 2>/dev/null

# 2. Remove ported crates
rm -rf crates/ruvector-*

# 3. Remove Docker artifacts
rm -f docker/Dockerfile.supabase-ruvector
rm -f docker/sql/ruvector_setup.sql
# Revert docker-compose.yml changes via git

# 4. Revert workspace Cargo.toml
git checkout -- Cargo.toml

# 5. Verify clean state
cargo check  # existing crates still compile
git status    # only untracked deletions, no broken state
```

Librefang reverts to pre-Phase-0 state: no vector extension, no ruvector crates.
Semantic search remains available via whatever backend was configured before Phase 0.
