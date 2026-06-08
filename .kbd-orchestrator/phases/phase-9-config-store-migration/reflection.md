# Phase 9 — Config-Store Migration · Reflection

**Phase:** phase-9-config-store-migration
**Reflected:** 2026-06-08
**Backend:** native-tool
**Outcome PRs:** GQAdonis/librefang **#74** (C-001…C-008, 10 commits) + **#75**
(C-009 + a C-008 cutover-safety fix) — both MERGED to `main` (2026-06-08).
**Revised after C-009** to record the data-loss bug found+fixed during the cutover.

## Goal

Move runtime-mutable configuration out of `config.toml` into the SurrealDB
config store so the dashboard can persist changes when `config.toml` is mounted
read-only from a Kubernetes ConfigMap — eliminating the
`Read-only file system (os error 30)` the UI hit on config writes.

## Goal Achievement (per assessment gap)

| Gap | Description | Change(s) | Status |
| --- | --- | --- | --- |
| G-1 | System-scoped config table | C-001 | **MET** |
| G-2 | `ConfigStore` trait + impl | C-002 | **MET** |
| G-3 | Seed-once + provenance/content-hash merge (no mtime) | C-003 | **MET** |
| G-4 | Effective config = bootstrap ⊕ DB at boot | C-004 | **MET** |
| G-5 | Write endpoints → DB store | C-005 (MCP), C-005b (provider-default) | **MET** (MCP + provider); `config_set` → **C-005c** |
| G-6 | Reload re-resolves from store | C-006 | **MET** |
| G-7 | Determinism `ORDER BY` (#3298) | C-007 | **MET** |
| G-9 | One-time prod `config.toml` → DB import | C-008 (+C-009 fix) | **CODE MET** · prod run = HUMAN pending |
| G-8 | K8s ConfigMap revert (read-only) | C-009 | **CODE MET** (PR #75) · apply = HUMAN, gated on G-9 |

**Assessment design FLAWs (all corrected in the build):**
- FLAW 1 (mtime unreliable in K8s) → C-003 compares **content hashes, never mtime**.
- FLAW 2 (whole-file too coarse) → per-key, provenance-aware merge; a `runtime`
  row is only overridden by an explicit `BOSSFANG_CONFIG_BOOTSTRAP_REVISION` bump.
- FLAW 3 ("all settings" impossible/unsafe) → scope held to MCP + provider-default;
  secrets/auth/storage stay in file+env; generic `config_set` consciously deferred.

**Goal achievement: all 9 gaps CODE-MET and merged (#74 + #75).** The headline
objective — UI config writes survive a read-only `config.toml` for the high-value
paths (MCP servers + provider default) — is shipped. The only remaining work is
the **human prod cutover** (run C-008 import, then `kubectl apply` the C-009
revert, in that order) and the deferred **C-005c** (generic `config_set`).

### Late finding (C-009): a data-loss bug in the merged C-008
Planning the cutover surfaced that C-008's import wrote prod's live values as
`source=bootstrap`. Because the prod ConfigMap baseline is empty for these keys,
the first post-revert boot-seed would have taken `BootstrapUpdated` and
**overwritten prod's MCP/provider config with empty** (R-1 realized). Fixed in
#75: the import now writes `source=runtime` (→ `RuntimeProtected` on boot-seed),
with a `imported_values_survive_post_cutover_boot_seed` regression test.
**Lesson:** trace the *first boot after* a destructive ops change end-to-end
before shipping the change that triggers it — the bug was invisible until the two
halves (import provenance + revert) were considered together.

## Delivered Changes

| Change | Commit | Summary |
| --- | --- | --- |
| C-001 | `bf3c261` | `031_config_store` SurrealDB migration (one row per key) |
| C-002 | `f6cdb6d` | `ConfigStore` trait + `SurrealConfigStore` (provenance, content-hash, ORDER BY key) |
| C-004 | `6dba1c1` | API-owned boot overlay + process-global `shared_pool()`; kernel effective-list accessor/replacer |
| C-005 | `41a098e` | MCP write path → store (`source=runtime`); two-view sync; extensions converted |
| C-003 | `93f80ab` | seed-once + provenance merge (content-hash, never mtime) |
| C-006 | `dac087f` | `config_reload` re-resolves from store (no clobber) |
| C-007 | `a826231` | `list()` ORDER BY determinism guard (#3298) |
| C-005b | `b07a58e` | provider default-model → store (override-RwLock); full-router HTTP CRUD test; `MockKernelBuilder` temp-storage isolation |
| C-008 | `9263bad` | `librefang storage config-import` (idempotent, non-destructive, reuses boot-seed) |
| — | `89e1fcf` | incidental rustfmt of terminal/webchat (kept isolated) |

## Artifact Quality Summary

The `artifact-refiner` / `refine-validate` skill was **unavailable in this
session** for the entire phase (no `.refiner/artifacts/*` logs exist). QA was
therefore enforced per-change by the project's own gates instead:

| Metric | Value |
| --- | --- |
| Changes (code) delivered | 9 (C-001…C-008 incl. C-005b) |
| Per-change gate: `cargo check --workspace --lib` | pass (all) |
| Per-change gate: scoped `clippy -D warnings` | pass (all) |
| Per-change gate: brand audit (`enforce-branding.py --check`) | clean (all) |
| Integration/unit tests added | 53 passing across 5 binaries |
| Changes < 3 files (QA-skip per skill rules) | C-007 (1 file) |

Test coverage by binary: `config_store_overlay_test` (10), `mcp_http_crud_test`
(1), `providers_routes_test` (37), `librefang-storage config_store` (4),
`librefang-cli config_import` (1). All green on an isolated target dir (never the
shared `target/`).

### Recurring Constraint Violations
None recorded (no refiner run). No clippy/brand violations survived any change.

## Risks (assessment) — disposition

- **R-1 (high, data loss on cutover ordering):** mitigated. C-008 import is
  idempotent + **non-destructive** (UI rows `RuntimeProtected`); C-009 is
  hard-gated on the human-verified import. Encoded in `progress.json`
  (`k8s_revert_gated_on`) and the C-008 runbook.
- **R-2 (med, embedded single-lock):** mitigated by `shared_pool()` (one cached
  transport per path, process-wide) — discovered concretely during C-004 testing.
- **R-3 (med, determinism regression):** mitigated by C-007 guard test.
- **R-4 (low, multi-PR churn):** handled — shipped as one reviewable, merged PR.

## Technical Debt Introduced

1. **Sqlite-only API build is pre-existing-broken** (`open_trace_store` cfg
   mismatch in `plugins.rs`); the sqlite fallback paths added this phase are
   gated correct-by-construction but not full-build-verified. Untouched by us.
2. **`api_integration_test` doesn't compile on the base** (`sync_registry`
   3-vs-4 arg). Pre-existing; not in scope. Worth a separate fix.
3. **C-005c not done:** `POST /api/config/set` still writes `config.toml` →
   it will fail under the C-009 read-only mount for those (lower-value) paths
   until a kernel config-override merge layer lands (see Next Phase). Documented
   for operators in the C-009 change doc + PR #75.
4. **Generic-vs-specific test-name filters:** the C-009 cutover-survival test
   (`imported_values_survive_*`) was silently skipped by a `config_import`
   name-filter on first run — caught only by re-running the whole test module.
   Filter on the module path, not a substring, when adding differently-named
   tests to an existing module.

## Lessons Captured

1. **The kernel has no operational SurrealDB session at boot** — the assessment's
   "kernel reads the store" mechanism was wrong. The corrected design (API-owns
   the store, pushes resolved config into the kernel via accessor/replacer) is
   the load-bearing architectural decision (D9). Re-validate such assumptions
   against the actual boot path before planning DB reads from the kernel.
2. **Embedded RocksDB = one lock per path per process** → every store consumer
   must share one pool (`shared_pool()`); a fresh pool deadlocks. (D10 / R-2.)
3. **Single ordered JSON-array row beats row-per-server** for `mcp_servers`:
   preserves write order, and prompt determinism was already guaranteed by
   `render_mcp_summary`'s sort — so C-007 shrank from "new kernel logic" to a
   storage `ORDER BY` guard. Check existing invariants before adding new ones.
4. **Reuse the daemon's own seed logic in the import CLI** (C-008) so behaviour
   can't drift from boot — and it inherits the non-destructive guarantee for free.
5. **CWD-relative default `StorageConfig`** silently pollutes the repo in tests;
   `MockKernelBuilder` must pin temp storage. Watch for shared global state
   (pools, CWD paths) when adding a new persistence consumer that existing tests
   exercise.
6. **`cargo check --workspace --lib` skips binary crates** — CLI surreal-gated
   code needs `cargo check -p librefang-cli --features surreal-backend`.
7. **Process discipline that paid off:** `AskUserQuestion` at the two real scope
   forks (MCP-only for C-005; provider-default-only for C-005b) kept scope honest
   and surfaced the `config_set` difficulty instead of silently punting it.

## Recommended Focus for Next Phase

1. **Finish the cutover (ops, human-gated):** run C-008 import on prod PVC →
   verify → land **C-009** (read-only ConfigMap revert) as a manifest-only PR.
2. **C-005c — generic `config_set` to the store:** design a kernel-side
   config-override **merge layer** applied to `KernelConfig` at load + reload
   (no in-memory path-setter exists today), with a security review for
   section-wholesale writes (the `channels.*` depth-1 hazard).
3. **Pre-existing test-suite repair** (separate, small): `api_integration_test`
   `sync_registry` arity; consider whether the sqlite-only API build should be
   fixed or formally dropped.
