# librefang-types

Core types and traits for the [LibreFang](https://github.com/librefang/librefang) Agent OS.

Defines the shared data structures used across the kernel, runtime,
memory substrate, and wire protocol. **Contains no business logic** —
it is the schema spine that every other crate depends on.

## Public modules

`agent`, `approval`, `capability`, `comms`, `config`, `error`,
`event`, `goal`, `i18n`, `manifest_signing`, `media`, `memory`,
`message`, `model_catalog`, `oauth`, `registry_schema`, `scheduler`,
`serde_compat`, `subagent`, `taint`, `tool`, `tool_class`.

## Constants

- `VERSION: &str` — workspace version, set at compile time from
  `CARGO_PKG_VERSION`.

## Key dependencies

`serde`, `serde_json`, `chrono`, `uuid`, `thiserror`, `dirs`, `toml`,
`schemars`, `utoipa`.

## Schema drift

`KernelConfig` derives `JsonSchema`; the golden-file fixture lives in
`librefang-api`'s test suite (a PR touching `librefang-types` schema
automatically pulls `librefang-api` into the affected-crate set in CI).
The canonical OpenAPI / TOML example baselines are tracked under
[`xtask/baselines/`](../../xtask/baselines/).

See the [workspace README](../../README.md).
