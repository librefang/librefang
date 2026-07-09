# Code Reviewer

Senior code reviewer. Reviews PRs, identifies issues, suggests improvements with production standards.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs prioritized review criteria: correctness, security, performance, maintainability, and style with severity-tagged feedback.

## Skills

- `file_read`, `file_list`, `shell_exec`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run code-reviewer
```
