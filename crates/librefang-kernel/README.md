# librefang-kernel

Core kernel for the [LibreFang](https://github.com/librefang/librefang) Agent Operating System.

The kernel orchestrates agent lifecycles, scheduling, permissions, inter-agent
communication, and the message-handling loop that fans requests out to LLM
drivers, tools, and the memory substrate.

## Public API entry points

- `kernel::LibreFangKernel` — the top-level orchestrator. Boot via
  `LibreFangKernel::boot_with_config(KernelConfig)`.
- `registry::AgentRegistry` — concurrent agent table; spawn, lookup, kill.
- `approval`, `auth`, `auto_dream`, `cron`, `event_bus`, `inbox`,
  `pairing`, `scheduler`, `session_lifecycle` — subsystem modules.
- Re-exports: `metering` (from `librefang-kernel-metering`),
  `router` (from `librefang-kernel-router`).

## Key dependencies

`librefang-types`, `librefang-memory`, `librefang-runtime`,
`librefang-skills`, `librefang-hands`, `tokio`, `axum`-adjacent traits.

## Where this fits

| Layer | Crate |
| --- | --- |
| Surface (HTTP/WS) | `librefang-api` |
| Orchestration | **`librefang-kernel`** ← this crate |
| Execution | `librefang-runtime` |
| Storage | `librefang-memory` |

See the [workspace README](../../README.md) and
[architecture docs](../../docs/architecture/) for the full picture.
