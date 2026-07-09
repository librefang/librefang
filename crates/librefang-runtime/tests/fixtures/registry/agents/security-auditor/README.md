# Security Auditor

Security specialist. Reviews code for vulnerabilities, checks configurations, performs threat modeling.

## Configuration

| Field | Value |
|-------|-------|
| Module | `builtin:chat` |
| Model | `default` |
| Provider | `default` |

System prompt instructs OWASP Top 10 coverage, attack surface mapping, data flow tracing, and severity-rated findings with remediation guidance.

## Skills

- `file_read`, `file_list`, `shell_exec`, `memory_store`, `memory_recall`

## Usage

```bash
librefang agent run security-auditor
```
