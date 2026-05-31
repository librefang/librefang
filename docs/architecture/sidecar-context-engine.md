# Sidecar context engine

The context engine decides what the LLM sees each turn: what to recall, how to trim and order the window, and what to do after a turn.
That **policy** is high-churn and a natural fit for a hot-swappable, out-of-process implementation.
The **mechanism** it relies on — the LLM driver and token streaming used by compaction — is substrate that stays in Rust.

`engine = "sidecar"` runs the policy hooks in a subprocess (any language) and keeps the rest in the built-in engine.

## What crosses the boundary

The [`ContextEngine`](../../crates/librefang-runtime/src/context_engine.rs) trait has more methods than the sidecar bridges.
Only the async, non-LLM lifecycle hooks are delegated; everything else stays in the wrapped built-in engine, which is also the fallback for every bridged call.

| Method | Where it runs | Why |
| --- | --- | --- |
| `ingest` | sidecar | memory recall policy — pure transformation |
| `assemble` | sidecar | window trim/reorder — pure transformation |
| `after_turn` | sidecar | post-turn bookkeeping |
| `bootstrap` | both | inner is bootstrapped (it owns the memory substrate and is the fallback); the sidecar gets a best-effort notification |
| `compact` | **inner (Rust)** | takes `Arc<dyn LlmDriver>` — an LLM handle cannot cross a process boundary |
| `truncate_tool_result`, `should_compress`, `update_model`, metrics | **inner (Rust)** | synchronous and cheap; not worth an IPC round-trip |

Calls happen roughly once per turn, so the round-trip cost is acceptable; per-token streaming never crosses the boundary.

## Robustness: never break a turn

The context engine is on the per-turn critical path, so a flaky sidecar must not break a turn.
**Every bridged call falls back to the built-in engine on any failure** — spawn failure, write error, request timeout, malformed reply, or a crashed process.
A crash degrades that one call to the built-in engine; the sidecar is then re-spawned lazily on a later call (rate-limited by a cooldown so a persistently-broken command can't spawn-storm) via `librefang_subprocess::SupervisedTransport`.

## Wire protocol

Newline-delimited JSON, request/reply, over the subprocess's stdio.

- **Daemon → sidecar (stdin)**, one object per line: `{"id": <u64>, "method": "<name>", "params": {…}}`
- **Sidecar → daemon (stdout)**, one object per line: `{"id": <u64>, "ok": {…}}` or `{"id": <u64>, "error": "<msg>"}`
- **stderr** is free-form and forwarded to the daemon log.

Requests carry monotonically increasing ids; replies are matched by id, so a sidecar may reply out of order.

### Methods

`ingest`
- params: `{ "agent_id": "<uuid>", "user_message": "<text>", "peer_id": "<id>" | null }`
- ok: `{ "recalled_memories": [ <MemoryFragment>, … ] }`

`assemble`
- params: `{ "agent_id", "messages": [<Message>], "system_prompt", "tools": [<ToolDefinition>], "context_window_tokens" }`
- ok: `{ "messages": [<Message>], "recovery": <RecoveryStage> }` — the returned `messages` array replaces the window verbatim.
- `recovery` is one of `"None"`, `{"AutoCompaction": {"removed": N}}`, `{"OverflowCompaction": {"removed": N}}`, `{"ToolResultTruncation": {"truncated": N}}`, `"FinalError"`.

`after_turn`
- params: `{ "agent_id", "messages": [<Message>] }`
- ok: `{}` (ignored)

`bootstrap`
- params: `{ "context_window_tokens", "max_recall_results", "stable_prefix_mode" }`
- ok: `{}` (ignored)

`Message`, `ToolDefinition`, and `MemoryFragment` are serialized with their `librefang-types` serde representations; a passthrough sidecar can treat them as opaque JSON.

## Configuration

```toml
[context_engine]
engine = "sidecar"

[context_engine.sidecar]
command = "python3"
args = ["/home/me/.librefang/context_engines/recall.py"]
request_timeout_secs = 30   # 0 → default 30s; a slower call falls back for that turn
```

Unlike third-party skills, the sidecar command is operator-supplied trusted configuration, so its environment is inherited (not cleared).

## Reference implementation

A dependency-free Python reference that recalls nothing and keeps the most recent slice of the window is in [`docs/examples/context_engine_sidecar.py`](../examples/context_engine_sidecar.py).
Note the two stdio pitfalls it avoids: read with `sys.stdin.readline()` (not `for line in sys.stdin`, which read-ahead-buffers) and `sys.stdout.flush()` after every reply (a long-lived process's block-buffered stdout would otherwise never reach the daemon).

## Known limitations (v1)

- **Lazy respawn, not active supervision.** A crashed sidecar is re-spawned on the next call (after a cooldown), not eagerly with a ready-handshake/backoff loop like the channels adapter. The exit is logged at WARN and counted (`subprocess_transport_exited{label="context_engine"}`) so it is visible rather than silent. Calls during a down window fall back to the built-in engine.
- **Per-turn serialization cost.** `assemble` ships the whole message window (and tool definitions) to the sidecar and reads a rewritten window back, each turn. This is the price of letting an external process decide the window; for very large histories it is a real per-turn cost. A delta/keep-drop protocol is a possible future optimization.
- **Reply-line size cap.** The reader caps a single newline-delimited reply at 16 MiB; a buggy sidecar that streams without ever emitting `\n` trips the cap, the transport is dropped, and calls fall back to the built-in engine rather than growing memory without bound.
- **Deterministic ordering is the sidecar's responsibility.** The sidecar fully controls the order of the `assemble` window and the `ingest` memories it returns, both of which reach the LLM prompt. The built-in engine re-runs `validate_and_repair` on the sidecar's window (so a malformed `tool_use`/`tool_result` pairing or a missing leading-user turn can never reach the provider), but it does **not** re-sort otherwise-valid content. A sidecar that emits non-deterministic ordering across identical inputs will silently invalidate provider prompt caches (refs #3298) — emit a stable order.

## What stays in Rust (the substrate line)

- The LLM driver, streaming, and compaction (`compact`).
- The agent-loop state machine, session lifecycle, and the per-turn trigger for compaction.
- Prompt-cache-determinism (#3298): the built-in engine still owns final ordering.
- Capability enforcement, taint tracking, and the sandboxes — the sidecar is trusted operator config, but it has no privileged host channel; it only transforms the JSON it is handed.
