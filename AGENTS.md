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

The workspace contains 31 crates under `crates/` plus an `xtask` crate:

| Crate | Purpose |
|---|---|
| `librefang-types` | Core types, traits, and data models shared across all crates |
| `librefang-http` | Shared HTTP utilities |
| `librefang-wire` | OFP (Open Fang Protocol): agent-to-agent P2P networking |
| `librefang-telemetry` | OpenTelemetry + Prometheus metrics instrumentation |
| `librefang-testing` | Test infrastructure: mock kernel, mock LLM driver, API route test utilities |
| `librefang-migrate` | Migration engine: import from other agent frameworks |
| `librefang-kernel` | Central kernel: agent registry, scheduling, orchestration, event bus, metering |
| `librefang-kernel-handle` | KernelHandle trait — breaks runtime↔kernel circular dependency |
| `librefang-kernel-router` | Kernel model routing layer |
| `librefang-kernel-metering` | Token/cost metering |
| `librefang-runtime` | Agent execution: LLM drivers, tool runner, MCP client, context engine, A2A protocol |
| `librefang-runtime-mcp` | MCP client implementation |
| `librefang-runtime-oauth` | OAuth2 PKCE runtime integration |
| `librefang-runtime-wasm` | WASM sandbox runtime |
| `librefang-llm-driver` | LlmDriver trait + error types (interface only) |
| `librefang-llm-drivers` | Concrete provider impls (anthropic, openai, gemini, uar, …) |
| `librefang-memory` | Memory substrate: SurrealDB backends + SQLite fallback, conversation history, vector search |
| `librefang-memory-wiki` | Durable file-based knowledge vault (markdown pages, backlinks, frontmatter) |
| `librefang-storage` | **BossFang** — SurrealDB storage abstraction layer + 24 SurrealQL migrations |
| `librefang-api` | HTTP/WebSocket API server, route handlers, middleware, dashboard |
| `librefang-cli` | CLI binary (interactive TUI with ratatui) |
| `librefang-desktop` | Native desktop app (Tauri 2.0) |
| `librefang-skills` | Skill system: registry, loader, marketplace, WASM sandbox |
| `librefang-hands` | Hands system: curated autonomous capability packages |
| `librefang-extensions` | Extension system: MCP server setup, credential vault, OAuth2 PKCE |
| `librefang-channels` | Channel bridge layer: 40+ messaging integrations (Discord, Slack, Telegram, WeCom, etc.) |
| `librefang-uar-spec` | **BossFang** — UAR-AGENT-MD spec types and AgentManifest translator |
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

## BossFang Fork Additions — Always Present After Upstream Merge

These crates/features exist in BossFang but NOT in upstream LibreFang. They must
survive every upstream merge intact.

### SurrealDB Storage (`librefang-storage`)

Default storage backend replacing upstream's SQLite-only approach. Contains 24+ SurrealQL
migration files in `crates/librefang-storage/src/migrations/sql/`. Feature: `surreal-backend`
(default). After upstream merge, map any new upstream SQLite schema changes to new `.surql`
migration files and register them in `src/migrations/mod.rs`.

**Version pin**: `surrealdb = "=3.0.5"` in workspace `Cargo.toml`. Do NOT upgrade without
coordinating surreal-memory and UAR git refs — version drift breaks the build.

### surreal-memory Integration (`librefang-memory` surreal backends)

BossFang memory uses `surreal-memory` from `https://github.com/Prometheus-AGS/surreal-memory-server`.
Implementation in `crates/librefang-memory/src/backends/surreal*.rs` (9 backend files).

Never remove; never switch to upstream's SQLite memory backend. The `embedded` feature must
remain active (no external SurrealDB service required).

When upstream changes `librefang-memory`'s storage API (e.g., `Arc<Mutex<Connection>>` →
r2d2 `Pool`), update the surreal backend dual-path code to use the new API for the
SQLite fallback path (keep the SurrealDB path first).

### Universal Agent Runtime (`librefang-uar-spec`, `UarDriver`)

- `librefang-uar-spec` crate: AgentManifest ↔ UAR IR translation
- `librefang-llm-drivers` feature `uar-driver`: wraps UAR's liter-llm for 142+ providers
- When `uar-driver` is enabled, UAR gets `surreal-backend` to share our SurrealDB version

After upstream merge: update `UarDriver` if `LlmDriver` trait signature changes;
update `librefang-uar-spec/src/types.rs` if `AgentManifest` shape changes.

## Important Notes

- **Do not modify `librefang-cli`** without explicit instruction -- it is under active development.
- `PeerRegistry` is `Option<PeerRegistry>` on the kernel but `Option<Arc<PeerRegistry>>` on `AppState`.
- Config fields added to `KernelConfig` MUST also be added to its `Default` impl.
- The `AgentLoopResult` response field is `.response`, not `.response_text`.
- The CLI daemon command is `start` (not `daemon`).
