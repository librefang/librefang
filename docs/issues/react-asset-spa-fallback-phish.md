# [Medium] API surface hygiene roundup — `react_asset`, registry validation, UNC, test `target_id`, auth providers leak

**Severity:** Medium · **Domain:** API attack surface
**Status:** Merges 4 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | `react_asset` SPA fallback maps any extensionless `/dashboard/*` path → `index.html`, amplifying the phishing surface | `webchat.rs:258-293` |
| registry validation | `register_registry_content` has weak identifier validation (accepts overly broad characters) | `routes/registry.rs` |
| UNC bypass | `react_asset` is bypassed by Windows UNC `\\server\share` forms | `webchat.rs` (path parsing) |
| channel test target_id | Channel test helpers splice `target_id` directly into a URL without validation | `routes/channels.rs` (test endpoints) |
| auth providers leak | `/api/auth/providers` is unconditionally public, exposing the IdP list (information gathering) | `middleware.rs` allowlist + `routes/oauth.rs:.../providers` |

## Why merged

The five issues are scattered across `webchat.rs` / `registry.rs` / `oauth.rs` / `routes/channels.rs` but all are "small entry-point API surface" hygiene issues; a single audit pass is more efficient.

## Combined fix plan

1. **(this) SPA route allowlist**: maintain `SPA_ROUTES: &[&str]`; extensionless paths that don't match return 404 rather than fall through to `index.html`.
2. **(registry validation) Tighten identifier validation**: `register_registry_content` enforces `^[a-zA-Z0-9._-]+$` + a length cap (e.g. 128).
3. **(UNC bypass)**: in `react_asset` path parsing, reject any input starting with `\\` or `//`; canonicalize + verify `starts_with(dashboard_dir)` (same as "react-asset-path-traversal").
4. **(channel test target_id)**: parse `target_id` via `Uuid::from_str`, reject with 400 on parse failure; never splice raw strings into URLs.
5. **(auth providers leak)**: gate `/api/auth/providers` on `require_auth_for_reads`; by default only authenticated users see it; in public mode, expose only provider names (no URL / scope).

## Tests

- `GET /dashboard/security-alert` → 404; `GET /dashboard/agents` → 200 SPA.
- `POST /api/registry/content` with body `name = "../etc"` → 400.
- `react_asset` with input `\\server\share` → 400.
- Channel test endpoint with `target_id = "; DROP"` → 400.
- Unauthenticated `GET /api/auth/providers` in strict mode → 401; in open mode returns only the names array.
