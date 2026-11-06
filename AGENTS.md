# AGENTS.md — AI Assistant Context for LibreFang

## Project Overview

LibreFang is an open-source **Agent Operating System** written in Rust.
It manages AI agents (LLM-backed), their tools, memory, messaging channels, and inter-agent networking.

- Language: Rust, edition 2021, MSRV 1.95.0
- Async: tokio
- Web: axum 0.8 (HTTP + WebSocket)
- DB: SurrealDB via `librefang-storage` (embedded RocksDB or remote ws/http); SQLite via bundled rusqlite remains the upstream-compatible fallback
- Config: TOML at `~/.librefang/config.toml`
- API: `http://127.0.0.1:4545` (default)

## Default operating mode: delegate to the agent team

Every non-trivial task in this repo runs through the **`bossfang-stewards`** agent team (`.agent-team/project-routing.json` → `.agent-team/bossfang-stewards/team.json`).
The main session is the orchestrator: it plans, dispatches roles, combines their results, and reports.
It does not do role work itself when a role owns that work.
Only trivial single-file changes (a typo, a one-line obvious fix) skip delegation.

| Work | Role |
|---|---|
| Roadmap, feature ledger, acceptance criteria, KBD phases and OpenSpec changes (`docs/bossfang/`, `.kbd-orchestrator/`, `openspec/changes/`) | `product-manager` |
| Any upstream `librefang/librefang` sync (`docs/upstream-merges/`, the upstream-merge skill, `scripts/enforce-branding.py`) | `upstream-merge-manager` |
| SurrealDB schema and migrations, `librefang-storage`, the Surreal backends in `librefang-memory` (the surreal-memory substrate), SurrealDB / surreal-memory version pins | `surrealdb-schema-engineer` |
| UAR (`librefang-uar-spec`, the `UarDriver`, the runtime pin), the config store overlay, desktop identity, branding assets | `bossfang-feature-steward` |
| Independent review of every upstream sync before it merges to `main` | `merge-reviewer` |

How to use the team:

- **Use it to the maximum.**
  Split work so that every role with an independent piece runs, and run those roles **in parallel**.
- **Isolate parallel roles.**
  Each role gets its own linked worktree (branched from the shared base) and its own `CARGO_TARGET_DIR`, so their commits and builds never collide.
  The orchestrator combines the commits afterwards and re-verifies the combined tree.
- **Invoke by role.**
  Use the harness's native definition for the role (`.claude/agents/`, `.codex/agents/`, `.kimi-code/agents/`, `.opencode/agents/`, `.minimax/agents/`).
  Where the harness has no native delegation, follow the role instructions sequentially and say so.
- **Work no role owns** (general kernel, API, dashboard, CLI or hook code) still goes to a delegated agent in its own worktree, which follows this file.
- **Review is independent.**
  Nothing merges to `main` without a review in a fresh context, never the builder's.
  `merge-reviewer` reviews upstream syncs; every other change gets a fresh-context reviewer agent (for example `code-reviewer`, `rust-reviewer`, or `security-reviewer` for hooks and auth).
- **Hand off explicitly.**
  A role that finds work owned by another role reports it as a handoff.
  The orchestrator dispatches it rather than letting the finder absorb it.

## Workspace Structure

There are 31 crate directories under `crates/` plus an `xtask` crate.
29 are `[workspace] members`; `librefang-storage` and `librefang-uar-spec` are BossFang path dependencies, not members, so `--workspace` does not select them — use `-p` explicitly.

| Crate | Purpose |
|---|---|
| `librefang-types` | Core types, traits, shared data models |
| `librefang-http` | Shared HTTP client plumbing |
| `librefang-kernel` | Agent registry, scheduling, orchestration, event bus |
| `librefang-kernel-handle` | `KernelHandle` trait — lets runtime call kernel without a circular dep |
| `librefang-kernel-router` | Model / provider routing |
| `librefang-kernel-metering` | Token accounting and budget metering |
| `librefang-runtime` | Agent loop, tools, plugins, OAuth, WASM sandbox, context engine, A2A |
| `librefang-runtime-mcp` | MCP client |
| `librefang-runtime-audit` | Runtime audit trail |
| `librefang-runtime-media` | Media handling |
| `librefang-runtime-sandbox-docker` | Docker-backed tool sandbox |
| `librefang-llm-driver` | LLM driver trait + error types (interface only) |
| `librefang-llm-drivers` | Concrete provider impls: anthropic, openai, gemini, uar, … |
| `librefang-api` | HTTP/WebSocket server, routes, middleware, dashboard |
| `librefang-channels` | Channel-bridge infra: sidecar trampoline + shared bridge types (per-channel adapters live as Python sidecars under `sdk/python/librefang/sidecar/adapters/`) |
| `librefang-subprocess` | Persistent JSON-over-stdio transport shared by the sidecar bridges |
| `librefang-memory` | Memory substrate: SurrealDB backends + SQLite fallback, conversation history, vector search |
| `librefang-memory-wiki` | Durable markdown knowledge vault (provenance frontmatter, Obsidian export) |
| `librefang-storage` | **BossFang** — SurrealDB storage abstraction layer + 24 SurrealQL migrations |
| `librefang-wire` | OFP — agent-to-agent P2P |
| `librefang-skills` | Skill registry, loader, marketplace, WASM sandbox |
| `librefang-hands` | Curated autonomous capability packages |
| `librefang-extensions` | MCP server setup, credential vault, OAuth2 PKCE |
| `librefang-cli` | CLI binary (ratatui TUI) |
| `librefang-desktop` | Native desktop app (Tauri 2.0) |
| `librefang-acp` | Agent Client Protocol adapter — embeds agents in Zed / VSCode / JetBrains over stdio JSON-RPC |
| `librefang-uar-spec` | **BossFang** — UAR-AGENT-MD spec types and AgentManifest translator |
| `librefang-import` | Import from other agent frameworks |
| `librefang-rl-export` | Long-horizon RL rollout trajectory exporter |
| `librefang-telemetry` | OpenTelemetry + Prometheus |
| `librefang-testing` | Mock kernel, mock LLM, route test utilities |
| `xtask` | Dev task runner |

## Build Commands

```bash
cargo build --workspace              # Full build
cargo build --workspace --lib        # Build libraries only (use when CLI binary is locked)
cargo test --workspace               # Run all tests
cargo clippy --workspace --all-targets -- -D warnings  # Lint (zero warnings policy)
```

### Verify locally. Never wait on CI.

**CI is not a verification step in this repo.** It is slow, and waiting on it stalls work a local `cargo check` answers in a minute or two.

- **Never** push a branch and then poll GitHub Actions for the result.
- **Never** describe a change as "verified by CI", and never defer a failing or skipped check to CI.
- **Never** treat a red or skipped CI lane as a reason to pause — verify the same property locally instead.
- **Never** build `Dockerfile.rust-dev` to run a check. That image is for hosts with no native toolchain; this host has one, and building it costs ~20 minutes.

There is a native `cargo` toolchain on the development host. Use it:

```bash
export CARGO_TARGET_DIR=/tmp/librefang-target-<worktree>   # keep off the shared target/
export SKIP_DASHBOARD_BUILD=1                              # build.rs soft-skips without pnpm

cargo check --workspace --lib
cargo test -p <crate>
```

`SKIP_DASHBOARD_BUILD=1` stops `librefang-api/build.rs` shelling out to `pnpm`, which dominates a cold check and is irrelevant to Rust correctness.

Read cargo's **own** exit status, not a wrapper's: `VAR=$?` followed by a pipe makes the shell report the pipe's status, so a failed build reads as success. Write it as the final action (`cmd > log 2>&1; echo $? > log.status`) and read that file.

## OpenSpec

The repo is initialized for [OpenSpec](https://github.com/Fission-AI/OpenSpec) spec-driven development.
`openspec/` (config + specs + changes) is tracked and is the shared artefact.

The per-tool instruction shims — the `/opsx:*` slash commands and skills that OpenSpec writes into `.claude/`, `.codex/`, `.opencode/`, `.kimi/` — are **not** tracked.
They are generated files, and this repo keeps every agent-tool config local (see the `.claude/*` allowlist and `.codex` entry in `.gitignore`).

Generate them for your own tool once per clone:

```bash
openspec init --tools claude        # or: codex, opencode, kimi, cursor, windsurf, cline, …
openspec update                     # refresh the shims after an OpenSpec upgrade
```

`openspec init --tools` accepts `all`, `none`, or a comma-separated list.
Zed is not among the supported targets.

Note on the KBD lifecycle: `.kbd-orchestrator/project.json` pins `specBackend: native-kbd` while phase-10 is in flight, because that phase's change specs were written as native KBD change files.
New phases should adopt the OpenSpec backend.

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

## Process Discipline

How you change code in this repo. Six principles synthesised from Andrej
Karpathy's behavioural rules and Boris Cherny's principles + self-improvement
loop. Apply on top of the worktree-isolation rule and BossFang preservation
rules below. (Claude Code-specific tool names appear in the canonical version
in `CLAUDE.md § Process Discipline` — for other AI tools, use the equivalent
plan-mode / task-tracker / persistent-memory affordances of your runtime.)

1. **Think Before Coding** — State assumptions explicitly. Ask if unclear.
   If multiple approaches exist, present them. If a simpler approach exists,
   push back. For non-trivial work, plan first (with plan mode or a brief
   numbered plan with verify-steps).
2. **Simplicity First** — Minimum code that solves the problem. No features
   beyond what was asked. No speculative abstractions or "configurability".
   No error handling for impossible scenarios. If you can delete lines
   instead of adding them, do that.
3. **Surgical Changes** — Touch only what you must. Don't "improve" adjacent
   code or refactor what isn't broken. Match existing style. Every changed
   line traces directly to the user's request. Mention unrelated dead code
   you spot — don't delete it inline.
4. **Root-Cause Discipline** — Find the underlying issue; no band-aids,
   `#[ignore]`'d tests, suppressed lints, or `--no-verify` flags. A failing
   test is information.
5. **Goal-Driven Verification** — Transform vague asks into verifiable
   goals ("fix the bug" → "add a test that reproduces it, then make it
   pass"). Verify before claiming done: `cargo check --workspace --lib`
   AND scoped `cargo test -p <crate>` AND
   `python3 scripts/enforce-branding.py --check` AND any audit script the
   change implicates.
6. **Capture Corrections** — When the user corrects you, or confirms a
   non-obvious approach worked, persist the rule somewhere your tool will
   recall next session (Claude Code: write a `feedback` memory; other
   tools: use your equivalent persistent-instructions affordance). Don't
   make the user repeat themselves.

Trivial tasks (typo fix, single-line obvious change) can skip the heavier
parts of this discipline — use judgement. The bias toward caution is
correct for any change that touches more than one file, modifies
behaviour, or interacts with the BossFang preservation surface.

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

### Boundaries
- **Don't modify a PR a human maintainer has already reviewed or approved**
  unless the maintainer asks for the edit. Open a follow-up PR instead.
- **Don't close a PR or issue you did not open** unless the maintainer
  directly instructs you to. By default, recommend closure in a
  comment and let the maintainer act. When directed to close, the close
  comment must state the substantive reason (review bugs, superseded
  by, scope mismatch) — see `CLAUDE.md` for the full close-comment
  contract.
- **Don't force-push to someone else's branch.** Force-push to your own
  branch is acceptable only while the PR is still un-reviewed.
- **Don't bypass git verification flags.** No `--no-verify`, no
  `--no-gpg-sign`, no skipping `commit-msg` / `pre-push` hooks.
- **Don't add Claude / AI attribution** to commit messages or PR bodies
  (`Co-Authored-By: Claude`, `🤖 Generated with …`, etc.). The `commit-msg`
  hook rejects these.
- **Don't edit files in the main worktree.** Always work from a linked
  worktree (`git worktree add`).


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

**Version pin**: `surrealdb`, `surrealdb-core` **and** `surrealdb-types`, all `=4.0.0`, in workspace `Cargo.toml` — the three move together, since `=` on the client does not constrain the other two.
Do NOT upgrade without moving the `surreal-memory` rev in step (currently `c220263`, which pins `=4.0.0` itself) — version drift breaks the build.
UAR is an independently owned remote service; its storage dependencies are not linked into BossFang.

### surreal-memory Integration (`librefang-memory` surreal backends)

BossFang memory uses `surreal-memory` from `https://github.com/Prometheus-AGS/surreal-memory-server`.
Implementation in `crates/librefang-memory/src/backends/surreal*.rs` (9 backend files).
The dependency is pinned by `rev` (currently `c220263`), not a branch, so a bump is a deliberate edit of the workspace `Cargo.toml` followed by `cargo update -p surreal-memory`.
Its `surrealdb = "=4.0.0"` pin is exact, so the rev and our SurrealDB pins move together.

Never remove; never switch to upstream's SQLite memory backend. The `embedded` feature must
remain active (no external SurrealDB service required).

When upstream changes `librefang-memory`'s storage API (e.g., `Arc<Mutex<Connection>>` →
r2d2 `Pool`), update the surreal backend dual-path code to use the new API for the
SQLite fallback path (keep the SurrealDB path first).

### Universal Agent Runtime (`librefang-uar-spec`, `UarDriver`)

BossFang never bundles, installs, spawns, stops or supervises UAR.
The Boss owns its managed UAR lifecycle; BossFang borrows an authenticated configured instance through selected connection APIs.
New admissions use the selected connection; admitted runs retain their original instance, workspace, endpoints and runtime epoch without migration or replay.
Preserve this ownership boundary after every upstream merge.

- `librefang-uar-spec` crate: AgentManifest ↔ UAR IR translation
- `UarDriver` uses the selected UAR's remote model/runtime transport.
- Preserve the existing `uar-driver` feature chain for compatibility; its terminal `librefang-llm-drivers` feature is empty, with no `universal-agent-runtime` dependency or UAR storage-feature forwarding.

After upstream merge: update `UarDriver` if `LlmDriver` trait signature changes;
update `librefang-uar-spec/src/types.rs` if `AgentManifest` shape changes.

## Important Notes

- **Do not modify `librefang-cli`** without explicit instruction -- it is under active development.
- `PeerRegistry` is `Option<PeerRegistry>` on the kernel but `Option<Arc<PeerRegistry>>` on `AppState`.
- Config fields added to `KernelConfig` MUST also be added to its `Default` impl.
- The `AgentLoopResult` response field is `.response`, not `.response_text`.
- The CLI daemon command is `start` (not `daemon`).

### Agent Fabric Convergence phase gate

For `agent-fabric-convergence` and its `release-cadence-optimization` child, follow the approved [delivery contract](docs/plans/agent-fabric-convergence/.kbd-orchestrator/phases/agent-fabric-convergence/children/release-cadence-optimization/delivery-contract.md): finish the complete production increment, then run one real integration gate and the required local package build. Test-first, per-edit, per-route and scoped cargo-check/test examples elsewhere in project or role instructions are deferred to that completed-phase boundary for this initiative. The single shared heavy build writer and KBD lead coordinate the boundary; observer roles cannot authorize it. This scoped precedence leaves other projects and phases unchanged.

<!-- prometheus-team-routing:start v1 -->
For every code task, read `.agent-team/project-routing.json`, then its active team manifest and the relevant role instructions. Default to that team, selecting only roles whose responsibilities and ownership match the work. Preserve native permissions, models, concurrency limits and existing project instructions.
For UI work, load the role-bound `prometheus-ui-ux` or `prometheus-ui-review` skill. Prefer `.agents/UI_UX_PROTOCOL.md` when present; otherwise use the installed `prometheus-ui-ux/references/UI_UX_PROTOCOL.md`. Backend work must not load UI guidance.
Use native delegation when available. If unavailable, follow the selected role instructions sequentially and report that limitation. Review in the builder context is not independent review. Keep reviewers dormant until the complete implementation phase; allow one batched correction/confirmation cycle. Respect user-only skill invocation restrictions. Zed external ACP agents use their own native configuration; parallel UI threads are not an automatic delegation API.
<!-- prometheus-team-routing:end -->

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
