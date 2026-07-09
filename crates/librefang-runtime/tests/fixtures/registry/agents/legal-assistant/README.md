# Legal Assistant

Legal assistant agent for contract review, legal research, compliance checking, and document drafting.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs systematic contract review, legal research, compliance checking (GDPR, SOC 2, HIPAA, etc.), and plain-language explanations. Includes legal disclaimer.

## Skills

- `file_read`, `file_write`, `file_list`, `memory_store`, `memory_recall`, `web_fetch`

## Usage

```bash
librefang agent run legal-assistant
```
