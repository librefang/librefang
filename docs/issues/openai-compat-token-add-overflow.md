# [Low] Defensive coding sweep — overflow, refactor-fragile `unreachable!`, dropped `error_tx`, unsupervised spawn, precision loss

**Severity:** Low
**Category:** Defensive coding
**Status:** Merges 5 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description |
|--------|-------------|
| this | `openai_compat` `total_tokens: input + output` (`u64`) is not `saturating_add` |
| u64 as usize | `u64 as usize` truncates provider tokens on 32-bit targets (Windows ARM, etc.) |
| MCP unreachable | The MCP transport-kind secondary match uses `unreachable!()`; the comment admits it is "to work around the borrow checker" — refactor-fragile |
| channel_bridge error_tx | `channel_bridge` `let _ = error_tx.send(...)`: when the receiver is gone, kernel-panic information is silently dropped |
| unsupervised spawn | Background-maintenance `tokio::spawn` does not use `supervised_spawn`; a panic → task abort, and the retention sweep never runs again |
| f64 precision | `(ctx_window as f64 * 0.70) as usize`: large catalog values (> 2^53) lose precision in f64, distorting the trim threshold |

## Affected files

- `crates/librefang-api/src/openai_compat.rs:357-358` (this)
- `crates/librefang-runtime/src/agent_loop/mod.rs:1033`, `run_streaming.rs:799` (u64 as usize)
- `crates/librefang-runtime/src/agent_loop/mod.rs:888`, `run_streaming.rs:558` (f64 precision)
- `crates/librefang-runtime-mcp/src/lib.rs:2032, 2115` (MCP unreachable)
- `crates/librefang-api/src/channel_bridge.rs:548-648` (channel_bridge error_tx)
- `crates/librefang-kernel/src/kernel/background_lifecycle.rs:492, 1214` (unsupervised spawn)
- `crates/librefang-kernel/src/supervised_spawn.rs` (existing helper, not used at these two sites)

## Why merged

All six are single-point defensive-coding hygiene items with low value when tracked individually; folding them into a single sweep PR is more efficient.

## Combined fix plan

1. **Numeric boundaries (this / u64 as usize / f64 precision)**:
   ```rust
   // this
   input.saturating_add(output)
   // u64 as usize
   (response.usage.input_tokens.min(usize::MAX as u64)) as usize
   // f64 precision
   let ctx = ctx_window.min(usize::MAX as u64 / 2);
   if ctx != ctx_window { warn!(original = ctx_window, clamped = ctx, "ctx_window clamped"); }
   let threshold = (ctx as f64 * 0.70) as usize;
   ```
   Additionally, at the driver-response deserialization boundary, pre-clamp `input_tokens` / `cache_*` to a sane upper bound (`u32::MAX as u64`).
2. **Eliminate `unreachable!()` (MCP unreachable)**: either replace with a descriptive panic message, or refactor into `if let McpInner::Rmcp(client) = &mut self.inner { ... } else { return Err(_); }` to remove the twin-match outright.
3. **Make `error_tx` failures visible (channel_bridge error_tx)**:
   ```rust
   if status_tx.send(status).is_err() {
       warn!(?agent_id, "status_tx receiver gone — kernel result {:?} dropped", status_for_log);
   }
   ```
4. **Universal `supervised_spawn` adoption (unsupervised spawn)**: replace both background `tokio::spawn` sites with the existing `supervised_spawn`, which logs + restarts on panic. Sweep `rg "tokio::spawn" crates/librefang-kernel/src/kernel/` and migrate every "long-lived background task" over.

## Tests

- (this / u64 as usize / f64 precision) Unit tests: `u64::MAX` tokens → no panic, no wrap, saturating / clamped output, warning log (where applicable).
- (MCP unreachable) After refactor, no `unreachable!()`; constructing a mismatched `McpInner` returns `Err`.
- (channel_bridge error_tx) Drop the `error_tx` receiver and send → "receiver gone" appears in the log.
- (unsupervised spawn) `panic!()` inside the background sweep → `supervised_spawn` restarts the task and emits `error!`.
