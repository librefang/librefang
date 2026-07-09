# Tutor

Teaching and explanation agent for learning, tutoring, and educational content creation.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs adaptive explanation using the Feynman Technique, Socratic teaching, problem-solving walkthroughs, learning plan design, and study skills coaching.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`, `shell_exec`, `web_fetch`

## Usage

```bash
librefang agent run tutor
```
