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

## AI Agent Collaboration

LibreFang is an open-source project with heavy AI-assistant participation.
To keep human reviewers in control and avoid noisy / destructive behaviour,
AI agents working on this repo MUST observe the following boundaries.
Detailed enforcement (hooks, wait policy, conflict resolution) lives in
[`CLAUDE.md`](./CLAUDE.md#github-collaboration--wait-policy); this list is
the single-page summary.

### Boundaries
- **Never modify a PR a human maintainer has already reviewed or approved**
  unless the maintainer explicitly asks. Open a follow-up PR instead.
- **Never close a PR or issue you did not open.** Recommend closure in a
  comment and let a maintainer act.
- **Don't force-push to someone else's branch.** Force-push to your own
  branch is acceptable only while the PR is still un-reviewed.
- **Don't bypass git verification flags.** No `--no-verify`, no
  `--no-gpg-sign`, no skipping `commit-msg` / `pre-push` hooks.
- **Don't add Claude / AI attribution** to commit messages or PR bodies
  (`Co-Authored-By: Claude`, `🤖 Generated with …`, etc.). The `commit-msg`
  hook rejects these.
- **Don't edit files in the main worktree.** Always work from a linked
  worktree (`git worktree add`).

### Issue / PR interaction
- **One PR ↔ one issue** (or one tightly-related cluster). Don't bundle
  unrelated cleanups; open a separate PR.
- **At most 2 follow-up comments** on the same issue / PR thread without
  human input — then stop and wait. Repeated pings are noise.
- **PR body must list:** substantive changes, how they were verified
  (integration test names, scoped `cargo` invocations), and any
  out-of-scope follow-ups left for a future PR.

### CI wait policy
- **Don't poll status checks for more than ~5 minutes.** CI is slow;
  busy-waiting wastes turns. Push, report the run URL, and stop.
- **Don't pre-emptively retry a check before it has failed.**
- When you are blocked and cannot make progress without investigation,
  **stop and report** — don't auto-open a follow-up issue, don't silently
  switch plan.
- While waiting for review, **don't add reviewers, don't flip
  `ready-for-review`, don't re-request review** unless a maintainer
  has set up an explicit convention asking for it.

### Conflict resolution
- A human maintainer's most recent intent **always wins** over an earlier
  AI-authored change. When rebasing or resolving merge conflicts, preserve
  both sides' intent — never silently drop a maintainer's edit because it
  made the diff smaller.

## Important Notes

- **Do not modify `librefang-cli`** without explicit instruction -- it is under active development.
- `PeerRegistry` is `Option<PeerRegistry>` on the kernel but `Option<Arc<PeerRegistry>>` on `AppState`.
- Config fields added to `KernelConfig` MUST also be added to its `Default` impl.
- The `AgentLoopResult` response field is `.response`, not `.response_text`.
- The CLI daemon command is `start` (not `daemon`).
