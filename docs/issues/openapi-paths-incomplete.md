# [High] OpenAPI `paths(...)` block is missing 76+ annotated handlers; entire MCP-OAuth flow absent from `openapi.json`

**Severity:** High · **Domain:** Architecture · **Source:** `audit-06-architecture.md`

## Location
- 377 `#[utoipa::path]` annotations across `crates/librefang-api/src/`
- Only 301 references in `openapi.rs::ApiDoc::paths(...)`
- Confirmed missing handlers:
  - `auth_start` (`routes/mcp_auth.rs:264`)
  - `approve_all_for_session` (`routes/approvals.rs:852`)
  - `approval_count` (`routes/approvals.rs:1074`)
- `grep` against `openapi.json` for `/api/mcp/servers/{name}/auth/*` → **zero hits**. The entire MCP-OAuth flow is undocumented.

## Problem
Published `openapi.json` is silently incomplete. Generated SDK consumers lack 76+ endpoints. The pre-push hook claims to gate OpenAPI/SDK drift (per CLAUDE.md), but the actual `scripts/hooks/pre-push` does no such thing (see "rustfmt-loses-spaced-paths" cluster).

## Fix
**Reflection test** asserting every `#[utoipa::path]` handler in the workspace is present in `ApiDoc::paths(...)`:

```rust
#[test]
fn every_annotated_handler_is_in_openapi() {
    let annotated: HashSet<&str> = inventory::iter::<UtoipaPathHandler>
        .map(|h| h.path).collect();
    let documented: HashSet<&str> = ApiDoc::openapi()
        .paths.iter().flat_map(|(p, _)| p.split(',')).collect();
    let missing: Vec<_> = annotated.difference(&documented).collect();
    assert!(missing.is_empty(), "missing from OpenAPI: {missing:?}");
}
```

Backfill the 76+ missing entries as part of the same PR.

## Tests
- The reflection test above.
- CI gate that fails when the SDK is regenerated and changes are not committed.
