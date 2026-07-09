# Assistant

General-purpose assistant agent. The default LibreFang agent for everyday tasks, questions, and conversations.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs versatile capabilities: conversation, task execution, research, writing, problem solving, agent delegation, and knowledge management.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`, `web_fetch`, `shell_exec`, `agent_send`, `agent_list`

## Usage

```bash
librefang agent run assistant
```
