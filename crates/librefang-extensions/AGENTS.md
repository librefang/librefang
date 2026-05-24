# librefang-extensions — agent notes

> **This file is a stub.** The previous content drifted out of sync with
> the live source (it described a `shared_client()` helper that does not
> exist, a `Vault` struct that is actually `CredentialVault`, and other
> renamed APIs). See the top-level `CLAUDE.md` "Crate map" section for
> the authoritative one-line description of this crate's role; treat the
> source files in `src/` as the canonical reference for the surface API.

## Purpose

This crate is the "everything-side-of-an-agent" toolkit that does not fit
in `librefang-runtime` or `librefang-kernel`: MCP server catalog,
credential vault, OAuth2 PKCE / Dynamic Client Registration, provider
health probes, plugin installer, shared HTTP client builder, `.env`
parsing.

## Module map (verify against `src/` before relying on this)

- `catalog` — MCP catalog metadata (`~/.librefang/mcp/catalog/`).
- `credentials` — auth-source unification (`resolve` / `resolve_all`).
- `dotenv` — `.env` parsing for agent workspaces.
- `health` — provider liveness probes.
- `http_client` — shared `reqwest` client builder
  (`client_builder()` / `new_client()` — there is no `shared_client()`;
  use whichever fits the call site).
- `installer` — MCP server install / update / uninstall flows.
- `oauth` — OAuth2 PKCE client; PKCE + Dynamic Client Registration
  (RFC 7591) for MCP.
- `vault` — AES-256-GCM credential vault (`CredentialVault`). Master
  key in OS keyring (Linux / Windows) or file fallback (macOS, see the
  `use_os_keyring` plumbing).

## Cross-cutting rules

The cross-cutting rules (Docker callback URLs, OAuth flow ownership
between daemon and API, vault key handling, `LIBREFANG_VAULT_KEY`
constraints, the auth middleware allowlist, etc.) live in the top-level
`CLAUDE.md`. Read that before adding code that crosses the crate
boundary; do not duplicate the rules here, where they tend to rot.

## Boundaries

- Owns: vault, MCP catalog, OAuth client, shared HTTP client builder,
  `.env` parsing, plugin installer.
- Does NOT own: kernel callback wiring (the `McpOAuthProvider` trait
  lives in `librefang-runtime`; the implementation lives in
  `librefang-api`). HTTP routing. Channel adapters.
- Sits above `librefang-kernel`; nothing in this crate may depend on
  the API / CLI / desktop layers.
