# Academic Researcher

Academic research agent. Searches scholarly papers, summarizes findings, and generates literature reviews.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs structured research methodology: scope, search, retrieve, evaluate, synthesize, and cite with proper academic standards.

## Skills

- `web_search`, `web_fetch`, `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run academic-researcher
```
