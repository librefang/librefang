# [High] `/api/providers/github-copilot/oauth/*` is unauthenticated and lacks CSRF — hostile page hijacks `GITHUB_TOKEN`

**Severity:** High · **Domain:** Auth & secrets · **Source:** `audit-01-auth-secrets.md`

## Location
- Public prefix: `crates/librefang-api/src/middleware.rs:714` (`PublicRoute::prefix_any("/api/providers/github-copilot/oauth/")`)
- Start: `crates/librefang-api/src/routes/providers.rs:2143-2180`
- Poll → completion side effect: `crates/librefang-api/src/routes/providers.rs:2220-2236`

## Problem
The `prefix_any` covers both `POST start` and `GET poll/{id}`. No auth, no rate limit, no per-IP cap. On completion the poll handler calls `write_secret_env(..., "GITHUB_TOKEN", &access_token)` and `set_env_var_guarded("GITHUB_TOKEN", ...)` — the device-flow access token is written into the daemon's environment and `secrets.env`.

## Exploit
Pop-under page in a victim's browser:
1. `POST http://localhost:4545/api/providers/github-copilot/oauth/start` (no preflight required for a simple POST, no origin check).
2. Display the returned `user_code` + `verification_uri` in the attacker's UI, OR social-engineer the user to enter the code at `github.com/login/device`.
3. On success the daemon now uses the **attacker's** GitHub Copilot credential for all outbound LLM calls — attacker pays nothing, the victim's daemon traffic and prompts flow through the attacker's GitHub account.

## Fix
Remove the public-prefix entry. The dashboard already authenticates before initiating the flow; no legitimate unauthenticated caller exists. Defense in depth: Origin allowlist enforcement on state-mutating endpoints.

## Tests
- `POST /api/providers/github-copilot/oauth/start` without `Authorization` → 401.
- `GET .../poll/{id}` without authn → 401.
- Origin check rejects cross-site requests when authn cookie present.
