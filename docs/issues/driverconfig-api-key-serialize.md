# `DriverConfig::api_key` default `Serialize` emits cleartext

**Severity:** Medium
**Category:** Secrets & credential handling
**Labels:** `security`, `secrets`, `medium`

## Affected files
- `crates/librefang-llm-driver/src/lib.rs:555-616` (struct + `Serialize` derive)
- `crates/librefang-llm-driver/src/lib.rs:664-691` (hand-written `Debug`, properly redacted)

## Description

`Debug` is hand-written and redacted, but `Serialize` is derived and emits the `String` verbatim. Any `serde_json::to_*` / `toml::to_*` of `DriverConfig` — cache dump, future diagnostic dump, `mcp_config.json`, cross-process snapshot — lands the API key in cleartext in the artifact.

`proxy_url` has the same problem: `Debug` redacts, `Serialize` doesn't; proxy URLs commonly carry `user:pass@host`.

## Recommendation

Pick one:

1. Drop `Serialize` / `Deserialize` outright (the kernel does not need to JSON-roundtrip `DriverConfig`; test fixtures aside);
2. Annotate `api_key` and `proxy_url` with `#[serde(skip_serializing)]`, and provide an explicit `serialize_secrets()` path for the few sites that genuinely need a round-trip.

Add a unit test: `serde_json::to_string(&driver_config).unwrap()` output does not contain the full api_key string.
