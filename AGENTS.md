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
cargo check --workspace --lib                          # compile-check only — full build runs in CI
cargo test -p <crate>                                  # scoped tests; workspace-wide form is forbidden (target/ contention)
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

## Gotchas

- Do not modify `librefang-cli` without explicit instruction. It is under active development.
- `PeerRegistry`: `Option<PeerRegistry>` on the kernel, `Option<Arc<PeerRegistry>>` on `AppState`.
- New `KernelConfig` field MUST appear in its `Default` impl. Build fails otherwise.
- `AgentLoopResult` field is `.response`. Not `.response_text`.
- CLI daemon command is `start`. Not `daemon`.
