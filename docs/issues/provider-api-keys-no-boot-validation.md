# `provider_api_keys` env vars are not validated at boot

**Severity:** Low
**Category:** Secrets & credential handling
**Labels:** `config`, `observability`, `low`

## Affected files
- `crates/librefang-types/src/config/types.rs:5748-5760`
- credential-pool config: the `api_key_env` field

## Description

`provider_api_keys` maps provider names to **environment-variable names**. The runtime resolves them on demand; when the env var is unset or empty, it silently falls back to other sources (auth_profiles / default key).

Consequence: a typical typo — `OPENAI_API_KEY1` vs `OPENAI_API_KEY_1` (the pool config sample even shows this) — silently downgrades the operator's intended posture with no log trail.

## Recommendation

At boot, walk `provider_api_keys` and `credential_pools[].keys[].api_key_env`; for every env var that is declared but unset or empty, emit a `WARN` log naming the provider + env-var name.

This extends the existing length-check path for `LIBREFANG_VAULT_KEY`.
