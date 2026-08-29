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
- Force-push to `main` / `master`; `--no-verify` / `--no-gpg-sign` on any git command.
- Staging sensitive files (`.env*`, `*.pem`, `id_rsa`, `credentials*`, …) and broad `git add -A` / `git add .` — stage specific paths.
- Claude / Anthropic attribution in a commit message, **or** a commit author identity that resolves to Claude / Anthropic.
- `rm -rf` against dangerous targets (`/`, `~`, `$HOME`, `target`, `.git`, `/usr`, `/etc`, …).
- Launching the daemon (`librefang start`, `target/*/librefang start|daemon`) — port 4545 belongs to the user's session, and live testing is human-only.
- A changelog fragment in an unrecognised `changelog.d/` section directory, or an `[Unreleased]` addition missing `(@user)` attribution.

**Enable the git-side hooks once per clone**: `just setup` (or `cargo xtask setup`), which sets `git config core.hooksPath scripts/hooks`.

## Project Overview

LibreFang is an open-source Agent Operating System written in Rust (29 crates in `crates/`, plus `xtask/`).

- Config: `~/.librefang/config.toml`
- Default API: `http://127.0.0.1:4545`
- CLI binary: `target/release/librefang`, `.exe` suffix on Windows (debug builds at the matching `target/debug/` path)

### Crate map

- **Core types & utilities**: `librefang-types`, `librefang-http`, `librefang-wire`, `librefang-telemetry`, `librefang-testing`, `librefang-import`, `librefang-subprocess` (JSON-over-stdio transport for sidecar bridges)
- **Kernel**: `librefang-kernel` (orchestration), `librefang-kernel-handle` (trait used by runtime to call kernel without circular dep), `librefang-kernel-router`, `librefang-kernel-metering`
- **Runtime**: `librefang-runtime` (agent loop, tools, plugins, OAuth, WASM sandbox), `librefang-runtime-mcp`, `librefang-runtime-audit`, `librefang-runtime-media`, `librefang-runtime-sandbox-docker`
- **LLM drivers**: `librefang-llm-driver` (trait + error types — interface only) and `librefang-llm-drivers` (concrete provider impls: anthropic, openai, gemini, …)
- **Memory**: `librefang-memory` (SQLite substrate), `librefang-memory-wiki` (durable markdown knowledge vault)
- **Surface**: `librefang-api` (HTTP server + dashboard SPA at `crates/librefang-api/dashboard/`), `librefang-cli`, `librefang-desktop`, `librefang-acp` (ACP adapter for Zed / VSCode / JetBrains)
- **Extensibility**: `librefang-skills`, `librefang-hands`, `librefang-extensions`, `librefang-channels`, `librefang-rl-export`

## Build & Verify Workflow

**Do NOT run `cargo build` or `cargo run` locally.**
**`cargo test` is allowed only when scoped with `-p <crate>` / `--package <crate>`** — the unscoped workspace-wide form contends with the user's other sessions on the shared `target/` directory.
Full workspace build / test runs in CI.

After every change:

```bash
cargo check --workspace --lib                          # Compile-check only
cargo clippy --workspace --all-targets -- -D warnings  # Zero warnings
cargo test -p <crate>                                  # Only when verifying behavior in one crate

# Quick unit-only lane, mirrors CI's Test / Unit (lib+bin):
cargo nextest run --workspace -E 'kind(lib) | kind(bin)' --no-fail-fast
```

`docs/development/build-and-verify.md` covers the rest: the two CI test lanes and why the nextest filter expression is used instead of `--lib --bins`, the `librefang-desktop` Windows exclusion (#6729), and how to verify **without a native toolchain** via `Dockerfile.rust-dev` and a per-worktree target volume.
Read it before declaring a change unverified on a host that has no `cargo`.

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
