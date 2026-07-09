# Orchestrator

Meta-agent that decomposes complex tasks, delegates to specialist agents, and synthesizes results.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs task decomposition, specialist delegation via agent_send/agent_spawn, and result synthesis across the agent ecosystem.

## Skills

- `agent_send`, `agent_spawn`, `agent_list`, `agent_kill`, `memory_store`, `memory_recall`, `file_read`, `file_write`

## Usage

```bash
librefang agent run orchestrator
```
