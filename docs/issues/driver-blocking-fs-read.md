# [High] Blocking `std::fs::read` in async driver paths — image attachments

**Severity:** High · **Domain:** LLM driver & MCP · **Source:** `audit-08-llm-mcp.md`

## Location
- `crates/librefang-llm-drivers/src/drivers/anthropic.rs:1276`
- `crates/librefang-llm-drivers/src/drivers/gemini.rs:359`
- `crates/librefang-llm-drivers/src/drivers/ollama.rs:213`
- `crates/librefang-llm-drivers/src/drivers/openai.rs:868`

Moonshot's `convert_message` correctly uses `tokio::fs::read` — proving the pattern is known, but the bulk path regressed.

## Problem
Synchronous `std::fs::read` on a tokio worker reading `ContentBlock::ImageFile`. For a multi-MB image (common in agent workflows that screenshot or generate plots), this stalls the executor for tens to hundreds of milliseconds — blocking every other request on the same worker.

## Fix
Replace with `tokio::fs::read(path).await`:
```rust
let bytes = tokio::fs::read(&path).await
    .map_err(|e| DriverError::Io(format!("read {}: {e}", path.display())))?;
```

Apply the same fix to all four drivers. Consider pre-loading at the agent loop layer so drivers receive `Bytes` rather than a `PathBuf`.

## Tests
- Bench: a 5 MB image attachment is read without blocking the executor (measured via `tokio::time::sleep(0)` on another worker not getting starved).
- Unit: each driver's `convert_message` returns the same bytes as the previous sync path for the same input.
