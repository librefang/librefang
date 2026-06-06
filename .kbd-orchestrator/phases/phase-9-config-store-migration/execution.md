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
| C-002 | PENDING | `cargo check -p librefang-storage --lib` + round-trip unit test |
| C-003 | PENDING | `cargo test -p librefang-kernel config_store_sync` |
| C-004 | PENDING | `cargo test -p librefang-kernel` (effective-config) |
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

**Next:** C-002 — `ConfigStore` trait + SurrealDB impl.
