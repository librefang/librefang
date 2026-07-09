# Email Assistant

Email triage, drafting, scheduling, and inbox management agent.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs email classification by urgency, professional drafting with tone adaptation, scheduling, template management, and thread summarization.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`, `web_fetch`

## Usage

```bash
librefang agent run email-assistant
```
