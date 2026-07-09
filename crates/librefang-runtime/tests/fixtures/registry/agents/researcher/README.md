# Researcher

Research agent. Fetches web content and synthesizes information.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs decompose-search-deep dive-cross reference-synthesize methodology with source evaluation and confidence-rated findings.

## Skills

- `web_search`, `web_fetch`, `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run researcher
```
