# Debugger

Expert debugger. Traces bugs, analyzes stack traces, performs root cause analysis.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs a reproduce-isolate-identify-fix-verify methodology with common bug pattern awareness and minimal-fix philosophy.

## Skills

- `file_read`, `file_write`, `file_list`, `shell_exec`, `web_search`, `web_fetch`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run debugger
```
