# librefang-runtime — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for cross-cutting rules.

## Purpose

Agent execution. Tool dispatch. Context management. Audit. A2A peer protocol. Channel registry. Sandboxes (browser, docker, process).
Re-exports OAuth subsystems from `librefang-runtime-oauth`.

## Boundary

- Owns: `agent_loop`, `tool_runner`, `compactor`, `context_budget`, `context_compressor`, `context_overflow`, `audit`, `auth_cooldown`, `aux_client`, `browser`, `catalog_sync`, `channel_registry`, `checkpoint_manager`, `dangerous_command`, `docker_sandbox`, `media`, `model_catalog` (the type), `mcp` (client), `prompt_builder`.
- Does NOT own: agent registry / scheduler / cron / orchestration → `librefang-kernel`. HTTP routing → `librefang-api`. Channel transport adapters → `librefang-channels`. Skill loader → `librefang-skills`.
- Depends on: `librefang-types`, `librefang-http`, `librefang-kernel-handle` (NOT `librefang-kernel` directly — that would be circular).

## Module map

- `agent_loop` — turn-by-turn execution. ~10k LOC; a god module slated for extraction (#3710). Don't grow it without coordination.
- `tool_runner` — tool execution path. ~9.7k LOC, also targeted by #3710.
- `model_catalog::ModelCatalog` — registry of 130+ models / 28 providers. Kernel wraps it in `arc_swap::ArcSwap` (#3384). Changes go through kernel's `model_catalog_update(|cat| ...)`.
- `mcp` — MCP client. OAuth state lives in `mcp_auth_states`; the OAuth provider trait is `McpOAuthProvider` (kernel side implements it).
- `a2a` — Agent-to-Agent peer protocol.
- `apply_patch` — tool-level patch application.

## KernelHandle trait

Lives in the sibling `librefang-kernel-handle` crate (NOT here). Kernel implements; runtime + API consume. Use `KernelHandle` whenever you need a kernel callback from runtime. Never depend on `librefang-kernel` directly.

## Cross-cutting invariants

- **Deterministic prompt ordering (#3298)**: tool definitions, MCP server summaries, capability lists must be sorted before stringifying. `BTreeMap` / `BTreeSet`, not `HashMap`.
- **Identity files** live at `{workspace}/.identity/`, NOT workspace root. `read_identity_file()` falls back to root for pre-migration workspaces; `migrate_identity_files()` runs on every spawn.
- **`USER_AGENT` constant** is mandatory on every outbound HTTP call (`req.header("User-Agent", librefang_runtime::USER_AGENT)`). Audit hook flags missing UAs.

## Async boundaries

- `ErrorTranslator` (from `RequestLanguage`) is `!Send`. Any `.await` must happen AFTER `drop(t)` or you get a cryptic axum `Handler<_, _>` trait-bound error.
- No synchronous `std::fs` / `std::sync::RwLock` inside async handlers. Use `tokio::fs` / `arc_swap` / `parking_lot` (refs #3579).
- No tokio `block_on` here either.

## Testing

- This crate has historically had ZERO integration tests (#3696). New runtime work SHOULD include at least one `#[tokio::test]` exercising the new path.
- Scoped: `cargo test -p librefang-runtime`.

## Taboos

- No `librefang-kernel` import. Use `KernelHandle`.
- No `librefang-api` import. API consumes runtime, not the other way.
- No new `agent_loop.rs` or `tool_runner.rs` file additions; both files are slated to shrink, not grow (#3710).
- No `unwrap()` / `panic!()` on values that come off the wire.
- No mocking the kernel by faking `KernelHandle` inline — use `librefang-testing::MockKernelBuilder`.
- No raw `cargo build`; use `cargo check --workspace --lib`. Real builds run in CI.
