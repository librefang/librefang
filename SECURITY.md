# Security Policy

## Supported Versions

| Version | Supported          |
|---------|--------------------|
| `main`  | :white_check_mark: |
| Latest LibreFang release | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in LibreFang, please report it privately.

**Do NOT open a public GitHub issue for security vulnerabilities.**

### How to Report

1. Use GitHub's private vulnerability reporting flow:
   `https://github.com/librefang/librefang/security/advisories/new`
2. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Affected versions
   - Potential impact assessment
   - Suggested fix (if any)

### What to Expect

- **Acknowledgment** within 48 hours
- **Initial assessment** within 7 days
- **Fix timeline** communicated within 14 days
- **Credit** given in the advisory (unless you prefer anonymity)

### Scope

The following are in scope for security reports:

- Authentication/authorization bypass
- Remote code execution
- Path traversal / directory traversal
- Server-Side Request Forgery (SSRF)
- Privilege escalation between agents or users
- Information disclosure (API keys, secrets, internal state)
- Denial of service via resource exhaustion
- Supply chain attacks via skill ecosystem
- WASM sandbox escapes

## Security Architecture

LibreFang implements defense-in-depth with the following security controls:

### Access Control
- **Capability-based permissions**: Agents only access resources explicitly granted
- **RBAC multi-user**: Owner/Admin/User/Viewer role hierarchy
- **Privilege escalation prevention**: Child agents cannot exceed parent capabilities
- **API authentication**: Bearer token with loopback bypass for local CLI

### Input Validation
- **Path traversal protection**: `safe_resolve_path()` / `safe_resolve_parent()` on all file operations
- **SSRF protection**: Private IP blocking, DNS resolution checks, cloud metadata endpoint filtering
- **Image upload validation**: exact-match MIME allowlist on
  `/api/agents/{id}/upload` covers `image/png`, `image/jpeg`, `image/gif`,
  `image/webp`; scriptable formats like `image/svg+xml` are rejected.
  Upload size is capped by `KernelConfig.max_upload_size_bytes` (default
  10 MiB — tighten it in `config.toml` if your threat model demands a
  smaller limit).
- **Prompt injection heuristics** *(best-effort, not a security boundary)*: Skill content is
  scanned for a short hard-coded list of English override phrases and exfiltration keywords
  (`ignore previous instructions`, `exfiltrate`, `post to https`, …) via case-insensitive
  substring match. Matches emit warnings and block installation of ClawHub skills whose
  `prompt_context` contains a *critical* pattern. This is a warning layer for obviously
  malicious content, **not** a defence against a motivated attacker: Unicode homoglyphs,
  zero-width separators, line-split keywords, Base64/other encodings, markdown/link
  obfuscation, and non-English phrasing all bypass it. The actual runtime safety of
  installed skills comes from the capability system and the WASM / subprocess sandbox
  (see **Runtime Isolation**), which bound what a skill can do regardless of what its
  prompt text says.

### Cryptographic Security
- **Ed25519 signed manifests**: Agent identity verification
- **HMAC-SHA256 wire protocol**: Mutual authentication with nonce-based replay protection
- **Secret zeroization**: `Zeroizing<String>` on all API key fields, wiped on drop

### Runtime Isolation
- **WASM dual metering**: Fuel limits + epoch interruption with watchdog thread
- **Subprocess sandbox**: Environment isolation (`env_clear()`), restricted PATH
- **Taint tracking**: Information flow labels prevent untrusted data in privileged operations

### Network Security
- **GCRA rate limiter**: Cost-aware token buckets per IP
- **Security headers**: CSP, X-Frame-Options, X-Content-Type-Options, HSTS
- **Health redaction**: Public endpoint returns minimal info; full diagnostics require auth
- **CORS policy**: Restricted to localhost when no API key configured

### Audit
- **Merkle hash chain**: Tamper-evident audit trail for all agent actions
- **Tamper detection**: Chain integrity verification via `/api/audit/verify`

## Dependencies

Security-critical dependencies are pinned and audited:

| Dependency | Purpose |
|------------|---------|
| `ed25519-dalek` | Manifest signing |
| `sha2` | Hash chain, checksums |
| `hmac` | Wire protocol authentication |
| `subtle` | Constant-time comparison |
| `zeroize` | Secret memory wiping |
| `rand` | Cryptographic randomness |
| `governor` | Rate limiting |
