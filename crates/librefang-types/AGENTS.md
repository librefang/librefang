# librefang-types — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for cross-cutting rules.

## Purpose

The schema spine. Shared data structures used across the kernel, runtime, memory substrate, and wire protocol.
**Contains no business logic.** Pure types + small derive-only helpers.

## Boundary

- Owns: every cross-crate type — agent, approval, capability, comms, config, error, event, goal, i18n, manifest_signing, media, memory, message, model_catalog, oauth, registry_schema, scheduler, serde_compat, subagent, taint, tool, tool_class.
- Does NOT own: implementation. Functions that *do* something belong in the crate that uses the type, not here.
- Depends on: `serde`, `serde_json`, `chrono`, `uuid`, `thiserror`, `dirs`, `toml`, `schemars`, `utoipa`. **No** workspace crate. We're at the bottom of the dep DAG.

## Schema-mirror invariant (refs #3144 → #3162 → #3167)

`librefang-types` defines the schema, but the golden-file guard (`kernel_config_schema_matches_golden_fixture`) lives in `librefang-api`. Any change to a `KernelConfig` field — addition, rename, type change — requires regenerating the golden fixture in api/tests.

CI catches this via the changed-lanes rule: a `librefang-types`-only PR auto-pulls `librefang-api` into the affected test set. Don't try to defeat that rule; it exists for a reason.

## Adding a new type

1. Place under the matching submodule. New module = decide if it's truly a cross-crate type or belongs in the consuming crate.
2. Derive the standard quartet: `Debug`, `Clone`, `Serialize`, `Deserialize`. Add `PartialEq` / `Eq` / `Hash` only when needed downstream.
3. For OpenAPI surface types: also derive `utoipa::ToSchema`.
4. For configuration types: also derive `schemars::JsonSchema` (driven by the kernel-config golden fixture).
5. Use `BTreeMap` / `BTreeSet` instead of `HashMap` / `HashSet` for any field that ends up in an LLM prompt (refs #3298).

## Configuration field ritual

When adding a field to a config struct:

1. Add field with `#[serde(default)]` for forward-compat with old TOML.
2. Add to the `Default` impl. Build silently breaks otherwise.
3. Add a doc comment — `schemars` surfaces it as the field's `description` in the JSON Schema.
4. Re-run the kernel-config golden in `librefang-api` (CI will fail otherwise).

## Error types

This crate exports `LibreFangError` and friends. Per #3541 / #3711 we are migrating away from `Result<_, String>` and `anyhow::Error` in trait boundaries — new error variants belong here, not as ad-hoc `String`s in consumer crates.

When adding a new variant: preserve the `source()` chain (#3745). `#[from]` on a wrapped enum is the standard idiom.

## Public API surface

- `VERSION: &str` — workspace version, set at compile time from `CARGO_PKG_VERSION`.
- All modules listed above.

## Taboos

- No `tokio` here. Sync types only.
- No `reqwest` here. Wire types are data-only; HTTP code lives in consumers.
- No `librefang-*` imports. We're the bottom of the DAG; reverse the dependency.
- No implementation. If you find yourself writing a function body longer than 5 lines, it probably belongs in a consumer crate.
- No `HashMap<K, V>` for prompt-bound types (#3298).
- No silently dropping a serde field. `#[serde(default)]` or fail at compile-time.
