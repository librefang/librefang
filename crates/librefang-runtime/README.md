# librefang-runtime

Agent runtime and execution environment for [LibreFang](https://github.com/librefang/librefang).

Hosts the agent execution loop (`agent_loop`), tool dispatch (`tool_runner`),
context management (compactor, context budget, overflow), audit trail, A2A
peer protocol, channel registry, browser/docker/process sandboxes, and re-exports
the OAuth subsystems.

## Public API entry points

- `agent_loop` — turn-by-turn agent execution.
- `a2a` — Agent-to-Agent peer protocol.
- `apply_patch`, `tool_runner` (in sibling submodules) — tool execution path.
- `audit`, `auth_cooldown`, `aux_client`, `browser`, `catalog_sync`,
  `channel_registry`, `checkpoint_manager`, `compactor`,
  `context_budget`, `context_compressor`, `context_engine`,
  `context_overflow`, `dangerous_command`, `docker_sandbox` — runtime subsystems.
- Re-exports: `chatgpt_oauth`, `copilot_oauth` (from `librefang-runtime-oauth`).
- Constant: `USER_AGENT` (sent on all outbound HTTP).

## Key dependencies

`librefang-types`, `librefang-http`, `librefang-kernel-handle`,
`librefang-runtime-mcp`, `librefang-runtime-oauth`,
`librefang-runtime-wasm`, `librefang-llm-drivers`, `tokio`.

## Where this fits

The runtime is called from `librefang-kernel` when an agent receives a
message; it never depends on the kernel directly — communication goes
through the `KernelHandle` trait in `librefang-kernel-handle` to avoid
a circular dependency.

See the [workspace README](../../README.md).
