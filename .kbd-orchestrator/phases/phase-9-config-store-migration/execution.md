# Phase Execution: phase-9-config-store-migration

**Phase:** phase-9-config-store-migration
**Backend:** `native-tool` (claude, in-session)
**Worktree:** `/tmp/librefang-config-store-assess` on `kbd/phase-9-config-store-assessment`
**Started:** 2026-06-06

## Backend selection

No OpenSpec, no evolver — native KBD. Changes are implemented in-session by the
`claude` agent against scoped `cargo check` / `cargo test -p <crate>` gates per
the repo's Build & Verify rules (no `cargo build`, no workspace-wide `cargo test`).

## Dispatch contract

| Change | Status | Verify gate |
|---|---|---|
| C-001 | **DONE** | `cargo check -p librefang-storage --lib` + migration-invariants test + embedded-DB apply test |
| C-002 | **DONE** | `cargo check -p librefang-storage --lib` + round-trip unit test + clippy + sqlite-only build |
| C-003 | PENDING | `cargo test -p librefang-kernel config_store_sync` |
| C-004 | **DONE** | `cargo check --workspace --lib` + `cargo test -p librefang-api --test config_store_overlay_test` |
| C-005 | PENDING | `cargo test -p librefang-api` (TestServer) |
| C-006 | PENDING | `cargo test -p librefang-api` (reload) |
| C-007 | PENDING | `cargo test -p librefang-kernel` (determinism) |
| C-008 | PENDING | HUMAN cluster verify |
| C-009 | PENDING | HUMAN deploy, gated on C-008 |

## C-001 — config_store SurrealDB migration · DONE (2026-06-06)

**Files:**
- `crates/librefang-storage/src/migrations/sql/031_config_store.surql` (new)
- `crates/librefang-storage/src/migrations/mod.rs` (registered `config_store_v1` v31)

**What landed:**
- New SYSTEM-scoped `config_store` table (UNIQUE on `key` alone — distinct from the
  AGENT-scoped `kv_store` which is UNIQUE on `(agent_id, key)`).
- Fields: `key`, `value: option<object> FLEXIBLE`, `source`, `content_hash`,
  `revision: int`, `updated_at` — the provenance + content-hash + revision shape
  the corrected conflict mechanism (plan D3) needs. No mtime field by design.
- Header comment documents: BossFang-exclusive (no upstream SQLite equivalent),
  the kv_store distinction, and the provenance/conflict model.

**Verification (all green):**
- `cargo check -p librefang-storage --lib` → exit 0 (3m49s).
- `cargo test -p librefang-storage --test surreal_migration_invariants_test`
  → 3 passed (strictly-increasing version 31>30; no banned `FLEXIBLE TYPE` syntax).
- `cargo test -p librefang-runtime --lib backends::surreal_audit::tests::append_and_verify_chain`
  → 1 passed — applies the FULL migration set (incl. v31) against a real embedded
  SurrealDB, proving migration 31 applies cleanly.
- `python3 scripts/enforce-branding.py --check` → exit 0.

**QA gate:** skipped (2 files modified, under the 3-file artifact-refiner threshold).

## C-002 — ConfigStore trait + SurrealDB impl · DONE (2026-06-07)

**Files:**
- `crates/librefang-storage/src/config_store.rs` (new)
- `crates/librefang-storage/src/lib.rs` (module + re-exports)

**What landed:**
- `ConfigSource` (Bootstrap/Runtime), `ConfigEntry`, async `ConfigStore` trait
  (`get`/`list`/`upsert`/`delete`), `content_hash()` (canonical, object-key-order
  independent SHA-256), and `SurrealConfigStore` — a single impl over
  `Surreal<Any>` covering embedded AND remote.
- Storage details: `value` enveloped as `{ "data": <v> }` to fit the v31
  `option<object>` column while supporting arrays/scalars; deterministic record
  id = SHA-256(key) for idempotent upsert/delete; `list()` is `ORDER BY key ASC`
  (determinism #3298, prepares C-007).
- `take()` returns `Vec<serde_json::Value>` then `serde_json::from_value`
  (SurrealDB 3.0.5 requires `SurrealValue` on `take`, same workaround as the
  migration runner).

**Verification (all green):**
- `cargo check -p librefang-storage --lib` → exit 0.
- `cargo test -p librefang-storage --lib config_store` → 3 passed.
- `cargo clippy -p librefang-storage --all-targets -- -D warnings` → clean.
- `cargo check -p librefang-storage --lib --no-default-features --features sqlite-backend` → exit 0.
- `enforce-branding.py --check` → clean.

**QA gate:** skipped (2 files modified, under threshold).

**Next:** C-004 — kernel effective-config read path (bootstrap ⊕ DB).

## C-004 — effective-config read path (API-owned) · DONE (2026-06-07)

**Architecture revision (user decision D9 + D10):** the kernel holds no
operational SurrealDB session at boot, so config-store access is **API-owned**.
A process-global **`shared_pool()`** was added to resolve the embedded-RocksDB
single-lock-per-path constraint (assessment R-2, confirmed concretely during
testing).

**Files (8):**
- `crates/librefang-kernel/src/kernel/mcp_setup.rs` — `effective_mcp_servers()` +
  `replace_effective_mcp_servers()`.
- `crates/librefang-storage/src/pool.rs` + `lib.rs` — `shared_pool()`.
- `crates/librefang-api/src/config_store_overlay.rs` (new) — boot overlay.
- `crates/librefang-api/src/server.rs` — wired into `run_daemon` (seed→overlay→connect order).
- `crates/librefang-api/src/lib.rs` — module (feature-gated).
- `crates/librefang-api/src/routes/storage.rs` — 2 pool sites → `shared_pool()`.
- `crates/librefang-api/tests/config_store_overlay_test.rs` (new) — 2 tests.

**Verification (green):** `cargo check --workspace --lib`; overlay test (2 passed);
storage lib (11 passed); clippy clean on changed crates (one pre-existing
`large_enum_variant` lint in untouched `librefang-runtime` under feature
unification, not mine); brand audit clean.

**QA gate:** 8 files (≥3) → artifact-refiner applies; `/refine-validate` skill
unavailable in session, so QA covered by the gates above.

**Next:** C-005 — re-target write endpoints to `ConfigStore` (via shared pool).
