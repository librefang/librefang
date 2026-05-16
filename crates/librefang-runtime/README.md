# librefang-runtime

Agent runtime and execution environment for [LibreFang](https://github.com/librefang/librefang).

Hosts the agent execution loop (`agent_loop`), tool dispatch (`tool_runner`),
context management (compactor, context budget, overflow), audit trail, A2A
peer protocol, channel registry, the in-tree WASM host functions / sandbox,
the ChatGPT and Copilot OAuth flows, the apply-patch tool, and the
browser / Docker / process sandboxes (the last two as feature-gated leaf
crates that this crate re-exports under their historical module paths).

## Public API entry points

- `agent_loop` — turn-by-turn agent execution.
- `a2a` — Agent-to-Agent peer protocol.
- `apply_patch`, `tool_runner` (in sibling submodules) — tool execution path.
- `audit`, `auth_cooldown`, `aux_client`, `browser`, `catalog_sync`,
  `channel_registry`, `chatgpt_oauth`, `checkpoint_manager`, `compactor`,
  `context_budget`, `context_compressor`, `context_engine`,
  `context_overflow`, `copilot_oauth`, `dangerous_command`, `docker_sandbox`,
  `host_functions`, `media`, `media_understanding`, `sandbox`,
  `subprocess_sandbox` — runtime subsystems.
- Constant: `USER_AGENT` (sent on all outbound HTTP).

## Feature gates

Default-on: `media`, `browser`, `docker-sandbox`. Building with
`--no-default-features` swaps each gated module for a minimal stub that
returns `feature disabled` or no-ops, so build-time configurations can
omit whole subsystems for size or security. `audit`, `docker_sandbox`,
and `media` are re-exported from sibling leaf crates
(`librefang-runtime-audit`, `librefang-runtime-sandbox-docker`,
`librefang-runtime-media`) under their historical module paths so
downstream call sites stay untouched.

## Key dependencies

`librefang-types`, `librefang-http`, `librefang-kernel-handle`,
`librefang-runtime-audit`, `librefang-runtime-mcp`,
`librefang-runtime-media` (feature `media`),
`librefang-runtime-sandbox-docker` (feature `docker-sandbox`),
`librefang-llm-drivers`, `tokio`.

## Where this fits

The runtime is called from `librefang-kernel` when an agent receives a
message; it never depends on the kernel directly — communication goes
through the `KernelHandle` trait in `librefang-kernel-handle` to avoid
a circular dependency.

See the [workspace README](../../README.md).
