# Network byte quota

`agent.toml: [resources] max_network_bytes_per_hour` caps how many bytes an agent may pull in from outside the host over a rolling hour.
`0` means unlimited, the same convention `max_tool_calls_per_minute` uses and the one the dashboard's `0 = unlimited` placeholder has always shown.
The compiled default is 100 MB (`ResourceQuota::default()` in `crates/librefang-types/src/agent.rs`), so an agent whose manifest says nothing about network bytes is capped at 100 MB per hour.

This is a **bandwidth** cap and nothing more.
Which hosts an agent may reach at all is a separate, independently enforced control — `capabilities.network` in the manifest, projected into `Capability::NetConnect` by `manifest_to_capabilities` and backed by the SSRF guard in `crates/librefang-runtime/src/web_fetch.rs`.
A spent byte cap refuses transfers; it never widens or narrows the set of reachable hosts, and it never stops the agent from doing local work.

## Where the bytes are counted

The only code that knows how many bytes crossed the wire is the primitive doing the reading, and those primitives sit several layers below the dispatcher that knows which agent to charge.
`crates/librefang-runtime/src/network_meter.rs` bridges the two with a task-local counter: `tool_runner::dispatch::execute_tool_raw` installs one around every tool call, the primitives call `network_meter::record` as they read, and the dispatcher hands the total to the kernel when the call returns.
Both halves — the pre-check and the report — need a kernel handle *and* an attributed caller agent, so a tool call that has neither is neither charged nor capped: the `/mcp` HTTP bridge without an `X-LibreFang-Agent-Id` header has no agent to charge, and inventing one would be worse than metering nothing.

One reporter cannot see that task-local at all.
A WASM guest runs under `tokio::task::spawn_blocking` (`WasmSandbox::execute`), and a tokio task-local is set on the polling thread only for the duration of `TaskLocalFuture::poll`, so a `network_meter::record` call made on the blocking thread is silently dropped.
`execute` therefore takes a handle to the counter with `network_meter::current()` before the hop, carries it into `GuestState`, and `host_net_fetch` re-installs it with `network_meter::scoped` around its own `block_on`.
The counter is the same `Arc` the dispatching tool call is holding, so a guest's reads are charged as they happen and are kept even if the guest then traps or blows its epoch budget.
`network_meter::tests::a_report_from_a_blocking_thread_does_not_reach_the_installing_task` pins the property that makes the hand-off necessary.

Counted:

| Path | What is counted |
| --- | --- |
| `web_fetch` (`WebFetchEngine::fetch_with_options`) | every response chunk read, including the one that trips `max_response_bytes` |
| `web_fetch` legacy fallback (`tool_runner::web_legacy`) | every response chunk read, including the one that trips the 10 MB cap |
| `web_fetch_to_file` | every downloaded chunk, including the one that trips the per-call byte cap |
| WASM `net_fetch` host call (`host_functions::read_body_capped`) | every response chunk a guest's fetch reads |
| MCP tool calls | the response payload the server returned |

Not counted, and deliberately so:

- **Headless-browser tools.** The browser fetches subresources on its own; the runtime never sees a byte count it could report honestly, and a fabricated one would be worse than none.
- **Search-provider JSON responses** (Brave, Tavily, Jina, Perplexity, SearXNG). These are parsed straight into typed structs, are bounded by `max_results`, and are kilobyte-scale. `web_search` is still refused once the cap is spent — it just does not itself move the needle.
- **Egress outside a tool call**, such as the agent loop's own link prefetch (`link_understanding`, `web_augment`) and channel bridges. No tool call is in flight, so no meter is installed and `record` is a no-op.
- **Bytes sent.** The cap is on what comes in. A large POST body counts only through whatever response it produces.
- **Ephemeral workers** (`spawn_ephemeral`). A worker registers no agent and so has no scheduler quota entry: its transfers are charged to nothing and capped by nothing. Its USD spend already bills to the parent — `ephemeral_spawn.rs` runs `check_quota` against `parent.manifest.resources` — but the byte meter has no equivalent hook, so a parent's byte cap does not follow the workers it spawns.

A cached `web_fetch` hit is not counted either, because nothing was fetched.

## Where the cap is enforced

`execute_tool_raw` (`crates/librefang-runtime/src/tool_runner/dispatch.rs`) asks `AgentControl::check_network_quota` before dispatching an **egress tool** — `web_fetch`, `web_fetch_to_file`, `web_search`, or any MCP tool — and refuses with a soft `ToolExecutionStatus::Denied` result if the rolling hour is already at or above the cap.
Soft rather than hard on purpose: a spent byte cap is a policy refusal the model can work around, and a hard failure would count toward the consecutive-hard-failure abort and tear the turn down.

The gate sits on `execute_tool_raw` rather than the outer `execute_tool` wrapper because the deferred-approval resume path (`Kernel::build_deferred_tool_exec_context`) re-enters dispatch there.
A `web_fetch` a human approves an hour after the model asked for it goes through the same check as one that never needed approval.

Two consequences worth stating plainly:

- **A WASM skill's egress counts but does not refuse.** A skill tool has an arbitrary name that the egress-tool test cannot recognise, so its transfers push the agent over the cap and the refusal then lands on the agent's next `web_fetch` or MCP call rather than on the skill itself.
- **Enforcement is post-charge.** The transfer in flight when the cap is crossed is allowed to finish and is counted; what the cap buys is the refusal of the *next* transfer. This is the same shape `max_tool_calls_per_minute` has, and it is why the comparison is `>=` rather than `>`.

## Where the bytes are kept

`AgentScheduler` (`crates/librefang-kernel/src/scheduler.rs`) holds the accounting, next to the token and tool-call windows:

- `UsageTracker::network_byte_timestamps` is a deque of `(Instant, bytes)`, and it is the only network figure kept. It evicts entries older than an hour on every push and on every read.
- Its rolling sum is what `check_network_quota` compares against the cap *and* what `UsageSnapshot::network_bytes` reports, so `GET /api/metrics` (`librefang_network_bytes`) and `GET /api/budget` show the operator the exact number a refusal was computed from. `AgentScheduler::get_usage` takes the entry mutably for that reason — summing the deque evicts from it.

There is deliberately no second counter on the tumbling hourly window that the token and tool-call figures use.
A tumbling window is not a cap of N bytes per hour by any reading an operator would recognise — an agent that spends its whole cap in the last second of one window may spend it again in the first second of the next — and reporting one alongside a rolling gate is worse still: it reads near zero for up to an hour after a flip while every `web_fetch` is being refused, which is the "the surface says one thing, the code does another" failure this whole document exists to close.
`reset_if_expired` therefore leaves the deque to its own eviction; an explicit `reset_usage` (session reset, operator action) clears it.

## Configuring it

```toml
# agent.toml
[resources]
# 1 MB per rolling hour — a containment posture for an untrusted agent.
max_network_bytes_per_hour = 1048576
```

```toml
# agent.toml
[resources]
# No bandwidth cap. Host reachability is still bounded by capabilities.network.
max_network_bytes_per_hour = 0
```

Editing the field in the dashboard's agent manifest form writes the same key.
A manifest hot-reload takes the new value through `AgentScheduler::update_quota`, which replaces the limit without resetting the window the agent has already spent.
