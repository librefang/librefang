# librefang-kernel — AGENTS.md

Telegraph style. Short sentences. One idea per line.
See repo-root `CLAUDE.md` for the cross-cutting rules (worktree, hooks, CI wait policy, conventional commits).

## Purpose

Agent orchestration. Scheduling. Permissions. Inter-agent communication. Owns the message-handling loop that fans requests out to LLM drivers, tools, and the memory substrate.

## Boundary

- Owns: registry, scheduling, approval, auth, auto_dream, cron, event_bus, inbox, pairing, scheduler, session_lifecycle, metering, router (last two re-exported from `librefang-kernel-metering` / `librefang-kernel-router`).
- Does NOT own: agent loop body, tool dispatch, channel adapters, HTTP routing, dashboard SPA. Those live in `librefang-runtime`, `librefang-channels`, `librefang-api` respectively.
- Does NOT depend on: `librefang-api`, `librefang-extensions`. Reverse the dependency via the `KernelHandle` trait (defined in `librefang-runtime`) when runtime / extensions need a kernel callback.

## Module map

- `kernel::LibreFangKernel` — top-level orchestrator. Boot via `LibreFangKernel::boot_with_config(KernelConfig)`. Currently a god-struct (~18k LOC, 50+ fields — #3565). Don't add new fields without coordination.
- `registry::AgentRegistry` — concurrent agent table; spawn / lookup / kill.
- `kernel::cron` — cron scheduling. `session_mode` resolution lives here (per-job > manifest > historical Persistent).
- `kernel::event_bus` — broadcast event bus. History is `parking_lot::Mutex<VecDeque<Arc<Event>>>` since #3385 — do NOT switch back to `RwLock<VecDeque<Event>>`.
- `kernel::session_lifecycle` — session state machine.
- `metering` (re-exported) — token + cost accounting; uses kernel's `model_catalog`.
- `router` (re-exported) — model router, alias resolution.

## Hot fields and their lock strategy

- `model_catalog: arc_swap::ArcSwap<ModelCatalog>` — atomic-load reads (#3384). Writers go through `model_catalog_update(|cat| ...)` (RCU). Don't add `RwLock<ModelCatalog>` back.
- `skill_registry: std::sync::RwLock<SkillRegistry>` — hot-reload on install/uninstall. Reads should be brief; copy out what you need.
- `running_tasks: dashmap::DashMap<(AgentId, SessionId), RunningTask>` — keyed by `(agent, session)`, NOT by `AgentId` alone. Pre-#3172 it was the latter, which silently overwrote concurrent loops. Don't degrade.
- `mcp_oauth_provider: Arc<dyn McpOAuthProvider + Send + Sync>` — pluggable. Implemented in `librefang-api` to keep daemon free of HTTP. New OAuth flows go through this trait, not direct kernel logic.

## Determinism (refs #3298)

Anything that reaches an LLM prompt MUST be ordered before stringifying. Use `BTreeMap` / `BTreeSet`. `HashMap` iteration order varies across processes and silently invalidates provider prompt caches. Regression tests live next to each boundary — see `kernel::tests::mcp_summary_is_byte_identical_across_input_orders`.

## Configuration knobs (kernel-side)

- `KernelConfig.max_history_messages` — global default; clamped up to `MIN_HISTORY_MESSAGES = 4` with a WARN log. Per-agent override in `agent.toml`.
- `KernelConfig.queue.concurrency.trigger_lane` (default 8) — global semaphore on `Lane::Trigger`.
- `KernelConfig.queue.concurrency.default_per_agent` (default 1) — fallback when `agent.toml: max_concurrent_invocations` is unset.
- `KernelConfig.workflow_stale_timeout_minutes` — `recover_stale_running_runs` cutoff at boot.

## Adding a new field to `LibreFangKernel`

1. Field must be `pub(crate)` unless an external crate truly needs read access.
2. Add to the `Default` impl on `KernelConfig` if it has a config-side counterpart, else build is silently broken.
3. If the field is `Option<Arc<dyn Trait>>`, mark `#[serde(skip)]` and implement Serialize/Deserialize/Clone/Debug manually.
4. Decide lock strategy:
    - Hot read, rare write → `arc_swap::ArcSwap`.
    - Hot read, hot write → `parking_lot::Mutex` or `dashmap`.
    - Append-only history → `parking_lot::Mutex<VecDeque<Arc<T>>>`.

## Testing

- Most kernel-unit testing lives inside `crates/librefang-kernel/src/kernel/`. Integration tests against a real router live in `librefang-api/tests/` — that's where `#[tokio::test]` against `TestServer` belongs (refs #3721).
- Workspace-wide `cargo test` is **forbidden** (target/ contention with the user's session). Use `cargo test -p librefang-kernel`.
- `cargo build` is forbidden too. Use `cargo check --workspace --lib`. Real build runs in CI.

## Taboos

- No daemon spawning here. CLI binary owns `start`. Kernel just runs.
- No tokio `block_on` in this crate. We're inside a runtime; nest at peril.
- No direct LLM HTTP calls. Go through `librefang-runtime` drivers.
- No new `KernelHandle::*` method that returns `Result<_, String>` (#3541) — use a typed error.
- No `HashMap<K, V>` in any field that ends up in an LLM prompt. Use `BTreeMap` (#3298).
