# MCP OAuth Discovery & Authentication

**Date:** 2026-04-12
**Status:** Draft
**Branch:** `feat/mcp-oauth-discovery`

## Summary

Add automatic OAuth authentication for MCP servers using the Streamable HTTP
transport. Servers that require OAuth (like Notion's hosted MCP at
`https://mcp.notion.com/mcp`) will be authenticated transparently — zero config
for servers that implement MCP spec discovery, optional explicit config for
servers that don't.

## Goals

1. **Zero-config OAuth** for MCP servers that advertise OAuth metadata via
   `WWW-Authenticate` headers and `.well-known/oauth-authorization-server`
2. **Config fallback** for servers without discovery support
3. **Full token lifecycle** — cache in vault, refresh on expiry, re-auth when
   needed
4. **Non-blocking startup** — daemon boots cleanly; auth happens asynchronously
5. **Dashboard integration** — auth state visible in existing `#/mcp-servers`
   section with inline authorize/revoke actions

## Non-Goals

- OAuth for SSE or Stdio transports (only Streamable HTTP)
- Device code flow for headless environments (URL-to-terminal fallback instead)
- Replacing the existing extensions OAuth infrastructure

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Where does OAuth logic live? | `runtime/mcp_oauth.rs` with trait injection | Keeps runtime dependency-free; follows `KernelHandle` pattern |
| When does auth trigger? | UI-initiated only; daemon detects 401 but does not start flows | Avoids unreachable callback ports in Docker; user drives auth from dashboard |
| Callback routing? | Through API server on port 4545, not ephemeral localhost port | Works in Docker/headless — same port as dashboard, no extra forwarding |
| Blocking or non-blocking? | Non-blocking with degraded state | Daemon shouldn't wait on browser interaction |
| Discovery or config? | Discovery first, config.toml fallback | Zero-config for spec-compliant servers, explicit for others |
| How to detect 401? | Match rmcp's `AuthRequired` error type | rmcp already parses 401 and extracts `WWW-Authenticate` header |
| Headless auth? | Print URL to logs, user opens manually | Simple, covers Docker/remote server use case |

---

## Architecture

### Dependency Graph (unchanged)

```
runtime  ←  extensions  ←  kernel
                ↑              ↑
                └──────────────┘
```

No new inter-crate dependencies. Runtime defines a trait, kernel implements it
using extensions.

### New Files

| File | Crate | Purpose |
|------|-------|---------|
| `runtime/src/mcp_oauth.rs` | librefang-runtime | OAuth discovery, `WWW-Authenticate` parsing, `McpOAuthProvider` trait, `OAuthMetadata`, `McpAuthState` |
| `kernel/src/mcp_oauth_provider.rs` | librefang-kernel | Implements `McpOAuthProvider` using extensions vault + PKCE flow |
| `api/src/routes/mcp_auth.rs` | librefang-api | API endpoints for auth start/status/callback/revoke |

### Modified Files

| File | Change |
|------|--------|
| `runtime/src/mcp.rs` | `connect_streamable_http` accepts `Option<Arc<dyn McpOAuthProvider>>`, 401 detection and retry logic |
| `types/src/config/types.rs` | Add `McpOAuthConfig` struct, add `oauth: Option<McpOAuthConfig>` to `McpServerConfigEntry` |
| `kernel/src/kernel.rs` | Pass `McpOAuthProvider` into `connect_mcp_servers()`, track `McpAuthState` per server |
| `api/src/server.rs` | Register new `/api/mcp/{name}/auth/*` routes |
| Dashboard HTML/JS | Auth state badges and authorize/revoke buttons in `#/mcp-servers` |

---

## Trait Design

```rust
// In librefang-runtime/src/mcp_oauth.rs

/// Trait for OAuth token management — implemented by kernel using extensions.
/// Follows the KernelHandle pattern to avoid runtime depending on extensions.
#[async_trait]
pub trait McpOAuthProvider: Send + Sync {
    /// Load a cached access token for this server URL.
    /// Returns None if no token cached or token is expired with no refresh token.
    async fn load_token(&self, server_url: &str) -> Option<String>;

    /// Store OAuth tokens in the vault, keyed by server URL.
    async fn store_tokens(&self, server_url: &str, tokens: OAuthTokens) -> Result<(), String>;

    /// Clear cached tokens for this server URL.
    async fn clear_tokens(&self, server_url: &str) -> Result<(), String>;

    /// Start the PKCE authorization flow. Returns the auth URL for the browser.
    /// The provider sets up the localhost callback server and waits for the code
    /// exchange in the background.
    async fn start_auth_flow(
        &self,
        server_url: &str,
        metadata: OAuthMetadata,
    ) -> Result<AuthFlowHandle, String>;
}

/// Handle returned by start_auth_flow — allows waiting for completion.
pub struct AuthFlowHandle {
    /// URL the user needs to open in their browser.
    pub auth_url: String,
    /// Receiver that resolves when the user completes auth.
    pub completion: oneshot::Receiver<Result<OAuthTokens, String>>,
}
```

---

## OAuth Metadata Discovery

Three-tier resolution, first match wins:

### Tier 1: `WWW-Authenticate` Header (from rmcp `AuthRequired` error)

rmcp returns a structured `AuthRequiredError` with the parsed
`www_authenticate_header` string. Parse it to extract:

- `resource_metadata` parameter → URL to fetch full OAuth metadata JSON
- If no `resource_metadata`, extract `realm` as a hint

### Tier 2: `.well-known` Discovery

Fetch `{origin}/.well-known/oauth-authorization-server` where `origin` is
derived from the MCP server URL. Parse the standard OAuth Authorization Server
Metadata response (RFC 8414):

```json
{
  "issuer": "https://mcp.notion.com",
  "authorization_endpoint": "https://mcp.notion.com/oauth/authorize",
  "token_endpoint": "https://mcp.notion.com/oauth/token",
  "response_types_supported": ["code"],
  "code_challenge_methods_supported": ["S256"]
}
```

### Tier 3: `config.toml` Fallback

Use explicitly configured OAuth parameters:

```toml
[[mcp_servers]]
name = "custom-server"
transport = { type = "http", url = "https://my-server.com/mcp" }

[mcp_servers.oauth]
auth_url = "https://my-server.com/oauth/authorize"
token_url = "https://my-server.com/oauth/token"
client_id = "my-client-id"
scopes = ["read", "write"]
```

### Merge Behavior

Discovery results merge with config — config values take precedence where both
exist. This allows overriding a discovered `client_id` while using discovered
endpoints, for example.

---

## Connection Flow

### Initial Connect (daemon startup) — detection only

At boot, the daemon **only detects** whether a server needs OAuth. It does
**not** start PKCE flows or generate auth URLs — that is entirely UI-driven.

```
connect_mcp_servers() for each Http transport:
│
├─ provider.load_token(url)
│  ├─ Token found and valid → add Authorization header
│  ├─ Token found but near expiry → try refresh, use new token
│  └─ No token → proceed without auth header
│
├─ Attempt rmcp StreamableHttpClientTransport connection
│  ├─ Success → state = Authorized or NotRequired, done
│  │
│  └─ Error: AuthRequired(AuthRequiredError { www_authenticate_header })
│     ├─ state = NeedsAuth (no flow started, no auth URL generated)
│     ├─ Log: "MCP server {name} requires OAuth — authorize via dashboard"
│     └─ Daemon continues booting
│
└─ Other error → log, skip server (existing behavior)
```

### UI-Initiated Auth Flow

The entire OAuth handshake is initiated from the dashboard, ensuring the
callback goes through the API server's exposed port (4545):

```
User clicks "Authorize" in dashboard:
│
├─ Dashboard calls POST /api/mcp/{name}/auth/start
│  ├─ Discover OAuth metadata (tiers 1→2→3)
│  ├─ Dynamic Client Registration (RFC 7591) if no client_id
│  ├─ Generate PKCE verifier/challenge + state
│  ├─ Store verifier + state in vault (keyed by server name)
│  ├─ Build auth URL with redirect_uri = {origin}/api/mcp/servers/{name}/auth/callback
│  │   (origin derived from request Host header — works behind proxies)
│  ├─ state = PendingAuth { auth_url }
│  └─ Return { auth_url } to dashboard
│
├─ Dashboard opens auth_url in new browser tab
│  └─ User completes consent on OAuth provider's page
│
├─ Provider redirects to GET /api/mcp/{name}/auth/callback?code=...&state=...
│  ├─ Validate state matches stored value
│  ├─ Exchange code for tokens (POST to token_endpoint with code_verifier)
│  ├─ provider.store_tokens(url, tokens)
│  ├─ Retry MCP connection with Bearer token
│  ├─ state = Authorized
│  └─ Return HTML: "Authorization complete. You can close this tab."
│
└─ Dashboard polls /auth/status → sees "authorized" → updates badge
```

### Tool Call with Expired Token

```
call_tool(name, args):
│
├─ Execute via rmcp client.call_tool()
│  ├─ Success → return result
│  │
│  └─ Error matches AuthRequired or auth-related?
│     ├─ provider.load_token(url) with force refresh
│     │  ├─ Got new token → reconnect with new token, retry tool call once
│     │  └─ No refresh token → state = NeedsAuth, return error
│     │     "MCP server {name} requires re-authorization"
│     │
│     └─ Retry succeeded → return result
│
└─ Other error → return error as-is
```

---

## Config Types

### New: `McpOAuthConfig`

Added to `librefang-types/src/config/types.rs`:

```rust
/// Optional OAuth configuration for an MCP server.
/// Used as fallback when server doesn't support .well-known discovery,
/// or to override discovered values.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpOAuthConfig {
    /// OAuth authorization endpoint URL.
    #[serde(default)]
    pub auth_url: Option<String>,

    /// OAuth token endpoint URL.
    #[serde(default)]
    pub token_url: Option<String>,

    /// OAuth client ID. If omitted, uses the value from discovery.
    #[serde(default)]
    pub client_id: Option<String>,

    /// OAuth scopes to request.
    #[serde(default)]
    pub scopes: Vec<String>,
}
```

### Modified: `McpServerConfigEntry`

```rust
pub struct McpServerConfigEntry {
    pub name: String,
    pub transport: Option<McpTransportEntry>,
    pub timeout_secs: u64,
    pub env: Vec<String>,
    pub headers: Vec<String>,
    pub oauth: Option<McpOAuthConfig>,  // NEW
}
```

### New: Auth State Types (in runtime)

```rust
/// Authentication state for an MCP server connection.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum McpAuthState {
    /// Server connected without requiring auth.
    NotRequired,
    /// Server authenticated with a valid token.
    Authorized {
        expires_at: Option<String>,  // ISO 8601
    },
    /// Waiting for user to complete browser auth flow.
    PendingAuth {
        auth_url: String,
    },
    /// Token expired and no refresh token available.
    Expired,
}
```

---

## API Endpoints

New routes under `/api/mcp/{name}/auth/`:

### `GET /api/mcp/{name}/auth/status`

Returns the current auth state for a named MCP server.

**Response:**

```json
{
  "server": "notion",
  "state": "pending_auth",
  "auth_url": "https://mcp.notion.com/oauth/authorize?client_id=...&response_type=code&..."
}
```

```json
{
  "server": "notion",
  "state": "authorized",
  "expires_at": null
}
```

### `POST /api/mcp/{name}/auth/start`

Initiates OAuth discovery and PKCE flow for a server. Returns the auth URL.
Called by the dashboard "Authorize" button. This is the **only** entry point
for starting an OAuth flow — the daemon never initiates flows at boot.

Steps performed server-side:
1. Discover OAuth metadata (tiers 1→2→3)
2. Dynamic Client Registration (RFC 7591) if no `client_id` cached or configured
3. Generate PKCE verifier/challenge + CSRF state
4. Store verifier + state in vault keyed by server name
5. Build authorization URL with `redirect_uri` derived from the incoming
   request's `Origin` / `X-Forwarded-Host` / `Host` header:
   `{origin}/api/mcp/servers/{name}/auth/callback`
   (no hardcoded port — works behind reverse proxies and in Docker)
6. Return the auth URL to the dashboard

**Response:**

```json
{
  "auth_url": "https://mcp.notion.com/authorize?client_id=...&code_challenge=...&state=...&redirect_uri=https%3A%2F%2Fyour-host%2Fapi%2Fmcp%2Fservers%2Fnotion%2Fauth%2Fcallback"
}
```

### `GET /api/mcp/servers/{name}/auth/callback`

OAuth redirect callback on the API server. After the user completes consent,
the OAuth provider redirects the browser here. The URL is derived from the
request host at `/auth/start` time, so it works regardless of port or proxy.

This runs on the **same port as the API server**, so it works in Docker and
headless setups with no extra port forwarding. No ephemeral localhost listener
is needed.

Steps performed:
1. Validate `state` matches the stored value (CSRF protection)
2. Load stored PKCE `code_verifier` from vault
3. Exchange authorization `code` for tokens (POST to token_endpoint)
4. Store tokens in vault
5. Retry MCP server connection with the new Bearer token
6. Update auth state to `Authorized`

**Query params:** `code`, `state`

**Response:** HTML page — "Authorization complete. You can close this tab."

### `DELETE /api/mcp/{name}/auth/revoke`

Clears cached tokens from vault and disconnects the MCP server.

**Response:**

```json
{
  "server": "notion",
  "state": "not_required"
}
```

---

## Dashboard Changes

Modifications to the existing `#/mcp-servers` section in the Alpine.js SPA:

### Auth State Badge

Each MCP server row gets a status badge:

- **Connected** (green) — `NotRequired` or `Authorized`
- **Authorize** (amber, clickable) — `PendingAuth`
- **Expired** (red, clickable) — `Expired`
- **Disconnected** (gray) — connection failed for non-auth reasons

### Authorize Button

For `PendingAuth` and `Expired` states, clicking the badge:

1. Calls `POST /api/mcp/{name}/auth/start`
2. Opens `auth_url` in new tab via `window.open()`
3. Polls `GET /api/mcp/{name}/auth/status` every 2 seconds
4. Updates badge when state changes to `authorized`

### Revoke Action

Authorized servers show a "Revoke" option (in dropdown or secondary action):

1. Calls `DELETE /api/mcp/{name}/auth/revoke`
2. Server disconnects, badge updates

---

## Token Storage

### Vault Key Scheme

Tokens stored in the encrypted vault (`~/.librefang/vault.enc`), keyed by
server URL:

```
mcp_oauth:{url}:access_token    — the Bearer token
mcp_oauth:{url}:refresh_token   — refresh token (if provided)
mcp_oauth:{url}:expires_at      — Unix timestamp (if provided)
mcp_oauth:{url}:metadata        — cached discovery metadata as JSON
```

### Token Refresh Logic

```
load_token(server_url):
  1. Read access_token from vault → None? return None
  2. Read expires_at from vault → None? return access_token (no expiry, e.g. Notion)
  3. expires_at > 60s from now? return access_token
  4. Read refresh_token from vault → None? return None (triggers re-auth)
  5. POST to token_url with grant_type=refresh_token
  6. Store new tokens in vault
  7. Return new access_token
```

### Vault Unavailability

If the vault is unavailable (no keyring, no env var), tokens are stored
in-memory only. The user re-authorizes on every daemon restart. This matches
existing vault fallback behavior — no new failure modes introduced.

---

## Headless / Docker Support

The OAuth flow is **entirely UI-driven** — initiated from the dashboard, with
the callback routed through the API server on port 4545 (the same port the
dashboard is served on). No ephemeral localhost listener is needed.

This means:
- **No extra port forwarding** — if you can reach the dashboard, OAuth works
- **No browser on the server** — the user's browser handles the consent page
- **Docker-friendly** — only port 4545 needs to be exposed (already required)

At daemon startup, if an MCP server returns 401, it is marked as `NeedsAuth`
and a log message directs the user to the dashboard:

```
MCP server "notion" requires OAuth — authorize via dashboard
```

---

## Testing Strategy

### Unit Tests (runtime/mcp_oauth.rs)

- Parse `WWW-Authenticate` headers → extract `resource_metadata` URL
- Parse `.well-known` JSON → `OAuthMetadata`
- Merge discovered metadata with config overrides
- Token expiry logic: fresh / near-expiry / expired / no-expiry
- Vault key generation from server URLs

### Integration Tests (librefang-testing)

- Mock MCP server returning 401 with `WWW-Authenticate` header
- Mock `.well-known` endpoint returning OAuth metadata
- Mock token endpoint accepting PKCE code exchange
- Full flow: connect → 401 → discover → auth → retry → connected
- Cached token flow: vault has token → connect succeeds without auth
- Expired token flow: tool call → 401 → refresh → retry → success

### Manual Testing (against Notion)

- Fresh: no token → boot → pending_auth → authorize via dashboard → connected
- Restart: boot → vault token loaded → connected immediately
- Revoke: API delete → disconnected → re-authorize flow

### Not Tested in CI

The browser PKCE flow itself — already covered by existing tests in
`librefang-extensions/src/oauth.rs`. We test everything around it via the
`McpOAuthProvider` trait with a mock implementation.

---

## Open Questions

1. **rmcp `AuthRequired` type visibility** — Need to verify that
   `AuthRequiredError` is a public type we can pattern-match on. If not, fall
   back to string matching on the error message. Check during implementation.

2. **Callback port for Docker** — The localhost callback binds to a random port.
   In Docker, this port isn't exposed. The dashboard API path (`POST /auth/start`
   → browser on host) handles this, but we should document it clearly.

3. **Multiple concurrent auth flows** — If two MCP servers need auth
   simultaneously at startup, each gets its own localhost callback server on
   different ports. Should work, but worth a test.
