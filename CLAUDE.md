# LibreFang — Agent Instructions

## Project Overview
LibreFang is an open-source Agent Operating System written in Rust (14 crates).
- Config: `~/.librefang/config.toml`
- Default API: `http://127.0.0.1:4545`
- CLI binary: `target/release/librefang.exe` (or `target/debug/librefang.exe`)

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

## Architecture Notes
- **Don't touch `librefang-cli`** — user is actively building the interactive CLI
- `KernelHandle` trait avoids circular deps between runtime and kernel
- `AppState` in `server.rs` bridges kernel to API routes
- New routes must be registered in `server.rs` router AND implemented in `routes.rs`
- Dashboard is Alpine.js SPA in `static/index_body.html` — new tabs need both HTML and JS data/methods
- Config fields need: struct field + `#[serde(default)]` + Default impl entry + Serialize/Deserialize derives

## Mandatory Dev Workflow

**Every code change follows the 10-step workflow. These rules are always active and cannot be disabled.**

1. **Sherlock baseline before code** — run `cargo test -p <crate>` and record the pass count. No implementation claim is valid without a before/after test count.
2. **AQE pre-code required** — `qe-requirements-validator`, `qe-devils-advocate`, `qe-impact-analyzer` run before the coder agent on any non-trivial change.
3. **No claim without evidence** — no session summary, PR description, or commit message may contain an implementation claim unless it cites a passing test name from the PLAN exit criteria OR a Sherlock verdict.
4. **RED before GREEN** — all failing tests written before any implementation code. Tests must fail for a logic reason, not a compile error.
5. **Clippy gate** — `cargo clippy --workspace --all-targets -- -D warnings` must exit zero before a round is closed. Vendor code we wrote is not exempt.
6. **SPEC required** — always. Minimum: acceptance criteria + `Claims requiring verification` section.
7. **PLAN required** — always. Minimum: Verified Baseline Facts + TDD cycle + Exit Criteria commands.

Full operational guide: `.claude/skills/flow-coach-codex-dev/SKILL.md`
Governance: `docs/adr/ADR-013-development-workflow.md`

## Git Conventions
**Never include "generated by Claude Code" in commit messages** — omit the Co-Authored-By footer entirely
- **Format**: Use conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `ci:`, `perf:`, `test:`)
- **Worktree**: Use `git worktree add` under `/tmp/` for new features (e.g. `/tmp/librefang-<feature>`), never develop on the main worktree

## Common Gotchas
- `librefang.exe` may be locked if daemon is running — use `--lib` flag or kill daemon first
- `PeerRegistry` is `Option<PeerRegistry>` on kernel but `Option<Arc<PeerRegistry>>` on `AppState` — wrap with `.as_ref().map(|r| Arc::new(r.clone()))`
- Config fields added to `KernelConfig` struct MUST also be added to the `Default` impl or build fails
- `AgentLoopResult` field is `.response` not `.response_text`
- CLI command to start daemon is `start` not `daemon`
- On Windows: use `taskkill //PID <pid> //F` (double slashes in MSYS2/Git Bash)
