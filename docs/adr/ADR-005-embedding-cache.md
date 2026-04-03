# ADR-005: Embedding Generation Cache Policy

**Status**: Accepted
**Date**: 2026-03-15
**Authors**: Daniel Alberttis

## Context

Every call to `recall_with_embedding` or `remember_with_embedding` that generates an embedding calls `EmbeddingDriver::embed()` unconditionally. In agentic loops, agents repeat near-identical queries — "what do I know about X?" fires 3–5 times per session. Each miss is an API call (latency + cost). No caching exists in ADR-003 Phase 1.

The question is whether to add an LRU cache at the `SemanticStore` level, and if so, what the key derivation, scope, invalidation, and configuration policy should be.

## Decision

**Note**: ADR-003 v1.8 and v1.9 resolved this in favour of adopting the cache (with the config key corrected to `memory.embedding_cache_capacity` in v1.9). This ADR is the standalone decision record. The detail below reflects the resolved design.

ADR-003 Phase 1c shipped embedding LRU cache. This ADR governed the acceptance gate -- **Phase 1c COMPLETE 2026-03-20** (159 tests passing, clippy clean, Sherlock verified).

If caching is adopted:
- **Location**: LRU cache on `SemanticStore` in `engine.rs`
- **Key**: `SHAKE256("{dimension}:{content_bytes}")` — dimension prefix is mandatory to invalidate on model upgrade
- **Scope**: per-`SemanticStore` instance — prevents cross-agent timing side-channels
- **Capacity**: configurable via `memory.embedding_cache_capacity` (default: 1,000 entries)
- **Eviction**: LRU
- **Applied at**: `remember_with_embedding` (embedding content) and `recall_with_embedding` (embedding query). Not applied to `update_embedding` (pre-computed embedding supplied by caller).
- **Memory bound**: `dimension × capacity × 4 bytes` (e.g. 768-dim × 1,000 × 4 ≈ 3 MB)
- **Known limitation**: dimension prefix does not protect against same-dimension model upgrades. Mitigation: set `memory.embedding_cache_capacity = 0` during model transitions.
- **Implementation**: SHAKE256 already available via `rvf-crypto`. No new crate required.

## Consequences

### If caching is adopted
**Positive**
- Eliminates redundant `EmbeddingDriver::embed()` calls on repeated queries within a session
- Reduces API cost and latency proportionally to cache hit rate

**Negative**
- Same-dimension model upgrade silently serves stale embeddings until cache rolls over
- Adds per-agent memory overhead proportional to capacity × dimension

**Neutral**
- Per-instance scope is required for correctness — shared cache would require additional isolation

### If caching is not adopted
- No stale-embedding risk
- Every embed call hits the driver — cost and latency grow with query frequency

## Dependencies
- ADR-003 (migration contract — must be Accepted first)

## Related
- ADR-003 §8 — original embedding cache specification (moved here)
- **[SPEC-003](../specs/SPEC-003-embedding-cache.md)** *(Status: Active)* — `EmbeddingCache` struct and `embed_or_cached` signature
