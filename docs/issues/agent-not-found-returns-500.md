# [High] API route error-code normalization — AgentNotFound → 404, AgentAlreadyExists → 409

**Severity:** High · **Domain:** Error handling
**Status:** Merges 1 earlier issue into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | Session/model handlers map `KernelError::LibreFang(AgentNotFound)` to 500; should be 404 | `routes/agents.rs:3063, 3102, 3153, 3206, 4836` (5 sites) |
| agent_clone status | `agent_clone` maps `AgentAlreadyExists` to 500; should be 409 | `routes/agents.rs` (clone handler) |

## Why merged

Both items need to be fixed in the same axum `KernelError → StatusCode` translation layer; tracking them separately would mean two PRs touching the same code.

## Combined fix plan

Extract a helper:
```rust
fn kernel_err_to_status(e: &KernelError) -> StatusCode {
    match e {
        KernelError::LibreFang(LibreFangError::AgentNotFound(_))      => StatusCode::NOT_FOUND,
        KernelError::LibreFang(LibreFangError::AgentAlreadyExists(_)) => StatusCode::CONFLICT,
        // ... extend as needed
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
```

Call it uniformly from all 5 `AgentNotFound` sites plus the `agent_clone` site.

The correct pattern already exists in this file: `agents.rs:1920-1933` (`send_message`) and `:4969-4979` (hand-config).

## Tests

- `tests/agents_404_test.rs`: `GET /api/agents/nonexistent/sessions` → 404; same coverage for `POST sessions`, `set_agent_model`, `switch_agent_session`, `export_session`.
- `POST /api/agents` creating a duplicate-name agent → 409.
