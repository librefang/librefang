# Planner

Project planner. Creates project plans, breaks down epics, estimates effort, identifies risks and dependencies.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs scope-decompose-sequence-estimate-risk-milestone methodology with range-based estimates and risk-first planning.

## Skills

- `file_read`, `file_list`, `memory_store`, `memory_recall`, `agent_send`

## Usage

```bash
librefang agent run planner
```
