# ADR-006: Episodic Memory Retention and Eviction Policy

**Status**: Accepted
**Date**: 2026-03-15
**Authors**: Daniel Alberttis

## Context

Long-running agents accumulate episodic memories without bound. The four memory scopes are:

| Scope | Value | Lifetime | Eviction |
|-------|-------|----------|----------|
| `Episodic` | `'episodic'` | Session fragments | Count + age thresholds (this ADR) |
| `Semantic` | `'semantic'` | Long-term knowledge | Never auto-evicted |
| `Procedural` | `'procedural'` | Learned skills/patterns | Never auto-evicted |
| `Working` | `'working'` | Current-task transient context | Always evicted (see below) |

`Semantic` and `Procedural` memories are long-term knowledge and should never be automatically evicted. `Episodic` memories (scope='episodic') are session fragments with a natural shelf life. `Working` memories (scope='working') are ephemeral task-scoped context that should be aggressively purged. Without a retention policy, the per-agent `.rvf` grows unboundedly and HNSW performance degrades.

ADR-003 Phase 1 does not include eviction. This ADR decides whether to add a configurable eviction policy and, if so, what the semantics are.

## Decision

**Note**: ADR-003 v1.8 and v1.9 resolved this in favour of adopting both count-based and age-based eviction (both defaulting to `None` / disabled). This ADR is the standalone decision record.

ADR-003 Phase 1c shipped age/count eviction. This ADR governed the acceptance gate -- **Phase 1c COMPLETE 2026-03-20** (159 tests passing, clippy clean, Sherlock verified).

If eviction is adopted:
- **Scope**: Episodic (scope='episodic') and Working (scope='working') entries. Semantic and Procedural memories are never evicted by these policies.
- **Trigger**: Kernel consolidation cycle (`consolidation_interval_hours` in `openfang.toml`) — same as `compact_if_needed`. `SemanticStore::new()` spawns nothing.
- **Count-based eviction** (`max_episodic_memories: Option<usize>`, default: `None`):
  - When side-store count of `scope='episodic'` entries exceeds threshold, evict oldest entries sorted by `accessed_at ASC, importance ASC` until within threshold (low-importance old entries evicted first)
  - `accessed_at` defaults to ingest timestamp (enforced in DDL: `DEFAULT (strftime("%s", "now"))`) — no NULL ambiguity
  - `store.delete(ids)` + side-store tombstone → `compact_if_needed()`
- **Age-based eviction** (`episodic_max_age_days: Option<u64>`, default: `None`):
  - Entries not accessed within N days are directly tombstoned on the consolidation cycle
  - Direct tombstone, not via confidence decay (decay has multiplicative floor of 0.1, never zeroes — unreliable for eviction timing)
- **Working memory eviction** (`max_working_memories: Option<usize>`, default: `None`; `working_memory_session_scoped: bool`, default: `true`):
  - `scope=3` entries are intended as ephemeral task context. When `working_memory_session_scoped = true`, all `scope=3` entries are tombstoned on `SessionEnd` regardless of count/age thresholds.
  - When `max_working_memories` is set, count-based eviction applies to `scope='working'` with a tighter ceiling than episodic (no age grace period — oldest-first immediate tombstone).
  - Working memory is never promoted to episodic automatically; callers must re-store with `scope='episodic'` if persistence is intended.
- **Per-entry TTL** (`expires_at: Option<u64>` in side-store DDL — Unix timestamp, default `NULL`):
  - Any entry with `expires_at IS NOT NULL AND expires_at < strftime('%s', 'now')` is tombstoned on the consolidation cycle, regardless of scope (including Semantic and Procedural — per-entry TTL overrides the "never evict" policy for those scopes).
  - TTL is set by callers at ingest time. Default is `NULL` (no expiry). Useful for time-sensitive context (e.g. meeting notes, access tokens) that would otherwise require manual `forget` calls.
  - TTL check runs before count/age eviction in the same consolidation pass.
- **Eviction sort order** (count-based): `ORDER BY importance ASC, accessed_at ASC` — lowest-importance, oldest-accessed entries evicted first.
- **CONTENT_MAP_SEG interaction**: tombstoned entries removed from in-memory `content_map` during pre-compaction flush in `compact_if_needed`

## Consequences

### If eviction is adopted
**Positive**
- Prevents unbounded episodic and working memory growth on long-running agents
- Count, age, and TTL thresholds are operator/caller-configurable and default to disabled (no behavior change unless opted in)
- Working memory session-scoped eviction keeps per-task context from leaking across sessions
- Per-entry TTL enables time-sensitive memories (tokens, meeting context) to self-expire without manual `forget` calls
- `importance` sort bias prevents accidental eviction of high-value episodic memories when threshold is reached

**Negative**
- Episodic memories that are still useful may be evicted if thresholds are set too aggressively
- Per-entry TTL overrides the "never evict" guarantee for Semantic/Procedural entries if callers set `expires_at` — callers must be careful
- Adds logic to the consolidation cycle — must not block on large side-stores
- `importance` column adds one `f32` per side-store row; negligible at scale but non-zero

**Neutral**
- Semantic and Procedural memories without `expires_at` are entirely unaffected
- Same consolidation scheduling as upstream OpenFang — no new background tasks
- Working memory session eviction fires on `SessionEnd` only — no new background task needed

## Dependencies
- ADR-003 (migration contract — must be Accepted first)
- ADR-004 (if CONTENT_MAP_SEG is used, its deletion/compaction semantics should be settled before implementing eviction)

## Related
- ADR-003 §1.5b — original episodic eviction specification (moved here)
- **[SPEC-004](../specs/SPEC-004-episodic-eviction.md)** *(Status: Active)* — `evict_memories` implementation (covers Episodic + Working + per-entry TTL)
