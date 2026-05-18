# [Low] Auth Pass-2 Low roundup — react_asset substring check, blocking DNS, static api_key injection, Tauri CORS risk

**Severity:** Low · **Domain:** HTTP API authentication & authorization
**Status:** Merges 3 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `webchat::react_asset` path traversal check is just `path.contains("..")` substring matching — Windows `\..\` segments bypass it | `webchat.rs:262-263, 144-163` |
| blocking DNS | `is_url_safe_for_ssrf` runs blocking `ToSocketAddrs::to_socket_addrs` inside an async handler — slow DNS stalls a tokio worker | `routes/network.rs:659` and 3 callers |
| static api_key | Holders of a static API key are not injected with the `AuthenticatedApiUser` extension — owner-bound uploads get rejected (consistency bug) | `middleware.rs:1171-1174`, `routes/agents.rs:6152-6172` |
| Tauri CORS | CORS allowlist includes `tauri://localhost` + `Any` headers/methods — becomes high-risk the day `allow_credentials(true)` lands | `server.rs:1119-1145` |

## Combined fix plan

1. (this) Canonicalize + verify `starts_with(dashboard_dir)`; split on both `/` and `\` and reject any segment equal to `..`. Share a single path validator with the "react-asset-spa-fallback-phish" cluster's UNC-bypass sub-finding.
2. (blocking DNS) Move DNS resolution into `tokio::task::spawn_blocking`, or simply remove the route-level pre-check (rely on `A2aClient`'s existing SSRF guard + DNS pinning).
3. (static api_key) When the static api_key matches, inject a synthetic `AuthenticatedApiUser { role: Owner, name: "api_key", user_id: UserId::system() }` to align with the semantics of the other auth branches.
4. (Tauri CORS) Add an inline comment at `server.rs:1142-1145` pinning the "never `allow_credentials(true)`" invariant; add a unit test asserting the CORS layer builder excludes credentials.

## Tests

- (this) `GET /dashboard/..%5C..%5Cetc/passwd` → 400/404, no off-tree read.
- (blocking DNS) With a slow DNS server, sending requests leaves the tokio runtime responsive (spawn_blocking isolation).
- (static api_key) `LIBREFANG_API_KEY` request to an owner-bound upload URL → 200.
- (Tauri CORS) Unit test: assert the CORS-builder snapshot does not contain `allow_credentials(true)`.
