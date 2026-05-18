# [Medium] Error handling roundup — spawn panic swallow, `let _ =`, reqwest builder `expect`, i32 overflow, JSON parse silent reshape

**Severity:** Medium · **Domain:** Error handling
**Status:** Merges 5 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `tokio::spawn(connect_mcp_servers)` panic → the task silently dies and the server never connects | `routes/skills.rs:4261, 4367, 4485` → `kernel/mcp_setup.rs:33` |
| request_approval spawn | Detached `request_approval` spawn — same class; panics are lost | `request_approval` spawn site |
| JSON reshape | When workflow step output JSON fails to parse, the code silently reshapes it into a "wrapped" form, hiding real bugs | workflow step output handling |
| create_dir_all swallow | `let _ = create_dir_all(...)` swallows EACCES; the directory never appears, and subsequent writes fail | several `create_dir_all` sites |
| reqwest builder expect | `reqwest::ClientBuilder::build().expect(...)` panics on bad proxy / missing TLS roots | reqwest builder call sites |
| counter overflow | `(2..).find(...).expect("counter is unbounded")` overflows at i32::MAX → panic | counter generator |

## Why merged

All six are error-handling / `unwrap` / silent-swallow hygiene items; a single sweep PR is more economical than six.

## Combined fix plan

1. **(this) / (request_approval spawn) wrap spawn in catch_unwind**:
   ```rust
   tokio::spawn(async move {
       if let Err(e) = AssertUnwindSafe(fut).catch_unwind().await {
           tracing::error!(?e, "background task panicked");
       }
   });
   ```
   Or migrate to the existing `supervised_spawn` helper (see "openai-compat-token-add-overflow" cluster).
2. **(JSON reshape) JSON parse failure → explicit error**: return `Err(WorkflowError::OutputParse(e))`; the caller decides whether to soft-handle. **Do not** silently reshape.
3. **(create_dir_all swallow) propagate `create_dir_all`**: change `let _ = create_dir_all(p)` to `create_dir_all(p)?;`, or explicitly `match` distinguishing `AlreadyExists` from other IO errors.
4. **(reqwest builder expect) Return Result from ClientBuilder**:
   ```rust
   let client = ClientBuilder::new()...build().map_err(BootError::HttpClient)?;
   ```
   Move the panic into the boot fail-fast layer.
5. **(counter overflow) Guard the counter**: switch to `u32` or add a saturating check; replace `expect` with a typed error.

## Tests

- Force a panic inside a spawn → log appears and the daemon stays up.
- Workflow step returns invalid JSON → test asserts the caller receives `OutputParse`, not silent success.
- `create_dir_all` on a read-only filesystem → returns an IO error rather than silently continuing.
- ClientBuilder with missing TLS roots → boot fails with a structured error, not a panic.
- Counter at i32::MAX-100 → stress test asserts no panic.
