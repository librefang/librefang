# `X-Forwarded-Proto` is honored without a trusted-proxy check, allowing `Secure`-cookie downgrade

**Severity:** Medium
**Category:** HTTP API authentication & authorization
**Labels:** `security`, `auth`, `medium`

## Affected files
- `crates/librefang-api/src/server.rs:340-346` (`request_is_https`)
- `crates/librefang-api/src/server.rs:352-358` (`session_cookie_attrs`)
- `crates/librefang-api/src/server.rs:1061-1067` (`trusted_proxies` config exists but is not consumed)

## Description

`request_is_https` reads the request's `X-Forwarded-Proto` header unconditionally, and `session_cookie_attrs` uses that to decide whether to attach `Secure`. The `trusted_proxies` config already exists but is not consulted here.

- An attacker can set `X-Forwarded-Proto: https` against a plain-HTTP bind to force their own `Secure` flag (low impact).
- **The critical case**: when a TLS proxy fails to forward that header (a common naive nginx setup), real HTTPS sessions emit cookies **without `Secure`**; a single accidental HTTP redirect leaks them on the wire.

## Recommendation

Pick one:

1. Whenever any authentication is enabled, always attach `Secure`, and document "HTTP is dev-only"; or
2. Trust `X-Forwarded-Proto` only when the request source is in `state.trusted_proxies`, mirroring the existing pattern in `client_ip.rs`.

Add an integration test for "no trusted proxy configured + forged HTTP header" → does not attach `Secure`.
