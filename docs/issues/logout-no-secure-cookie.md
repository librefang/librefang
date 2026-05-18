# [Medium] `dashboard_logout` does NOT clear `Secure` cookie when invoked over plain HTTP

**Severity:** Medium · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
- `crates/librefang-api/src/server.rs:611-664` (`dashboard_logout`)
- Cookie attrs: `crates/librefang-api/src/server.rs:352` (`session_cookie_attrs`)

## Problem
`session_cookie_attrs` derives the cookie string from the request's TLS posture. A logout over plain HTTP emits no `Secure` flag, so the browser doesn't match it against the originally-`Secure` cookie and the **original cookie is not invalidated client-side**. The server-side session is removed (so the token is dead), but the browser keeps presenting it until the next failed auth.

## Fix
Always emit `Secure` on the cookie-clear path. Modern browsers accept `Secure` on `Max-Age=0` Set-Cookie responses regardless of transport.

## Tests
- Logout via HTTP → response sets `Secure; Max-Age=0`; subsequent request shows browser dropped the cookie.
