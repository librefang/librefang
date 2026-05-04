# AGENTS.md — Telegraph Style. Short sentences. One idea per line.

LibreFang is an open-source Agent Operating System in Rust.
It runs LLM-backed agents with tools, memory, channels, and P2P networking.
See `CLAUDE.md` for the full agent contract (worktree rules, hooks, integration testing).

## Stack

- Language: Rust, edition 2021, MSRV 1.94.1
- Async: tokio
- Web: axum 0.8 (HTTP + WebSocket)
- DB: SQLite via bundled rusqlite
- Config: TOML at `~/.librefang/config.toml`
- API: `http://127.0.0.1:4545` (default)

## Layout

15 crates under `crates/` plus `xtask/`.

| Crate | Purpose |
|---|---|
| `librefang-types` | Core types, traits, shared data models |
| `librefang-kernel` | Agent registry, scheduling, orchestration, event bus, metering |
| `librefang-runtime` | Agent loop, LLM drivers, tools, MCP client, context engine, A2A |
| `librefang-api` | HTTP/WebSocket server, routes, middleware, dashboard |
| `librefang-channels` | 40+ messaging bridges (Discord, Slack, Telegram, WeCom, …) |
| `librefang-memory` | History, vector search, knowledge storage |
| `librefang-wire` | OFP — agent-to-agent P2P |
| `librefang-skills` | Skill registry, loader, marketplace, WASM sandbox |
| `librefang-hands` | Curated autonomous capability packages |
| `librefang-extensions` | MCP server setup, credential vault, OAuth2 PKCE |
| `librefang-cli` | CLI binary (ratatui TUI) |
| `librefang-desktop` | Native desktop app (Tauri 2.0) |
| `librefang-migrate` | Import from other agent frameworks |
| `librefang-telemetry` | OpenTelemetry + Prometheus |
| `librefang-testing` | Mock kernel, mock LLM, route test utilities |
| `xtask` | Dev task runner |

## Build

```bash
cargo build --workspace                                # full build
cargo build --workspace --lib                          # libs only (CLI binary may be locked)
cargo test --workspace                                 # all tests
cargo clippy --workspace --all-targets -- -D warnings  # zero warnings
```

## Architecture

### `KernelHandle` trait
- Defined in `librefang-runtime`.
- Breaks the circular dep between runtime and kernel.
- Kernel implements it. Runtime and API consume it.

### `AppState` bridge
- Lives in `librefang-api/src/server.rs`.
- Wires the kernel into route handlers.
- New route = register in `server.rs` router AND implement under `librefang-api/src/routes/`.

### Dashboard
- React + TypeScript SPA, built with Vite.
- Path: `crates/librefang-api/dashboard/`.
- Pages: `dashboard/src/pages/`. Components: `dashboard/src/components/`.

### Agent manifests
- `agents/<name>/agent.toml`.

### `session_mode`
- `"persistent"` (default) reuses the agent's session.
- `"new"` starts fresh on every automated invocation (cron, triggers, `agent_send`).
- Per-trigger override via the trigger registration API.
- Hands honor `session_mode` — they share `AgentManifest` and the execution pipeline.

### Config pattern
Adding a `KernelConfig` field requires all four:
- struct field
- `#[serde(default)]`
- entry in `Default` impl
- `Serialize` / `Deserialize` derives

## API routes

Domain modules under `crates/librefang-api/src/routes/`:

`agents`, `budget`, `channels`, `config`, `goals`, `inbox`, `media`, `memory`, `network`, `plugins`, `prompts`, `providers`, `skills`, `system`, `workflows`.

## Conventions

- Errors: `thiserror` for libraries. `anyhow` for application code.
- Serialization: `serde` + `serde_json` + `toml`.
- Naming: `snake_case` functions/variables, `PascalCase` types.
- Async: `async fn` on tokio. `async-trait` only when a trait method must be async.
- Tests: `#[cfg(test)]` next to source. Integration helpers in `librefang-testing`.
- Commits: conventional — `feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `ci:`, `perf:`, `test:`.

## Gotchas

- Do not modify `librefang-cli` without explicit instruction. It is under active development.
- `PeerRegistry`: `Option<PeerRegistry>` on the kernel, `Option<Arc<PeerRegistry>>` on `AppState`.
- New `KernelConfig` field MUST appear in its `Default` impl. Build fails otherwise.
- `AgentLoopResult` field is `.response`. Not `.response_text`.
- CLI daemon command is `start`. Not `daemon`.
