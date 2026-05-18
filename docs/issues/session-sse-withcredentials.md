# [High] `useSessionStream` uses `withCredentials: true` for SSE but dashboard auth is Bearer-in-header

**Severity:** High · **Domain:** Dashboard · **Source:** `audit-09-dashboard.md`

## Location
`crates/librefang-api/dashboard/src/lib/queries/sessions.ts:138`

## Problem
`EventSource` constructed with `withCredentials: true` sends the dashboard's cookie. But the dashboard's actual auth is `Authorization: Bearer <token>` in the header. `EventSource` doesn't support custom headers, so the request lacks the Bearer token entirely.

Today the route returns mock data so the bug is invisible. Once feature #3078 lands and the route requires auth, every SSE attach attempt receives 401 and hits the silent no-op at line 186 — **no error surface to the user**.

## Fix
Two options:
1. Match the WebSocket pattern (`Sec-WebSocket-Protocol` sub-protocol carries the token) — `useSessionStream` should not use `EventSource`, switch to WebSocket.
2. Add a query-string token (`?token=...`) and refuse to log it server-side. Less safe than (1) because of URL caching.

Option (1) is recommended; existing WebSocket auth in the dashboard works correctly and uses the sub-protocol form (which is per CLAUDE.md best practice — `Sec-WebSocket-Protocol` from #3963 doesn't leak to logs).

## Tests
- Smoke against `start_full_router` with auth enabled — SSE/WS attach succeeds and streams events.
- Negative: missing token → connection rejected with surfaced error in the dashboard UI.
