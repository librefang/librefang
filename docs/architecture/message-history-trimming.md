# Message-history trimming

LibreFang trims an agent's stored conversation history at every turn so it
doesn't grow without bound. This document covers what gets trimmed, how
the safe-trim heuristic works, and how to configure the cap.

## Why we trim

Three pressures push for an upper bound on stored messages:

1. **Per-turn cost.** The full history is sent to the LLM on every turn.
   Without a cap, cost grows linearly with conversation length.
2. **Prompt-cache alignment.** Anthropic's prompt cache hits when the
   prefix is byte-identical across turns. Trimming at stable
   conversation-turn boundaries (rather than mid-tool-call) keeps the
   cache aligned.
3. **On-disk session blob growth.** Sessions are persisted in SQLite
   under `~/.librefang/data/`. An unbounded history would eventually OOM
   on reload.

## What gets trimmed

`safe_trim_messages` (in `crates/librefang-runtime/src/agent_loop.rs`)
operates on two slices on every turn:

- **`session.messages`** — the canonical persisted history. Trimmed
  *first* so the truncated form is what gets written to disk on the
  next `save_session_async`.
- **`messages`** — the per-turn LLM working copy, derived from
  `session.messages` minus system messages and plus any injected
  context (memory, canonical context). Trimmed second.

Both share the same cap.

## How safe-trim works

For each slice that exceeds the cap, the function:

1. Computes `desired = len - cap`.
2. Calls `session_repair::find_safe_trim_point(messages, desired)` to
   find a cut point at a turn boundary — never inside a
   `ToolUse`/`ToolResult` pair.
3. Drains the front of the slice up to the safe cut point.
4. Re-runs `validate_and_repair` and `ensure_starts_with_user` so the
   surviving slice is well-formed for strict providers (e.g. Gemini
   rejects histories that start with an assistant turn).
5. If fewer than 2 messages survive, synthesizes a minimal
   `[user_message]` so the LLM request body is never empty.

## Configuration

Three tiers, resolved in order at every loop entry:

```
manifest.max_history_messages       (Some — per-agent override)
  ↓ if None
kernel_config.max_history_messages  (Some — operator/global override)
  ↓ if None
DEFAULT_MAX_HISTORY_MESSAGES = 60   (compiled-in fallback)
```

Resolution lives in `resolve_max_history(&manifest, &opts)` inside
`agent_loop.rs`. Values below `MIN_HISTORY_MESSAGES = 4` are silently
clamped up with a `warn!` log carrying `agent`, `requested`, `applied`.
Justification: a single tool-use round-trip is 4 messages
(user → assistant tool_use → tool_result → assistant text); caps below
4 defeat the safe-trim heuristic.

### Global override

In `~/.librefang/config.toml`:

```toml
# Lower the default for all agents that don't override it themselves.
max_history_messages = 20
```

### Per-agent override

In any `agent.toml` (top-level field, NOT inside `[model]` or
`[autonomous]`):

```toml
[agent]
name = "fast-loop"
max_history_messages = 12
```

### Where the value flows through the runtime

- `KernelConfig.max_history_messages` (`librefang-types/src/config/types.rs`)
- → kernel populates `LoopOptions.max_history_messages` at every
  `run_agent_loop` / `run_agent_loop_streaming` entry
  (`librefang-kernel/src/kernel/mod.rs`).
- → `resolve_max_history` consults `manifest.max_history_messages`
  first, falls back to `LoopOptions`, finally to the constant.
- → resolved cap is passed into `prepare_llm_messages` and onward into
  `safe_trim_messages` as a function argument (no globals).

## Interaction with token-based context-window trimming

`DEFAULT_MAX_HISTORY_MESSAGES` is a *message-count* cap. There is also a
*token-count* cap (`DEFAULT_CONTEXT_WINDOW = 200_000` tokens) that the
runtime applies independently when the per-turn payload is too large.
The two caps are not combined — whichever fires first wins:

- **Many short messages** (cron pings, simple chat) — the message-count
  cap fires first.
- **Few long messages** (large tool outputs, long user prompts) — the
  token-count cap fires first.

If you only need to bound LLM cost, the message-count cap is the
simpler dial. The token cap is currently global and not per-agent.

## Cross-references

- Constant + helpers: `crates/librefang-runtime/src/agent_loop.rs`
  (`DEFAULT_MAX_HISTORY_MESSAGES`, `MIN_HISTORY_MESSAGES`,
  `resolve_max_history`, `clamp_max_history`, `safe_trim_messages`,
  `prepare_llm_messages`)
- Kernel wiring: `crates/librefang-kernel/src/kernel/mod.rs` — search
  for `max_history_messages` to find the four `LoopOptions`
  construction sites.
- Type definitions:
  - `KernelConfig.max_history_messages` in
    `crates/librefang-types/src/config/types.rs`.
  - `AgentManifest.max_history_messages` in
    `crates/librefang-types/src/agent.rs`.
- Repair primitives: `crates/librefang-runtime/src/session_repair.rs`
  (`find_safe_trim_point`, `validate_and_repair`,
  `ensure_starts_with_user`).
