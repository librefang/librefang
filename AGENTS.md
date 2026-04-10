# AGENTS.md — AI Assistant Context for LibreFang

## Project Overview

LibreFang is an open-source **Agent Operating System** written in Rust.
Manages AI agents (LLM-backed), their tools, memory, messaging channels, and inter-agent networking.

- **Language**: Rust (edition 2021, MSRV 1.75)
- **Async runtime**: tokio
- **Web framework**: axum 0.8 (HTTP + WebSocket)
- **Database**: SQLite via rusqlite (bundled), Supabase for vector store
- **Config**: TOML (`~/.librefang/config.toml`)
- **Default API**: `http://127.0.0.1:4545`

## Workspace Structure

| Crate | Purpose |
|---|---|
| `librefang-types` | Core types, traits, data models shared across all crates |
| `librefang-kernel` | Central kernel: agent registry, scheduling, orchestration, event bus |
| `librefang-runtime` | Agent execution: LLM drivers, tool runner, MCP client, context engine |
| `librefang-api` | HTTP/WebSocket API server, route handlers, middleware, dashboard |
| `librefang-channels` | Channel bridge: 40+ messaging integrations (Discord, Slack, Telegram, etc.) |
| `librefang-memory` | Memory substrate: conversation history, vector search, knowledge storage |
| `librefang-wire` | OFP (Open Fang Protocol): agent-to-agent P2P networking |
| `librefang-skills` | Skill system: registry, loader, marketplace, WASM sandbox |
| `librefang-hands` | Hands system: curated autonomous capability packages |
| `librefang-extensions` | Extension system: MCP server setup, credential vault, OAuth2 |
| `librefang-cli` | CLI binary (interactive TUI with ratatui) |
| `librefang-desktop` | Native desktop app (Tauri 2.0) |
| `librefang-migrate` | Migration engine: import from other frameworks |
| `librefang-telemetry` | OpenTelemetry + Prometheus metrics instrumentation |
| `librefang-testing` | Test infrastructure: mock kernel, mock LLM driver, test utilities |
| `ruvector-*` | PostgreSQL vector extension, solver, attention modules |
| `xtask` | Development task runner |

## Build Commands

```bash
cargo build --workspace              # Full build
cargo build --workspace --lib        # Build libraries only
cargo test --workspace               # Run all tests
cargo clippy --workspace --all-targets -- -D warnings  # Lint
cargo xtask release                  # Release flow
cargo xtask ci                       # Local CI
```

## Key Architecture Patterns

### KernelHandle trait
Defined in `librefang-runtime`, abstracts kernel interface to avoid circular deps.
Kernel implements it; runtime and API consume it.

### AppState bridge
In `librefang-api/src/server.rs`, bridges kernel to API route handlers.
New routes register in router AND implement in `librefang-api/src/routes/`.

### Dashboard
React + TypeScript SPA built with Vite at `crates/librefang-api/dashboard/`.

### Agent manifests
Agent definitions in `agents/` as directories with `agent.toml` files.

### Multi-Tenant
- AccountId middleware extracts from `X-Account-Id`
- Routes check `entry.account_id` against AccountId
- Supabase RLS for vector memory isolation
- Admin endpoints use `require_admin()` helper

## API Route Modules

Routes in `crates/librefang-api/src/routes/`:
`agents`, `budget`, `channels`, `config`, `goals`, `inbox`, `media`,
`memory`, `network`, `plugins`, `prompts`, `providers`, `skills`,
`system`, `workflows`

## Code Conventions

- **Error handling**: `thiserror` for libraries, `anyhow` for applications
- **Serialization**: `serde` with JSON and TOML
- **Naming**: snake_case functions/variables, PascalCase types
- **Async**: `async fn` with tokio; `async-trait` for trait methods
- **Testing**: Tests in `#[cfg(test)]` modules; integration helpers in `librefang-testing`
- **Commits**: Conventional (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `test:`)

## Important Notes

- Do not modify `librefang-cli` without explicit instruction
- `PeerRegistry` is `Option<Arc<PeerRegistry>>` on AppState
- Config fields added to `KernelConfig` MUST add to Default impl
- CLI daemon command is `start` (not `daemon`)
- Dashboard at `http://127.0.0.1:4545` by default

<!-- caliber:managed:pre-commit -->
## Before Committing

**IMPORTANT:** Before every git commit, you MUST ensure Caliber syncs agent configs with the latest code changes.

First, check if the pre-commit hook is already installed:
```bash
grep -q "caliber" .git/hooks/pre-commit 2>/dev/null && echo "hook-active" || echo "no-hook"
```

- If **hook-active**: the hook handles sync automatically — just commit normally. Tell the user: "Caliber will sync your agent configs automatically via the pre-commit hook."
- If **no-hook**: run Caliber manually before committing:
  1. Tell the user: "Caliber: Syncing agent configs with your latest changes..."
  2. Run: `caliber refresh && git add CLAUDE.md .claude/ .cursor/ .cursorrules .github/copilot-instructions.md .github/instructions/ AGENTS.md CALIBER_LEARNINGS.md .agents/ .opencode/ 2>/dev/null`
  3. After it completes, briefly tell the user what Caliber updated. Then proceed with the commit.

If `caliber` is not found, read `.agents/skills/setup-caliber/SKILL.md` and follow its instructions to install Caliber.
<!-- /caliber:managed:pre-commit -->

<!-- caliber:managed:learnings -->
## Session Learnings

Read `CALIBER_LEARNINGS.md` for patterns and anti-patterns learned from previous sessions.
These are auto-extracted from real tool usage — treat them as project-specific rules.
<!-- /caliber:managed:learnings -->
