# [High] OAuth refresh path (`mcp_oauth_provider.rs::try_refresh`) hardening — log leak, error classification, race, unwrap

**Severity:** High
**Category:** Secrets · Error handling · Concurrency
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Severity | Description |
|--------|----------|-------------|
| this | High | Error-response body is logged verbatim via `warn!` / `error!`, which may emit `access_token` / `refresh_token` in clear text |
| error classification | Medium | `try_refresh` collapses transient (5xx / timeout) and permanent (`invalid_grant` / revoked) outcomes into `Ok(None)`; callers wrongly conclude they must re-auth |
| concurrent refresh race | Medium | Concurrent refreshes lack a per-`server_url` single-flight; rotating-refresh-token providers (Google, GitHub Apps, Notion) invalidate sessions that are still valid |
| unwrap on refresh_token | Low | `find_any_with_refresh -> entry.refresh_token.unwrap()` relies on a documentation invariant; easy to panic under refactoring |

## Affected files

- `crates/librefang-kernel/src/mcp_oauth_provider.rs:183-187` (error string assembly)
- `crates/librefang-kernel/src/mcp_oauth_provider.rs:264-313` (`try_refresh` body)
- `crates/librefang-kernel/src/mcp_oauth_provider.rs:296-310` (`load_token` call)
- `crates/librefang-api/src/routes/mcp_auth.rs:799-809` (2xx parse-failure `body_preview`)
- `crates/librefang-api/src/oauth.rs:1358` (`refresh_token.unwrap()`)

## Why merged

All four live inside the same function or in its immediate call chain. Any fix touches the same code; tracking them separately would force four PRs against one function.

## Combined fix plan

1. **Log sanitization (this)**: never emit the token-endpoint response verbatim — only record status, Content-Type, and `sha256(body)[..8]`. The 2xx parse-failure `body_preview` is sanitized the same way (strip `access_token` / `refresh_token` / `id_token` / `client_secret`).
2. **Error classification (error classification)**:
   ```rust
   match status {
       s if s.is_success()                                    => Ok(Some(token)),
       s if s == 400 && err == "invalid_grant"                => Ok(None),                     // truly revoked
       s if s.is_server_error() || is_timeout                 => Err(RefreshError::Transient), // retry
       _                                                      => Err(RefreshError::Permanent),
   }
   ```
   Callers back off and retry on `Transient`; only `Ok(None)` flips the server back to `NeedsAuth`.
3. **Single-flight concurrency (concurrent refresh race)**:
   ```rust
   type RefreshLocks = Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>;
   ```
   After acquiring the lock, re-read `expires_at` from the vault — if a peer has already refreshed, return its token directly.
4. **Eliminate the unwrap (unwrap on refresh_token)**: change `find_any_with_refresh`'s return type to one that is non-optional:
   ```rust
   pub fn find_any_with_refresh(...) -> Option<(Subject, RefreshToken, Entry)> { ... }
   ```
   Or define `NonEmptyEntry { refresh_token: String, ... }`.

## Tests

- (log sanitization) Construct a response body containing `access_token: "secret"` → no log line contains `secret`.
- (error classification) 503 → retry; `invalid_grant` → `NeedsAuth`.
- (single-flight) Two concurrent `load_token` calls → only one HTTP request reaches the token endpoint.
- (unwrap) `find_any_with_refresh` returning `Some` is a compile-time proof that `refresh_token` is non-empty.
