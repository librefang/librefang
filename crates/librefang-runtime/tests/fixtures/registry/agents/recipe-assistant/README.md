# Recipe Assistant

Cooking assistant that helps with recipes, meal plans, ingredient substitutions, and portion adjustments.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs recipe discovery, portion scaling, ingredient substitution, meal planning, grocery list generation, and cooking technique guidance.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`, `web_fetch`, `web_search`

## Usage

```bash
librefang agent run recipe-assistant
```
