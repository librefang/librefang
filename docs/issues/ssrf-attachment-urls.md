# [Critical] SSRF via attachment URLs in `POST /api/agents/{id}/message`

**Severity:** Critical
**Domain:** API attack surface

## Location

`crates/librefang-api/src/routes/agents.rs:1514-1569` — `resolve_url_attachments`

## Problem

The handler builds a `proxied_client_builder()` and calls `client.get(&att.url).send()` with **no `validate_webhook_url` / `is_url_safe_for_ssrf` check**. Every other outbound-from-user-input path in the codebase (`/api/a2a/discover`, `/api/webhooks/events/{id}/test`, cron webhook delivery) does validate. The `User` role is explicitly allowed to POST `/message` per `middleware.rs:163-178`, so this is reachable by the lowest-privilege role that can produce LLM traffic.

## Exploit

```json
POST /api/agents/<id>/message
{
  "attachments": [{
    "url": "http://169.254.169.254/latest/meta-data/iam/security-credentials/<role>",
    "content_type": "image/png"
  }]
}
```

IMDS / internal-service contents land as base64 image blocks in the agent session, which the LLM happily transcribes / summarizes back to the attacker on the next turn.

## Fix

Call `crate::webhook_store::validate_webhook_url_resolved` before fetching and pin the resolved address with `.resolve(host, addr)` — the exact pattern already used at `webhooks.rs:738-744`. The validator handles RFC1918, loopback, link-local, IPv6 unique-local, and DNS-rebind via post-resolve re-check.

## Tests

- Integration test against `start_full_router` asserting:
  - `attachments[].url = "http://127.0.0.1:1/whatever"` → 400
  - `attachments[].url = "http://169.254.169.254/..."` → 400
  - `attachments[].url = "http://example.invalid"` → 400 (no resolve)
  - DNS rebind: domain that resolves to a public IP at resolution then 127.0.0.1 at connect → 400 (covered by `.resolve(host, addr)` pin)
