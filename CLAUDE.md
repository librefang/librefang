# LibreFang — Agent Instructions

## ⚠️ Before any work: verify you are in a worktree, not the main tree

The very first action in any task that will edit files **must** be:

```bash
test -d "$(git rev-parse --show-toplevel)/.git" && echo main || echo linked
```

- prints `main` → you are in the **main worktree**. **Stop.** Run `git worktree add /tmp/librefang-<feature> -b <feature-branch> origin/main` and continue all work from that path.
- prints `linked` → you are in a **linked worktree**. Continue.

Git stores the main worktree's `.git` as a directory and a linked worktree's `.git` as a text file, so the directory test is true exactly in the main worktree.
Do not substitute `git rev-parse --git-dir` (path-shaped output, varies with cwd) or path-matching against `pwd` (every clone lives somewhere different).

### Safety hooks — what will stop you

`.claude/hooks/` (Claude Code PreToolUse / SessionStart) and `scripts/hooks/` (version-controlled git hooks) enforce this contract in two independent layers.
Full enumeration: `docs/development/ai-safety-hooks.md`.
The short list of things that get blocked:

- Editing files or running mutating git commands in the **main worktree**.
  The one exception is `git pull --ff-only [origin [main]]` (optionally with `-q`, `-v` or `--prune`), which fast-forwards the checked-out `main` to `origin/main` and cannot create a commit unless repo config overrides the pull strategy (for example `pull.twohead`).
  Any other form of `git pull` stays blocked, because a later `--ff` / `--no-ff`, a strategy, `--squash` or another ref would defeat that guarantee.
- Force-push to `main` / `master`; `--no-verify` / `--no-gpg-sign` on any git command.
- Staging sensitive files (`.env*`, `*.pem`, `id_rsa`, `credentials*`, …) and broad `git add -A` / `git add .` — stage specific paths.
- Claude / Anthropic attribution in a commit message, **or** a commit author identity that resolves to Claude / Anthropic.
- `rm -rf` against dangerous targets (`/`, `~`, `$HOME`, `target`, `.git`, `/usr`, `/etc`, …).
- Launching the daemon (`librefang start`, `target/*/librefang start|daemon`) — port 4545 belongs to the user's session, and live testing is human-only.
- A changelog fragment in an unrecognised `changelog.d/` section directory, or an `[Unreleased]` addition missing `(@user)` attribution.

**Enable the git-side hooks once per clone**: `just setup` (or `cargo xtask setup`), which sets `git config core.hooksPath scripts/hooks`.

Also enable the orchestrator-file merge driver (prevents `.kbd-orchestrator/`
conflicts from surfacing during `git rebase` / `git merge`):
```bash
git config merge.ours.driver true
```
`.gitattributes` declares `merge=ours` for all `.kbd-orchestrator/**/*.json`
and `.kbd-orchestrator/**/*.md` files. Without the driver registered, git
falls back to manual conflict resolution on those files.

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
  Spawn the native agent (`.claude/agents/<role>.md`) when the harness exposes it; otherwise spawn a general-purpose agent and hand it the role file to follow.
- **Work no role owns** (general kernel, API, dashboard, CLI or hook code) still goes to a delegated agent in its own worktree, which follows this file.
- **Review is independent.**
  Nothing merges to `main` without a review in a fresh context, never the builder's.
  `merge-reviewer` reviews upstream syncs; every other change gets a fresh-context reviewer agent (for example `code-reviewer`, `rust-reviewer`, or `security-reviewer` for hooks and auth).
- **Hand off explicitly.**
  A role that finds work owned by another role reports it as a handoff.
  The orchestrator dispatches it rather than letting the finder absorb it.

## Process Discipline

How you change code in this repo. Six principles synthesised from Andrej
Karpathy's four behavioural rules and Boris Cherny's three principles +
self-improvement loop, adapted to the tools actually available in this
session (Plan/Explore agents, TaskCreate, auto-memory, Prometheus skill
pack). These apply on top of the worktree / hook safety net above and
the BossFang preservation rules below.

### 1. Think Before Coding

State assumptions explicitly. Don't hide confusion. Surface tradeoffs.

- Before any non-trivial implementation, use the **Plan agent**
  (`Agent(subagent_type="Plan")`) to design the approach, or write a
  brief numbered plan with verify-steps inline before touching code.
- For codebase research that will run >3 queries, use the **Explore
  agent** (`Agent(subagent_type="Explore")`) instead of repeated greps —
  it protects the main context window.
- If multiple interpretations of the ask exist, present them. Don't pick
  silently.
- If a simpler approach exists, say so. Push back when warranted.
- Consult the relevant Prometheus skill BEFORE writing code. The
  `coding-guidelines`, `rust-best-practices`, `error-handling`,
  `async-patterns`, `m05-type-driven`, `m09-domain`, `m10-performance`,
  `m12-lifecycle`, `m13-domain-error`, `m15-anti-pattern`, and
  `axum-patterns` skills are installed and cover the recurring decision
  points in this workspace.

### 2. Simplicity First

Minimum code that solves the problem. Nothing speculative.

- No features beyond what was asked.
- No abstractions for single-use code; three similar lines is better
  than a premature trait.
- No "flexibility" / "configurability" that wasn't requested.
- No error handling for impossible scenarios — trust internal
  invariants and only validate at boundaries (user input, external
  APIs).
- If you write 200 lines and it could be 50, rewrite it.
- If you can delete lines instead of adding them, do that.
- Ask: "Would a senior Rust engineer say this is overcomplicated?" If
  yes, simplify.

### 3. Surgical Changes (Minimal Impact)

Touch only what you must. Clean up only your own mess.

- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- Remove imports / variables / functions that YOUR changes orphaned.
  Don't remove pre-existing dead code unless asked.
- Every changed line should trace directly to the user's request.
- If you spot unrelated dead code or a bug, mention it — don't fix
  inline unless the user OKs. Use `mcp__ccd_session__spawn_task` for
  high-confidence out-of-scope items, or surface them in the PR body
  under "Out-of-scope follow-ups".

### 4. Root-Cause Discipline (No Laziness)

Find the underlying issue. No temporary fixes. No band-aids.

- A failing test is information. Don't `#[ignore]` it. Don't widen a
  tolerance. Don't `unwrap_or_default()` past the error. Fix what the
  test caught.
- If a clippy warning is hard to satisfy, the rule is "fix the code",
  not "suppress the lint".
- Never bypass git hooks (`--no-verify`, `--no-gpg-sign`) — the hook
  catches what you missed; investigating is cheaper than the
  cascading-failure debug session it prevents.
- Hold yourself to senior-engineer standards: a fix is "done" when the
  invariant it was meant to enforce can't be broken from the call
  sites you can see.

### 5. Goal-Driven Verification

Define success criteria. Loop until verified.

- Transform vague asks into verifiable goals:
  - "Add validation" → "Write tests for invalid inputs, then make them
    pass"
  - "Fix the bug" → "Add an integration test in
    `crates/librefang-api/tests/` that reproduces it, then make it pass"
  - "Refactor X" → "Run scoped `cargo test -p <crate>` before and
    after; both green"
- For multi-step work, use **TaskCreate** to capture the plan and
  **TaskUpdate** to flip states as you go. Mark a task `completed` the
  moment it's done — never batch.
- Strong success criteria let you loop independently. Weak criteria
  ("make it work") force the user to clarify after mistakes have
  landed.
- Verify before claiming done: `cargo check --workspace --lib` AND
  scoped `cargo test -p <crate>` AND
  `python3 scripts/enforce-branding.py --check` AND any audit script
  the change implicates. Code that compiles isn't code that works.

### 6. Self-Improvement via Memory (replaces `lessons.md`)

Boris Cherny's "anytime you do something wrong, add it to CLAUDE.md so
you don't repeat it" rule maps to our **auto-memory system** at
`/Users/gqadonis/.claude/projects/.../memory/` (see the "auto memory"
section in the system prompt). When the user corrects you OR confirms
a non-obvious approach worked, write a **feedback memory** with `Why:`
and `How to apply:` fields. Feedback memories are auto-loaded into
every future conversation in this repo, so the same mistake never
costs the user a second correction.

Do NOT create a `lessons.md` file — it would duplicate the memory
system and stay un-loaded.

### When to skip these rules

For trivial tasks (typo fix, single-line obvious change), use
judgement. These rules bias toward caution over speed. The bias is
correct for any change that touches more than one file, modifies
behaviour, or interacts with the BossFang preservation surface below.

## Project Overview
LibreFang is an open-source Agent Operating System written in Rust (31 crate directories under `crates/`, plus `xtask/`).
29 of them are `[workspace] members`; the two BossFang-exclusive crates — `librefang-storage` and `librefang-uar-spec` — build as path dependencies rather than members, so `--workspace` commands do not select them (check them explicitly with `-p`).
- Config: `~/.librefang/config.toml`
- Default API: `http://127.0.0.1:4545`
- CLI binary: `target/release/librefang`, `.exe` suffix on Windows (debug builds at the matching `target/debug/` path)

### Crate map

- **Core types & utilities**: `librefang-types`, `librefang-http`, `librefang-wire`, `librefang-telemetry`, `librefang-testing`, `librefang-import`
- **Kernel**: `librefang-kernel` (orchestration), `librefang-kernel-handle` (trait used by runtime to call kernel without circular dep), `librefang-kernel-router`, `librefang-kernel-metering`
- **Runtime**: `librefang-runtime` (agent loop, tools, plugins, OAuth, WASM sandbox), `librefang-runtime-mcp`, `librefang-runtime-audit`, `librefang-runtime-media`, `librefang-runtime-sandbox-docker`
- **LLM drivers**: `librefang-llm-driver` (trait + error types — interface only) and `librefang-llm-drivers` (concrete provider impls: anthropic, openai, gemini, uar, …)
- **Memory**: `librefang-memory` (SurrealDB + SQLite fallback substrate), `librefang-memory-wiki` (durable markdown knowledge vault)
- **Storage**: `librefang-storage` (BossFang — SurrealDB abstraction layer + 24 SurrealQL migrations; see BossFang-Exclusive Features below)
- **Surface**: `librefang-api` (HTTP server + dashboard SPA bundled at `crates/librefang-api/dashboard/`), `librefang-cli`, `librefang-desktop`, `librefang-acp` (ACP adapter for Zed / VSCode / JetBrains)
- **Extensibility**: `librefang-skills`, `librefang-hands`, `librefang-extensions`, `librefang-channels`, `librefang-subprocess` (sidecar stdio transport), `librefang-rl-export` (RL rollout exporter)
- **UAR spec** (BossFang): `librefang-uar-spec` (UAR-AGENT-MD spec types + AgentManifest translator; see BossFang-Exclusive Features below)

## Build & Verify Workflow

**Do NOT run `cargo build` or `cargo run` locally.**
**`cargo test` is allowed only when scoped with `-p <crate>` / `--package <crate>`** — the unscoped workspace-wide form contends with the user's other sessions on the shared `target/` directory.

### Verify locally. Never wait on CI.

**CI is not a verification step in this repo.** It is slow, and waiting on it stalls work that a local `cargo check` answers in a minute or two.

- **Never** push a branch and then poll GitHub Actions for the result.
- **Never** describe a change as "verified by CI", and never defer a failing or skipped check to CI.
- **Never** treat a red or skipped CI lane as a reason to pause — verify the same property locally instead.
- **Never** build `Dockerfile.rust-dev` to run a check. That image exists for hosts with no native toolchain; this host has one, and building it costs ~20 minutes.

There is a native `cargo` toolchain on the development host. Use it:

```bash
export CARGO_TARGET_DIR=/tmp/librefang-target-<worktree>   # keep off the shared target/
export SKIP_DASHBOARD_BUILD=1                              # build.rs soft-skips without pnpm

cargo check --workspace --lib                          # Compile-check only
cargo clippy --workspace --all-targets -- -D warnings  # Zero warnings
cargo test -p <crate>                                  # Behaviour, one crate at a time
```

Both env vars matter. `CARGO_TARGET_DIR` keeps a linked worktree off the shared `target/` so a check here cannot contend with another session. `SKIP_DASHBOARD_BUILD=1` stops `librefang-api/build.rs` shelling out to `pnpm`, which dominates a cold check and is irrelevant to Rust correctness.

Read cargo's **own** exit status, not a wrapper's. Writing `VAR=$?` and then piping to `tail` makes the shell report `tail`'s status — a failed build then looks like success. Write the status as the final action (`cmd > log 2>&1; echo $? > log.status`) and read that file.

## MANDATORY: Integration Testing (refs #3721)

Primary verification is automated: `#[tokio::test]` coverage in `crates/librefang-api/tests/` exercises every major route domain against a real axum router via `TestServer` (`start_test_server*` in `tests/api_integration_test.rs`).

For any route / wiring change:

1. **Add a `#[tokio::test]` against `TestServer`** in the matching `tests/*.rs` file — spawn the router, hit the endpoint with `reqwest`, assert status and shape; for write endpoints, follow up with a read and assert the side effect.
   This is what catches missing `server.rs` registrations, un-deserialized config fields, kernel↔API type drift, and empty/null payloads.
2. **Run scoped tests locally**: `cargo test -p librefang-api`.
3. **Reviewers gate PRs** on the presence of an integration test for each new endpoint.

**Live daemon + real LLM is HUMAN-only.** It is needed only for LLM call paths or end-to-end prompt/metering wiring that integration tests can't simulate.
Claude must not execute it — prepare the commands from `docs/development/build-and-verify.md` and hand them to the user.

## Architecture Notes

- **Deterministic prompt ordering (#3298)**: anything reaching an LLM prompt — tool definitions, MCP server summaries, skill / hand registries, capability lists, env passthrough lists — MUST be ordered before stringifying.
  Prefer `BTreeMap` / `BTreeSet` over `HashMap` / `HashSet` so the compiler enforces it; otherwise sort at the boundary.
  HashMap iteration order varies across processes and silently invalidates provider prompt caches even when content is unchanged.
  Regression tests sit next to each boundary — `kernel::tests::mcp_summary_is_byte_identical_across_input_orders`, `kernel::tests::mcp_summary_inner_tool_list_is_sorted`, `librefang_skills::registry::tests::all_tool_definitions_is_deterministic_across_insertion_orders`.
- **Agent workspace layout**: identity files (SOUL.md, IDENTITY.md, …) live in `{workspace}/.identity/`, not the workspace root.
  `read_identity_file()` checks `.identity/` first and falls back to root for pre-migration workspaces; `migrate_identity_files()` runs on every spawn.
- **Named workspaces** (`[workspaces]` in agent.toml): shared directories declared with `path` (relative to `workspaces_dir`) and `mode` (`rw` / `r`).
  Agents sharing a path never collide — identity files stay in their private `.identity/`.
  Resolved absolute paths are injected into TOOLS.md as `@name → /abs/path (mode)`.
  See `workspace_setup.rs: ensure_named_workspaces()`.
- `KernelHandle` trait avoids circular deps between runtime and kernel; `AppState` in `server.rs` bridges kernel to API routes.
- **Adding a route**: there is no single `routes.rs`.
  Handlers live in `crates/librefang-api/src/routes/`, split per domain (`agents.rs`, `memory.rs`, `system.rs`, …), each exporting its own `router()`; `server.rs::api_v1_routes()` composes them with `.merge()`, and some domains nest a second level (`routes/system.rs::router()` merges `agent_templates`, `approvals`, `pairing`, …).
  Implement the handler in the matching domain module and merge its `router()`.
  Drift guards: `tests/dead_route_audit_test.rs` and `tests/openapi_path_coverage_test.rs`.
- **Auth middleware allowlist**: unauthenticated endpoints go in the `is_public` allowlist in `middleware.rs` — NOT by reordering routes in `server.rs`. The auth layer applies to all routes.
- **Dashboard** is a React + TanStack Query SPA (not Alpine.js) in `crates/librefang-api/dashboard/`. See its `AGENTS.md` for detail.
  - All API access in pages/components MUST go through hooks in `src/lib/queries/` and `src/lib/mutations/`. No inline `fetch()` or `api.*` calls.
  - Query keys always come from the factories in `src/lib/queries/keys.ts` — never inline `["foo","bar"]`. Every factory is hierarchical (`all` / `lists()` / `list(filters)` / `details()` / `detail(id)`) so `invalidateQueries({ queryKey: xxxKeys.all })` invalidates the whole domain.
  - Every side-effecting mutation calls `invalidateQueries` with factory keys in `onSuccess` / `onSettled`, colocated with the mutation hook rather than at call sites.
- **Config fields** need all four: struct field, `#[serde(default)]`, `Default` impl entry, `Serialize`/`Deserialize` derives.
- **Trait injection pattern**: when runtime needs functionality from extensions/kernel, define the trait in runtime and implement it in kernel (e.g. `McpOAuthProvider`). Never make runtime depend on extensions (circular dep).
- **Docker callback URLs**: never bind ephemeral localhost ports for OAuth callbacks in daemon code — unreachable from outside Docker. Route callbacks through the API server's existing port.
- **MCP OAuth flow** is entirely UI-driven: the daemon only detects 401 and sets `NeedsAuth`.
  PKCE + callback are handled by the API layer (`routes/mcp_auth.rs`); Dynamic Client Registration (RFC 7591) kicks in when a server has a `registration_endpoint` but no `client_id`.
- **`session_mode`** (in `agent.toml`, **not** `config.toml`) decides whether an automated invocation reuses the persistent session (`"persistent"`, default) or creates a fresh one (`"new"`).
  Honoured by event triggers, `agent_send` and cron jobs; ignored by channel messages (always `SessionId::for_channel`) and forks (forced `Persistent` for prompt cache).
  Resolution order, the cron `Persistent`-vs-`New` session-id behaviour, and the `cron_fire_session_override` helper: `docs/architecture/session-mode-resolution.md`.
  When creating a trigger or cron, pick consciously — `Persistent` for continuity and cache reuse, `New` for isolation.
- **Per-agent `proactive_memory` / `skill_workshop` / `compaction` overrides live in `agent.toml`, NOT `config.toml`** (#5476).
  `KernelConfig` has no `agents` field, so `[agents.<name>.<key>]` in `config.toml` parses but never reaches any `AgentManifest`.
  The kernel emits a targeted `WARN` at boot and on `POST /api/config/reload` (`KernelConfig::detect_misplaced_per_agent_overrides`).
  Inside a `HAND.toml` the `[agents.<name>.<key>]` form *is* read, because `HandManifest` does have an `agents` table.
- **Message-history trim cap** is per-agent (`agent.toml: max_history_messages`) and global (`config.toml: max_history_messages`).
  Default `DEFAULT_MAX_HISTORY_MESSAGES = 60`; values below `MIN_HISTORY_MESSAGES = 4` are clamped up with a warning.
  Resolution: agent override > kernel config > compiled default. See `docs/architecture/message-history-trimming.md`.
- **Trigger dispatch concurrency** has three layered caps scoped to the **trigger dispatcher only** — `agent_send`, channel bridges and cron still serialize on the existing per-agent / per-session locks inside `send_message_full`.
  Global `Lane::Trigger` semaphore (`config.toml: queue.concurrency.trigger_lane`, default 8), per-agent semaphore (`agent.toml: max_concurrent_invocations`, fallback `queue.concurrency.default_per_agent` default 1), per-session mutex.
  `persistent` + cap > 1 auto-clamps to 1 with a `WARN` (concurrent writes to one session's history are undefined).
  Per-agent caps are **not** invalidated on manifest hot-reload — kill the agent and let it respawn, or restart the daemon; an in-place activate/status flip silently keeps the old cap.
  See `docs/architecture/trigger-dispatch-concurrency.md`.
- **Config hot-reload classification** — which `KernelConfig` fields hot-reload, which need a restart, which are read-live/noop — is decided by `build_reload_plan` in `crates/librefang-kernel/src/config_reload.rs`.
  Consult the drift-guarded table at `docs/operations/config-reload.md` before assuming a config edit takes effect on `POST /api/config/reload`.
- **Automatic memory is scoped three ways** (#7605): `capabilities.memory_read` / `memory_write` in `agent.toml` gate the auto paths, the #5227 `chat_scope` stamp separates chats, and the `session_scope` stamp separates sessions.
  The two capability lists are **tri-state** — an absent key means "unrestricted" as everywhere else in a manifest, but `memory_read = []` is a declared-empty list that denies, which is why they are `Option<Vec<String>>` and must be read through `ManifestCapabilities::allows_own_memory_read` / `allows_own_memory_write`.
  Session scoping is on by default (`config.toml: [proactive_memory] session_scoped_recall`, per-agent override in `agent.toml`); it uses the session the turn already belongs to, never a second notion of one.
  The one behaviour it changes for an agent that configures nothing is `session_mode = "new"`, whose fresh-per-invocation sessions no longer recall each other's memories.
  See `docs/architecture/proactive-memory-scoping.md`.
- **Skill workshop** (#3328) passively captures teaching signals from successful turns into draft skills under `~/.librefang/skills/pending/<agent>/<uuid>.toml`.
  **Default-OFF — opt in per agent** with `[skill_workshop] enabled = true` in `agent.toml` (or the matching `[agents.<name>]` section of a `HAND.toml`); source of truth is `SkillWorkshopConfig::default()` in `crates/librefang-types/src/agent.rs`.
  Approval routes through `evolution::create_skill`, so the prompt-injection scan runs at both `save_candidate` and `approve_candidate` — every artefact the agent can see has crossed the same security boundary as a marketplace skill.
  `review_mode = "threshold_llm"` uses the `AuxTask::SkillWorkshopReview` slot on the cheap-tier provider chain, and returns `Indeterminate` rather than billing the primary provider when no cheap-tier credentials exist (financial-DoS guard).
  CLI `librefang skill pending list / show / approve / reject`; HTTP `GET/POST /api/skills/pending[…]`; dashboard `PendingSkillsSection`.
  See `docs/architecture/skill-workshop.md`.

## BossFang Branding — Non-negotiable Rules

This is the **BossFang** fork of LibreFang. The product name, logo, and color palette
must survive every upstream merge unchanged. Upstream (LibreFang) branding must never
appear in any committed or pushed state.

### Authoritative brand tokens

| Token | Light mode | Dark mode |
|---|---|---|
| Primary brand color (`--brand-color`) | `#E04E28` (Muted Ember) | `#FF6A3D` (Bright Ember) |
| Brand muted (`--brand-muted`) | `#F2D6CF` | `rgba(255,106,61,0.15)` |
| Main background (`--bg-main`) | `#F7F7F8` | `#0B0F14` (Deep Charcoal) |
| Logo asset | `boss-libre.png` | same |
| Product name | **BossFang** | same |

Full spec: `docs/branding/branding-guide.html`.
Upstream's sky-blue tokens (`#0284c7`, `#38bdf8`, `#0ea5e9`) and dark background
(`#020617`) must **never** appear in dashboard source files.

### After every upstream merge — mandatory steps

Before committing the resolved merge, run all five steps:

```bash
# 1. Fix any upstream sky-blue tokens (idempotent)
python3 scripts/enforce-branding.py

# 2. Audit — fail-loud if any upstream token survived the fix pass
python3 scripts/enforce-branding.py --check   # exit 0 = clean

# 3. Check for new upstream SQLite schema changes and add SurrealDB equivalents
#    grep the diff for CREATE TABLE / ALTER TABLE / ADD COLUMN
git diff <last-merge-base> upstream/main -- "*.rs" | grep -A5 "CREATE TABLE\|ALTER TABLE\|ADD COLUMN"
#    For each new table/column: add a .surql migration in
#    crates/librefang-storage/src/migrations/sql/NNN_<name>.surql
#    and register it in crates/librefang-storage/src/migrations/mod.rs

# 4. Compile-check
cargo check --workspace --lib

# 5. Verify BossFang-exclusive feature crates still compile
cargo check -p librefang-storage -p librefang-uar-spec
```

The enforcement script scans `*.ts`, `*.tsx`, `*.css`, `*.html`, `*.json`,
and `*.rs` files under `crates/librefang-api/dashboard/src/`,
`crates/librefang-api/static/`, `crates/librefang-desktop/frontend/`, and
`crates/librefang-desktop/src/` — replacing upstream sky-blue tokens with
BossFang ember equivalents. Both modes are idempotent.

### Merge conflict resolution rules for branding files

| File | Strategy |
|---|---|
| `dashboard/src/index.css` `:root` / `:root.dark` blocks | **Always take ours** — keep ember tokens. Merge upstream's new utility classes (animations, layout helpers) manually. |
| `dashboard/src/App.tsx` sidebar brand block | **Always take ours** — logo must be `boss-libre.png`, name must be "BossFang". |
| `dashboard/src/App.tsx` mobile header block | **Always take ours** — same as sidebar. |
| New upstream TSX components with inline `style={{ background: "linear-gradient(135deg,#38bdf8,...)" }}` | Take upstream content, then replace sky-blue hex values with ember (`#FF6A3D` for the bright side, `#E04E28` for the muted side). Run `scripts/enforce-branding.py` to catch any you miss. |
| `dashboard/public/manifest.json` name / short_name | **Always take ours** — must say "BossFang Dashboard" / "BossFang". |
| `dashboard/index.html` title / meta tags | **Always take ours** — must say "BossFang". |
| `.github/workflows/deploy-web.yml` and `deploy-docs.yml` `--project-name=` | **Always take ours** — must be `bossfang` / `bossfang-docs`, never upstream's `librefang` / `librefang-docs`. The Cloudflare Pages project name becomes the public hostname (`bossfang.pages.dev`), so taking upstream's value publishes the fork's site under upstream branding. The projects live in the fork's own Cloudflare account; `CLOUDFLARE_API_TOKEN` / `CLOUDFLARE_ACCOUNT_ID` are set as repo secrets on GQAdonis/librefang. |
| `crates/librefang-desktop/tauri.conf.json` | **Always take ours** — `productName: "BossFang"`, `identifier: "ai.bossfang.desktop"`, descriptions, BossFang `shortDescription` / `longDescription`, and the `plugins.updater` block: endpoint pinned at `https://github.com/GQAdonis/librefang/releases/latest/download/latest.json` and the minisign `pubkey` whose key ID decodes to `E329A6B2863F1707` (NOT upstream's `BC91908BD3F1520D`). Upstream #6283 deleted the standalone `tauri.desktop.conf.json` and consolidated its updater config into this file; the merge-conflict surface for the updater endpoint + pubkey now lives here under `plugins.updater`. Only the version number changes from upstream's release-tag bumps. |
| `crates/librefang-desktop/tauri.{ios,android}.conf.json` | **Always take ours** — `identifier: "ai.bossfang.app"`. Mobile bundles use the `.app` suffix (matches Apple/Play conventions). |
| `crates/librefang-desktop/icons/*` | **Always take ours** — binary icon assets are BossFang-branded; never overwrite from upstream. Regenerate from `docs/branding/boss-libre.png` via `cargo tauri icon docs/branding/boss-libre.png` if a refresh is needed. |

### What the enforcement script does NOT fix

- Newly added `.tsx` components that render an **SVG fang glyph** inside a sky-blue gradient
  box as the logo — those must be manually replaced with `<img src="/boss-libre.png" alt="BossFang" ...>`.
- The `card-glow` / `glow-text` CSS helpers if upstream rewrites them with
  hardcoded sky-blue rgba — check `index.css` after running the script.
- Tauri `productName` / `identifier` string flips in any new `tauri.*.conf.json`
  upstream introduces — the merge-conflict rules above are the source of truth.
- New hardcoded `github.com/librefang` URLs in upstream Rust code — search with
  `grep -rn 'github.com/librefang' crates/` after merge and repoint to
  `github.com/GQAdonis` (the registry has a `[registry] base_url` knob; the
  dashboard tarball is hardcoded in `webchat.rs`).

## BossFang-Exclusive Features — Always Preserved After Upstream Merge

These features exist in BossFang but NOT in upstream LibreFang. They MUST be present
and functional after every upstream merge.

### 1. SurrealDB as the Default Storage Backend

Upstream LibreFang uses SQLite exclusively. BossFang replaces this with SurrealDB 3.x
as the default persistent storage, with SQLite retained only as a legacy opt-in feature.

**Crate**: `librefang-storage` (`crates/librefang-storage/`)
**Migration files**: `crates/librefang-storage/src/migrations/sql/*.surql` (24+ migrations)
**Feature flags**: `surreal-backend` (default), `sqlite-backend` (opt-in legacy)
**Version pin**: `surrealdb`, `surrealdb-core` **and** `surrealdb-types`, all `=4.0.0`, in workspace `Cargo.toml` — the three lines move together.
  `=` on `surrealdb` does NOT transitively constrain `surrealdb-core` or `surrealdb-types`, so `librefang-storage` lists both as direct optional deps to make the pins load-bearing.
  Coordinate with surreal-memory before changing.

**After every upstream merge:**
1. Check if upstream added new SQLite `CREATE TABLE` or `ALTER TABLE` statements
2. For each, create a matching SurrealDB migration file:
   `crates/librefang-storage/src/migrations/sql/NNN_<name>.surql`
3. Register the new migration in `crates/librefang-storage/src/migrations/mod.rs`
4. If upstream switches a storage call from `Arc<Mutex<Connection>>` to r2d2 `Pool`,
   update BossFang's SurrealDB dual-path code to use `pool.get()` in the SQLite fallback

### 2. surreal-memory as the Memory Substrate

Upstream LibreFang uses its own SQLite-backed memory layer. BossFang uses the
`surreal-memory` crate (Prometheus-AGS/surreal-memory-server) which provides a
SurrealDB-native memory substrate with semantic search, knowledge graphs, and task tracking.

**Crate**: `librefang-memory` (`crates/librefang-memory/`)
**Backends**: `crates/librefang-memory/src/backends/surreal*.rs` (9 backend files)
**Dependency**: `surreal-memory = { git = "https://github.com/Prometheus-AGS/surreal-memory-server.git", rev = "c220263e0a5a1fb520f22acc0d36fb806c03c51b", default-features = false, features = ["embedded"] }` — pinned by `rev`; keep the URL/rev form byte-identical (see the comment in `Cargo.toml`).

**After every upstream merge:**
1. If upstream changes the `librefang-memory` API surface (e.g., `Arc<Mutex<Connection>>`
   → r2d2 `Pool`), update BossFang's surreal backend implementations to match
2. Never remove the `surreal-backend` feature from `librefang-memory/Cargo.toml`
3. The `embedded` feature on surreal-memory must remain — no external SurrealDB service needed
4. surreal-memory is pinned by `rev`, not a branch, so a bump is a deliberate edit of the workspace `Cargo.toml` followed by `cargo update -p surreal-memory`.
   Its `surrealdb` pin is exact, so a new rev usually moves our `surrealdb` / `surrealdb-core` / `surrealdb-types` pins with it (see Shared SurrealDB Version Pin).
   Since rev `b7e2093` it retries SurrealDB 3.3 `TransactionConflict` errors on every write and shares one `Surreal<Any>` session rather than cloning per call (in SDK 3.x a clone is a new server-side session).
   Two operational env vars surface from its internals: `SURREAL_QUERY_TIMEOUT_MS` (per-query deadline, default 10000 ms) and `SURREAL_EMBEDDED_MAX_INFLIGHT` (concurrent embedded ops, default 16 = RocksDB default stripe count).

### 3. Universal Agent Runtime (UAR) as a Runtime Provider

BossFang borrows an authenticated configured Universal Agent Runtime (Prometheus-AGS/universal-agent-runtime) through remote model/runtime transport.
BossFang never bundles, installs, spawns, stops or supervises UAR.
The Boss owns its managed UAR lifecycle independently; external UAR remains externally owned.
New admissions use the selected connection; admitted runs retain their original instance, workspace, endpoints and runtime epoch without migration or replay.

**Crates**:
- `librefang-uar-spec` (`crates/librefang-uar-spec/`) — spec types and AgentManifest translator
- `librefang-llm-drivers/src/drivers/uar.rs` — `UarDriver` LLM driver implementation

**Feature flags**:
`uar-driver` is **off by default and opt-in**; preserve the existing compatibility feature chain:

- `librefang-cli`: `uar-driver` → `librefang-api/uar-driver`
- `librefang-api`: `uar-driver` → `librefang-kernel/uar-driver`
- `librefang-kernel`: `uar-driver` → `librefang-runtime/uar-driver`
- `librefang-runtime`: `uar-driver` → `librefang-llm-drivers/uar-driver`
- `librefang-llm-drivers`: `uar-driver` is an empty compatibility feature.
- No link enables `dep:universal-agent-runtime` or forwards `surreal-backend` to UAR; UAR owns its dependencies and storage independently.

**Every crate in that chain must declare the feature.** `--features` names are resolved
against the *selected package*, so the production image's
`cargo build --bin librefang --features telemetry,surreal-backend,uar-driver`
(`Dockerfile`) resolves `uar-driver` against **`librefang-cli`**. If any link is missing, the
image build fails with *"none of the selected packages contains these features"*.

Historical in-process integration (fixed 2026-07-11, #C-001; not the current connection-only architecture):
`librefang-kernel/Cargo.toml` carried a hardcoded
`librefang-runtime = { …, features = ["uar-driver"] }`, and since cargo unions features
across the graph and `librefang-cli → librefang-kernel` is not feature-gated, **every** build
compiled UAR in with no way to turn it off. That single line:

- pulled UAR's entire transitive tree (`liter-llm`, `burn`, `tonic`, `mimalloc`) plus a
  `build.rs` that fetches `models.dev` over the network and builds a pnpm frontend, into
  every build; and
- imposed UAR's exact `surrealdb = "=3.2.1"` pin on the whole workspace. Cargo cannot unify
  two exact `=` pins, so a UAR-side surrealdb bump broke **every** build rather than an
  opt-in feature.

It also masked a latent bug: `librefang-cli` never declared `uar-driver` at all, so the
Dockerfile's `--features …,uar-driver` only "worked" because the kernel was force-enabling
it behind cargo's back. Never re-add a hardcoded `features = […]` to a path dependency to
paper over a missing feature declaration — declare the feature and forward it.

**After every upstream merge:**
1. If upstream changes the `LlmDriver` trait signature, update `UarDriver` in
   `crates/librefang-llm-drivers/src/drivers/uar.rs` to match
2. If upstream changes `AgentManifest` shape, update `librefang-uar-spec/src/types.rs`
3. Never remove `librefang-uar-spec` from the workspace
4. Ensure `provision_uar_namespace()` in `librefang-storage` is still called at boot
5. Preserve selected-instance borrowing and original admitted-run authority; never reintroduce UAR bundling, installation, spawning, stopping or supervision.

### Shared SurrealDB Version Pin

librefang-storage and surreal-memory link the same in-process surrealdb client.
The workspace `Cargo.toml` pins:
```toml
surrealdb = { version = "=4.0.0", default-features = false, features = ["kv-surrealkv", "protocol-ws", "protocol-http"] }
surrealdb-core = { version = "=4.0.0", default-features = false }
surrealdb-types = { version = "=4.0.0", default-features = false }
```
Upgrade the three lines together, and check surreal-memory first.
Version drift causes duplicate dep link errors that break the entire build.
Verify with `cargo tree -i surrealdb`, `-i surrealdb-core`, `-i surrealdb-types` and `-i surreal-memory`: each must show exactly one version.

What each system demands, as of the 4.0.0 bump:

- `surreal-memory` (rev `c220263`) pins an **exact** `surrealdb = "=4.0.0"` and `surrealdb-types = "=4.0.0"` — rigid, so its rev and our pin must move in step.
- `universal-agent-runtime` is not linked in-process: the terminal `uar-driver` feature is empty and BossFang connects to an independently owned UAR service, so UAR dependency/storage pins do not bind this workspace.
- The remote SurrealDB server (`k8s/base/surrealdb-statefulset.yaml`) runs `surrealdb/surrealdb:v4.0.0`, the same minor as the client.
- SurrealDB 4.0 removed RocksDB: the embedded engine is SurrealKV (`kv-surrealkv`, `surrealkv://` URLs). A pre-4.0 RocksDB datastore is migrated in place, one way, the first time a 4.0 client opens it; take a copy of the embedded `librefang.surreal` / `librefang-memory.surreal` directories under the configured storage data dir before the first 4.0 boot if a rollback to 3.x must stay possible.

### 4. Env-var aliases (BOSSFANG_* preferred, LIBREFANG_* fallback)

BossFang adds higher-priority `BOSSFANG_*` aliases for two env vars and leaves the
matching `LIBREFANG_*` names as fallbacks. The deliberate non-goal: we DO NOT alias
every `LIBREFANG_*` env var the codebase reads (70+ of them). The merge cost of
aliasing everything outweighs the benefit, and most of those names are operational
plumbing nobody types into a shell.

**Aliased today:**

| Primary (new) | Fallback (legacy) | Resolution helper |
|---|---|---|
| `BOSSFANG_HOME` | `LIBREFANG_HOME` | `librefang_home()` in `crates/librefang-kernel/src/config.rs:431` |
| `BOSSFANG_VAULT_KEY` | `LIBREFANG_VAULT_KEY` | `vault_key_from_env()` in `crates/librefang-extensions/src/vault.rs` + `vault_key_env()` in `crates/librefang-cli/src/doctor.rs:148` |

**Resolution order** for both: `BOSSFANG_*` wins if set, `LIBREFANG_*` wins if only the
legacy is set, otherwise the compiled default applies. Both are checked at the same
call site (no scattered direct reads).

**Default home directory stays `~/.librefang/`** so existing installations upgrade
in-place with zero data migration. Renaming the default to `~/.bossfang/` would
orphan every existing user's config / vault / registry-cache and is deliberately
deferred. Power users on a fresh install who want the BossFang-flavoured path can
set `BOSSFANG_HOME=~/.bossfang` and pay the migration cost themselves.

**Function names stay `librefang_home`, `VAULT_KEY_ENV_LEGACY`, etc.** These are
Layer-Internal Rust identifiers and renaming them explodes upstream-merge conflict
surface (see the Phase-2 three-layer principle elsewhere in this file).

**To upstream this back to LibreFang**: drop the `BOSSFANG_*` branches from both
helpers; the function signatures don't change.

## Git Conventions

- **Format**: conventional commits (`feat:`, `fix:`, `docs:`, `refactor:`, `chore:`, `ci:`, `perf:`, `test:`).
- **No AI / Claude attribution** in commit messages, PR bodies, or comments — the `commit-msg` hook enforces it, and so does the PreToolUse Bash hook.
- **Worktree**: `git worktree add` on an external disk for new features, falling back to `/tmp/librefang-<feature>`. Never develop on the main worktree.
- **Worktree continuation = drive to PR.** When continuing half-done work (uncommitted changes or unmerged commits), the workflow is **commit → push → open or update PR**; don't stop at "local commits only".
  Everything left in the worktree counts as real work, including a regenerated `Cargo.lock` after a rebase — commit it, don't `git checkout` it away.
- **Re-entering a worktree you did not just create → `git fetch` and diff `HEAD..origin/<branch>` BEFORE editing.**
  Collaborator pushes, the `auto-update-branches` cron, and the `openapi-drift` auto-codegen commit all move a remote tip without your involvement.
  Editing on a stale base and force-pushing silently clobbers that newer work; a non-fast-forward rejection is the last line of defense, not the plan.

### Changelog: a new file under `changelog.d/`, NOT an edit to `CHANGELOG.md`

Appending to the single `## [Unreleased]` section conflicts with every other open PR doing the same, in a file where the conflict carries no information — both sides are correct and the resolution is always "keep both".

Write one fragment per entry in the section directory matching its `### ` heading (`added/`, `fixed/`, `changed/`, `security/`, `documentation/`), named after the PR or issue number so fragments sort usefully: `changelog.d/fixed/6623-wire-max-content-chars.md`.
The file holds the bullet body **without** the leading `- `, one sentence per line, continuation lines indented two spaces, ending `(#PR) (@your-github-login)`.
A fragment in an unrecognised section directory is rejected by `scripts/check-changelog-attribution.py`, because assembly has no heading to render it under and would drop it silently.
Format and a worked example: `changelog.d/README.md`.

**The entry ends up in the GitHub release body verbatim**, so write prose that explains *why*, not a restatement of the PR title.
`cargo xtask collect-fragments` folds the fragments into `## [Unreleased]` and deletes the files consumed; `cargo xtask release` runs it before cutting the dated section, and `.github/workflows/release.yml` slices that section into the release notes, Dev.to article and social post.
Generated `- <PR title> (#N) (@author)` lines fill only the gaps: a PR whose number appears in a curated bullet's trailing `(#N)` group gets no generated line, so it is described once, in the words someone chose.

## Prose wrapping: no column limit; break only at sentence boundaries

Do not hard-wrap at 72, 80, 100, or any other character count.
The only legitimate line break inside a paragraph is at a complete sentence boundary — one sentence, one line, regardless of length.
Hand-tuned column wraps split noun phrases at awkward points, force a single-word edit to re-flow the whole paragraph (polluting `git blame` and review diffs), and carry no semantic information.

This applies to markdown anywhere in the repo, `CHANGELOG.md` bullets, PR titles / bodies / comments posted via `gh`, source doc-comments (`//!`, `///`, JSDoc, …) and any multi-line prose comment block, and commit message bodies.
Commit *subject* lines still follow git's ~72-char display convention — that is a tooling limit, not prose wrapping.

The rule applies to new prose and to any paragraph you are already touching.
Files written under the old convention are not retroactively rewrapped; don't rewrap an untouched paragraph just to enforce this.

## GitHub Collaboration & Wait Policy

Full policy, with the incidents that produced each rule: `docs/development/github-collaboration.md`.
The rules you must not break without asking:

- **Don't close, reassign, re-label or re-milestone** PRs / issues opened by others unless the maintainer instructs you to.
  Recommend closure in a comment instead; when directed to close, state the substantive reason (review bugs, superseded by, scope mismatch).
- **Force-push only to your own branches, only before review.** After a reviewer has loaded the diff, prefer fixup commits or a follow-up PR.
- **One PR ↔ one issue** (or one tight cluster). Don't bundle unrelated refactors.
- **Fix what you found — don't punt it to a "follow-up".**
  Anything you noticed while reading or writing the code in this PR is in-scope by definition: review nits, wrong HTTP status codes, missing log fields, redundant lookups, stale comments, small clippy noise.
  "Follow-up", "next PR", "future cleanup", "leave for later" are red flags in your own output — they almost always mean "I saw the problem and decided to defer".
  Bar to defer: would fixing it require touching a *different* crate or domain than the one you're already in? If no, fix it now; if yes, ask the human with the concrete trade-off rather than deciding unilaterally.
  Re-classifying a deferred item as a "non-issue" requires file/line evidence in the same response — "I looked again and it's fine" is another form of punting.
- **PR body must enumerate** substantive changes, verification performed (integration test names, `cargo check --workspace --lib`, scoped `cargo test -p <crate>`), and any deferred work. Bullet form, no marketing prose.
- **CI polling budget: ~5 minutes total, in 60–270 s chunks** (Anthropic's prompt cache TTL is 5 min — keep each wake-up inside it).
  Then push, leave the run URL, and **stop**.
  Don't pre-emptively re-run a check that hasn't failed; retry once, after a recorded failure.
  Don't open follow-up issues or pivot the plan while waiting — report status and yield.
  Don't add reviewers, flip `ready-for-review`, or `gh pr ready` on someone else's behalf.
- **At most two follow-up comments** on a thread without human input, then stop. No "looks good" drive-bys. Every reply links evidence: commit SHAs, file paths, test names.
- **Latest maintainer intent wins** in conflict resolution, and preserve both sides' intent — dropping a hunk because "it'll be reapplied later" is how regressions land.
- **Batch merging is runner-pool bound, not merge bound.** Merging >10 PRs back-to-back saturates the free-plan `ubuntu-latest` pool; merge in batches, cancel *superseded* runs (never a run whose `head_sha` **is** its branch tip — `CI Gate` fails on `cancelled` and does not re-evaluate), and remember a stalled queue is not a CI failure.
  What saturates the pool is not the merges, it is the **housekeeping fan-out per merge**.
  Every push to `main` fires `TODO to Issue`; every PR close fires `Contributor Role` and `Issue-PR Link Labels`; every completed `Release` run fires `Release / Notify`.
  At a normal merge rate that is invisible.
  At 250 merges it produced ~750 queued runs sitting ahead of the 196 queued `CI` runs, and nothing executed for over three hours.
  The recovery is to cancel the housekeeping runs, which is safe: the `main` ruleset requires only `CI Gate`, GitHub auto-merge waits only on required checks, and `CI Gate`'s `cancelled` test covers only the 22 jobs in its own `needs` list — all inside `ci.yml`.
  Never cancel `secrets` to buy queue capacity; a security scan is not the thing to trade away for speed.
- **`gh run list --limit N` silently truncates, and the queue is usually deeper than it shows.**
  Three separate diagnoses in one incident were wrong because `--limit 100` returned exactly 100 and that was read as the total.
  `gh api "repos/OWNER/REPO/actions/runs?status=queued&per_page=1" -q '.total_count'` gives the real number; the paginated listing itself stops at 1000 results, so a queue deeper than that has to be cleared and re-enumerated in rounds.
- **Deciding whether CI is alive has exactly one honest test, and every shortcut lies.**
  A workflow file GitHub cannot parse produces a run that completes as `failure` within seconds *without ever taking a runner*, so a queue full of those looks identical to throughput.
  `gh run list --status in_progress` counts **runs**, not jobs, and those startup failures flicker through `in_progress` on their way to `completed` — so a poll on "in_progress != 0" reports a live pool against a dead one.
  The only condition that means anything: **a run whose workflow is not one of the known startup-failing ones reached `success` or `failure`, with a timestamp later than a baseline you recorded before you started watching.**
  Three separate monitors in one session got this wrong three different ways — polling `in_progress`, comparing a timestamp against a `jq` expression that yields `null` on an empty array (`null != baseline`, so it fires forever), and counting matches with a `select(...)` whose `test(...)|not` did not bind to the field intended.
  Write the check to produce the *timestamped run itself*, print it, and read it — never a derived boolean.
- **A suspiciously fast `cargo check` is not evidence of anything until you prove the compiler saw your tree.**
  In a shared `CARGO_TARGET_DIR` a check can print `Finished in 0.35s` with no `Checking` lines because the artifacts are already there.
  `touch`ing sources often does not force a rebuild.
  Appending `compile_error!("SENTINEL");` to the crate's `lib.rs` and confirming cargo reports it takes one command and is conclusive; do that before trusting a green that arrived too quickly, and before reporting one.
- **Two green PRs can still break `main` together.** The `main` ruleset has no `strict_required_status_checks_policy`, so each PR merges on CI run against its own base. Group a merge sweep by changed file, re-run CI on later PRs in a group, and verify `main` itself after the batch.

## Common Gotchas

- Windows: `librefang.exe` may be locked while the daemon runs — use `cargo check --lib` or kill the daemon first. (Linux / macOS let you overwrite a running binary.)
- Windows: use `taskkill //PID <pid> //F` (double slashes in MSYS2 / Git Bash).
- `PeerRegistry` is `Option<PeerRegistry>` on the kernel but `Option<Arc<PeerRegistry>>` on `AppState` — wrap with `.as_ref().map(|r| Arc::new(r.clone()))`.
- A new `KernelConfig` field MUST also appear in the `Default` impl or the build fails.
- `AgentLoopResult`'s field is `.response`, not `.response_text`.
- The CLI subcommand to start the daemon is `start`, not `daemon`.
- `Option<Arc<dyn Trait>>` fields on structs deriving `Serialize`/`Deserialize`/`Clone`/`Debug` need `#[serde(skip)]` plus manual impls of the affected traits.
- `ErrorTranslator` (from `RequestLanguage`) is `!Send` — every `.await` must happen AFTER `drop(t)`, or the axum handler fails with a cryptic `Handler<_, _>` trait bound error.
- `LIBREFANG_VAULT_KEY` must base64-decode to exactly 32 **bytes** (`openssl rand -base64 32` gives 44 chars). 32 ASCII chars ≠ 32 bytes.
- Linux desktop: `zbus/tokio` conflicts with Tauri's worker threads — a blocking session-bus connection (tauri-plugin-notification / notify-rust) inside a Tokio worker panics with "Cannot start a runtime from within a runtime."
  Force the `async-io` backend of the zbus / ksni crate so it runs on a separate reactor.
- `CLAUDE_CODE_HOME` overrides the home directory the `claude-code` driver hands to the spawned Anthropic `claude` CLI.
  **This is a LibreFang-private contract — the Anthropic CLI does not read this variable**; the driver resolves it kernel-side and projects it onto the platform-native home var (`$HOME` / `%USERPROFILE%`) before spawn.
  Distinct from `LIBREFANG_HOME`, which relocates the daemon's data dir (`crates/librefang-kernel/src/config.rs: librefang_home`); the two never share a value.
  It exists for containers that drop to a numeric uid without a passwd entry and inherit a placeholder home (`/nonexistent`, `/var/empty`, `/dev/null`), leaving the CLI unable to find `~/.claude/.credentials.json`.
  The override is ignored when the inherited home is already a real directory, and when it points at a non-directory the driver logs a `WARN` and falls back rather than honouring it.
- When parallel agents modify the same crate, `Option::None` defaults for new fields compile silently but disable the feature. Always write the integration test at the injection site, not just the implementation site.

### Agent Fabric Convergence phase gate

For `agent-fabric-convergence` and its `release-cadence-optimization` child, follow the approved [delivery contract](docs/plans/agent-fabric-convergence/.kbd-orchestrator/phases/agent-fabric-convergence/children/release-cadence-optimization/delivery-contract.md): finish the complete production increment, then run one real integration gate and the required local package build. Test-first, per-edit, per-route and scoped cargo-check/test examples elsewhere in this file or role instructions are deferred to that completed-phase boundary for this initiative. The single shared heavy build writer and KBD lead coordinate the boundary; observer roles cannot authorize it. This scoped precedence leaves other projects and phases unchanged.

<!-- prometheus-team-routing:start v1 -->
For every code task, read `.agent-team/project-routing.json`, then its active team manifest and the relevant role instructions. Default to that team, selecting only roles whose responsibilities and ownership match the work. Preserve native permissions, models, concurrency limits and existing project instructions.
For UI work, load the role-bound `prometheus-ui-ux` or `prometheus-ui-review` skill. Prefer `.agents/UI_UX_PROTOCOL.md` when present; otherwise use the installed `prometheus-ui-ux/references/UI_UX_PROTOCOL.md`. Backend work must not load UI guidance.
Use native delegation when available. If unavailable, follow the selected role instructions sequentially and report that limitation. Review in the builder context is not independent review. Keep reviewers dormant until the complete implementation phase; allow one batched correction/confirmation cycle. Respect user-only skill invocation restrictions. Zed external ACP agents use their own native configuration; parallel UI threads are not an automatic delegation API.
<!-- prometheus-team-routing:end -->
