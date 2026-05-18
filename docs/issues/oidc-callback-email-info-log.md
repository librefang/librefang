# [Low] OAuth / IdP secrets hygiene Low — email PII log, escape_env, empty sub

**Severity:** Low · **Domain:** Auth & secrets
**Status:** Merges 2 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | OIDC `auth_callback` logs the user's email at INFO level (PII) | `oauth.rs:1064-1068` |
| escape_env asymmetry | `escape_env_value` is asymmetric — write and read rules diverge; corner cases leak | env helper |
| empty sub | `auth_userinfo` does not guard against an empty `sub` claim — an empty `sub` silently becomes the "anonymous user" or panics | `oauth.rs::auth_userinfo` |

## Combined fix plan

1. (this) Demote to `debug!`, or log only the email domain: `info!(domain = %email.rsplit('@').next().unwrap_or(""), "login")`.
2. (escape_env asymmetry) Write an `escape_env_value_roundtrip_proptest`: random string → escape → unescape == original. Fix the asymmetry it surfaces.
3. (empty sub) `auth_userinfo` returns 400 / `invalid_token` immediately when `sub.is_empty()`, and never enters user provisioning.
