# Upload route bypasses the global body-limit and buffers the entire body into memory

**Severity:** High
**Category:** DoS / resource exhaustion
**Labels:** `security`, `dos`, `memory`, `high`

## Affected files
- `crates/librefang-api/src/server.rs:1266-1274` (`upload_routes` mount point)
- `crates/librefang-api/src/server.rs:1370-1378` (where `RequestBodyLimitLayer` is applied)
- `crates/librefang-api/src/routes/agents.rs:5947-6013` (handler, `body: axum::body::Bytes`)

## Description

`upload_routes` is merged into `app` **before** `RequestBodyLimitLayer` is applied. The handler signature `body: axum::body::Bytes` forces axum to **buffer the entire request body into RAM** before `agents.rs:6008` finally checks `body.len() > upload_limit`.

The 10 MiB `max_upload_size_bytes` is an **after-the-fact** check, not a wire-level cap. An authenticated user (the route sits inside the auth-required tree) can push a 50 GB body and exhaust the daemon's RAM. The comment at `server.rs:1370-1374` already acknowledges the exemption, but the handler does not compensate.

## Recommendation

Wrap `upload_routes` in its own `RequestBodyLimitLayer` before merging:

```rust
let upload_routes = upload_routes.layer(
    RequestBodyLimitLayer::new(
        state.kernel.config_ref().max_upload_size_bytes + MULTIPART_OVERHEAD
    )
);
```

Or rewrite the handler to consume a `BodyStream`, accumulating bytes and aborting once `limit` is exceeded. Either approach pins peak RAM to the configured value.

Regression test: an authenticated `POST` with `Content-Length: 5GB` (streamed body) asserts that RAM does not grow.
