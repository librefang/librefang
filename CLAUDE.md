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
- Daemon launches: `librefang start`, `target/{debug,release}/librefang start|daemon`
  (port 4545 contention with the user's session — Live Integration Testing is human-only)
- `cargo add` / `cargo remove` / `cargo upgrade` (deps need explicit user OK)

`session-start-worktree-check.sh` (SessionStart) emits a banner telling
the model whether the session started in the main tree or a linked worktree,
and warns if `core.hooksPath` hasn't been pointed at `.githooks/`.

### Version-controlled git-side hooks (`scripts/hooks/`)

These run inside `git` itself (regardless of which tool invoked the commit),
giving defense in depth on top of the Claude Code PreToolUse layer.

- `pre-commit` — runs `cargo fmt --check` on staged Rust files; CHANGELOG
  duplicate-`[Unreleased]` guard; CHANGELOG `(@user)` attribution check on
  staged additions to `[Unreleased]` (#3400); `detect-secrets` scan against
  `.secrets.baseline` (soft-warn if not installed). Target: < 2s.
- `pre-push` — `cargo clippy --workspace --all-targets -- -D warnings`;
  OpenAPI / SDK drift detection — fails the push if `openapi.json` or
  generated SDKs are stale. Expected 30-90s on a warm cache.
- `commit-msg` — rejects commit messages containing Claude / Anthropic
  attribution (catches heredocs and `git commit -F file` that the PreToolUse
  Bash hook cannot see).

**Enable once per clone** by running setup:
```bash
just setup        # or: cargo xtask setup
```
This sets `git config core.hooksPath scripts/hooks`, which makes the in-repo
hooks active and keeps them current with `git pull` automatically. The
`session-start-worktree-check.sh` banner reminds you if it isn't configured
yet.

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

### CI test lanes (refs #3696)

CI splits tests into two separate jobs so a unit failure surfaces quickly:

- **Unit-fast** (`Test / Unit (lib+bin)`, ~2 min): `cargo nextest run --workspace --lib --bins --no-fail-fast`
  — lib and binary unit tests only; no integration test binaries. Run this locally for quick iteration.
- **Integration** (`Test / Ubuntu (shard N/4)`, ~10-20 min): sharded across 4 Ubuntu runners via
  `--partition hash:N/4`; also single jobs on macOS and Windows. Runs all `--tests` targets.

Local equivalents:
```bash
# Fast lane — unit tests only:
cargo nextest run --workspace --lib --bins --no-fail-fast

# Full validation — integration tests (mirrors the Ubuntu shard lane):
cargo nextest run --workspace --no-fail-fast
```

## MANDATORY: Integration Testing (refs #3721)

**Primary verification is automated.** The repo has comprehensive
`#[tokio::test]` integration coverage in `crates/librefang-api/tests/`,
landed via the #3571 PR series (~30 PRs). Every major route domain —
`agents`, `a2a`, `approvals`, `audit`, `authz`, `auto-dream`, `budget`,
`channels` (incl. webhooks), `config`, `goals`, `hands`, `hooks`,
`inbox`, `mcp_auth`, `media`, `memory`, `network`/`peers`/`comms`,
`oauth`, `pairing`/`backup`, `plugins`, `profiles`/`templates`,
`prompts`, `providers`/`models`, `skills`, `terminal`, `tools`/`sessions`,
`v1` (OpenAI compat), `workflows` — is exercised against a real axum
router via `TestServer` (see `start_test_server*` in
`tests/api_integration_test.rs`). Plus dedicated files:
`auth_public_allowlist.rs`, `daemon_lifecycle_test.rs`, `load_test.rs`,
`mcp_oauth_flow_test.rs`, `openapi_spec_test.rs`, `pairing_test.rs`,
`tools_invoke_test.rs`, `totp_flow_test.rs`, `users_test.rs`. CI runs
these on every push.

### What you MUST do for any route / wiring change

1. **Add a `#[tokio::test]` against `TestServer`** in the matching
   `tests/*.rs` file. Pattern: spawn router via `start_test_server()`,
   hit the endpoint with `reqwest`, assert status and response shape;
   for write endpoints, follow up with a read and assert the side
   effect. This is the canonical replacement for the old curl checklist
   — it catches missing `server.rs` registrations, un-deserialized
   config fields, kernel↔API type drift, and empty/null payloads.
2. **Run scoped tests locally**: `cargo test -p librefang-api`
   (workspace-wide `cargo test` is forbidden — see Build & Verify above).
3. **Reviewers gate PRs** on the presence of an integration test for
   each new endpoint. PRs that change route shape without a test
   should be sent back.

### When live LLM verification is required (HUMAN-only)

Live daemon + real LLM is needed **only** when the change touches an
LLM call path or end-to-end prompt/metering wiring that integration
tests can't simulate (e.g., real provider streaming, real Groq token
accounting, dashboard HTML smoke). Claude must NOT execute these steps
— they require `cargo build --release` and a long-lived daemon on
port 4545, both blocked by `.claude/hooks/`. Prepare commands and
payloads for the user; they paste output back.

```bash
# Stop any running daemon (Windows / Git Bash):
tasklist | grep -i librefang && taskkill //PID <pid> //F && sleep 3

# Build + start with provider key:
cargo build --release -p librefang-cli
GROQ_API_KEY=<key> target/release/librefang.exe start &
sleep 6 && curl -s http://127.0.0.1:4545/api/health

# Real LLM round-trip + side-effect check:
AGENT_ID=$(curl -s http://127.0.0.1:4545/api/agents | python3 -c "import sys,json; print(json.load(sys.stdin)[0]['id'])")
curl -s -X POST "http://127.0.0.1:4545/api/agents/$AGENT_ID/message" \
  -H "Content-Type: application/json" -d '{"message":"Say hello in 5 words."}'
curl -s http://127.0.0.1:4545/api/budget          # cost should have increased
curl -s http://127.0.0.1:4545/api/budget/agents   # per-agent spend visible

# Cleanup:
taskkill //PID <pid> //F
```

The daemon command is `start` (not `daemon`).

### What was retired

- The old 8-step manual curl checklist (Steps 1–8) is gone; Steps 4
  and 6 are now `#[tokio::test]` cases. Step 7 (dashboard
  `grep -c newComponentName`) is dropped — it broke under Vite
  minification. Dashboard UI verification is the dashboard test
  suite's responsibility (see `crates/librefang-api/dashboard/`).
- The "Key API Endpoints for Testing" table is gone; the canonical
  enumeration is the OpenAPI spec (`openapi.json`, regenerated by the
  pre-commit hook) and the integration tests themselves.

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
- `session_mode` in `AgentManifest` (agent.toml, **not** config.toml) controls whether automated invocations reuse the persistent session (`"persistent"`, default) or create a fresh one (`"new"`). Per-trigger override via `Trigger.session_mode: Option<SessionMode>`. Per-cron override via `CronJob.session_mode: Option<SessionMode>`. Resolution order: per-trigger / per-job override > agent manifest default. Session resolution in `execute_llm_agent` (`kernel/mod.rs` ~6959).
  - **Honors `session_mode`**: event triggers, `agent_send`, **cron jobs** (since #3597 / #3657 — see below).
  - **Ignores `session_mode`**: channel messages (always `SessionId::for_channel(agent,"<channel>:<chat>")`), forks (forced `Persistent` at ~5543 to preserve prompt cache).
  - **Cron + `session_mode`** (resolution at `kernel/mod.rs` ~13609, helper `cron::cron_fire_session_override`):
    - Effective mode = per-job `CronJob.session_mode` > agent manifest `session_mode` > historical `Persistent`.
    - `Persistent` (or unset): the cron tick synthesizes `SenderContext{channel:"cron"}` and `send_message_full` derives `SessionId::for_channel(agent,"cron")`, so all fires of all cron jobs for that agent share one `(agent,"cron")` persistent session (historical behaviour, prompt-cache reuse).
    - `New`: `cron_fire_session_override` returns an explicit `SessionId::for_cron_run(agent, "<job_id>:<rfc3339_fire_time>")` which is passed as `session_id_override` into `send_message_full`. The override path wins over the channel-derived branch, so each fire lands on its own deterministic, isolated session — prior fires never leak into the current run, and the persistent `(agent,"cron")` session stays untouched.
  - When creating a trigger or cron, consciously pick: `Persistent` (continuity, cache reuse) vs `New` (isolation, fresh context per fire).
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

## GitHub Collaboration & Wait Policy

LibreFang is an open-source project with heavy AI-assistant traffic. The
rules below codify the boundaries summarised in `AGENTS.md` ("AI Agent
Collaboration") so that maintainers stay in control of their own PRs and
issue threads.

### Touching other people's work

- **Maintainer-reviewed PRs are off-limits.** Once a human maintainer has
  left review comments, an `Approve`, or a `Request changes` on a PR, do
  not push additional commits to that branch unless the maintainer
  explicitly asks for them. The right move is a follow-up PR that
  references the original.
- **Never close a PR or issue opened by someone else.** If you believe an
  issue is fixed or a PR is superseded, post a comment recommending
  closure with the linking commit / PR — let a maintainer pull the trigger.
- **Force-push only to your own branches, only before review.** Once a
  reviewer has loaded the diff, prefer fixup commits or a follow-up PR
  over rewriting history. Force-push to `main` / `master` is blocked by
  `guard-bash-safety.sh` and requires explicit user OK regardless.
- **Don't reassign, re-label, or re-milestone** issues / PRs you did not
  open unless asked. Adding `needs-review` or self-assigning a triage
  label is fine; flipping `priority` / `release` labels is not.

### Commit & PR hygiene

- **No Claude / Anthropic / AI attribution** in commit messages, PR
  bodies, or issue comments. The `commit-msg` git hook rejects matching
  strings; the PreToolUse Bash hook catches the inline-flag form. Don't
  try to route around either — the rule exists because attribution
  pollutes `git log` and signals provenance the project does not want to
  imply.
- **One PR ↔ one issue (or one tight cluster).** Don't bundle unrelated
  refactors with the requested change. If you find a real problem
  out-of-scope, open a separate issue or follow-up PR; mention it in the
  current PR's "Out-of-scope follow-ups" section.
- **PR body must enumerate** the substantive changes, the verification
  performed (integration test names, `cargo check --workspace --lib`
  output, scoped `cargo test -p <crate>` runs), and any deferred work.
  Bullet form, no marketing prose.

### CI wait policy

CI is shared infrastructure and frequently slow. Polling it from an AI
session burns turns without producing information.

- **Hard cap: ~5 minutes of polling.** After that, push, leave the run
  URL in the PR / report, and **stop**. Don't loop a `gh run watch` for
  half an hour.
- **Don't pre-emptively re-run a check** that has not yet failed. Only
  retry after a recorded failure, and only once.
- **Don't open follow-up issues or pivot the plan** while waiting for CI
  or review. If you cannot make further progress without information you
  do not have, the correct action is to report status and yield — not to
  invent more work.
- **Don't add reviewers, flip `ready-for-review`, or `gh pr ready`** on
  someone else's behalf, and don't re-request review on your own PR
  unless a maintainer has explicitly asked you to ping them. Maintainers
  pull work into their queue; AI agents do not push it onto theirs.

### Issue / PR comment etiquette

- **At most two follow-up comments** on the same thread without human
  input. After that, stop and wait — repeated AI-generated pings on a
  silent thread are noise, not progress.
- **Don't comment on threads you have no action on.** "Looks good"
  drive-bys from an AI account add nothing.
- **When you reply, link evidence:** commit SHAs, file paths, test
  names. No vibes-only comments.

### Conflict resolution

- **Latest maintainer intent wins.** When rebasing or resolving merge
  conflicts that touch a human-authored hunk, keep the maintainer's
  edit. If the two sides genuinely disagree, surface the conflict in the
  PR body and ask — don't silently pick the smaller diff.
- **Preserve both sides' intent** during conflict resolution. Dropping a
  hunk because "it'll be reapplied later" is how regressions land.

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
