# Phase Plan: phase-9-config-store-migration

**Phase:** phase-9-config-store-migration
**Date:** 2026-06-06
**Backend:** native KBD (no OpenSpec, no evolver bridge)
**Worktree:** `/tmp/librefang-config-store-<change>` per change (worktree discipline)
**Based on:** `assessment.md` (2026-06-06) — verdict: *proceed with corrected mechanism*.

---

## Decisions baked

| # | Decision | Baked |
|---|---|---|
| D1 | Storage substrate | **SurrealDB only** (embedded + remote, one `Surreal<Any>` impl). Per user clarification. SQLite legacy parity is best-effort behind `sqlite-backend`, not a blocker. |
| D2 | Table design | **New `config_store` table** (migration `031`), NOT a reuse of agent-scoped `kv_store`. Fields: `key UNIQUE`, `value: object FLEXIBLE`, `source: "bootstrap"\|"runtime"`, `content_hash: string`, `revision: int`, `updated_at: string`. |
| D3 | Conflict mechanism | **Content-hash + revision gate, per-key, provenance-aware.** NOT file mtime. NOT whole-file "newer wins". A bootstrap re-sync overwrites only `source="bootstrap"` keys, or keys whose section `revision` was explicitly bumped. UI-written (`source="runtime"`) values are never silently reverted. |
| D4 | Scope | **UI-mutable runtime subset only** (§3a of assessment). `[storage]`, secrets, auth, `api_listen`/network stay in file+env (§3b — bootstrap paradox + security). |
| D5 | Seeding source on existing prod | First boot of new code seeds the DB from **the live `config.toml` already on the PVC** (treat as bootstrap), not from a fresh ConfigMap — preserves existing prod UI edits. |
| D6 | K8s flip ordering | ConfigMap-revert (drop `init-config`) is the **last** change, gated on verified prod import. Behavior is additive until then — the file path keeps working. |
| D7 | Determinism | All prompt-reaching lists (`mcp_servers`) queried with explicit `ORDER BY` — TOML insertion order is gone once rows. Regression test mirrors existing `mcp_summary_*` tests (#3298). |
| D8 | Effective-config model | `effective = bootstrap_defaults ⊕ db_overrides`, resolved at boot and on `POST /api/config/reload`. The kernel gains a `ConfigStore` read path; `KernelConfig` stays the in-memory shape. |

---

## Ordered change list

9 changes. Dependency-ordered. The critical path is C-001 → C-002 → C-004 →
C-005 → C-003 → C-006, with C-007 (determinism test) attachable once C-004
lands, and C-008/C-009 (prod import + K8s flip) strictly last.

```
C-001 (table)──┐
               ├─▶ C-002 (ConfigStore trait/impl)──┬─▶ C-004 (kernel read path)──┬─▶ C-006 (reload)
               │                                    │                             ├─▶ C-007 (determinism test)
               │                                    └─▶ C-005 (write endpoints)───┘
               │
               └─────────────────────────────────────────────────────────────────▶ C-003 (seed+merge)
                                                                                          │
                                                                  C-008 (prod import) ◀───┘
                                                                          │
                                                                  C-009 (K8s ConfigMap revert) ← LAST, gated
```

| Change | Title | Gap | Effort | Depends on | Agent |
|---|---|---|---|---|---|
| C-001 | `config_store` SurrealDB migration | G-1 | S | none | claude |
| C-002 | `ConfigStore` trait + SurrealDB impl | G-2 | M | C-001 | claude |
| C-003 | Seed-once + content-hash/revision/provenance merge | G-3 | M | C-002 | claude |
| C-004 | Kernel effective-config read path (bootstrap ⊕ DB) | G-4 | M | C-002 | claude |
| C-005 | Re-target write endpoints to `ConfigStore` | G-5 | M | C-002, C-004 | claude |
| C-006 | Reload path reads DB store | G-6 | M | C-004 | claude |
| C-007 | Determinism `ORDER BY` + regression test | G-7 | S | C-004 | claude |
| C-008 | One-time prod `config.toml` → DB import + verify | G-9 | M | C-003, C-005 | claude (human-run verify) |
| C-009 | K8s: drop `init-config`, restore read-only ConfigMap | G-8 | S | C-008 verified | claude (human-deploy) |

---

## Per-change detail

### C-001 — `config_store` SurrealDB migration

**Files:**
- `crates/librefang-storage/src/migrations/sql/031_config_store.surql` (new)
- `crates/librefang-storage/src/migrations/mod.rs` (register `config_store_v1`)

**Depends on:** none · **Effort:** S · **Agent:** claude

- [ ] New `.surql`: `DEFINE TABLE config_store SCHEMAFULL`; fields `key: string`,
  `value: option<object> FLEXIBLE`, `source: string` (`"bootstrap"|"runtime"`),
  `content_hash: string`, `revision: int`, `updated_at: string`; UNIQUE index
  on `key`.
- [ ] Register in `mod.rs` migrations array as `config_store_v1` (next sequence
  after `030_composite_indexes`).
- [ ] BossFang rule: this is a NEW BossFang-exclusive table — note it in the
  migration header comment (no upstream SQLite equivalent expected).

**Done when:** `cargo check -p librefang-storage --lib` exit 0 and the migration
applies cleanly against an embedded instance in a `-p librefang-storage` test.

---

### C-002 — `ConfigStore` trait + SurrealDB impl

**Files:**
- `crates/librefang-storage/src/config_store.rs` (new) — trait + `Surreal<Any>` impl
- `crates/librefang-storage/src/lib.rs` (export)

**Depends on:** C-001 · **Effort:** M · **Agent:** claude

- [ ] Trait: `get(key) -> Option<ConfigEntry>`, `list(prefix) -> Vec<ConfigEntry>`,
  `upsert(key, value, source, content_hash, revision)`, `delete(key)`.
  `ConfigEntry` carries `value`, `source`, `content_hash`, `revision`, `updated_at`.
- [ ] One impl over the existing `Surreal<Any>` handle — covers embedded AND
  remote (no second impl). `list()` MUST `ORDER BY key` for determinism.
- [ ] `#[cfg(feature = "sqlite-backend")]` parity impl is best-effort; gate it so
  the default `surreal-backend` build is the load-bearing path.

**Done when:** `cargo check -p librefang-storage --lib` exit 0; unit test in
`-p librefang-storage` round-trips upsert→get→list→delete on an embedded DB.

---

### C-003 — Seed-once + content-hash/revision/provenance merge

**Files:**
- `crates/librefang-kernel/src/config_store_sync.rs` (new)
- `crates/librefang-kernel/src/kernel/mod.rs` or boot path (call seed at boot)

**Depends on:** C-002 · **Effort:** M · **Agent:** claude

- [ ] At boot: for each in-scope section (§3a), compute a content hash of the
  bootstrap value. If the DB has no row for that key → seed with
  `source="bootstrap"`, store the hash + `revision` (from bootstrap, default 0).
- [ ] If a row exists: compare bootstrap `content_hash`. Only merge when the
  hash changed **AND** (the existing row is `source="bootstrap"` OR the bootstrap
  `revision` is strictly greater than the stored `revision`). NEVER overwrite a
  `source="runtime"` row on a mere hash difference.
- [ ] **No `std::fs` mtime is read anywhere in this logic.** Add a code comment
  citing assessment FLAW 1.
- [ ] Unit tests: (a) fresh seed, (b) unchanged-hash no-op, (c) runtime row
  protected from bootstrap re-sync, (d) revision-bump overrides bootstrap row.

**Done when:** `cargo check -p librefang-kernel --lib` exit 0; the four merge
unit tests pass under `cargo test -p librefang-kernel config_store_sync`.

---

### C-004 — Kernel effective-config read path (bootstrap ⊕ DB)

**Files:**
- `crates/librefang-kernel/src/config.rs` (resolve effective config)
- `crates/librefang-kernel/src/kernel/subsystems/mcp.rs` (populate
  `effective_mcp_servers` from DB, not `cfg.mcp_servers`)

**Depends on:** C-002 · **Effort:** M · **Agent:** claude

- [ ] After bootstrap `KernelConfig` loads, overlay DB `config_store` values for
  in-scope keys → effective config held in the existing `ArcSwap`.
- [ ] `effective_mcp_servers` RwLock filled from the DB-overlaid value.
- [ ] Out-of-scope sections (§3b) untouched — read straight from file/env.

**Done when:** `cargo check -p librefang-kernel --lib` exit 0; a
`-p librefang-kernel` test asserts a DB-stored MCP server appears in
`effective_mcp_servers` after boot with an empty `cfg.mcp_servers`.

---

### C-005 — Re-target write endpoints to `ConfigStore`

**Files:**
- `crates/librefang-api/src/routes/skills.rs` (`upsert_mcp_server_config`,
  `remove_mcp_server_config`)
- `crates/librefang-api/src/routes/providers.rs` (`persist_default_model`)
- `crates/librefang-api/src/routes/config.rs` (`config_set` allowlisted subset)

**Depends on:** C-002, C-004 · **Effort:** M · **Agent:** claude

- [ ] Each write goes to `ConfigStore.upsert(..., source="runtime", ...)` instead
  of editing TOML. Keep all existing validation (allowlist, transport checks,
  duplicate-name checks) unchanged.
- [ ] Preserve `config_write_lock` serialization semantics (or a DB transaction).
- [ ] Out-of-scope writes (secrets/auth/storage) keep their current file/env path.
- [ ] **MANDATORY integration test** per repo rule (#3721): `#[tokio::test]`
  against `TestServer` — `POST /api/mcp/servers` then read back asserts the
  entry came from the DB store and survives a simulated reload.

**Done when:** `cargo test -p librefang-api` green including the new
TestServer case; `POST /api/mcp/servers` no longer touches `config.toml`.

---

### C-006 — Reload path reads DB store

**Files:**
- `crates/librefang-kernel/src/kernel/config_reload_ops.rs`
- `crates/librefang-kernel/src/config_reload.rs` (`build_reload_plan`)

**Depends on:** C-004 · **Effort:** M · **Agent:** claude

- [ ] `POST /api/config/reload` re-resolves effective config (bootstrap ⊕ DB) and
  rebuilds the `ReloadPlan`; a DB-only `mcp_servers` change triggers
  `HotAction::ReloadMcpServers` exactly as a file change did.
- [ ] Integration test: change an MCP server via the DB store, call reload,
  assert the connection set updates.

**Done when:** `cargo check -p librefang-kernel --lib` + `cargo test -p librefang-api`
green; reload reflects DB changes with no file edit.

---

### C-007 — Determinism `ORDER BY` + regression test

**Files:**
- `crates/librefang-storage/src/config_store.rs` (confirm `ORDER BY key`)
- `crates/librefang-kernel/src/kernel/subsystems/mcp.rs` (stable order into summary)
- test next to existing `mcp_summary_*` tests

**Depends on:** C-004 · **Effort:** S · **Agent:** claude

- [ ] Assert the MCP summary is byte-identical regardless of DB row
  insertion order (mirror `mcp_summary_is_byte_identical_across_input_orders`).

**Done when:** new determinism test passes; `cargo test -p librefang-kernel`
green.

---

### C-008 — One-time prod `config.toml` → DB import + verify

**Files:**
- `crates/librefang-cli/src/commands/storage.rs` (add `config import` subcommand
  — BossFang storage command surface already exists)
- `crates/librefang-cli/src/cli.rs` (wire subcommand)

**Depends on:** C-003, C-005 · **Effort:** M · **Agent:** claude (verify = human)

- [ ] CLI: `librefang storage config-import [--from <path>]` reads the existing
  on-PVC `config.toml`, seeds the DB store treating every in-scope value as
  `source="bootstrap"` revision 0 — idempotent (skips keys already present).
- [ ] On normal boot, C-003's seed handles fresh installs; this command is the
  explicit, auditable path for the existing prod PVC.
- [ ] **HUMAN verification (Live, port 4545 / cluster):** scale to a maintenance
  pod, run the import, confirm `GET /api/mcp/servers` returns the pre-existing
  prod entries from the DB. Claude prepares the exact commands; human runs them.

**Done when:** human confirms the current prod MCP servers + provider default are
present in the DB store after import, with `source="bootstrap"`.

---

### C-009 — K8s: drop `init-config`, restore read-only ConfigMap (LAST)

**Files:**
- `k8s/base/bossfang-deployment.yaml` (remove `init-config` initContainer;
  restore read-only ConfigMap subPath mount; rename volume back to `config`)
- `k8s/overlays/production-gke/patches/bossfang-pvc-storage.yaml` (PVC keeps
  `/data` for embedded SurrealDB + npm/uv caches — size unchanged)

**Depends on:** C-008 **verified in prod** · **Effort:** S · **Agent:** claude (deploy = human)

- [ ] Revert to read-only ConfigMap mount; the file is now bootstrap-defaults only.
- [ ] `config.toml` writes from the UI now land in the DB, so the read-only mount
  no longer blocks them — the original `os error 30` symptom is structurally gone.
- [ ] **Do NOT merge this until C-008 confirms prod data is safely in the DB.**

**Done when:** `kubectl apply -k k8s/overlays/production-gke` rolls out; UI MCP
add succeeds with a read-only ConfigMap mounted; no `os error 30` in logs.

---

## Risks carried from assessment

- **R-1 (high):** C-009 before C-008-verified = data loss. Hard-gated above.
- **R-2 (med):** embedded SurrealDB connection bootstrap stays in file+env (D4) —
  avoids the chicken-and-egg deadlock.
- **R-3 (med):** determinism — covered by C-007.
- **R-4 (low):** multi-PR change; no outage pressure (init-config works today).
  Land additive (C-001..C-008) behind existing behavior, flip K8s last (C-009).

## Verification gates (every change)

```
cargo check --workspace --lib
cargo check -p librefang-storage -p librefang-uar-spec -p librefang-memory
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p <crate>            # scoped only
python3 scripts/enforce-branding.py --check
```

## Next

`/kbd-execute C-001`
