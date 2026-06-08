# Assessment — Phase 9: Database-Backed Config Store (MCP & runtime-mutable settings)

**Project:** LibreFang (BossFang fork)
**Date:** 2026-06-06
**Phase:** phase-9-config-store-migration
**Author:** kbd-assess
**Status:** Gap report — design verdict + scoped recommendation

---

## 0. The ask (restated)

Move MCP server registrations — and other registrations currently stored in
config files — into the database, with this lifecycle:

1. On first install, load the config files.
2. Check whether values already exist in the DB. If not, seed the DB from the files.
3. After seeding, **read config from the DB** as the operating source of truth.
4. When a config file changes, **merge new values into the DB**.
5. On conflict (file value ≠ DB value), **newer entry wins** (file mtime vs. DB record timestamp).
6. Must work for **all supported storage backends** (per your clarification: SurrealDB
   **embedded** and **remote** — both already supported via `librefang-storage`).
7. If the design is sound, **revert config.toml on K8s back to a read-only ConfigMap**
   (drop the `init-config` copy-to-PVC workaround).
8. Adapt **any setting that can be changed in the application** so K8s config edits
   are never blocked by a read-only mount.

---

## 1. Verdict (sycophancy-corrected)

**The core idea is correct and industry-aligned. Three specifics of the
described mechanism are wrong and would cause production bugs. Adopt the idea;
change the mechanism.**

This is not a rubber-stamp. The instruction was to critically assess, so below
are the parts that hold up and the parts that do not — with concrete failure
modes, not vibes.

### 1a. What is right

- **Runtime-mutable, UI-editable config belongs in a database, not a file.**
  This is the *External Configuration Store* pattern (Microsoft) and the
  "your config file should be a database" argument (DoltHub). The moment a
  config value is edited by a GUI by a user (not a Git commit by an operator),
  a file is the wrong home — you are fighting Kubernetes' read-only ConfigMap
  mount and the immutable-infrastructure grain. The current `init-config`
  initContainer that copies `config.toml` onto a PVC is exactly the smell this
  pattern exists to remove.
- **The substrate already exists.** `librefang-storage` is SurrealDB-backed
  (embedded + remote), the default backend, with 30+ migrations. There is
  already a `kv_store` table (`crates/librefang-storage/src/migrations/sql/006_kv_store.surql`)
  carrying `version: int` and `updated_at: string` fields — the exact shape a
  config store needs. (Note: `kv_store` is **agent-scoped** via a UNIQUE
  `(agent_id, key)` index, so it is *not* directly reusable for system config;
  a sibling `config_store` table is the clean move — see §4.)
- **It makes future multi-replica HA easier, not harder.** Today the daemon is
  single-replica behind a `daemon.lock` (`Recreate` strategy). A shared DB
  config store is a prerequisite for ever running >1 replica; the file-on-PVC
  (`ReadWriteOnce`) approach actively blocks it. This is a real, if not
  immediate, bonus.
- **Reverting to a read-only ConfigMap is the right end-state** *once* the DB is
  the source of truth — the ConfigMap becomes immutable, GitOps-versioned
  bootstrap defaults, which is precisely the recommended hybrid (versioned
  defaults in files, runtime overrides in DB).

### 1b. What is wrong — three concrete flaws

**FLAW 1 — "Newer wins by file mtime" is unreliable in Kubernetes and will
silently clobber UI edits.**
A ConfigMap projected as a volume is updated by the kubelet via an atomic
symlink swap on *every* sync, and the projected file's mtime changes even when
the **content is identical**. With "file newer ⇒ file wins," every pod restart
or unrelated ConfigMap update would make the file look newer than the user's
older UI edits and **revert them**. mtime is not a content-change signal here.
→ **Correction:** compare a **content hash** of the bootstrap source, not mtime.
Re-seed/merge only when the bootstrap content actually changed.

**FLAW 2 — Whole-file "newer wins" is too coarse; it must be per-key and
provenance-aware.**
"If a config from the file is different from the DB version, the newer entry
wins" applied across the file means: an operator edits the ConfigMap to fix
*one* value, and every UI-set value that happens to be older gets reverted in
the same merge. That is a footgun.
→ **Correction:** track **provenance per key** (`source: "bootstrap" | "runtime"`).
A bootstrap re-sync only overwrites keys whose last writer was *also* bootstrap,
or keys the operator explicitly bumped via a **revision counter** in the
ConfigMap. UI-set (`runtime`) values are never silently reverted by a file edit.

**FLAW 3 — "All settings" is provably impossible and partly unsafe.**
- **Bootstrap paradox:** the `[storage]` section (backend kind, namespace,
  database, remote URL, `password_env`) **cannot** live in the DB — you need it
  to *connect* to the DB. There is an irreducible bootstrap set that must stay
  in file/env.
- **Security surface:** `/api/config/set` already enforces a deliberate
  **allowlist** and **blocks** `api_key`, `default_model.*`, `providers.*`,
  `auth.*`, `network`, `dashboard_pass*`
  (`crates/librefang-api/src/routes/config.rs` ~L2725–2875). Secrets and auth
  config should not be migrated into a general config table.
→ **Correction:** scope the migration to the **UI-mutable runtime subset**
  (see §3), explicitly excluding the bootstrap set and secrets.

### 1c. Corrected design in one paragraph

ConfigMap (read-only) = immutable, Git-versioned **bootstrap defaults**.
SurrealDB `config_store` table = **runtime source of truth** for the mutable
subset, seeded **once** from the bootstrap source on first boot. Re-sync from
the file is **content-hash-gated and revision-gated**, **per-key**, and
**provenance-aware** — never an automatic mtime race, and never overwriting a
UI-set value unless the operator explicitly bumps the bootstrap revision.
Bootstrap/secret/storage-connection settings stay in file+env. Then, and only
then, drop the `init-config` PVC-copy workaround.

---

## 2. Current state (as-built)

| Aspect | Current implementation | File |
|---|---|---|
| Config source of truth | `config.toml` (TOML), parsed at boot | `librefang-kernel/src/config.rs:load_config` |
| Top-level struct | `KernelConfig` | `librefang-types/src/config/types.rs:2851` |
| MCP entries | `Vec<McpServerConfigEntry>` field `mcp_servers` | `…/types.rs:3064`, entry struct `:5584` |
| Add MCP server | `POST /api/mcp/servers` → `upsert_mcp_server_config()` writes TOML | `librefang-api/src/routes/skills.rs:4316,4827` |
| Remove MCP server | `DELETE /api/mcp/servers/{name}` → `remove_mcp_server_config()` | `…/skills.rs:4869` |
| Default model | `POST /api/providers/{name}/default` → `persist_default_model()` writes TOML | `librefang-api/src/routes/providers.rs:1977` |
| Generic config set | `POST /api/config/set` (allowlisted, `toml_edit`) | `librefang-api/src/routes/config.rs:2413` |
| Write serialization | `AppState.config_write_lock: tokio::Mutex<()>` | `librefang-api/src/routes/mod.rs:181` |
| Live MCP snapshot | `McpSubsystem.effective_mcp_servers: RwLock<Vec<…>>` | `librefang-kernel/src/kernel/subsystems/mcp.rs:63` |
| Hot reload | `POST /api/config/reload` → `reload_config()` → `build_reload_plan` → `reload_mcp_servers()` | `…/kernel/config_reload_ops.rs`, `config_reload.rs`, `kernel/mcp_setup.rs:324` |
| Change detection | **None** — no file watcher; reload is explicit only | (deliberate) |
| Storage backend | SurrealDB (embedded default, remote opt-in); SQLite legacy | `librefang-storage` |
| Existing KV table | `kv_store` (agent-scoped, `version`+`updated_at`) | `…/migrations/sql/006_kv_store.surql` |
| K8s config delivery | ConfigMap → `init-config` initContainer copies to PVC `/data/config.toml` (writable) | `k8s/base/bossfang-deployment.yaml` |

**Key finding:** config storage is **TOML-only today**. The DB stores
operational data (sessions, audit, usage, approvals, memory) but **no config**.
There is no `ConfigStore` abstraction in the kernel — config is a plain
deserialized `KernelConfig` struct held in an `ArcSwap`.

---

## 3. Scope — what migrates, what stays

### 3a. MIGRATE to `config_store` (UI-mutable runtime subset)
- `mcp_servers` (the headline case)
- `default_model` selection (currently blocked from `/api/config/set`, has a
  dedicated endpoint — move the *persistence target*, keep the validation)
- `provider_urls` (custom base URLs)
- The current `/api/config/set` allowlist scalars/sections: `log_level`,
  `max_history_messages`, `ui.*`, `approval.*`, `language`, `mode`,
  `channels.*`, `web.*`, `tool_policy.*`, `memory.*`, `extensions.*`

### 3b. STAYS in file + env (bootstrap / secret / connection set)
- **`[storage]`** — backend kind, namespace, database, remote URL,
  `password_env` (bootstrap paradox — needed to reach the DB)
- **Secrets / auth** — `api_key`, `auth.*`, `dashboard_pass*`, vault key, provider
  API keys (these are Secrets/env in K8s, not config)
- **`api_listen` / `network` / bind address** — process-level, set before the DB
  is reachable
- `LIBREFANG_ALLOW_NO_AUTH`, `BOSSFANG_HOME`, `BOSSFANG_VAULT_KEY` — already env

> The "any setting that can be changed in the application" goal is satisfied by
> 3a. The K8s read-only-mount problem only ever bites the **UI-mutable** subset,
> which is exactly what 3a covers. Nothing in 3b is editable from the app UI, so
> none of it needs the DB.

---

## 4. Gaps to close (work items)

| # | Gap | Sketch | Size |
|---|---|---|---|
| G-1 | No system-scoped config table | New migration `031_config_store.surql`: `key UNIQUE`, `value: object FLEXIBLE`, `source: "bootstrap"\|"runtime"`, `content_hash: string`, `revision: int`, `updated_at`. (Do **not** reuse agent-scoped `kv_store`.) | S |
| G-2 | No `ConfigStore` abstraction | Trait in `librefang-storage` (or kernel) with `get/list/upsert/delete`, impl over SurrealDB (embedded+remote both via existing `Surreal<Any>` handle — one impl covers both). SQLite legacy parity if `sqlite-backend` must keep working. | M |
| G-3 | No file→DB seed-once + gated re-sync | Boot step: read bootstrap `KernelConfig`, compute per-section content hash, compare to stored `content_hash`+`revision`. Seed if absent; merge **per-key, provenance-aware** if bootstrap revision bumped. **No mtime comparison.** | M |
| G-4 | Config reads are file-only | Kernel must resolve effective config = bootstrap defaults ⊕ DB overrides at boot and on reload; `effective_mcp_servers` populated from DB, not `cfg.mcp_servers`. | M |
| G-5 | Write endpoints write TOML | Re-target `upsert_mcp_server_config`, `remove_mcp_server_config`, `persist_default_model`, `config_set` to write the DB store (stamp `source="runtime"`). Keep `config_write_lock` semantics (or move to DB transaction). | M |
| G-6 | Reload path keyed on file | `POST /api/config/reload` re-reads DB store (+ bootstrap), rebuilds `ReloadPlan`. A DB-config change can trigger the same `HotAction::ReloadMcpServers`. | M |
| G-7 | Determinism (#3298) | DB query for `mcp_servers` (and any prompt-reaching list) **must `ORDER BY`** deterministically — TOML's implicit insertion order is gone once it's rows. Add a regression test mirroring the existing `mcp_summary_*` tests. | S |
| G-8 | K8s revert | After G-1..G-7 land and verify: drop `init-config` initContainer, restore read-only ConfigMap mount, keep PVC only for `/data` runtime state (npm/uv caches, workspaces, vault, **the SurrealDB embedded files**). | S |
| G-9 | Migration/back-compat for existing prod | Existing prod PVC already has a live `config.toml` with UI edits. First boot of the new code must seed the DB **from that existing file** (treat as bootstrap), not from the ConfigMap, or those edits are lost. One-time import path. | M |

---

## 5. Risks & sequencing

- **R-1 (high):** G-9 ordering. If you revert to ConfigMap (G-8) *before* the
  DB seed reliably imports the existing PVC `config.toml`, you lose current
  production UI edits (MCP servers, provider default). **G-8 must be the last
  step, gated on G-9 verification.**
- **R-2 (med):** Embedded SurrealDB lives on the same PVC. The config store and
  its own connection bootstrap must not deadlock at first boot (storage config
  stays in file — see §3b — which avoids this).
- **R-3 (med):** Determinism regression (G-7) — easy to miss, breaks prompt
  caching silently. Covered by a test, per repo rule #3298.
- **R-4 (low):** This is a multi-PR architectural change, not a bug fix. The
  current `init-config` workaround **works today**, so there is no outage
  pressure — sequence deliberately, land behind the existing behavior, flip at
  the end.

**Suggested order:** G-1 → G-2 → G-4 → G-5 → G-3 → G-6 → G-7 → G-9 → G-8.
(Storage primitives first, then read path, then write path, then seed/merge,
then reload, then determinism test, then prod-safe import, then K8s flip.)

---

## 6. Recommendation

**Proceed — with the corrected mechanism.** Build the SurrealDB `config_store`
(embedded + remote, one impl), seed-once from bootstrap, **content-hash + revision
+ per-key provenance** merge (not mtime, not whole-file, not "newer-always-wins"),
scoped to the UI-mutable subset (§3a), bootstrap/secret/storage-connection set
stays in file+env (§3b). Revert K8s to a read-only ConfigMap **only as the final
step**, gated on a verified one-time import of the existing production
`config.toml`.

Do **not** implement the literal "file mtime newer wins, applied to all settings"
design as stated — flaws 1–3 (§1b) make it a regression generator in exactly the
Kubernetes environment it is meant to fix.

---

## 7. Sources (best-practice research)

- [External Configuration Store Pattern — Microsoft Azure Architecture Center](https://learn.microsoft.com/en-us/azure/architecture/patterns/external-configuration-store)
- [Your config file should be a database — DoltHub](https://www.dolthub.com/blog/2023-05-15-your-config-file-should-be-a-database/)
- [Why you Shouldn't Rely on Configuration in your Database — Medium (counterpoint)](https://medium.com/@connercharlebois/why-you-shouldnt-rely-on-configuration-in-your-database-bcab3c4bb614)
- [Last-Write-Wins conflict resolution — OneUptime](https://oneuptime.com/blog/post/2026-01-30-last-write-wins/view)
- [Last Write Wins vs CRDTs — DZone](https://dzone.com/articles/conflict-resolution-using-last-write-wins-vs-crdts)
- [Configuration drift in Kubernetes — garden.io](https://garden.io/blog/configuration-drift)
- [Azure App Configuration best practices (K8s, hot reload)](https://learn.microsoft.com/en-us/azure/azure-app-configuration/howto-best-practices)
- [About sources of truth — GKE Config Sync](https://docs.cloud.google.com/kubernetes-engine/config-sync/docs/concepts/sources-of-truth)
