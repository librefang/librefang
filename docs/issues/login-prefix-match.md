# `/api/auth/login` allowlist entry is missing its trailing slash, leaving a prefix-match foot-gun

**Severity:** Medium
**Category:** HTTP API authentication & authorization
**Labels:** `security`, `auth`, `middleware`, `medium`

## Affected files
- `crates/librefang-api/src/middleware.rs:726` (`PublicRoute::prefix_get("/api/auth/login")`)
- `crates/librefang-api/src/middleware.rs:808` (prefix-match implementation: `path.starts_with(route.path)`)

## Description

The allowlist entry has no trailing slash and uses `starts_with` prefix matching → every GET whose path starts with `/api/auth/login` is treated as public, including `/api/auth/login-status`, `/api/auth/logins`, `/api/auth/loginhack`, etc.

Today, `server.rs:92-96` only registers exact `/api/auth/login` and `/api/auth/login/{provider}`, so there is no live exposure. But every other prefix entry in this slice (`/api/budget/agents/`, `/api/hands/`, `/dashboard/assets/`, `/api/providers/github-copilot/oauth/`) ends with a slash — the inconsistency is a latent foot-gun: whoever later adds a sibling GET (`/api/auth/login-debug`, say) silently inherits unauthenticated access.

## Recommendation

Mirror the `/api/budget/agents` pattern, splitting into:

```rust
PublicRoute::exact_get("/api/auth/login"),
PublicRoute::prefix_get("/api/auth/login/"),
```

Add a unit test verifying that `/api/auth/login-foo` is not classified as public.
