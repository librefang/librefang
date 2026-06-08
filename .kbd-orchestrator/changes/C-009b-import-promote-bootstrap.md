# Change C-009b — Import must PROMOTE boot-seeded bootstrap rows to runtime

**Phase:** phase-9-config-store-migration
**Status:** CODE DONE (2026-06-08) — fixes a gap found during the live deploy of C-009
**Gap:** G-9 (corrected) · **Effort:** S · **Depends on:** C-008, C-009 · **Agent:** claude

## Why
C-009's import wrote prod values as `source=runtime` via **seed semantics** —
which only writes when the store value DIFFERS from config.toml. But on the live
cluster the new image (`bdf5fc802`) had already deployed, and its **boot-seed**
wrote prod's `config.toml` values into the store as `source=bootstrap`. So the
import would read the same values, compute `Unchanged`, **write nothing**, and
leave the rows `bootstrap`. The C-009 ConfigMap revert would then read the empty
baseline and `BootstrapUpdated`-overwrite them → **prod config wiped (R-1)**.

The C-009 fix was correct only for an empty store (import-before-first-boot); CI
deploying on merge invalidated that assumption. Found by inspecting the actual
cluster state before applying the revert (deployment still had `init-config`;
image already on the new SHA).

## What landed
- `config_store_overlay.rs`: new `ImportOutcome { Seeded, Promoted, AlreadyRuntime }`
  + `promote_or_seed(store, key, config_value)`. The import now **preserves the
  store's current value** (the post-deploy source of truth) and re-stamps it
  `runtime`; it only falls back to config.toml's value when no row exists. A
  `bootstrap` row is **promoted** to runtime (value unchanged) even when
  value-equal — the case a seed would skip. `runtime` rows are left untouched.
- `import_mcp_servers` / `import_default_model` return `ImportOutcome`.
- CLI `storage.rs`: `import_config_values` + `describe` + tests updated; dry-run
  preview unchanged (already reports each key's current store `source`).

## Verification (code, green)
- `cargo test -p librefang-cli --features surreal-backend -- commands::storage::tests`:
  - `config_import_is_idempotent_and_non_destructive` (Seeded → AlreadyRuntime;
    UI-edit preserved on re-import).
  - `import_promotes_boot_seeded_bootstrap_rows_to_runtime` (boot-seed bootstrap
    → import Promoted → post-revert boot-seed `RuntimeProtected`, value survives).
  - `imported_values_survive_post_cutover_boot_seed`.
- `cargo test -p librefang-api --test config_store_overlay_test` (seed path intact).
- clippy + brand.

## Deploy implication
The cutover image must be **≥ this fix** before running the import. Since the
currently-deployed image (`bdf5fc802`) predates it, the cluster must roll to the
new SHA first, THEN run `librefang storage config-import` daemon-down, THEN apply
the C-009 revert. Same strict order; this just makes the import actually
protective for the already-running-daemon case.
