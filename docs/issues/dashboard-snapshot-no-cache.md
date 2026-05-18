# [High] `dashboard_snapshot_inner` re-enriches all agents every 5s with no generation cache

**Severity:** High · **Domain:** Performance · **Source:** `audit-05-performance.md`

## Location
`crates/librefang-api/src/routes/config.rs:3063-3084`

## Problem
The dashboard's main poll endpoint walks every agent and enriches with manifest data, current session, recent message count, etc., every 5 seconds. Nothing has changed for 99% of agents between consecutive polls, but full work is repeated.

## Fix
Generation-based cache keyed on `(manifest_rev, session_rev)`:
```rust
struct SnapshotCache {
    rev: (u64, u64),
    payload: Arc<DashboardSnapshot>,
}
```
Bump `manifest_rev` on `replace_manifest`, `session_rev` on session-write. Return the cached payload when both match.

## Tests
- 10 sequential polls in <1s → cache hit on polls 2-10 (no DB reads).
- Modify one agent's manifest → next poll regenerates.
