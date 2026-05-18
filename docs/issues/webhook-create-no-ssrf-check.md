# [High] `create_event_webhook` / `update_event_webhook` — only `url::Url::parse`, no SSRF blocklist at registration

**Severity:** High · **Domain:** API attack surface · **Source:** `audit-02-api-attack-surface.md`

## Location
`crates/librefang-api/src/routes/webhooks.rs:386-440, 443-490`

## Problem
Both create and update validate via `url::Url::parse` only — no `validate_webhook_url` call. Internal URLs (`http://169.254.169.254/...`, `http://localhost:6379/`, `http://10.0.0.1/...`) **persist in the webhook store**. The `/test` endpoint and the daemon's normal delivery path re-validate, but:

1. Defense in depth — the validator already exists (`webhook_store::validate_webhook_url`).
2. The cron equivalent (`librefang_types::scheduler::validate_webhook_url`) already does reject at write time.
3. Stored hostile URLs survive across daemon restarts; if a future code path adds a "test without validation" or "bulk dispatch" feature, every existing stored URL becomes a live exploit.

## Fix
```rust
let url = url::Url::parse(&req.url).map_err(|_| HttpError::bad_request("invalid url"))?;
crate::webhook_store::validate_webhook_url(&url)?;
```

Apply at both create and update.

## Tests
- `POST /api/webhooks` with `url = "http://127.0.0.1:6379"` → 400.
- `PATCH /api/webhooks/{id}` rewriting to internal URL → 400.
