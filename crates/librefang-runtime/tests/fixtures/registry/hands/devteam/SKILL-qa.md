---
name: devteam-qa-skill
version: "1.0.0"
description: "QA reference knowledge — review checklist, security audit patterns, test philosophy"
runtime: prompt_only
---

# QA Reference Knowledge

## Code Review Checklist

### Correctness
- Logic errors, off-by-one, unhandled edge cases
- Error handling: all error paths tested?
- Null/undefined safety
- Resource leaks (memory, file handles, connections)
- Concurrency: race conditions, deadlocks, TOCTOU

### Security (OWASP Top 10)
- Injection (SQL, command, XSS, SSTI)
- Authentication/authorization flaws
- Sensitive data exposure (logging secrets, hardcoded keys)
- Input validation gaps
- Insecure cryptographic usage
- Path traversal, SSRF
- Deserialization attacks

### Performance
- Algorithmic complexity (O(n²) loops, unbounded recursion)
- Unnecessary allocations, copies
- N+1 queries, missing caching
- I/O patterns (blocking in async, unbuffered reads)

## Review Format

```
## Summary
[approve / request changes / needs discussion]

## Findings
### [MUST FIX] file.rs:42 — Off-by-one in loop bound
...
### [SHOULD FIX] handler.rs:88 — Missing input validation
...
### [NIT] utils.rs:12 — Consider renaming for clarity
...
### [PRAISE] auth.rs:55 — Clean error handling pattern
...
```

## Testing Philosophy

- Tests document behavior, not implementation
- Test the interface, not the internals
- Every test should fail for exactly one reason
- Prefer fast, deterministic tests
- Test name pattern: `test_{function}_{scenario}_{expected}`
- Arrange → Act → Assert

## gh CLI for Reviews

```bash
# Approve
gh pr review NUMBER --repo OWNER/REPO --approve --body "QA passed"

# Request changes
gh pr review NUMBER --repo OWNER/REPO --request-changes --body "Found issues, see comments"

# Add line comment (via API, gh CLI doesn't support line comments directly)
gh api repos/OWNER/REPO/pulls/NUMBER/reviews \
  --method POST \
  -f event=REQUEST_CHANGES \
  -f body="See line comments" \
  --input - <<< '{"comments":[{"path":"src/file.rs","line":42,"body":"Off-by-one here"}]}'
```
