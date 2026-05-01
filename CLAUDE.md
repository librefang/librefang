# LibreFang — Agent Instructions

## ⚠️ Before any work: verify you are in a worktree, not the main tree

The very first action in any task that will edit files **must** be:
```bash
pwd && git rev-parse --git-dir
```
If `pwd` ends in `/Workspace/libre/librefang` (or wherever the user keeps the
main clone) **and** `git rev-parse --git-dir` prints `.git` (a directory, not
a `gitdir: ...` file), you are in the main worktree. **Stop.** Run:
```bash
git worktree add /tmp/librefang-<feature> -b <feature-branch> origin/main
```
and continue all work from that path. The `forbid-main-worktree` hook
(`.claude/hooks/forbid-main-worktree.sh`) will block edits and mutating git
commands targeted at the main tree if you forget — but the hook is a safety
net, not your plan.

### Other AI safety hooks (`.claude/hooks/`)

`guard-bash-safety.sh` (PreToolUse on Bash) blocks:
- Force-push to `main` / `master` (incl. `+main` refspec) — get explicit user OK first
- `--no-verify` / `--no-gpg-sign` on commit/push/rebase/merge/am/cherry-pick/pull
- Staging known-sensitive files (`.env*`, `*.pem`, `*.p12`, `id_rsa`, `id_ed25519`,
  `credentials*`, `secrets*`, `vault_*.key`); also broad `git add -A` / `git add .`
  (CLAUDE.md global rule: stage specific paths)
- Commit messages containing Claude attribution (`Co-Authored-By: Claude`,
  `🤖 Generated with [Claude Code]`, etc.)
- `rm -rf` against dangerous targets (`/`, `~`, `$HOME`, `target`, `.git`,
  `/Users`, `/usr`, `/etc`, `/var`, `/opt`, …)

`session-start-worktree-check.sh` (SessionStart) emits a banner telling
the model whether the session started in the main tree or a linked worktree.

## Project Overview
LibreFang is an open-source Agent Operating System written in Rust (24 crates in `crates/`, plus `xtask/`).
- Config: `~/.librefang/config.toml`
- Default API: `http://127.0.0.1:4545`
- CLI binary: `target/release/librefang.exe` (or `target/debug/librefang.exe`)

### Crate map
- **Core types & utilities**: `librefang-types`, `librefang-http`, `librefang-wire`, `librefang-telemetry`, `librefang-testing`, `librefang-migrate`
- **Kernel**: `librefang-kernel` (orchestration), `librefang-kernel-handle` (trait used by runtime to call kernel without circular dep), `librefang-kernel-router`, `librefang-kernel-metering`
- **Runtime**: `librefang-runtime` (agent loop, tools, plugins), `librefang-runtime-mcp`, `librefang-runtime-oauth`, `librefang-runtime-wasm`
- **LLM drivers**: `librefang-llm-driver` (trait + error types — interface only) and `librefang-llm-drivers` (concrete provider impls: anthropic, openai, gemini, …)
- **Memory**: `librefang-memory` (SQLite substrate)
- **Surface**: `librefang-api` (HTTP server + dashboard SPA bundled at `crates/librefang-api/dashboard/`), `librefang-cli`, `librefang-desktop`
- **Extensibility**: `librefang-skills`, `librefang-hands`, `librefang-extensions`, `librefang-channels`

## Build & Verify Workflow
**Do NOT run `cargo build`, `cargo run`, or `cargo install` locally.**
**`cargo test` is allowed only when scoped with `-p <crate>` / `--package <crate>`** —
the unscoped, workspace-wide form is blocked because it contends with the user's
other sessions on the shared `target/` directory. Full workspace build / test
runs in CI.

After every change, run:
```bash
cargo check --workspace --lib                          # Compile-check only
cargo clippy --workspace --all-targets -- -D warnings  # Zero warnings
cargo test -p <crate>                                  # Only when verifying behavior in one crate
```

## MANDATORY: Live Integration Testing
**This section is a HUMAN workflow.** Claude must NOT execute these steps —
they require `cargo build` and starting a long-lived daemon, both forbidden
to the AI per the rules above. Claude's role is to prepare exact commands /
payloads and ask the user to run them, then read the user's pasted output.
Do not auto-spawn the daemon or `cargo build --release` yourself.

**After implementing any new endpoint, feature, or wiring change, the user MUST run live integration tests.** Unit tests alone are not enough — they can pass while the feature is actually dead code. Live tests catch:
- Missing route registrations in server.rs
- Config fields not being deserialized from TOML
- Type mismatches between kernel and API layers
- Endpoints that compile but return wrong/empty data

### How to Run Live Integration Tests

#### Step 1: Stop any running daemon
```bash
tasklist | grep -i librefang
taskkill //PID <pid> //F
# Wait 2-3 seconds for port to release
sleep 3
```

#### Step 2: Build fresh release binary
```bash
cargo build --release -p librefang-cli
```

#### Step 3: Start daemon with required API keys
```bash
GROQ_API_KEY=<key> target/release/librefang.exe start &
sleep 6  # Wait for full boot
curl -s http://127.0.0.1:4545/api/health  # Verify it's up
```
The daemon command is `start` (not `daemon`).

#### Step 4: Test every new endpoint
```bash
# GET endpoints — verify they return real data, not empty/null
curl -s http://127.0.0.1:4545/api/<new-endpoint>

# POST/PUT endpoints — send real payloads
curl -s -X POST http://127.0.0.1:4545/api/<endpoint> \
  -H "Content-Type: application/json" \
  -d '{"field": "value"}'

# Verify write endpoints persist — read back after writing
curl -s -X PUT http://127.0.0.1:4545/api/<endpoint> -d '...'
curl -s http://127.0.0.1:4545/api/<endpoint>  # Should reflect the update
```

#### Step 5: Test real LLM integration
```bash
# Get an agent ID
curl -s http://127.0.0.1:4545/api/agents | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])"

# Send a real message (triggers actual LLM call to Groq/OpenAI)
curl -s -X POST "http://127.0.0.1:4545/api/agents/<id>/message" \
  -H "Content-Type: application/json" \
  -d '{"message": "Say hello in 5 words."}'
```

#### Step 6: Verify side effects
After an LLM call, verify that any metering/cost/usage tracking updated:
```bash
curl -s http://127.0.0.1:4545/api/budget       # Cost should have increased
curl -s http://127.0.0.1:4545/api/budget/agents  # Per-agent spend should show
```

#### Step 7: Verify dashboard HTML
```bash
# Check that new UI components exist in the served HTML
curl -s http://127.0.0.1:4545/ | grep -c "newComponentName"
# Should return > 0
```

#### Step 8: Cleanup
```bash
tasklist | grep -i librefang
taskkill //PID <pid> //F
```

### Key API Endpoints for Testing
| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/api/health` | GET | Basic health check |
| `/api/agents` | GET | List all agents |
| `/api/agents/{id}/message` | POST | Send message (triggers LLM) |
| `/api/budget` | GET/PUT | Global budget status/update |
| `/api/budget/agents` | GET | Per-agent cost ranking |
| `/api/budget/agents/{id}` | GET | Single agent budget detail |
| `/api/network/status` | GET | OFP network status |
| `/api/peers` | GET | Connected OFP peers |
| `/api/skills/{name}` | GET | Skill detail with evolution history |
| `/api/a2a/agents` | GET | External A2A agents |
| `/api/a2a/discover` | POST | Discover A2A agent at URL |
| `/api/a2a/send` | POST | Send task to external A2A agent |
| `/api/a2a/tasks/{id}/status` | GET | Check external A2A task status |
| `/api/approvals/{id}/approve` | POST | Approve (body: `{totp_code?}`) |
| `/api/approvals/totp/setup` | POST | Generate TOTP secret + URI |
| `/api/approvals/totp/confirm` | POST | Confirm TOTP enrollment |
| `/api/approvals/totp/status` | GET | Check TOTP enrollment status |
| `/api/approvals/totp` | DELETE | Revoke TOTP enrollment |

## Architecture Notes
- **Deterministic prompt ordering (#3298)**: anything that reaches an LLM prompt — tool definitions, MCP server summaries, skill registries, hand registries, capability lists, env passthrough lists — MUST be ordered before stringifying. Prefer `BTreeMap` / `BTreeSet` over `HashMap` / `HashSet` for those types so the compiler enforces it; otherwise sort at the boundary. HashMap iteration order varies across processes and silently invalidates provider prompt caches even when content is unchanged. Regression tests live next to each boundary — see `kernel::tests::mcp_summary_is_byte_identical_across_input_orders`, `kernel::tests::mcp_summary_inner_tool_list_is_sorted`, and `librefang_skills::registry::tests::all_tool_definitions_is_deterministic_across_insertion_orders` / `tool_definitions_for_skills_is_deterministic_across_insertion_orders`.
- **Agent workspace layout**: identity files (SOUL.md, IDENTITY.md, etc.) live in `{workspace}/.identity/`, not the workspace root. `read_identity_file()` checks `.identity/` first, falls back to root for pre-migration workspaces. `migrate_identity_files()` is called on every spawn to auto-move any root-level files.
- **Named workspaces** (`[workspaces]` in agent.toml): declare shared directories with `path` (relative to `workspaces_dir`) and `mode` (`rw` / `r`). Multiple agents sharing the same path never collide — identity files stay in their private `.identity/`. Resolved absolute paths are injected into TOOLS.md as `@name → /abs/path (mode)`. See `workspace_setup.rs: ensure_named_workspaces()`.
- `KernelHandle` trait avoids circular deps between runtime and kernel
- `AppState` in `server.rs` bridges kernel to API routes
- New routes must be registered in `server.rs` router AND implemented in `routes.rs`
- Dashboard is React+TanStack Query SPA (not Alpine.js) in `crates/librefang-api/dashboard/`
- **Dashboard data layer rule**: all API access in pages/components MUST go through hooks in `src/lib/queries/` and `src/lib/mutations/`. No `fetch()` or `api.*` calls inline in pages/components. Adding a new endpoint = add a query/mutation hook in the matching domain file, then import it. See `crates/librefang-api/dashboard/AGENTS.md` for details
- **Dashboard query keys**: always use the factories in `src/lib/queries/keys.ts`. Never inline `["foo","bar"]` arrays. Every factory must be hierarchical (`all` / `lists()` / `list(filters)` / `details()` / `detail(id)`) so `invalidateQueries({ queryKey: xxxKeys.all })` invalidates the whole domain
- **Dashboard mutations**: each mutation with side-effects must call `invalidateQueries` with factory keys in `onSuccess` (or `onSettled`). Colocate invalidation with the mutation hook, not at call sites
- Config fields need: struct field + `#[serde(default)]` + Default impl entry + Serialize/Deserialize derives
- **Trait injection pattern**: When runtime needs functionality from extensions/kernel, define a trait in runtime and implement it in kernel (e.g., `McpOAuthProvider`). Never make runtime depend on extensions (circular dep).
- **Auth middleware allowlist**: Unauthenticated endpoints must be added to the `is_public` allowlist in `middleware.rs` — NOT by reordering routes in `server.rs`. The auth layer applies to all routes.
- **Docker callback URLs**: Never bind ephemeral localhost ports for OAuth callbacks in daemon code — the port is unreachable from outside Docker. Route callbacks through the API server's existing port instead.
- **MCP OAuth flow**: Entirely UI-driven — daemon only detects 401 and sets `NeedsAuth` state. PKCE + callback handled by API layer (`routes/mcp_auth.rs`). Dynamic Client Registration (RFC 7591) used when server has `registration_endpoint` but no `client_id`.
- `session_mode` in `AgentManifest` (agent.toml, **not** config.toml) controls whether automated invocations reuse the persistent session (`"persistent"`, default) or create a fresh one (`"new"`). Per-trigger override via `Trigger.session_mode: Option<SessionMode>`. Resolution order: per-trigger override > agent manifest default. Session resolution in `execute_llm_agent` (`kernel/mod.rs` ~6959).
  - **Honors `session_mode`**: event triggers, `agent_send`.
  - **Ignores `session_mode`**: channel messages (always `SessionId::for_channel(agent,"<channel>:<chat>")`), **cron jobs** (synthesize `SenderContext{channel:"cron"}` at `kernel/mod.rs` ~12195, which takes the channel branch before `session_mode` is consulted — all cron fires for an agent share one `(agent,"cron")` session), forks (forced `Persistent` at ~5543 to preserve prompt cache).
  - When creating a trigger or cron, consciously pick: persistent (continuity, cache reuse) vs new (isolation). Don't rely on `session_mode` for per-fire fresh cron sessions — it won't work without dispatcher changes.
- **Message-history trim cap** is configurable per-agent
  (`agent.toml: max_history_messages`) and globally
  (`config.toml: max_history_messages`). Default is
  `DEFAULT_MAX_HISTORY_MESSAGES = 40`; values below
  `MIN_HISTORY_MESSAGES = 4` are clamped up with a warning.
  Resolution: agent override > kernel config > compiled default. See
  `docs/architecture/message-history-trimming.md`.
- **Trigger dispatch concurrency** has three layered caps, scoped to
  the **trigger dispatcher only** (`agent_send`, channel bridges, and
  cron still serialize at the existing per-agent / per-session locks
  inside `send_message_full`). Global `Lane::Trigger` semaphore
  (`config.toml: queue.concurrency.trigger_lane`, default 8) caps
  total in-flight trigger fires kernel-wide. Per-agent semaphore
  (`agent.toml: max_concurrent_invocations`, fallback
  `queue.concurrency.default_per_agent` default 1) caps that one
  agent's parallelism. Per-session mutex applies inside
  `send_message_full` only when the dispatcher materialized a fresh
  `SessionId` — which it does for `session_mode = "new"` fires. The
  resolver auto-clamps `persistent + cap > 1` to 1 with a `WARN` log
  (concurrent writes to a single session's history are undefined).
  Per-agent caps are NOT invalidated on manifest hot-reload — to pick
  up a new cap, **kill the agent and let it respawn** (or restart the
  daemon); an in-place activate/status flip will silently keep the old
  cap. See `docs/architecture/trigger-dispatch-concurrency.md`.

## Git Conventions
**Never include "generated by Claude Code" in commit messages** — omit the Co-Authored-By footer entirely
- **Format**: Use conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `ci:`, `perf:`, `test:`)
- **Worktree**: Use `git worktree add` on an external disk for new features; fall back to `/tmp/librefang-<feature>` only if no external disk is available. Never develop on the main worktree
- **Worktree continuation = drive to PR**: When asked to continue half-done work in an existing worktree (uncommitted changes or unmerged commits), the workflow is **commit → push → open or update PR**. Don't stop at "local commits only". A new branch needs a fresh PR; an existing branch with an open PR gets a follow-up push to update it. If the dirty changes aren't real work (e.g., stale `Cargo.lock` after rebase on an already-merged branch), discard them with `git checkout` instead of half-committing

## Common Gotchas
- `librefang.exe` may be locked if daemon is running — use `--lib` flag or kill daemon first
- `PeerRegistry` is `Option<PeerRegistry>` on kernel but `Option<Arc<PeerRegistry>>` on `AppState` — wrap with `.as_ref().map(|r| Arc::new(r.clone()))`
- Config fields added to `KernelConfig` struct MUST also be added to the `Default` impl or build fails
- `AgentLoopResult` field is `.response` not `.response_text`
- CLI command to start daemon is `start` not `daemon`
- When adding `Option<Arc<dyn Trait>>` fields to structs that derive `Serialize`/`Deserialize`/`Clone`/`Debug`, mark them `#[serde(skip)]` and implement the affected traits manually
- `ErrorTranslator` (from `RequestLanguage`) is `!Send` — any `.await` must happen AFTER `drop(t)`, or the axum handler will fail with a cryptic `Handler<_, _>` trait bound error
- `LIBREFANG_VAULT_KEY` env var must base64-decode to exactly 32 bytes (use `openssl rand -base64 32` which gives 44 chars). 32 ASCII chars ≠ 32 bytes.
- When parallel agents modify the same crate, `Option::None` defaults for new fields will silently compile but disable features. Always write integration tests at the injection site, not just the implementation site.
- On Windows: use `taskkill //PID <pid> //F` (double slashes in MSYS2/Git Bash)
