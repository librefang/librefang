# Agent-Created Workflows

Agents can create workflows during a conversation via the `workflow_create` tool.
This document covers the design, validation, persistence, and observability of that path.

## Tool surface

`workflow_create` is registered in `ALWAYS_NATIVE_TOOLS`, so every agent has it without manifest configuration.
It accepts `name`, `description`, `steps[]`, `input_schema[]`, and `total_timeout_secs` — the same shape the dashboard canvas and `POST /api/workflows` use.

Each step carries `name`, `agent` (name or UUID), `prompt_template` (with `{{input}}` for previous-step output and `{{var}}` for named variables), optional `depends_on` (DAG execution), `output_var`, `mode`, `timeout_secs`, and `error_mode`.

## Validation

Two layers, mirroring the HTTP API:

1. **Name validation** (runtime and kernel): `[A-Za-z0-9_-]`, 1–64 chars — path traversal is structurally impossible.
2. **Semantic validation** (`Workflow::validate()`): same checks as the canvas/API path — operator-node/DAG conflicts, invalid transform templates, empty branch arms, etc.

## Persistence

Workflows are written atomically (tmp file + rename) to `~/.librefang/workflows/<name>.workflow.toml`, then hot-registered in the `WorkflowEngine`.
The engine also persists its own JSON copy (`<id>.workflow.json`) — both are scanned at boot and deduped by UUID.
A failed write never leaves a partial file on disk.

## Budget and recursion guards

- **Budget attribution**: the ephemeral worker's usage is billed against the spawning agent's `agent_id`, so the cost appears in `GET /api/budget/agents/{parent}`.
- **Recursion depth**: `current_agent_depth() >= max_agent_call_depth` is checked before the loop runs (same quota as `agent_send` and workflow-step dispatch), and the loop future is wrapped in `with_agent_call_depth` (which also boxes the ~56 KB future — the #6659 stack-overflow hazard).

## Observability

Workflow creation logs `workflow=<name> registered_id=<uuid>` at info.
Workflow runs persist in SQLite via `WorkflowStore` with per-step results, timestamps, and terminal state.

## Dashboard integration

- Chat: the "Save as Workflow" button (GitBranch icon) on agent messages extracts the first JSON code block containing a `steps` array (iterating all fenced blocks, falling back to the raw message), converts steps to canvas nodes, and opens the canvas pre-populated.
- WorkflowsPage: the empty state shows "Create your first workflow" instead of auto-redirecting to templates.

## Skill

`skills/workflow-creator/` is a promptonly skill teaching agents when and how to design workflows — patterns, examples, best practices.
Installed as `prompt_context.md` so the content actually reaches the agent's context.

## Related

- `crates/librefang-runtime/src/tool_runner/workflow.rs` — tool implementations
- `crates/librefang-kernel/src/kernel/handles/workflow_runner.rs` — kernel handle + persistence
- `crates/librefang-kernel/src/workflow.rs` — engine, validation, run store
