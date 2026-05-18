# [Medium] MCP OAuth callback hardening — empty code, http fallback, error info leak

**Severity:** Medium · **Domain:** Auth & secrets
**Status:** Merges 2 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | The callback POSTs to the IdP without rejecting an empty `code`, so the PKCE verifier ends up in IdP access logs | `routes/mcp_auth.rs:627-632` |
| http fallback | The callback URL Host fallback uses `http://` (not HTTPS) | `routes/mcp_auth.rs` (Host-inference section) |
| auth_status leak | The MCP `auth_status` error response leaks internal information (paths, stack fragments, etc.) | `routes/mcp_auth.rs` (`auth_status` handler) |

## Why merged

All three live in the same handler cluster in `mcp_auth.rs`; any PR will touch them together.

## Combined fix plan

1. **(this) Reject empty `code` early**:
   ```rust
   if code.is_empty() { return auth_failed("Missing authorization code."); }
   ```
   Mirror the `None` branch.
2. **(http fallback) Force https in Host fallback**: use `http://` only when the request is HTTPS (with `X-Forwarded-Proto: https` from a trusted proxy — see "x-forwarded-proto-trusted-proxies") or when a local-dev switch is explicitly enabled; otherwise default to `https://`.
3. **(auth_status leak) Sanitize `auth_status` errors**: return a typed error code rather than internal messages:
   ```rust
   #[derive(Serialize)]
   struct AuthStatusError { code: &'static str, hint: Option<String> }
   ```
   Restrict to a small set of pre-defined codes (`needs_auth`, `refresh_failed_transient`, `refresh_failed_permanent`, `invalid_config`).

## Tests

- `GET .../callback?code=&state=valid` → 400, no outbound network call.
- The `auth_status` error response body contains no path or stack.
- Host fallback produces `https://` under a real HTTPS deployment.
