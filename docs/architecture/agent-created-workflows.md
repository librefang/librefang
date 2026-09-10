# Agent-Created Workflows

Agents can create workflows during a conversation via the `workflow_create` tool.
This document covers the design, validation, persistence, and observability of that path.

## Tool surface

`workflow_create` is part of the default tool catalogue, so an ordinary agent has it without manifest configuration and a `tool_blocklist` entry is what takes it away.
It is *not* in `ALWAYS_NATIVE_TOOLS` — that array is the lazy-load shortlist, so an agent running with `lazy_tools = true` has to reach `workflow_create` (and `workflow_run`) through `tool_search` / `tool_load` rather than finding them already declared.
It accepts `name`, `description`, `steps[]`, `input_schema[]`, and `total_timeout_secs` — the same shape the dashboard canvas and `POST /api/workflows` use.

Each step carries `name`, `agent` (name or UUID), `prompt_template` (with `{{input}}` for previous-step output and `{{var}}` for named variables), optional `depends_on` (DAG execution), `output_var`, `mode`, `timeout_secs`, and `error_mode`.

## Validation

Two layers, mirroring the HTTP API:

1. **Name validation** (runtime and kernel): `[A-Za-z0-9_-]`, 1–64 chars — path traversal is structurally impossible.
2. **Semantic validation** (`Workflow::validate()`): same checks as the canvas/API path — operator-node/DAG conflicts, invalid transform templates, empty branch arms, etc.

## Persistence

Workflows are written atomically (tmp file + rename) to `~/.librefang/workflows/<id>.workflow.json`, keyed by the workflow's UUID, then hot-registered in the `WorkflowEngine`.
A failed write never leaves a partial file on disk.

## Budget and recursion guards

- **Budget attribution**: the ephemeral worker's usage is billed against the spawning agent's `agent_id`, so the cost appears in `GET /api/budget/agents/{parent}`.
- **Recursion depth**: `current_agent_depth() >= max_agent_call_depth` is checked before the loop runs (same quota as `agent_send` and workflow-step dispatch), and the loop future is wrapped in `with_agent_call_depth` (which also boxes the ~56 KB future — the #6659 stack-overflow hazard).

## Observability

Workflow creation logs at info with the fields `workflow_id`, `workflow_name`, `caller_agent_id`, `owner` and `step_count`, under the message `Agent created a workflow`.
`caller_agent_id` and `owner` are the two the line exists for: which agent turn assembled the spec, and which principal it was acting for (#7744).
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

## Step agent types, fresh instances, and required skills

Steps can reference an **agent type** (a template) instead of a concrete agent:

```json
{ "type": "researcher" }
```

Resolution is find-or-spawn: reuse the registered agent with that name, otherwise load the template manifest (searched in order: `agent-types/`, then `workspaces/agents/`, then `registry/agents/`) and spawn it with the canonical name-derived UUID. There is deliberately no `"fresh": true` variant that would spawn a throwaway instance per run (#7714); to isolate a step from the type's prior context, set `WorkflowStep::session_mode` to `New` instead. A missing template fails the step with a ByType-specific error.

Steps can also declare `required_skills`. The engine checks them right after agent resolution, before dispatch, and fails the step with a precise error distinguishing two cases:

- "does not declare" — the skill is absent from the resolved agent's (or template's) skill allowlist;
- "declares but are not installed" — the skill is in the allowlist but the registry hasn't loaded it (see the pending-declarations surface on the agents API).

## Run ownership and billing

A run records its `owner_agent_id` (migration v48): the caller of `workflow_run` / `workflow_start` / the channel `/workflow run` command. Template-spawned step agents are parented to the owner and their usage records bill to it. Resume and operator-action paths copy the original run's owner.

## Agent types

Agent types are templates: the canonical spelling is "agent type" everywhere, in prose and UI, though the API stays at `/api/templates` (there is deliberately no `/api/agent-types` alias, #7722; the TUI templates screen is titled "Agent types"). Agents can author types with the `agent_type_create` tool (same validation as the API), and ephemeral workers spawned from a type get a uid display name plus a transient mission workspace under `~/.librefang/transient/<name>` that is deleted when the run ends.

The editable agent-type surface is exactly seven fields: `name`, `description`, `system_prompt`, `provider`/`model` (spawn defaults), `tools` and `skills`.
That is what `AgentTypeSpec` carries, what `agent_type_spec_of` projects out of a manifest, and — because the struct is `#[serde(deny_unknown_fields)]` — the complete set a `PUT` accepts back; anything else in the body is a 422 rather than a partial save.
The dashboard editor exposes those seven with catalog-backed pickers for skills and tools, and the shared `AgentTypeSpec` / `AgentTypeSpec::apply_to` in `librefang-types` is the single source of truth for the API route and the tool, so the two authoring surfaces cannot drift.
Everything else in a manifest — `channels`, the `[routing]` tiers, triggers, MCP servers, exec policy — starts at its default on a type and is edited on the manifest afterwards; `apply_to` leaves those fields untouched rather than resetting them.
