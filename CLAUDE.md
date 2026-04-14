# LibreFang — Agent Instructions

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
After every feature implementation, run ALL THREE checks:
```bash
cargo build --workspace --lib          # Must compile (use --lib if exe is locked)
cargo test --workspace                 # All tests must pass (currently 2100+)
cargo clippy --workspace --all-targets -- -D warnings  # Zero warnings
```

## MANDATORY: Live Integration Testing
**After implementing any new endpoint, feature, or wiring change, you MUST run live integration tests.** Unit tests alone are not enough — they can pass while the feature is actually dead code. Live tests catch:
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
- **Don't touch `librefang-cli`** — user is actively building the interactive CLI
- `KernelHandle` trait avoids circular deps between runtime and kernel
- `AppState` in `server.rs` bridges kernel to API routes
- New routes must be registered in `server.rs` router AND implemented in `routes.rs`
- Dashboard is React+TanStack Query SPA (not Alpine.js) in `crates/librefang-api/dashboard/`
- Config fields need: struct field + `#[serde(default)]` + Default impl entry + Serialize/Deserialize derives
- **Trait injection pattern**: When runtime needs functionality from extensions/kernel, define a trait in runtime and implement it in kernel (e.g., `McpOAuthProvider`). Never make runtime depend on extensions (circular dep).
- **Auth middleware allowlist**: Unauthenticated endpoints must be added to the `is_public` allowlist in `middleware.rs` — NOT by reordering routes in `server.rs`. The auth layer applies to all routes.
- **Docker callback URLs**: Never bind ephemeral localhost ports for OAuth callbacks in daemon code — the port is unreachable from outside Docker. Route callbacks through the API server's existing port instead.
- **MCP OAuth flow**: Entirely UI-driven — daemon only detects 401 and sets `NeedsAuth` state. PKCE + callback handled by API layer (`routes/mcp_auth.rs`). Dynamic Client Registration (RFC 7591) used when server has `registration_endpoint` but no `client_id`.
- `session_mode` in `AgentManifest` controls whether automated invocations (cron, triggers, agent_send) reuse the persistent session (`"persistent"`, default) or create a fresh one (`"new"`). Per-trigger overrides via `Trigger.session_mode: Option<SessionMode>`. Session resolution in `execute_llm_agent` (kernel.rs). Channel-derived sessions are unaffected.

## Git Conventions
**Never include "generated by Claude Code" in commit messages** — omit the Co-Authored-By footer entirely
- **Format**: Use conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `ci:`, `perf:`, `test:`)
- **Worktree**: Use `git worktree add` on an external disk for new features; fall back to `/tmp/librefang-<feature>` only if no external disk is available. Never develop on the main worktree

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
