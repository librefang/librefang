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
| C-003 | **DONE** | `cargo test -p librefang-api --test config_store_overlay_test` (7 passed) |
| C-004 | **DONE** | `cargo check --workspace --lib` + `cargo test -p librefang-api --test config_store_overlay_test` |
| C-005 | **DONE** | `cargo test -p librefang-api --test config_store_overlay_test` + `cargo check --workspace --lib` |
| C-005b | **DONE** | overlay test (10) + `mcp_http_crud_test` (1) + `providers_routes_test` (37) |
| C-005c | PENDING | generic config_set (needs kernel config-override merge layer) |
| C-006 | **DONE** | `cargo test -p librefang-api --test config_store_overlay_test` (8 passed) |
| C-007 | **DONE** | `cargo test -p librefang-storage --lib config_store` (4 passed) |
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

## C-005 — MCP write path → config store · DONE (2026-06-08)

**Scope (user decision):** MCP write path only; provider-default + `config_set`
deferred to **C-005b**.

**What landed:**
- kernel: `replace_mcp_servers()` now syncs BOTH `config.mcp_servers` (via
  `ArcSwap::rcu`) and the effective list → existing read sites (dup-checks, GET
  list, uninstall lookup) stay correct unchanged; added to the `KernelApi` trait.
- api: `write_mcp_servers()` helper + feature-gated `apply_mcp_mutation()`
  (surreal → store write + sync + bg reconnect; sqlite → legacy config.toml +
  `reload_config`). All 4 `mcp.rs` handlers + both `extensions.rs` install/
  uninstall sites converted. Legacy `upsert/remove_mcp_server_config` helpers +
  3 tests + `json_to_toml_value` import gated `not(surreal-backend)`.

**Files (6 src + 1 test):** `mcp_setup.rs`, `kernel_api.rs`,
`config_store_overlay.rs`, `routes/skills/{mcp,extensions,mod}.rs`,
`tests/config_store_overlay_test.rs`.

**Verification (green):** `config_store_overlay_test` 3 passed (incl.
`runtime_write_persists_and_survives_restart`); `cargo check --workspace --lib`;
clippy (changed crates); brand audit.

**Deferred (tracked as C-005b):** provider-default + `config_set` write paths;
route-level HTTP TestServer coverage (needs a temp-storage `start_full_router`
variant — the existing harness uses CWD-relative storage). Sqlite-only api build
is pre-existing-broken (`open_trace_store` cfg mismatch, untouched), so the
fallback path is gated correct-by-construction but not full-build-verified.

**Next:** C-003 — seed-once (write bootstrap `config.mcp_servers` into the store
as `source=bootstrap` on first boot, before the overlay).

## C-003 — seed-once + provenance merge · DONE (2026-06-08)

**What landed:** `seed_mcp_servers` + `SeedOutcome` + `bootstrap_revision()` +
`seed_config_store(kernel)` in `config_store_overlay.rs`; wired into `run_daemon`
before the overlay (seed → overlay → connect). Content-hash + provenance merge,
**never mtime**; a `runtime` row is only overridden when the operator advances
`BOSSFANG_CONFIG_BOOTSTRAP_REVISION` past the stored revision.

**Files (3):** `config_store_overlay.rs`, `server.rs`,
`tests/config_store_overlay_test.rs`.

**Verification (green):** `config_store_overlay_test` 7 passed (4 new seed
tests: fresh/unchanged, bootstrap-update, runtime-protected, revision-override);
clippy `-p librefang-api` clean; brand audit clean.

**Next:** C-006 — reload path re-resolves `effective_mcp_servers` from the store.

## C-006 — reload re-resolves from the store · DONE (2026-06-08)

**What landed:** `config_reload` re-runs **seed → overlay → reload_mcp_servers**
after `reload_config()` (surreal-gated), so a reload that re-reads config.toml
never reverts a DB `runtime` value to the bootstrap file. Widened
`seed_config_store`/`overlay_mcp_servers` to `&dyn KernelApi` (handler holds
`Arc<dyn KernelApi>`); boot call sites coerce automatically.

**Files (3):** `routes/config/manage.rs`, `config_store_overlay.rs`,
`tests/config_store_overlay_test.rs`.

**Verification (green):** `config_store_overlay_test` 8 passed (new
`reload_reresolve_preserves_runtime_over_bootstrap`); clippy `-p librefang-api`
clean; brand audit clean.

**Next:** C-007 — determinism `ORDER BY` + regression test (#3298).

## C-007 — determinism guard · DONE (2026-06-08)

**Structural finding:** `mcp_servers` is a single ordered row (not row-per-server),
and `render_mcp_summary` already sorts servers + tools — so prompt output is
byte-stable regardless of stored order (existing kernel `mcp_summary_*` tests
cover it). No kernel change needed. Added a config-store `list()` ORDER-BY guard
test (`list_is_sorted_by_key_regardless_of_insertion_order`) for the future
multi-key prompt-reaching path.

**Files (1):** `crates/librefang-storage/src/config_store.rs`.

**Verification (green):** `cargo test -p librefang-storage --lib config_store`
4 passed; clippy `-p librefang-storage --all-targets` clean; brand clean.

## C-005b — provider-default → store + HTTP route tests · DONE (2026-06-08)

**Scope (user):** provider-default + HTTP route test; generic config_set deferred
to C-005c (needs a kernel config-override merge layer).

**What landed:** `default_model` write/overlay/seed reusing the
`default_model_override_ref()` RwLock pattern; both default-model writers
(`set_default_provider`, `set_provider_key`) route through
`persist_default_model_durable` (store under surreal; config.toml fallback under
sqlite). `overlay_default_model` added to boot + reload pipelines.
`MockKernelBuilder` now isolates `config.storage` to a tempdir (the default is
CWD-relative). New full-router `mcp_http_crud_test` (POST→GET→DELETE→GET).

**Files (8):** `config_store_overlay.rs`, `server.rs`, `routes/config/manage.rs`,
`routes/providers.rs`, `librefang-testing/mock_kernel.rs`, +3 test files.

**Verification (green):** overlay 10, http-crud 1, providers 37; `cargo check
--workspace --lib`; clippy `-p librefang-api -p librefang-testing`; brand clean.
(Pre-existing, unrelated: `api_integration_test` doesn't compile on base —
`sync_registry` arity — untouched.)

**Next:** C-008 — one-time prod config.toml → DB import CLI + HUMAN verify.

---

**Core migration + provider-default COMPLETE.** Remaining: C-005c (deferred config_set +
HTTP route tests), C-008 (prod import), C-009 (K8s revert, gated on C-008).
