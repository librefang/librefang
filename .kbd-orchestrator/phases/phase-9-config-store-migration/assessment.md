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

---

# C-005d Assessment — Migrate `memory` + `channels` settings to SurrealDB (2026-06-14)

**Status:** Gap report — design verdict + scoped recommendation
**Scope (user):** Move memory and channel settings from `config.toml` into the database, using the same C-005c method — load the file first, write new/changed settings into the DB, then run everything from the DB.

## 1. Verdict

**Proceed — and the credential blocker I flagged when deferring this is smaller than it looked.**
The actual secret *values* never need to enter the database: channels already separates them, and memory only stores an env-var *pointer*.
The migration reuses the C-005c `config_overrides` infrastructure end-to-end, plus a small "trusted-handler section" apply path so these typed endpoints aren't gated by the generic `config_set` allowlist.

## 2. Current state (the file-writing paths)

- **memory** — `memory_config_patch` (`PATCH /api/memory/config`, `crates/librefang-api/src/routes/memory.rs:1592`) reads `config.toml`, edits the `[memory]` (`embedding_provider`, `embedding_model`, `embedding_api_key_env`, `decay_rate`) and `[proactive_memory]` (`enabled`, `auto_memorize`, `auto_retrieve`, `extraction_model`, `max_retrieve`) tables, then `std::fs::write`s the whole file (`memory.rs:1666`) and `reload_config()`s. Zero config-store references — fails with `os error 30` under the read-only ConfigMap.
- **channels** — `configure_sidecar_channel` (`POST /api/channels/sidecar/{name}/configure`, `crates/librefang-api/src/routes/channels.rs:809`) **already splits the payload**: secret values go to `~/.librefang/secrets.env` (`channels.rs:862,884`), and only the non-secret `[[sidecar_channels]]` block goes to `config.toml` via `sidecar_toml::upsert_sidecar_block` (`crates/librefang-api/src/routes/sidecar_toml.rs:11`). The config.toml half is what fails under the read-only mount.

KernelConfig fields involved: `memory: MemoryConfig` (`types.rs:3170`), `proactive_memory: ProactiveMemoryConfig` (`types.rs:3630`), `sidecar_channels: Vec<SidecarChannelConfig>` (`types.rs:3569`).

## 3. Design reframe — why this is now safe

The deferral reason was that memory/channels carry credential-redirect fields (`embedding_api_key_env`, `token_env`/`secret_env`) that the `config_set` allowlist blocks via its `_env` / depth-2 rules. On inspection:

1. **Secret VALUES already stay out of `config.toml`.** Channels writes credentials to `secrets.env`; the migration touches only the `[[sidecar_channels]]` *structure* (name/command/args/channel_type/non-secret env). secrets.env is left exactly as-is — secrets never enter the DB (assessment §3b security rationale holds).
2. **`embedding_api_key_env` is a POINTER, not a secret.** It holds the *name* of an env var (e.g. `OPENAI_API_KEY`); the key itself lives in the env var (a K8s Secret). Storing the pointer in the DB exposes no secret value.
3. **The `_env` / depth-2 allowlist is the boundary for the UNTRUSTED generic `config_set` endpoint** (an API caller with a leaked key must not write arbitrary dotted paths). The memory PATCH and channels configure endpoints are **dedicated, typed, structurally-validated** handlers — they are on the same trust footing as `persist_budget`. So they may write their own section to the store via a trusted path; the typed `deserialize → validate_config_for_reload` at resolve time is the guard, exactly as for budget (C-005d budget already shipped, PR #84).

The resolve-time allowlist re-check in `resolve_config_with_overrides` is belt-and-suspenders against a *directly-tampered store row* — but anyone who can write the embedded SurrealDB has already cleared the daemon's trust boundary, so it is not a hard gate. The hard gate is the API surface, which these typed handlers own.

## 4. Gap table

| Gap | Setting domain | Resolution | Effort |
|---|---|---|---|
| G-d1 | `[memory]` (incl. `embedding_api_key_env` pointer) | Store as a trusted whole-section override `memory`; resolve applies it. | S |
| G-d2 | `[proactive_memory]` | Store as override `proactive_memory` (no credential fields at all). | S |
| G-d3 | `[[sidecar_channels]]` array (config only) | Store the array as override `sidecar_channels`; secrets stay in `secrets.env`. | M |
| G-d4 | Trusted-section apply path | resolve must apply `memory`/`proactive_memory`/`sidecar_channels` overrides WITHOUT the `config_set` allowlist gate (they are NOT in `is_writable_config_path`, and must not be — config_set users still can't write them). | S |
| G-d5 | Read-back consistency | `GET /api/memory/config` + the channel-row synthesizers read `kernel.config_ref()` (the live, overlay-resolved config), so once the override is applied via `replace_config` the reads are correct with no change. | — |

## 5. Proposed mechanism (reuse C-005c)

Add a **trusted-section** variant of the override apply:

- `crates/librefang-api/src/config_store_overlay.rs`: a `TRUSTED_SECTION_KEYS: &[&str] = &["budget", "memory", "proactive_memory", "sidecar_channels"]` set, and make `resolve_config_with_overrides` apply an override when its key is allowlisted **OR** in `TRUSTED_SECTION_KEYS` (instead of only `is_writable_config_path`). Budget already round-trips via the exact-allowlist; folding it into the trusted set unifies the two and removes the `"budget"` allowlist entry (so config_set still can't write whole sections, but the typed handlers can).
- **memory handler** (`memory.rs:1592`): under `surreal-backend`, build the resulting `[memory]` and `[proactive_memory]` tables (config.toml ⊕ patch), persist them as overrides `memory` + `proactive_memory`, resolve + `replace_config`. sqlite-only keeps the file path.
- **channels handler** (`channels.rs:809`): keep the `secrets.env` write unchanged. Replace the `config.toml` `upsert_sidecar_block` write (surreal branch) with: read current `sidecar_channels` (from `kernel.config_ref()`), upsert the named block in memory, store the resulting `Vec<SidecarChannelConfig>` as override `sidecar_channels`, resolve + `replace_config`. The `reload_channels_from_disk` restart of the bridge manager stays.

This is the same load-file → write-DB → run-from-DB flow as budget; the boot/reload overlay (`overlay_config_overrides`, `config_store_overlay.rs:550`) already re-applies all `config_overrides` entries, so memory/channels survive restart automatically once stored.

## 6. Security analysis (the crux)

- **Never store a secret value.** Channels secrets stay in `secrets.env` (untouched). Audit the channels migration to confirm NO secret-valued field is folded into the `sidecar_channels` override — only the schema-managed non-secret keys + `*_env` pointers.
- **`*_env` pointers in the DB are acceptable** (they name an env var; the value is in the env). Document this explicitly; it is the one judgment call.
- **Tampered-store-row risk is unchanged** from budget — DB write access == daemon trust boundary already breached. The typed `deserialize → validate_config_for_reload` still runs at resolve, so a malformed override is rejected, live config unchanged.
- **config_set stays locked.** `memory`/`channels`/`sidecar_channels` remain OUT of `is_writable_config_path` — only the typed endpoints can write them; the generic `config_set` API still cannot, preserving the `_env` / depth-2 protection on the untrusted surface.

## 7. Proposed changes (C-005d)

1. **C-005d.1** — `resolve_config_with_overrides` trusted-section apply path + `TRUSTED_SECTION_KEYS` (fold in `budget`; remove the `"budget"` exact-allowlist entry). Tests: trusted key applies; non-trusted non-allowlisted key still skipped; a tampered blocked path still skipped.
2. **C-005d.2** — memory handler → store (`memory` + `proactive_memory` overrides). Test: PATCH → store → live config reflects it → survives restart via overlay; `config.toml` untouched.
3. **C-005d.3** — channels handler → store (`sidecar_channels` override); secrets.env unchanged. Test: configure → secrets.env still written, sidecar_channels override applied, `config.toml` untouched, bridge reload still fires; **security test** — a secret-valued field is NOT present in the stored override.

## 8. Risks

- **R-d1 (med):** channels' `upsert_sidecar_block` has include-file shadowing logic (`channels.rs:805,880`) — the DB path must preserve the "refuse if an included file already declares the block" guard or it silently shadows. Keep the pre-write include check.
- **R-d2 (low):** `[memory].sqlite_path` is a storage path; moving it to a runtime override could relocate the memory DB mid-run. It is restart-required in the reload plan, so `replace_config` swaps the config but the live store keeps its boot path — acceptable, same as today's reload.
- **R-d3 (low):** `MemoryConfig` / `SidecarChannelConfig` must round-trip cleanly through `serde_json → toml_edit inline → KernelConfig` (the `env: HashMap` becomes an inline table). Covered by the round-trip tests.

## 9. Verification

- Unit/integration in `crates/librefang-api/tests/config_store_overlay_test.rs`: per-section resolve + trusted-apply + the security (no-secret-value) assertion.
- `cargo check --workspace --lib`; `clippy -p librefang-api`; `enforce-branding.py --check`.
- Live (HUMAN/cluster, after deploy): `PATCH /api/memory/config` and `POST /api/channels/sidecar/{n}/configure` return success under the read-only `config.toml`, values survive a pod restart, and `config.toml` stays read-only.

**Suggested order:** C-005d.1 → C-005d.2 → C-005d.3.
