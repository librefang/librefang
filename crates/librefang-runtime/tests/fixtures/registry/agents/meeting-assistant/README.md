# Meeting Assistant

Meeting notes, action items, agenda preparation, and follow-up tracking agent.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs structured agenda creation, transcript processing, action item extraction with owner/deadline tracking, and follow-up management.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run meeting-assistant
```
