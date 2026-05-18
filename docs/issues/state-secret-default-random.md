# [Critical] `LIBREFANG_STATE_SECRET` defaults to a per-process random key — OIDC state breaks across restart and multi-replica deployments

**Severity:** Critical
**Domain:** Auth & secrets

## Location

`crates/librefang-api/src/oauth.rs:281-286`

```rust
static KEY: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    std::env::var("LIBREFANG_STATE_SECRET").unwrap_or_else(|_| uuid::Uuid::new_v4().to_string())
});
```

## Problem

This is the HMAC key for OAuth `state` tokens (the CSRF/nonce envelope used by `build_state_token` / `verify_state_token` on every `/api/auth/callback`). With no env var set:

1. **Restart-time DoS:** every in-flight state token verifies as "signature failed" after a daemon restart, even though the IdP already exchanged the code. Users see intermittent "Invalid or expired state parameter" with no operator signal.
2. **Multi-replica silently broken:** the documented Cloudflare Tunnel / reverse-proxy posture (`trusted_proxies` + `client_ip.rs`) requires shared state across replicas — each replica signs with its own `Uuid::new_v4()`, so callbacks landing on a different node reject every legitimate user.

An attacker who learns the deployment posture can lock out legitimate users by forcing a restart (no auth needed if `/api/health` is the LB probe).

## Fix

Require `LIBREFANG_STATE_SECRET` (or a `state_signing_key` in `config.toml`) when `external_auth.enabled = true`, refuse to boot otherwise. Mirror the `LIBREFANG_VAULT_KEY` 32-byte base64 contract so the value is rotateable. Document the multi-replica implication in the same place as `trusted_proxies`.

## Tests

- Boot test: `external_auth.enabled = true` with no `LIBREFANG_STATE_SECRET` → daemon refuses to start with a clear error.
- Boot test: malformed env var (not 32 bytes base64) → refuses to start.
- Integration: start two daemons with the same key, complete a login flow that bounces between them (simulated via two `start_full_router` instances sharing a Redis-mocked nonce store) → succeeds.
