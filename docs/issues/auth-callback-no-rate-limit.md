# [High] `/api/auth/callback` not in auth rate-limit allowlist — captured `state` can DoS pending logins

**Severity:** High · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
- Public route: `crates/librefang-api/src/middleware.rs:692` (`PublicRoute::exact_any("/api/auth/callback")`)
- Handlers: `crates/librefang-api/src/oauth.rs:614, 685`
- Rate-limit allowlist: `crates/librefang-api/src/rate_limiter.rs:350-361`

## Problem
The endpoint must be public, but the auth rate-limiter covers `dashboard-login`, `login*`, `introspect`, `refresh`, `approve`, `totp/confirm` — **not** `callback`. Every successful HMAC verify **eagerly consumes the `oauth_nonce_used` slot before code exchange** (`oauth.rs:744-758`). A captured state token (referer, proxy logs, browser history) can be replayed from the open internet to lock the real user out of completing their login. No per-IP cap and no rate-limit signal in logs.

## Exploit
Attacker scrapes a `state` value from a CF/proxy log of a victim's login flow, replays `POST /api/auth/callback` 50 ms before the legitimate GET arrives. Real user sees "OAuth callback already redeemed; please restart the sign-in flow." Free login DoS.

## Fix
Add `/api/auth/callback` (GET + POST, both `/api/` and `/api/v1/`) to the auth-rate-limit allowlist in `rate_limiter.rs::auth_rate_limit_layer`.

## Tests
- 11 sequential callbacks from same IP → 429 by request 11.
- Burst-friendly limits documented (don't break legitimate retry after IdP redirect timeout).
