# Sales Assistant

Sales assistant agent for CRM updates, outreach drafting, pipeline management, and deal tracking.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs personalized outreach via AIDA framework, CRM data management, pipeline forecasting, call prep, and competitive intelligence.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`, `web_fetch`

## Usage

```bash
librefang agent run sales-assistant
```
