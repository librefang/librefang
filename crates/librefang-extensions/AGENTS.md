# librefang-extensions — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for cross-cutting rules.

## Purpose

MCP server catalog. Credential vault. OAuth2 PKCE flows. Provider health probes. Plugin installer. Shared HTTP client.
This crate is the "everything-side-of-an-agent" toolkit that doesn't fit in `runtime` or `kernel`.

## Module map

- `catalog` — MCP catalog (`~/.librefang/mcp/catalog/`). Templates, available servers.
- `credentials` — auth-source unification (env var, vault, CLI login, …).
- `dotenv` — `.env` parsing for agent workspaces.
- `health` — provider liveness probes (now backed by `provider_health` in runtime).
- `http_client` — shared `reqwest::Client` builder. Use this everywhere; do NOT spin up bespoke clients.
- `installer` — MCP server install / update / uninstall flows.
- `oauth` — OAuth2 PKCE client; PKCE + Dynamic Client Registration (RFC 7591) for MCP.
- `vault` — AES-256-GCM credential vault. Master key in OS keyring (Linux/Windows) or file fallback (macOS, see #2766 lineage).

## Boundary

- Owns: vault, MCP catalog, OAuth client, shared HTTP client, dotenv, installer.
- Does NOT own: kernel callback wiring (`McpOAuthProvider` trait lives in runtime; the *implementation* lives in api). HTTP routing. Channel adapters.
- Depends on: `librefang-types`, `librefang-runtime`. **Not** depended on by kernel — extensions sit *above* kernel.

## Vault invariants

- Master key (32 bytes) loaded from `LIBREFANG_VAULT_KEY` (base64, MUST decode to 32 bytes — `openssl rand -base64 32` gives 44 chars), OS keyring, or file fallback.
- macOS skips the Keychain by default (#2766). Override with `[vault] use_os_keyring = true`. Migration path: one final read from Keychain on first boot, mirror to file, never touch Keychain again.
- File fallback path: `~/Library/Application Support/librefang/.keyring` (macOS), mode 0600.
- All vault operations through the `Vault` API. Don't read `.keyring` directly.
- Per-agent vault cache lives behind a `RwLock<HashMap<AgentId, Arc<Vault>>>`; invalidate on credential change.

## Shared HTTP client

`http_client::shared_client()` returns a configured `reqwest::Client` with:
- `User-Agent: librefang/<version>` (matches `librefang_runtime::USER_AGENT`).
- Sane timeout / redirect / TLS defaults.
- Connection pooling.

Bespoke clients in callers WILL be flagged in review. Use the shared one.

## Docker callback URLs

Don't bind ephemeral localhost ports for OAuth callbacks in daemon code — the port is unreachable from outside Docker. Route callbacks through the API server's existing port (api crate handles this).

## OAuth (MCP) flow

- Daemon detects 401 → sets `NeedsAuth` state on the connection.
- API layer (`routes/mcp_auth.rs`) drives the flow: PKCE generation, callback handling, token exchange, refresh.
- Dynamic Client Registration (RFC 7591) used when server has `registration_endpoint` but no `client_id`.
- This crate exposes the building blocks; the API crate owns the user-facing flow.

## Taboos

- No bespoke `reqwest::Client::new()`. Use `http_client::shared_client()`.
- No raw `tokio::process` for plugin installs. Go through `installer`.
- No `librefang-api` / `librefang-cli` / `librefang-desktop` imports. Extensions sit below those layers.
- No reading the vault `.keyring` file directly.
- No new credential providers without thinking through the unified `credentials::resolve()` precedence (env > vault > CLI login > file).
