# [High] `require_auth_for_reads = Some(false)` exposes `/api/agents` / `/api/sessions` / `/api/providers` on non-loopback bind

**Severity:** High · **Domain:** API attack surface · **Source:** `audit-02-api-attack-surface.md`

## Location
- Allowlist branch: `crates/librefang-api/src/middleware.rs:761-795`
- Effective resolution: `crates/librefang-api/src/middleware.rs:1048-1055`
- Bind/config: `crates/librefang-api/src/server.rs:1165-1200`

## Problem
The `Some(false)` value is intended as an "external auth proxy" escape hatch (e.g. nginx with auth_request). But the code path honors it **even when no external proxy exists**, leaving the operator's `/api/agents`, `/api/sessions`, `/api/providers` enumerable to any unauthenticated reader if config drifts or `bind = "0.0.0.0:..."` is paired with `require_auth_for_reads = Some(false)` by mistake.

The "external proxy in front" assumption is not enforced in code.

## Fix
Add an explicit `external_auth_proxy = true` flag (or rename to `require_auth_for_reads = "proxied"`) and refuse to honor the bypass unless that flag is set. On boot, warn if `bind` is non-loopback and `external_auth_proxy = false`.

## Tests
- Boot with `bind = "0.0.0.0:..."` + `require_auth_for_reads = false` + no proxy flag → warn at startup; reads still require auth.
- Same config + `external_auth_proxy = true` → reads bypass auth (the proxy is responsible).
