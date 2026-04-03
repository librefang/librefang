# ADR-004: Memory Store Content Durability in RVF Containers

**Status**: Accepted
**Date**: 2026-03-15
**Authors**: Daniel Alberttis

## Context

ADR-003 (Memory Store Implementation) migrates SemanticStore to RvfStore + HNSW. The initial design stored memory content text in `.access.db` (SQLite side-store). If `.access.db` is deleted or corrupted: vectors and HNSW index survive in `.rvf`, but all readable memory text is permanently lost. This is a durability regression relative to upstream OpenFang, where content lives in the always-open `openfang.db`.

Two options exist:
1. Accept `.access.db` as the sole durable content store — simple, no fork required, but durability regression stands.
2. Fork vendored `rvf-runtime` to add a `CONTENT_MAP_SEG (0x12)` custom segment type storing `HashMap<u64, Vec<u8>>` (vec_id → UTF-8 content bytes) inside the `.rvf` file itself — eliminates durability regression at the cost of a vendored fork.

## Decision

**Note**: ADR-003 v1.7 resolved this in favour of Option 2 (CONTENT_MAP_SEG fork) and recorded the full specification. This ADR exists as the standalone decision record for that choice. ADR-003 is now Accepted at v2.0 with CONTENT_MAP_SEG as the live design — `.access.db` as interim baseline is no longer applicable.

If Option 2 (CONTENT_MAP_SEG) is chosen:
- Segment discriminant: `ContentMap = 0x12` (currently unassigned; `Dashboard = 0x11`, next used value is `CowMap = 0x20`)
- `content` column removed from `.access.db` DDL
- Content lives in an in-memory `content_map: HashMap<u64, Vec<u8>>`, but writes use append-only `CONTENT_MAP_SEG` deltas on remember/forget and an explicit snapshot checkpoint on close/compact
- CONTENT_MAP_SEG entries carry explicit delete tombstones, so older segments cannot resurrect content after restart
- On `store.open()`, `extract_content_map()` replays CONTENT_MAP_SEG segments in `seg_id` order; snapshot segments reset state, delta segments apply upserts/deletes
- Compaction does **not** preserve old CONTENT_MAP_SEG segments byte-for-byte; it rewrites one canonical snapshot and discards superseded segments
- Fork scope grows slightly to include stricter header/entry validation and delta helpers in `rvf-types/src/content_map.rs`

## Consequences

### If Option 1 (`.access.db` only)
- Simple — no fork, no new file format
- `.access.db` loss = permanent content loss
- Durability regression vs upstream remains

### If Option 2 (CONTENT_MAP_SEG fork)
**Positive**
- `.access.db` loss downgraded to ranking degradation only — content recovered from `.rvf`
- Delete tombstones survive restart and compaction
- Per-write updates can be appended as small deltas instead of rewriting the full map each time

**Negative**
- Vendored fork creates permanent merge burden when syncing `ruvector-upstream`
- `.rvf` file is not encrypted — content privacy relies on OS file permissions
- Unless the caller flushes on each remember/forget, crash durability is only guaranteed at the documented checkpoints (`store.close()`, explicit flush, or pre-compaction snapshot), which is weaker than upstream SQLite's per-transaction durability
- CONTENT_MAP_SEG parsing is now strict: malformed headers, truncated entries, or mismatched counts fail visibly during extraction/open instead of being silently skipped

**Neutral**
- No public `RvfStore` API signatures change
- No callers outside `openfang-memory/src/store.rs` are affected

## Dependencies
- ADR-002 (design baseline)
- ADR-003 (migration contract — must be Accepted first)

## Related
- ADR-003 §7 — original CONTENT_MAP_SEG specification (moved here)
- **[SPEC-002](../specs/SPEC-002-content-durability.md)** *(Status: Active)* — struct layouts and method signatures
