# AGENTS.md — AI Assistant Context for LibreFang

## Project Overview

LibreFang is an open-source **Agent Operating System** written in Rust.
It manages AI agents (LLM-backed), their tools, memory, messaging channels, and inter-agent networking.

- **Language**: Rust (edition 2021, MSRV 1.94.1)
- **Async runtime**: tokio
- **Web framework**: axum 0.8 (HTTP + WebSocket)
- **Database**: SQLite via rusqlite (bundled)
- **Config**: TOML (`~/.librefang/config.toml`)
- **Default API address**: `http://127.0.0.1:4545`

## Workspace Structure

The workspace contains 15 crates under `crates/` plus an `xtask` crate:

| Crate | Purpose |
|---|---|
| `librefang-types` | Core types, traits, and data models shared across all crates |
| `librefang-kernel` | Central kernel: agent registry, scheduling, orchestration, event bus, metering |
| `librefang-runtime` | Agent execution: LLM drivers, tool runner, MCP client, context engine, A2A protocol |
| `librefang-api` | HTTP/WebSocket API server, route handlers, middleware, dashboard |
| `librefang-channels` | Channel bridge layer: 40+ messaging integrations (Discord, Slack, Telegram, WeCom, etc.) |
| `librefang-memory` | Memory substrate: conversation history, vector search, knowledge storage |
| `librefang-wire` | OFP (Open Fang Protocol): agent-to-agent P2P networking |
| `librefang-skills` | Skill system: registry, loader, marketplace, WASM sandbox |
| `librefang-hands` | Hands system: curated autonomous capability packages |
| `librefang-extensions` | Extension system: MCP server setup, credential vault, OAuth2 PKCE |
| `librefang-cli` | CLI binary (interactive TUI with ratatui) |
| `librefang-desktop` | Native desktop app (Tauri 2.0) |
| `librefang-migrate` | Migration engine: import from other agent frameworks |
| `librefang-telemetry` | OpenTelemetry + Prometheus metrics instrumentation |
| `librefang-testing` | Test infrastructure: mock kernel, mock LLM driver, API route test utilities |
| `xtask` | Development task runner |

## Build Commands

```bash
cargo build --workspace              # Full build
cargo build --workspace --lib        # Build libraries only (use when CLI binary is locked)
cargo test --workspace               # Run all tests
cargo clippy --workspace --all-targets -- -D warnings  # Lint (zero warnings policy)
```

## Key Architecture Patterns

### KernelHandle trait
Defined in `librefang-runtime`, this trait abstracts the kernel interface to avoid circular
dependencies between `librefang-runtime` and `librefang-kernel`. The kernel implements it;
the runtime and API consume it.

### AppState bridge
In `librefang-api/src/server.rs`, `AppState` bridges the kernel to API route handlers.
New routes must be registered in the `server.rs` router AND implemented in the corresponding
file under `librefang-api/src/routes/`.

### Dashboard
The web dashboard is a React + TypeScript SPA built with Vite, located at
`crates/librefang-api/dashboard/`. Source files are in `dashboard/src/` with pages under
`dashboard/src/pages/` and shared components under `dashboard/src/components/`.

### Agent manifests
Agent definitions live in `agents/` as directories containing `agent.toml` files.

### Session mode
Agents can control whether automated invocations (cron ticks, triggers, `agent_send`)
reuse the persistent session or start fresh. Set `session_mode = "new"` in `agent.toml`
for a fresh session per invocation, or `"persistent"` (default) to reuse the existing session.
Per-trigger overrides are supported via the trigger registration API. Hands also support
`session_mode` since they share the same `AgentManifest` and execution pipeline.

### Config pattern
Adding a config field requires: struct field with `#[serde(default)]`, a `Default` impl
entry, and `Serialize`/`Deserialize` derives. Fields go in `KernelConfig` in `librefang-kernel`.

## API Route Modules

Routes are organized by domain in `crates/librefang-api/src/routes/`:

`agents`, `budget`, `channels`, `config`, `goals`, `inbox`, `media`, `memory`,
`network`, `plugins`, `prompts`, `providers`, `skills`, `system`, `workflows`

## Code Conventions

- **Error handling**: `thiserror` for library errors, `anyhow` for application-level errors
- **Serialization**: `serde` with JSON (`serde_json`) and TOML (`toml`)
- **Naming**: Follow Rust standard conventions (snake_case for functions/variables, PascalCase for types)
- **Async**: Use `async fn` with tokio; `async-trait` where trait methods need to be async
- **Testing**: Tests live alongside source code in `#[cfg(test)]` modules; integration test helpers in `librefang-testing`
- **Commits**: Conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `ci:`, `perf:`, `test:`)

## BossFang Product Identity

This repo is the **BossFang** fork of LibreFang. Upstream branding (LibreFang name,
sky-blue `#0284c7`/`#38bdf8` palette, SVG fang glyph) must never appear in any merged
or committed state.

| Element | BossFang value |
|---|---|
| Product name | **BossFang** |
| Logo asset | `boss-libre.png` (dashboard public, desktop frontend, static) |
| Light primary | `#E04E28` (Muted Ember) |
| Dark primary | `#FF6A3D` (Bright Ember) |
| Dark background | `#0B0F14` (Deep Charcoal) |
| Full spec | `docs/branding/branding-guide.html` |

**After every upstream merge**, run before committing:
```bash
python3 scripts/enforce-branding.py
```
For detailed conflict-resolution rules see `CLAUDE.md § BossFang Branding`.

## Important Notes

- **Do not modify `librefang-cli`** without explicit instruction -- it is under active development.
- `PeerRegistry` is `Option<PeerRegistry>` on the kernel but `Option<Arc<PeerRegistry>>` on `AppState`.
- Config fields added to `KernelConfig` MUST also be added to its `Default` impl.
- The `AgentLoopResult` response field is `.response`, not `.response_text`.
- The CLI daemon command is `start` (not `daemon`).
