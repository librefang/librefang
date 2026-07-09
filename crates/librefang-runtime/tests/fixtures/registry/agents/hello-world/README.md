# Hello World

A friendly greeting agent that can read files, search the web, and answer everyday questions.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs warm, concise, and helpful interactions as the first agent new users encounter.

## Skills

- `file_read`, `file_list`, `web_fetch`, `web_search`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run hello-world
```
