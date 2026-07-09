# Ops

DevOps agent. Monitors systems, runs diagnostics, manages deployments.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs observe-diagnose-plan-execute-verify methodology with change management discipline and rollback planning.

## Skills

- `shell_exec`, `file_read`, `file_list`

## Usage

```bash
librefang agent run ops
```
