# [Medium] Performance misc — `file_read_tracker` leak, `save_session` clone, Vite lazy audit

**Severity:** Medium · **Domain:** Performance
**Status:** Merges 2 earlier issues into a single tracking item.

## Sub-findings rollup

| Origin | Description | Location |
|--------|-------------|----------|
| this | The `file_read_tracker` registry is purged only during context compression; the session-delete path is not hooked → long-lived daemons accumulate dead-session entries | `librefang-runtime/src/file_read_tracker.rs:177-180` |
| save_session clone | `save_session_async` deep-clones the entire `Session` (including all messages) before `spawn_blocking`, doubling peak memory for each write | `save_session_async` implementation |
| Vite lazy audit | Dashboard route-level `lazy()` has not been audited — some pages bloat the initial bundle; code-splitting is incomplete | `dashboard/src/App.tsx` route configuration |

## Why merged

All three are performance-hygiene items affecting long-lived daemons / dashboard startup.

## Combined fix plan

1. **(this) Session-delete hook**:
   ```rust
   // in delete_session path
   file_read_tracker.forget_session(&session_id);
   ```
   Keep context-compression GC as the fallback.
2. **(save_session clone) Pass Cow / Arc into spawn_blocking**: wrap messages in `Arc<Vec<...>>`; `spawn_blocking` only takes a read reference, and SQLite-writing code clones row payloads on demand.
3. **(Vite lazy audit) Dashboard code-split audit**: run `vite-bundle-visualizer` or similar; identify initial chunks > 200 KB gzipped and apply `lazy()` / per-route splits.

## Tests

- (this) Create + delete 1000 sessions → tracker map size stays bounded (≤ active session count).
- (save_session clone) Bench `save_session_async`: with 100 messages, peak RAM delta ≤ serialized byte size (no longer doubles).
- (Vite lazy audit) Initial-bundle gzipped size drops by X% (concrete value set after audit).
