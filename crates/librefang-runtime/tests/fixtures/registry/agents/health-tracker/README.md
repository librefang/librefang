# Health Tracker

Wellness tracking agent for health metrics, medication reminders, fitness goals, and lifestyle habits.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs health metrics logging, medication management, fitness goal tracking, habit building, and wellness reporting. Includes medical disclaimer.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run health-tracker
```
