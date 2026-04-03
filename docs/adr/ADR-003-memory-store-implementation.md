# ADR-003: Memory Store Implementation — SemanticStore to RvfStore Migration

**Status**: Accepted
**Date**: 2026-03-14
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-14 | Daniel Alberttis | Initial implementation contract. Per-agent store spec, shared store spec, SemanticStore API, module structure, implementation order. Supersedes the original ADR-003 that was voided by ADR-002 v0.3. |
| 0.2 | 2026-03-14 | Daniel Alberttis | Scope correction: rusqlite is retained. Only SemanticStore (4 methods on MemorySubstrate) is replaced. StructuredStore, SessionStore, UsageStore, PairedDeviceStore, TaskQueueStore, AuditLog are untouched. memory_store/memory_recall tools are KV ops on StructuredStore — not affected. |
| 0.3 | 2026-03-14 | Daniel Alberttis | Corrected §2.1 routing: shared.rvf is reached via `remember_with_embedding(shared_memory_agent_id(), ...)`, not via the memory_store/memory_recall tools. agent_id is the routing key, consistent with how OpenFang already routes KV to the shared namespace. |
| 0.4 | 2026-03-14 | Daniel Alberttis | Renamed internal store wrappers: AgentRvfStore → AgentSemanticStore, SharedRvfStore → SharedSemanticStore. Follows OpenFang {Domain}Store naming convention; removes implementation detail from type names. Updated all doc comments to reflect agent_id routing rather than tool routing. |
| 0.5 | 2026-03-14 | Daniel Alberttis | Renamed AgentMemoryEngine → SemanticStore. The public contract name on MemorySubstrate is unchanged — only the implementation switches from SQLite to RvfStore. Follows {Domain}Store convention used by all other stores. |
| 0.6 | 2026-03-14 | Daniel Alberttis | Completed type gap closure: (1) StoreRequest/RecallRequest are internal to engine.rs — removed from lib.rs public re-exports. (2) ranking.rs doc comment corrected — no ScoredMemory type; populates MemoryFragment.score/distance. (3) Phase 2 recall_shared_with_quality return type fixed from Vec<ScoredMemory> to Vec<MemoryFragment>. All ScoredMemory references eliminated. |
| 0.7 | 2026-03-14 | Daniel Alberttis | Changed AgentSideStore persistence from JSON file to SQLite (`.access.db`). Rationale: rusqlite already a dependency, WAL mode gives atomic targeted-row writes on the high-frequency recall path instead of full-file overwrite, consistent with existing operational stores pattern. Updated §1.4 spec, §1.6 lifecycle table, SemanticStore struct comments, and consequences section. |
| 0.8 | 2026-03-14 | Daniel Alberttis | Corrected consolidation.rs scheduling model. ADR-003 v0.1 described a self-scheduling Tokio background task spawned by SemanticStore::new(). Actual OpenFang pattern: ConsolidationEngine is a sync struct called by the kernel's background loop (consolidation_interval_hours in openfang.toml). Daxiom keeps this pattern — SemanticStore::new() does not spawn tasks. consolidation.rs gains new responsibilities (compact_if_needed, confidence decay on SideEntry.confidence_scaled) but remains kernel-driven. Updated §1.5, §2.2, §4 module comment. |
| 0.9 | 2026-03-14 | Daniel Alberttis | Added Phase 1 goal statement to §5 (explicit definition of done: all 7 ported tests + Phase 1c suite passing, O(log n) routing confirmed, all existing OpenFang integration tests pass unchanged, every agent using new SemanticStore). Renamed Phase 2 (enhanced shared store) to Phase 2 and inserted Phase 1c (hardening). Added §6 Testing Strategy: TDD approach with 9 minimum tests, stress test suite (1k/10k/100k vectors with p99 targets), 5 error handling/crash recovery scenarios, and regression gate (cargo test --workspace must pass clean). |
| 1.0 | 2026-03-14 | Daniel Alberttis | Source-verified cascade fixes from ADR-002 audit: §2.2 ResponseQuality::Approximate → Usable (actual variants: Verified/Usable/Degraded/Unreliable); §2.2 adaptive_n_probe field doesn't exist → quality_preference: QualityPreference::PreferQuality; Related section PLAN-001 reference updated (file was deleted, not yet rewritten). |
| 1.1 | 2026-03-15 | Daniel Alberttis | F1 design flaw cascade fix: `meta.rs` was a single file containing both per-agent constants (F_SCOPE=0) and shared-store constants (F_AGENT_ID=0, F_SCOPE=1) with identical numeric values at different semantics. Split into `agent_meta.rs` (per-agent field IDs) and `shared_meta.rs` (shared store field IDs). Updated diff surface list (§5), file count (7→8), and F_AGENT_ID attribution note. |
| 1.2 | 2026-03-15 | Daniel Alberttis | F3 fix: §7 Related section PLAN-001 entry updated from "(not yet written)" to a live link — docs/plans/PLAN-001-memory-store-phase1.md exists. |
| 1.3 | 2026-03-15 | Daniel Alberttis | F4 cross-reference: Phase 1 goal updated to include `sona_step()` no-op stub as an explicit acceptance criterion, with pointer to ADR-009 §6 Phase 0 for stub signature. |
| 1.4 | 2026-03-15 | Daniel Alberttis | S1 fix: §7 SPEC-001 Related entry now includes status indicator (Active). |
| 1.5 | 2026-03-15 | Daniel Alberttis | Content + UUID now stored in RVF metadata (F_CONTENT, F_UUID). `content` column removed from `.access.db` DDL. UUID bidirectional mapping simplified — u64→uuid reverse lookup eliminated (UUID returned via F_UUID from query results). `.access.db missing` failure mode downgraded from data loss to degraded ranking only. §1.2, §1.4, §6.3, Consequences updated. SPEC-001 §1.2–§1.4 + module map updated in lockstep. |
| 1.6 | 2026-03-15 | Daniel Alberttis | Reverted v1.5. Sherlock investigation confirmed `SearchResult` returns only `id: u64` + `distance: f32` — no metadata. RVF metadata is filter-only (write-in, never returned via any public API). F_CONTENT and F_UUID cannot be read back post-ingest. Original `.access.db` design was correct: content + uuid↔u64 mapping live in side-store. "Content not recoverable" failure mode in §6.3 restored. SPEC-001 reverted in lockstep. |
| 1.7 | 2026-03-15 | Daniel Alberttis | CONTENT_MAP_SEG fork decision. Root problem: `.access.db` loss = permanent content loss — `side_store.content` was the only durably stored copy of content text. Decision: fork vendored `rvf-runtime` to add a `ContentMap = 0x12` custom segment type storing a `HashMap<u64, Vec<u8>>` (vec_id → UTF-8 content bytes) inside the `.rvf` file. Compaction automatically preserves it — `scan_preservable_segments()` exclude-list (Vec/Manifest/Journal only) leaves all other types intact; zero compaction code changes needed (~238 lines across 4 files). Impact: `content` column removed from `.access.db` DDL; content moves to in-memory `content_map` (flushed to `.rvf` on close/compact); `.access.db` loss downgraded to ranking degradation only — content recoverable from CONTENT_MAP_SEG in `.rvf`. §1.4 DDL, §6.3 failure table, and Consequences updated. SPEC-001 §0 added. |
| 1.8 | 2026-03-15 | Daniel Alberttis | Three Phase 1c additions: §1.5b Age/Count Eviction (episodic memory ceiling + direct tombstone for age eviction; tombstoned entries purged from in-memory content_map on compact_if_needed); §8 Embedding LRU Cache (SHAKE256 key includes dimension prefix to prevent stale hits after model upgrade; per-instance scope to prevent cross-agent timing side-channels); §9 Session Witness Chain (create_witness_chain() confirmed in vendored rvf-crypto; embed in WITNESS_SEG on store.close()/SessionEnd; additive to per-query query_audited on shared.rvf). |
| 1.9 | 2026-03-15 | Daniel Alberttis | Review fixes: (1) §8 config key corrected from `ruvllm.embedding_cache_capacity` → `memory.embedding_cache_capacity` — embedding cache is a memory-layer concern, not LLM routing. (2) §1.5b `accessed_at` default specified — new entries default to ingest timestamp; NULL sort behaviour eliminated. (3) §6.1 minimum test set extended with 5 tests covering §1.5b (count + age eviction), §8 (cache hit + dimension invalidation), §9 (chain verifiability). (4) Consequences Positive extended with cache, eviction, and witness chain benefits. (5) §8 same-dimension model upgrade noted as known limitation. |
| 2.0 | 2026-03-15 | Daniel Alberttis | Status promoted to Accepted. §9 updated with two findings from source audit: (1) `embed_witness_chain` confirmed absent in vendored rvf-runtime — fork required (previously stated as "verify; if absent fork", now confirmed absent). (2) Wire format conflict documented: per-query WITNESS_SEGs (`append_witness` / `write_path.rs` format, variable-width) and session-chain WITNESS_SEGs (`rvf-crypto` format, fixed 73 bytes/entry) share discriminant 0x0A but are incompatible — `rvf verify-witness` CLI only validates the session-chain format; chain-break warnings on `shared.rvf` must not be interpreted as data corruption. |
| 2.2 | 2026-03-15 | Daniel Alberttis | §1.6 lazy-open note expanded: "lazy-open on first `remember`" clarified to include that existing `.rvf` files are opened eagerly on `MemorySubstrate` construction, and that `recall` before any `remember` on a non-existent store returns empty without creating a file. Removes ambiguity about the open vs create trigger. |
| 2.1 | 2026-03-15 | Daniel Alberttis | Internal consistency fixes from cross-doc alignment audit: (1) §4 "Eight new files" → "Seven new files" (lib.rs is updated, not new). (2) §5 "Four phases" → "Five phases" (Phase 0, 1a, 1b, 1c, 2 = five). (3) §1.6 lazy-open trigger corrected: "first `remember`" → "first use (either `remember` or `recall`)" — matches §6.3 which assumes opening on recall. (4) §6.1 `RvfStore` described as "trait" → correctly identified as concrete struct defined in `vendor/rvf/rvf-runtime/src/store.rs`. (5) §6.3 error type corrected: `MemoryError::StoreCorrupt` (does not exist) → `OpenFangError::Memory`. (6) §8 SPEC-001 cross-reference corrected: `§8` → `§7` (EmbeddingCache is in SPEC-001 §7). (7) §9 SPEC-001 cross-reference corrected: `§9` → `§8` (SessionAuditBuffer is in SPEC-001 §8). (8) Consequences `AuditLog::append` → `AuditLog::record` (actual method name in `openfang-runtime/src/audit.rs`). (9) Related PLAN-001 description scoped to "Phase 0, 1a, and 1b only" — PLAN-001 does not cover Phase 1c or Phase 2. |
| 2.3 | 2026-03-17 | ruvector-upstream sync | ruvector-upstream sync: `RvfStoreBackend<B>` and `RvfManifestMiddleware` now defined upstream; two DECISION markers added for `openfang-extensions` path routing and `openfang-skills` hot-reload adoption. |

---

## Context

ADR-002 established the dual-layer architecture (`{agent_id}.rvf` + `shared.rvf`) and the metadata field schemas. It did not specify the exact method-level mapping from the existing `SemanticStore` API to `RvfStore` calls, the vector ID scheme, the `access_count` side-store, the `update_embedding` delete-reingest cycle, the compact trigger policy, or the Phase 2 shared-store capabilities. This ADR provides those specifications. A developer should be able to implement `store.rs` and `engine.rs` directly from this document without guessing.

### North Star: swap the storage engine, preserve the framework

**The single guiding principle for every decision in this ADR:**

> Swap the vector storage engine. Touch nothing else.

OpenFang's framework — its scheduling model, configuration system, public API contracts, observability, and operational stores — is sound. The only problem worth solving is that `SemanticStore`'s vector search is O(n) and degrades silently at scale. RVF + HNSW fixes that problem. Everything else stays.

In practice this means:

| Layer | Owner | Rule |
|-------|-------|------|
| `MemorySubstrate` public API | OpenFang | **Unchanged.** All callers in `kernel.rs` and `agent_loop.rs` see nothing different. |
| `Memory` trait | OpenFang | **Unchanged.** Method signatures, return types, error types. |
| `ConsolidationEngine` scheduling | OpenFang kernel | **Unchanged.** `consolidation_interval_hours` config still controls it. `SemanticStore::new()` spawns nothing. |
| `EmbeddingDriver` trait | OpenFang | **Unchanged.** Injected at construction; the backend is irrelevant to this ADR. |
| All 6 operational stores | OpenFang | **Unchanged.** `rusqlite` is retained for all of them. |
| `SemanticStore` internals | Daxiom | **Replaced.** SQLite BLOB → RVF + HNSW. New side-store for mutable fields. |
| `MemoryFragment` | Daxiom | **Extended only.** `score` + `distance` fields added. All existing fields unchanged. |

This boundary also limits upstream merge risk. If OpenFang publishes a new release, the diff surface is confined to `semantic.rs`, `store.rs`, `engine.rs`, `agent_meta.rs`, `shared_meta.rs`, `ranking.rs`, and `consolidation.rs`. The kernel, runtime, and all other crates remain mergeable without conflict.

### Scope boundary — what this ADR does NOT touch

`rusqlite` is retained. The following six operational stores are **unchanged** (four live in `openfang-memory`; `A2aTaskStore` and `AuditLog` live in `openfang-runtime` — none are touched by this ADR):

| Store | Owned by | Why untouched |
|-------|----------|---------------|
| `StructuredStore` | `MemorySubstrate` | KV and agent registry — relational, not vector |
| `SessionStore` | `MemorySubstrate` | Channel sessions, canonical sessions — sequential, not vector |
| `UsageStore` | `MemorySubstrate` | Token metering — aggregation queries, not vector |
| Paired-device ops | `MemorySubstrate` (inline SQL, no named struct) | Device pairing — simple KV, not vector |
| A2A task queue | `openfang-runtime` (`A2aTaskStore` in `a2a.rs` — not in `openfang-memory`) | Queue semantics, not vector |
| `AuditLog` | `openfang-runtime` (not `openfang-memory`) | Append-only audit log — not vector |

The `memory_store` and `memory_recall` tools (`kernel.rs:5423` / `kernel.rs:5430`) call `structured_set` / `structured_get` on `StructuredStore`. They are KV operations, not vector operations. They are not affected.

`MemorySubstrate`'s public API signature does not change. Internally, the four vector methods (`remember_with_embedding`, `recall_with_embedding`, `update_embedding`, `forget`) are handled by the new `SemanticStore` implementation backed by `RvfStore` instead of SQLite. All other 40+ methods continue delegating to the SQLite connection.

### What is being replaced

`crates/openfang-memory/src/semantic.rs` — `SemanticStore` backed by SQLite via `rusqlite`. The four vector methods exposed on `MemorySubstrate`:

| Method | Behaviour being replaced |
|--------|--------------------------|
| `remember_with_embedding(agent_id, content, source, scope, metadata, embedding)` | `INSERT INTO memories` with optional BLOB |
| `recall_with_embedding(query, limit, filter, query_embedding)` | Fetch up to `limit * 10` rows, sort by cosine in Rust, truncate — O(n) |
| `forget(id)` | `UPDATE memories SET deleted = 1` |
| `update_embedding(id, embedding)` | `UPDATE memories SET embedding = blob` |

Note: `remember` (no embedding) calls `remember_with_embedding` with `embedding: None` — same replacement path.

The critical performance fault is in `recall_with_embedding`: it fetches `(limit × 10).max(100)` rows via SQL then re-ranks by cosine similarity in Rust. Latency is O(n) with respect to memory count. The dynamic fetch cap silently degrades recall quality once total memories exceed the cap.

---

## Decision

> **Implementation contract** — struct definitions, method signatures, SQL schemas, RVF import lists, and implementation order — are in **[SPEC-001](../specs/SPEC-001-memory-store.md)**. This ADR records what was decided and why; SPEC-001 records what to build. If they conflict, this ADR is authoritative and SPEC-001 must be updated first.

### 1. Per-Agent Store

#### 1.1 Distance metric

**Decision**: `DistanceMetric::Cosine`.

The existing `SemanticStore` ranks by cosine similarity. Using `Cosine` in `RvfStore` means the HNSW index is built and queried using cosine distance natively — no Rust-side re-rank step, and the stored `distance` in `SearchResult` is directly comparable across queries.

`DistanceMetric::L2` appears in ruvector reference examples because those use synthetic random vectors where L2 is adequate for illustration. OpenFang memory embeddings are normalized LLM vectors — cosine is the correct metric.

#### 1.2 Vector ID scheme

`MemoryId` is a `uuid::Uuid` (128-bit); `RvfStore` IDs are `u64` (64-bit). UUID cannot be truncated without collision risk.

**Decision**: per-store monotonic `u64` counter stored in the side-store `meta` table. Each `remember` call increments the counter and assigns the next value. `MemoryId` UUID is the canonical key in the side-store; `u64` is the RVF vector ID. Bidirectional map (UUID↔u64) enables delete/update_embedding by MemoryId and result hydration on recall.

Collision-free within a single `.rvf` file (counter is per-store). See SPEC-001 §1.2 for the full side-store mapping.

#### 1.3 Method mapping — per-agent store

| OpenFang method | RVF call | Notes |
|-----------------|----------|-------|
| `remember` / `remember_with_embedding` | `ingest_batch` (1 vector) | `embedding: None` → zero vector; `has_embedding = 0` in side-store |
| `recall` / `recall_with_embedding` | `store.query` | Requires `query_embedding` — `engine.rs` calls embedder if not supplied. No LIKE fallback in Phase 1. |
| `forget` | `store.delete` + side-store tombstone | `store.delete` is primary; `F_DELETED=1` metadata is set only on the subsequent reingest path |
| `update_embedding` | `store.delete` then `store.ingest_batch` at same `u64` ID | Held under per-store `Mutex` — atomic from caller's perspective. Crash recovery: `consolidation.rs` scans tombstoned IDs with `has_embedding=1` and reissues reingest |

`MemoryFilter.agent_id` is ignored in the per-agent `build_filter` — the `.rvf` file is already scoped to a single agent. If `agent_id` is set to a different agent, `engine.rs` returns empty without querying.

See SPEC-001 §1.3 for all `ingest_one`, `build_filter`, and `update_embedding_one` signatures.

#### 1.4 Access side-store

**Decision**: per-agent SQLite (`.access.db`, WAL mode) alongside each `.rvf` file.

**Why not RVF metadata**: RVF metadata is write-once per ingest. Updating `access_count` / `accessed_at` on every recall would require a delete-reingest cycle — unacceptable on the hot read path.

**Why SQLite over JSON**: WAL mode provides atomic targeted-row updates; a JSON full-file overwrite on every recall is crash-unsafe and slower. `rusqlite` is already a dependency (retained for the six operational stores), so no new build requirements.

**Why not a single shared DB**: per-agent files follow the same pattern as per-agent `.rvf` files — closed when the agent is idle, opened on first use, no contention across agents.

Files: `~/.openfang/agents/{agent_id}.access.db` and `~/.openfang/shared.access.db` (overridable via `OPENFANG_HOME` env var). The shared side-store adds an `agent_id` column for provenance and `MembershipFilter` construction (Phase 2).

**Additional side-store columns (Phase 1c)**:
- `importance REAL NOT NULL DEFAULT 0.5` — caller-supplied priority weight in `[0.0, 1.0]`. Biases eviction order: low-importance entries evicted before high-importance ones at the same age. Default 0.5 (neutral). Does not affect recall ranking (ADR-011 Constraint 2).
- `expires_at INTEGER DEFAULT NULL` — Unix timestamp. If set and `expires_at < now`, the entry is tombstoned on the next consolidation cycle regardless of scope. Enables time-sensitive memories to self-expire without manual `forget` calls (see ADR-006).

See SPEC-001 §1.4 for the full DDL schema and write-policy table.

#### 1.5 Compact trigger policy

**Decision**: compact when `dead_space_ratio > 0.10` OR absolute tombstone count `> 500`. Checked inline after `forget_one` and `update_embedding_one`; also on the kernel consolidation cycle.

Threshold rationale: below 10% tombstones the HNSW routing overhead is negligible; above 10% routing degrades faster than disk is wasted. The 500-tombstone floor catches edge cases where a small store has proportionally many tombstones but `dead_space_ratio` stays low.

Triggered by `ConsolidationEngine` (kernel-driven, same scheduling as upstream OpenFang) — not by `SemanticStore::new()`. See SPEC-001 §1.5 for `compact_if_needed` implementation.

#### 1.5b Age/Count Eviction (Phase 1c)

Checked on the kernel consolidation cycle (same cadence as `compact_if_needed`). Both thresholds are configurable in `openfang.toml`; both default to `None` (disabled).

**Scope constants** (named in `agent_meta.rs`):

| Constant | Value | Lifetime |
|----------|-------|----------|
| `SCOPE_EPISODIC` | `'episodic'` | Session fragments — evictable |
| `SCOPE_SEMANTIC` | `'semantic'` | Long-term knowledge — never auto-evicted |
| `SCOPE_PROCEDURAL` | `'procedural'` | Learned skills/patterns — never auto-evicted |
| `SCOPE_WORKING` | `'working'` | Current-task transient context — session-evicted (see ADR-006) |

Only `Episodic` (scope='episodic') and `Working` (scope='working') entries are subject to eviction — `Semantic` and `Procedural` memories are long-term knowledge and are never evicted by these policies (unless per-entry `expires_at` is set — see ADR-006).

**`max_episodic_memories: Option<usize>`** — default: `None`
When the side-store count of `scope='episodic'` entries exceeds this threshold, evict the oldest entries (sorted by `accessed_at ASC`) until the count is within the threshold. Eviction: `store.delete(ids)` + side-store tombstone. Triggers `compact_if_needed` after bulk deletion.

`accessed_at` defaults to the ingest timestamp for new entries (enforced in SPEC-001 §1.4 DDL as `DEFAULT (strftime('%s', 'now'))`). This means entries that have never been recalled sort before entries that have — the sort is always well-defined with no NULL ambiguity.

```sql
SELECT vec_id FROM side_store WHERE scope = 'episodic' ORDER BY importance ASC, accessed_at ASC LIMIT ?
```

**`episodic_max_age_days: Option<u64>`** — default: `None`
Episodic entries not accessed within N days are **directly tombstoned** on the consolidation cycle — not via the confidence decay path. The existing decay pass applies a multiplicative floor of 0.1 and never zeroes entries; relying on it for eviction would require two consolidation cycles and produce unpredictable timing. Direct tombstone is consistent with count eviction and fires in a single pass.

Check: `WHERE scope = 'episodic' AND accessed_at < (now - N days)` → `store.delete(ids)` + side-store tombstone.

**CONTENT_MAP_SEG interaction**: tombstoned entries are removed from the in-memory `content_map` during the pre-compaction flush in `compact_if_needed` (which filters tombstoned IDs before writing the canonical `CONTENT_MAP_SEG`). No separate cleanup step is needed.

See SPEC-001 §1.5b for the full `evict_episodic` implementation.

#### 1.6 Store lifecycle

`AgentSemanticStore` holds `store: Option<RvfStore>` — lazy-open on first `remember` (creates `.rvf` + `.access.db`). Existing `.rvf` files are opened eagerly on `MemorySubstrate` construction. `recall` before any `remember` on a non-existent store returns empty — no file created. No eager creation on `MemorySubstrate` construction. `store.close()` on drop flushes HNSW index and manifest to disk. See SPEC-001 §1.6 for the full lifecycle table.

---

### 2. Shared Store

#### 2.1 Phase 1 — baseline

Structurally identical to the per-agent store with two differences:

1. Metadata adds `F_AGENT_ID` (field 0, String) for provenance — defined in `shared_meta.rs`. Other field IDs shift by 1: all shared constants live in `shared_meta.rs` (not `agent_meta.rs`) to prevent cross-store constant aliasing.
2. All recalls use `query_audited` — not plain `query`.

**Routing key**: `agent_id`. When `agent_id == shared_memory_agent_id()` (`00000000-0000-0000-0000-000000000001`), `SemanticStore` routes to `store_shared` / `shared.rvf`. Any other `agent_id` routes to the per-agent store. `memory_store` / `memory_recall` tools are not involved — they call `structured_set` / `structured_get` on `StructuredStore` (unchanged).

**Why `query_audited` for shared, not per-agent**: per-agent recall is auditable via the session log; per-recall SHAKE256 witness overhead on high-frequency per-agent stores is not justified. The shared store is a cross-agent knowledge surface — every read must produce a tamper-evident record. The audit cost is justified by the trust boundary.

See SPEC-001 §2.1 for `ingest_shared` and `recall_shared` signatures.

#### 2.2 Phase 2 — Enhanced shared store

Additive to Phase 1. Gated by `features = ["shared-phase2"]` so Phase 1 is independently deployable.

**`query_with_envelope`**: returns `ResponseQuality` (`Verified` / `Usable` / `Degraded` / `Unreliable`). On `Degraded`: retry with `quality_preference: QualityPreference::PreferQuality`; if still degraded, merge with per-agent store results ranked by `fragment.score`.

**`MembershipFilter`**: dense bitmap gating visibility in HNSW traversal. Policy table in agent capability manifest maps `(caller_agent_id, excluded_writer_agent_id)` pairs. Rebuilt when the side-store records a new vector from an excluded writer. `filter.bump_generation()` prevents stale-filter replay.

**Adversarial detection**: after every `query_with_envelope`, run `is_degenerate_distribution` on the distance array (uniform CV = query-poisoning signature). On detection: retry with wider probe; if still degenerate, return empty results + emit `MemoryEvent::AdversarialDetected`.

**DoS hardening**: `BudgetTokenBucket` per caller at 10,000 tokens/second; each recall costs `k × log2(total_vectors)` tokens. `NegativeCache`: 3 consecutive `AdversarialDetected` events for the same query signature → blacklist for 60 seconds (immediate empty return, no HNSW touch).

**`derive` / COW branch**: `freeze()` then `derive()` for read-only snapshots; `freeze()` then `branch()` for COW staging. `freeze()` makes the live store temporarily read-only (<1ms). COW promotion is a manual operator action.

**Hybrid BM25+vector retrieval** (Phase 2, deferred): add a `recall_hybrid` path that runs vector ANN search and BM25 keyword search in parallel, then fuses results with reciprocal rank fusion. Motivation: pure cosine similarity degrades on sparse queries — exact identifiers, code symbols, agent names. BM25 fills the lexical gap. Implementation: a `BM25Index` per-store built over stored content text; `HybridSearch` fuses ranked lists from both channels. Not gated by `shared-phase2` feature flag — applies equally to per-agent and shared stores. Deferred to a Phase 2 sub-ADR once Phase 1 is complete.

See SPEC-001 §2.2 for all Phase 2 signatures and implementation details.

---

### 3. `SemanticStore` Public API

Six async methods. Public contract on `MemorySubstrate` is **unchanged**; only the four vector methods are rerouted internally.

| Method | Stores | Phase |
|--------|--------|-------|
| `store` / `forget` / `recall` / `update_embedding` | `agent_store` | 1a |
| `store_shared` / `recall_shared` | `shared_store` | 1b |

**Return type**: `MemoryFragment` extended with `score: f32` and `distance: f32` (Phase 1a) — not a new `ScoredMemory` type. Rationale: `MemoryFragment` already carries 95% of the fields; extending avoids a conversion layer and zero caller churn in `openfang-runtime`. `quality: Option<ResponseQuality>` is Phase 2 only (shared-store envelope — irrelevant until `query_with_envelope` is implemented).

`StoreRequest` and `RecallRequest` are internal to `engine.rs` — not re-exported from `lib.rs`.

See SPEC-001 §3 for the full struct definition and method signatures.

---

### 4. Module File Structure

Seven new files in `crates/openfang-memory/src/`: `engine.rs`, `store.rs`, `agent_meta.rs`, `shared_meta.rs`, `ranking.rs`, `consolidation.rs`, `audit.rs` (plus updated `lib.rs`). Note: `agent_meta.rs` holds per-agent field ID constants; `shared_meta.rs` holds shared store field ID constants — two separate modules to prevent silent constant aliasing at field slot 0.

Files removed: `semantic.rs` only. Files modified: `substrate.rs` (gains `SemanticStore` field; four vector methods rerouted). Files unchanged: `session.rs`, `usage.rs`, `structured.rs`, `migration.rs`, `knowledge.rs`.

Note: `knowledge.rs` (`KnowledgeStore` — entity/relation/graph store) is **not removed**. It is a separate operational store with no vector involvement. Removing it would violate the North Star constraint and break `substrate.rs`. It is untouched by this ADR.

See SPEC-001 §4 for the complete file tree with RVF import lists per file.

---

### 5. Implementation Order

Five phases: **Phase 0** (vendor RVF crates, extend `MemoryFragment`, update `substrate.rs`) → **Phase 1a** (per-agent store + 7 ported tests) → **Phase 1b** (shared store + audited recalls) → **Phase 1c** (hardening, stress tests, error handling) → **Phase 2** (enhanced shared store, feature-flagged).

**Phase 0 step 0 — rust-version**: `Cargo.toml` already declares `rust-version = "1.87"` (verified 2026-03-15). No action needed. `rvf-runtime` requires `1.87`; the workspace is already compliant.

**Phase 1 goal** (definition of done before Phase 2 work begins):

> `cargo test -p openfang-memory` passes with all 7 ported `SemanticStore` tests and the full Phase 1c test suite. `recall_with_embedding` routes through RVF + HNSW (O(log n)) instead of the O(n) SQLite scan. Per-agent `.rvf` / `.access.db` files are created on first use. All existing OpenFang integration tests pass unchanged — confirming the kernel, runtime, and all 6 operational stores are untouched. Every agent that calls `remember_with_embedding` or `recall_with_embedding` is routed through the new `SemanticStore` implementation. **`sona_step()` no-op stub must be present in `consolidation.rs` and called from `engine.rs` after every recall** — required by ADR-009 Phase 0 (see ADR-009 §6 for exact signature).

See SPEC-001 §5 for complete per-phase step lists and acceptance criteria.

---

### 6. Testing Strategy

#### 6.1 TDD approach

Phase 1a and 1b are written test-first (London School): tests for each of the four vector methods are written before the implementation. The mock boundary is `RvfStore` (a concrete struct in `vendor/rvf/rvf-runtime/src/store.rs`) — tests construct a `SemanticStore` with a test double substituted at the `RvfStore` boundary and assert behavior at the `MemorySubstrate` API level, not at the RVF wire level.

**Minimum test set (must pass before any phase is merged)**:

| Test | Phase | What it proves |
|------|-------|----------------|
| `test_remember_creates_rvf_entry` | 1a | `ingest_batch` is called; side-store row created with correct `uuid↔u64` mapping |
| `test_recall_returns_ranked_fragments` | 1a | `store.query` result hydrated into `MemoryFragment` with `score` + `distance` |
| `test_forget_tombstones_and_deletes` | 1a | `store.delete` + side-store `tombstoned=1` set; recalled vector is absent |
| `test_update_embedding_replaces_vector` | 1a | delete-then-reingest at same `u64` ID; new vector returned on recall |
| `test_agent_id_routing_isolation` | 1a | Agent A's vectors are not visible to Agent B |
| `test_remember_no_embedding_zero_vector` | 1a | `embedding: None` → zero vector stored; `has_embedding=0` in side-store |
| `test_shared_store_audited_recall` | 1b | `query_audited` called for shared; per-agent uses plain `query` |
| `test_kernel_memory_ops_unchanged` | 1b | Existing `memory_store`/`memory_recall` tool calls still route to `StructuredStore`, not `SemanticStore` |
| `test_operational_stores_unaffected` | 1b | `SessionStore`, `UsageStore`, and `StructuredStore` operations pass after the RVF swap |
| `test_episodic_count_eviction` | 1c | Oldest episodic entries evicted when `max_episodic_memories` ceiling hit; Semantic and Procedural entries untouched |
| `test_episodic_age_eviction` | 1c | Entries older than `episodic_max_age_days` tombstoned in one consolidation pass; never-recalled entries (accessed_at = ingest time) evict correctly |
| `test_embedding_cache_hit_skips_driver` | 1c | Cache hit on second call with same content returns same vector; `EmbeddingDriver::embed()` called exactly once |
| `test_embedding_cache_dimension_prefix_invalidation` | 1c | Cache key `"768:…"` ≠ `"1536:…"` — changing dimension produces a cache miss, not a stale hit |
| `test_session_witness_chain_verifiable` | 1c | Chain produced on `SessionEnd` passes `verify_witness_chain()` with correct event count and order |

#### 6.2 Stress tests (Phase 1c)

Goal: confirm HNSW performance holds at the scale OpenFang is expected to run.

| Test | Vector count | Acceptance criterion |
|------|-------------|----------------------|
| `stress_recall_1k` | 1,000 | `recall_with_embedding` p99 < 5ms |
| `stress_recall_10k` | 10,000 | p99 < 10ms |
| `stress_recall_100k` | 100,000 | p99 < 50ms (single agent, in-memory HNSW) |
| `stress_concurrent_agents` | 10 agents × 1k vectors | No cross-agent contamination; no deadlock under concurrent recall |

These are run with `cargo test --release -p openfang-memory -- --ignored stress` (ignored by default, opt-in in CI).

#### 6.3 Error handling and crash recovery (Phase 1c)

| Scenario | Expected behavior |
|----------|-------------------|
| Process killed between `store.delete` and `store.ingest_batch` in `update_embedding` | On next startup, `consolidation.rs` scans side-store for rows where `tombstoned=1` AND `has_embedding=1`; reissues reingest. Vector is restored. |
| `.access.db` missing on recall | `AgentSemanticStore` recreates the side-store from the `.rvf` manifest on open. All `access_count` values reset to 0; uuid↔u64 map rebuilt from manifest. **Content recovered from CONTENT_MAP_SEG in the `.rvf` file** via `extract_content_map()`. Ranking degradation only — no permanent data loss. |
| `.rvf` file present but corrupt / wrong magic | `RvfStore::open` returns `Err`; `MemorySubstrate` surfaces `OpenFangError::Memory`. Agent receives an error, not a panic. |
| `recall_with_embedding` called before any `remember` | Empty `Vec<MemoryFragment>` returned. No file created. |
| Side-store `next_id` counter missing from `meta` table | `engine.rs` initialises counter to `1` and continues. |

#### 6.4 Regression gate

Before Phase 1 is declared complete, the full OpenFang test suite must pass without modification:

```bash
cargo test -p openfang-runtime   # kernel + agent loop integration
cargo test -p openfang-memory    # all memory tests including phase 1c suite
cargo test --workspace           # full workspace — no regressions elsewhere
```

No test file outside `crates/openfang-memory/` should need to change. If any does, that is a North Star violation and must be resolved before merging.

---

### 7. RVF Fork: CONTENT_MAP_SEG

#### 7.1 Problem: Content Durability Regression

The design through v1.6 stored memory content text (`MemoryFragment.content`) in the `side_store.content` column of the per-agent `.access.db` SQLite file. This created a durability regression relative to upstream OpenFang, which stores content in the always-open `openfang.db`.

**Failure scenario**: if `{agent_id}.access.db` is deleted or corrupted:

1. The `.rvf` file survives — vectors and HNSW index are intact.
2. `AgentSemanticStore` recreates the side-store from the `.rvf` manifest.
3. `access_count` values reset to 0.
4. Every `MemoryFragment.content` is empty. All readable memory text is permanently gone.

The agent has a healthy vector index but no readable content. This is a significant regression from the upstream `memories` table, which is a core application table with the usual database backup and recovery protections.

#### 7.2 Decision: Fork rvf-runtime

**Fork the vendored `rvf-runtime` crate** (at `vendor/rvf/rvf-runtime/`) to add a new segment type `CONTENT_MAP_SEG (0x12)`. This segment stores a serialized `HashMap<u64, Vec<u8>>` (vec_id → UTF-8 content bytes) inside the `.rvf` file itself, eliminating the durability regression.

**Options evaluated and rejected**:

| Option | Reason rejected |
|--------|----------------|
| Accept `.access.db` as sole content store | Durability regression remains: `.access.db` loss = permanent content loss |
| Switch to `ruvector-core` | Loses persisted HNSW, SHAKE256 witness chains, COW branching — all required features per ADR-002 |
| Hybrid: ruvector-core for content read-back | Two vector backends; split ownership between two incompatible crate APIs; adds complexity for a column migration |
| Fork rvf-runtime | **~238 lines, no compaction changes, no upstream API breakage, fully reversible** |

The fork is minimal and self-contained. No public `RvfStore` API signatures change. No callers outside `store.rs` are affected. Future `ruvector-upstream` sync: apply the 4-file diff; no merge conflict risk on any public interface.

#### 7.3 CONTENT_MAP_SEG Wire Format

**Segment discriminant**: `ContentMap = 0x12`.

This value is currently unassigned in `rvf-types/src/segment_type.rs`. The range `0x12–0x1F` is empty (`Dashboard = 0x11`, next used value is `CowMap = 0x20`). The existing test `assert_eq!(SegmentType::try_from(0x12), Err(0x12))` confirms 0x12 is free — this assertion must be removed and replaced with `Ok(SegmentType::ContentMap)` as part of the fork.

**Header struct** (`vendor/rvf/rvf-types/src/content_map.rs`):

```
Magic: "RVCM" → CONTENT_MAP_MAGIC = 0x5256_434D

ContentMapHeader (64 bytes, all fields little-endian):
  0x00  content_map_magic  u32    0x5256434D
  0x04  header_version     u16    1
  0x06  compression        u16    0 = none (reserved for future gzip/brotli)
  0x08  entry_count        u64    number of entries
  0x10  entries_size       u64    total byte length of serialized entries (not including header)
  0x18  reserved           [u8;40] zero
  Total: 4+2+2+8+8+40 = 64 ✓
```

**Serialized entry format** (variable length, one per map entry):

```
vec_id       8 bytes   u64 LE
content_len  4 bytes   u32 LE
content      content_len bytes   UTF-8, no NUL terminator
```

See SPEC-001 §0.1 for the full `ContentMapHeader` struct, `to_bytes()`, `from_bytes()`, `serialize_entries()`, and `deserialize_entries()` implementations.

#### 7.4 Why Compaction Requires No Changes

`scan_preservable_segments()` in `store.rs` (function declaration at line 1974; exclusion check at lines 2018–2023) uses an **exclude-list**:

```rust
// Skip Vec, Manifest, and Journal segments -- these are
// reconstructed by the compaction logic itself.
if seg_type != SegmentType::Vec as u8
    && seg_type != SegmentType::Manifest as u8
    && seg_type != SegmentType::Journal as u8
{
    results.push((i, seg_id, payload_len, seg_type));
}
```

All segment types not in `{Vec=0x01, Manifest=0x05, Journal=0x04}` are preserved byte-for-byte. The doc comment above this function explicitly states: *"This ensures forward compatibility: segment types unknown to this version of the runtime (e.g., Kernel, Ebpf, or vendor extensions) survive a compact/rewrite cycle byte-for-byte."*

`ContentMap (0x12)` is not in the exclude list. The compaction loop (lines 740–771) copies it byte-for-byte to the compacted output. Zero changes to the compaction algorithm are required.

Multiple CONTENT_MAP_SEG segments may accumulate between compaction cycles (one per `store.close()` call). `extract_content_map()` merges all present segments by scanning forward — later entries for the same `vec_id` overwrite earlier ones. On `compact_if_needed()`, a fresh canonical segment is written covering all live (non-tombstoned) entries before compaction runs; the compaction then preserves that canonical segment.

#### 7.5 Impact on `.access.db`

`content` column is **removed** from the `side_store` DDL. Content text lives in memory during an active session (`AgentSideStore.content_map: HashMap<u64, Vec<u8>>`) and is written to a `CONTENT_MAP_SEG` in the `.rvf` file on:

- `store.close()` — flush on normal shutdown
- `compact_if_needed()` — flush before compacting (also filters tombstoned entries from the map)

On `store.open()`, `extract_content_map()` reloads the in-memory `content_map` from all CONTENT_MAP_SEG segments in the `.rvf` file.

**Updated failure modes**:

| Failure | Before fork (v1.6) | After fork (v1.7) |
|---------|--------------------|--------------------|
| `.access.db` missing | `access_count` resets to 0; **content permanently lost** | `access_count` resets to 0; **content recovered from CONTENT_MAP_SEG** |
| `.rvf` file corrupt or missing | Vectors and HNSW lost | Vectors, HNSW, **and content** lost (content is inside `.rvf`) |
| Process crash before `store.close()` | In-memory content since last open; `.access.db` had content intact | In-memory content since last open; `.rvf` has content as of last `close()` or `compact_if_needed()` |

`.access.db` loss is **ranking degradation only** (access_count resets; uuid↔u64 map rebuilt from manifest), not content loss.

#### 7.6 Fork Scope — Exact Files Changed

Total: ~238 lines across 4 files. No public `RvfStore` API signatures change. No callers outside `openfang-memory/src/store.rs` are affected.

| File | Change | ~Lines |
|------|--------|--------|
| `vendor/rvf/rvf-types/src/content_map.rs` | **New file**: `CONTENT_MAP_MAGIC`, `ContentMapHeader` struct, `to_bytes()`, `from_bytes()`, `serialize_entries()`, `deserialize_entries()` | ~110 |
| `vendor/rvf/rvf-types/src/segment_type.rs` | Add `ContentMap = 0x12` variant; add `0x12 => Ok(Self::ContentMap)` to `TryFrom<u8>`; update `invalid_value_returns_err` test; add `ContentMap` to `round_trip_all_variants` array | 3 + test |
| `vendor/rvf/rvf-runtime/src/store.rs` | Add `embed_content_map(&HashMap<u64, Vec<u8>>) -> Result<u64, RvfError>` and `extract_content_map() -> Result<HashMap<u64, Vec<u8>>, RvfError>` following the `embed_wasm`/`extract_wasm` pattern | ~90 |
| `vendor/rvf/rvf-runtime/src/write_path.rs` | Add `write_content_map_seg<W: Write + Seek>` following the `write_wasm_seg` pattern | ~35 |

See SPEC-001 §0 for exact struct layouts, field offsets, `to_bytes()` / `from_bytes()` implementations, and all method signatures.

---

### 8. Embedding LRU Cache (Phase 1c)

**Gap**: every call to `recall_with_embedding` or `remember_with_embedding` that generates an embedding calls `EmbeddingDriver::embed()` unconditionally. In agentic loops, agents repeat near-identical queries — "what do I know about X?" fires 3–5 times per session. Each miss is an API call (latency + cost). No caching exists anywhere in the current design.

**Decision**: LRU cache on `SemanticStore` in `engine.rs`, keyed by `SHAKE256("{dimension}:{content_bytes}")`.

The dimension prefix is mandatory. Without it, a model upgrade (new dimension, new weights) silently serves stale embeddings from the old model until the cache rolls over. Prefixing the dimension ties each cache entry to the specific embedding geometry — a dimension change invalidates all prior entries immediately.

| Parameter | Config key | Default |
|-----------|-----------|---------|
| Capacity | `memory.embedding_cache_capacity` | 1,000 entries — **Phase 1c: add this field to `MemoryConfig` in `crates/openfang-types/src/config.rs`** (not yet present) |
| Eviction | LRU | — |
| Scope | Per-`SemanticStore` instance | — |

**Per-instance scope** (not shared across agents): prevents cross-agent cache timing side-channels — Agent A's query patterns cannot be inferred from Agent B's cache hit rate.

**Memory bound**: `dimension × capacity × 4 bytes` (e.g. 768-dim × 1,000 × 4 = ~3 MB).

**Hit path**: `embed_or_cached(text)` checks cache before calling `EmbeddingDriver::embed()`. On miss: call driver, store result, return.

**Applied at**: both `remember_with_embedding` (embedding the content being stored) and `recall_with_embedding` (embedding the query). Not applied to `update_embedding` — that call already supplies a pre-computed embedding from the caller.

**Known limitation — same-dimension model upgrades**: the dimension prefix invalidates cache entries when output dimension changes. It does not protect against same-dimension model upgrades (e.g. switching between two models that both produce 768-dim vectors). In that case, stale embeddings from the old model will be served until the cache rolls over. Mitigation: set `memory.embedding_cache_capacity = 0` during model transitions to disable the cache, then restore after the transition is complete.

SHAKE256 is already a dependency via `rvf-crypto`. No new crate required. See SPEC-001 §7 for the `EmbeddingCache` struct and `embed_or_cached` signature.

---

### 9. Session Witness Chain (Phase 1c)

**Gap**: `query_audited` on `shared.rvf` produces per-recall tamper-evident hashes. That answers "did this query happen?" It does not answer "what did this agent do with memory across the full session and in what order?" — there is no session-level causal chain for per-agent memory operations.

**Decision**: `audit.rs` maintains a session-scoped `Vec<WitnessEntry>` for each active agent session. On `SessionEnd` (or `store.close()`), `create_witness_chain(&entries)` produces the chained bytes and they are embedded as a `WITNESS_SEG` in the agent's `.rvf` file.

`create_witness_chain(entries: &[WitnessEntry]) -> Vec<u8>` is confirmed present in `vendor/rvf/rvf-crypto/src/witness.rs` and re-exported via `rvf-crypto/src/lib.rs`.

**Events recorded** (in session order):

| Event | `witness_type` | `action_hash` content |
|-------|:--------------:|----------------------|
| `SessionStart { agent_id, session_id, timestamp_ns }` | `0x01` (PROVENANCE) | SHAKE256 of agent_id bytes |
| `VectorStored { vec_id, scope, content_hash }` | `0x02` (COMPUTATION) | SHAKE256 of vec_id LE bytes |
| `VectorRecalled { query_hash, k_returned, avg_distance }` | `0x02` | SHAKE256 of query embedding bytes |
| `VectorForgotten { vec_id }` | `0x02` | SHAKE256 of vec_id LE bytes |
| `SessionEnd { turn_count, recall_count }` | `0x01` (PROVENANCE) | SHAKE256 of session_id bytes |

**Embedding into `.rvf`**: `store.embed_witness_chain(chain_bytes)` does **not** exist in the vendored `rvf-runtime` (source-verified 2026-03-15). Fork required following the `CONTENT_MAP_SEG` pattern (~35 lines in `write_path.rs`). `write_witness_seg` and `append_witness` already exist but serve the per-query mechanism (see wire format note below) — `embed_witness_chain` is a separate, new method that writes the full chain bytes as a single WITNESS_SEG payload.

**Wire format distinction (source-verified 2026-03-15)**: there are two incompatible WITNESS_SEG wire formats in the vendored code sharing the same segment discriminant (`0x0A`):

| Mechanism | Written by | Payload format | `rvf verify-witness` CLI compatible? |
|-----------|------------|----------------|--------------------------------------|
| Per-query audit | `append_witness()` → `write_witness_seg()` | `type(1) + ts(8) + action_len(4) + action(N) + prev_hash(32)` — variable width | **No** |
| Session chain | `embed_witness_chain()` (to be added) | `prev_hash(32) + action_hash(32) + ts(8) + type(1)` × N entries — fixed 73 bytes/entry | **Yes** |

Consequence: `rvf verify-witness shared.rvf` will report chain verification failures for every per-query WITNESS_SEG written by `query_audited` — those entries use the `write_path.rs` format, which `rvf_crypto::verify_witness_chain` cannot parse (expects strict 73-byte entries). The per-query records are durable but are not verifiable by the existing CLI. A separate read path would be needed to audit them. Implementors must not interpret CLI chain-break warnings on `shared.rvf` as data corruption.

**Additive**: does not replace per-query `query_audited` on `shared.rvf`, which continues to fire for every cross-agent recall. The session chain answers "what did this agent do and in what order" from the `.rvf` file at rest, without external logs. See SPEC-001 §8 for `SessionAuditBuffer` struct and wiring in `engine.rs`.

---

## Consequences

### Positive

- O(log n) ANN search replaces the O(n) fetch-and-sort in `recall_with_embedding` for all query paths
- No row fetch cap — the 100-row ceiling that silently degrades recall quality at scale is removed
- The HNSW vector search path is pure Rust (`rvf-runtime` has no C dependencies). `rusqlite` / `libsqlite3` is retained for seven SQL surfaces: the six operational stores plus the per-agent `.access.db` side-store
- `update_embedding` now correctly handles the case where the caller has no initial embedding (lazy embedding registration via side-store `has_embedding` flag)
- Audited shared recalls produce tamper-evident SHAKE256 witness hashes automatically, replacing the manual `AuditLog::record` call that was previously required after every recall
- `access_count` and `accessed_at` tracking preserved via side-store at minimal overhead
- Phase 2 capabilities (adversarial detection, DoS hardening, membership gating) are additive and feature-flagged — no Phase 1 regression risk
- Embedding LRU cache eliminates redundant `EmbeddingDriver::embed()` calls on repeated queries within a session — reduces API cost and latency proportionally to cache hit rate
- Age/count eviction prevents unbounded episodic growth on long-running agents without touching Semantic or Procedural memory
- Session witness chain enables forensic replay of the full agent memory interaction sequence directly from the `.rvf` file at rest, without requiring external logs

### Negative

- One `.access.db` SQLite file per agent (plus `shared.access.db`) — additional file handles alongside the `.rvf` files. `rusqlite` is already a dependency so no new build requirements, but the total open file count per running deployment increases proportionally with active agent count.
- `content` is stored in `CONTENT_MAP_SEG` inside the `.rvf` file (written on `store.close()` and each `compact_if_needed()` call). Content written since the last `close()` or `compact_if_needed()` call is in-memory only — a crash before either flush loses content ingested in that window. The `.rvf` file is not encrypted at rest; content privacy relies on OS-level file permissions on `~/.openfang/`.
- `update_embedding` is a non-atomic delete-reingest at the RVF level. A crash between the two steps leaves the vector tombstoned with no active replacement. Mitigation: `engine.rs` holds the per-store `Mutex` across both calls; the operation is atomic from the caller's perspective. On startup, `consolidation.rs` scans for tombstoned IDs with `has_embedding: true` in the side-store and reissues the reingest.
- The COW branch workflow (Phase 2) requires `freeze()` on the live shared store before branching, making the store temporarily read-only. During the freeze window (expected <1ms), write calls to the shared store will block.

### Rollback

If Phase 1a introduces a regression, the rollback path is:
1. Revert `crates/openfang-memory/src/semantic.rs` to the SQLite implementation from the last green commit (`git revert` or branch swap)
2. Drop `vendor/rvf/` and the `path =` entries from `Cargo.toml`
3. The six operational stores and all non-vector code are untouched — no data migration required on rollback
4. The per-agent `.rvf` files and `.access.db` side-stores are abandoned; existing upstream `memories` table data is still intact in `openfang.db`

### Neutral

- The `SemanticStore` test suite (7 tests) is preserved and ported. The new tests exercise identical behaviour via the `SemanticStore` API.
- `session.rs` and `usage.rs` are unchanged — this ADR touches only the vector memory path.

---

## Related

- **ADR-001** — upstream OpenFang state; §3 identifies the O(n) scan as gap #1
- **ADR-002** — dual-layer architecture, metadata schemas, RVF crate vendoring; this ADR is the implementation contract for the design specified there
- **[SPEC-001](../specs/SPEC-001-memory-store.md)** *(Status: Active)* — implementation contract derived from this ADR: struct definitions, SQL schemas, method signatures, RVF import lists, phase-by-phase acceptance criteria
- **[PLAN-001](../plans/PLAN-001-memory-store-phase1.md)** — execution plan; task breakdown, test gates, and phase-by-phase acceptance criteria for Phase 0, 1a, and 1b only (Phase 1c and Phase 2 are not covered)

---

## Amendment 2.3: ruvector-upstream Sync — 2026-03-17

### 1. `RvfStoreBackend<B>` now available in ruvector-upstream

**Source**: `crates/rvAgent/rvagent-backends/src/rvf_store.rs`

`RvfStoreBackend<B>` is a generic `Backend` wrapper that implements the rvAgent backend protocol with RVF package awareness. It has landed in ruvector-upstream and is the ADR-106 Layer 3 runtime bridge.

**Path routing.** The backend intercepts any path that passes `is_rvf_path()`, which returns true for paths beginning with `"rvf://"` or `"/rvf/"`. All other paths fall through to the wrapped inner backend `B` unchanged. The two URI forms are equivalent: `"rvf://pkg-name/entry"` and `"/rvf/pkg-name/entry"` resolve to the same `(package_name, internal_path)` pair via `parse_rvf_path()`.

**Immutability.** `write_file()` and `edit_file()` both return early with the error string `"RVF packages are read-only"` when the path is an RVF path. `download_files()` and `upload_files()` similarly short-circuit with `FileOperationError::PermissionDenied` for RVF paths. Non-RVF paths are forwarded to the inner backend for all operations.

**Directory listing.**
- `ls_info("rvf://")` — returns one `FileInfo { is_dir: true, path: "rvf://<pkg-name>" }` entry per mounted package.
- `ls_info("rvf://<pkg-name>")` — returns one `FileInfo { is_dir: false }` entry per manifest entry inside that package, using the path `"rvf://<pkg-name>/<entry-name>"`. Name lookup is O(1) via the `name_index` in `MountTable`.
- `glob_info(pattern, "rvf://")` — iterates all mounted packages and filters manifest entry names by substring match (strips leading/trailing `*` from pattern).

**Shared state.** `RvfStoreBackend::with_mount_table(inner, config, mount_table: Arc<Mutex<MountTable>>)` accepts a caller-supplied mount table. Multiple backends or a backend and a middleware can share the same `Arc<Mutex<MountTable>>` so a package mounted via one component is immediately visible in all others.

**Relation to OpenFang.** OpenFang's `openfang-memory` currently manages its own `.rvf` files via the vendored `rvf-runtime`. `RvfStoreBackend<B>` provides the same `rvf://` namespace concept as a backend adapter rather than a low-level store. If OpenFang's extension system (`openfang-extensions`) ever needs to expose mounted packages as addressable paths to agent tool calls, this backend is the reference implementation for that path-routing layer.

### 2. `RvfManifestMiddleware` now available in ruvector-upstream

**Source**: `crates/rvAgent/rvagent-middleware/src/rvf_manifest.rs`

`RvfManifestMiddleware` is the ADR-106 Layer 2 manifest and signature convergence middleware. It implements the `Middleware` trait and participates in the agent pipeline via `before_agent()` and `tools()`.

**Tool naming convention.** Every tool entry in a mounted RVF package is exposed as an `RvfToolAdapter` whose `name()` is `"rvf:<entry-name>"` — e.g., a manifest entry named `"lint"` becomes the tool `"rvf:lint"`. Only manifest entries of type `RvfManifestEntryType::Tool` are exposed; `Skill`, `WasmComponent`, `DataSegment`, and `Middleware` entries are not included in the tool list.

**`before_agent()` state injection.** When called and the mount table is non-empty, `before_agent()` returns an `AgentStateUpdate` that inserts the key `"rvf_packages"` into `state.extensions`. The value is a JSON array where each element carries `package`, `version`, `verified` (boolean from `RvfVerifyStatus::is_valid()`), `tools` (array of tool-entry name strings), and `skills` (array of skill-entry name strings). When the mount table is empty the hook returns `None` and performs no state mutation.

**`rebuild_tool_cache()`.** Called automatically after each `mount_package()` call. It locks the mount table, iterates `all_tools()`, and rebuilds the `cached_tools: Arc<Mutex<Vec<RvfToolAdapter>>>` in full. The `tools()` method returns cloned adapters from this cache without re-locking the mount table.

**Invocation stub.** `RvfToolAdapter::invoke()` currently returns a placeholder string: `"RVF tool '<name>' (mount <id>:<generation>) invoked with args: <args>. Note: Full execution requires rvf-runtime integration."`. This is intentional — full execution requires the `rvf-compat` feature flag and `rvf-runtime` integration, neither of which is wired yet.

**Relation to OpenFang.** `openfang-skills` currently uses its own hot-reload mechanism for skill packages. `RvfManifestMiddleware` is the reference pattern for upgrading that system to RVF-backed package mounting: mount packages programmatically via `mount_package(manifest)`, share the `Arc<Mutex<MountTable>>` with an `RvfStoreBackend`, and the full `rvf://` namespace becomes addressable to agent backends in the same session.

### 3. ⚠️ DECISION: Should `rvf://` path routing become the standard interface for `openfang-extensions`?

**Context.** `openfang-extensions` currently uses its own path management for loading and addressing extension packages. `RvfStoreBackend<B>` in `crates/rvAgent/rvagent-backends/src/rvf_store.rs` provides a fully-tested, upstream-maintained implementation of `rvf://`-prefixed path routing over a shared `MountTable`. Adopting it for `openfang-extensions` would align OpenFang with the rvAgent/ruvix component model and eliminate bespoke path management code.

**Options.**
- **Option A — Adopt**: Replace `openfang-extensions`' path management with `rvf://` routing backed by `RvfStoreBackend`. Extension paths become `rvf://<pkg>/<entry>` everywhere. Requires depending on `rvagent-backends` or vendoring the relevant types.
- **Option B — Defer**: Keep existing path management. Revisit when Phase 2 work on `openfang-extensions` begins and the full rvAgent/ruvix integration story is clearer.
- **Option C — Parallel**: Add `rvf://` routing as an additional addressing mode alongside existing paths, without removing the existing system. Allows incremental migration.

**Decision needed before Phase 2 work on `openfang-extensions` begins.** No decision is binding at this amendment.

**Resolution (2026-03-17): Option B — Defer.**
OpenFang-AI Phase 1 scope is the memory layer. `openfang-extensions` is working and not blocking any current work. ADR-011 places Phase 1 entirely in the RVF Component Space layer — there is no Phase 1 requirement to adopt ruvix kernel component addressing. `rvf-runtime` invocation remains a stub; refactoring a working extension system to point at a path scheme that cannot execute anything yet introduces risk with no near-term benefit. Revisit at the start of Phase 2 when RuVix kernel integration begins and `openfang-extensions` is already being touched for other reasons. At that point Option A becomes the natural choice.

### 4. ⚠️ DECISION: Should `RvfManifestMiddleware`'s `rvf:<name>` tool injection replace or complement `openfang-skills`' existing skill loader?

**Context.** `openfang-skills` has an existing hot-reload path for skill packages. `RvfManifestMiddleware` in `crates/rvAgent/rvagent-middleware/src/rvf_manifest.rs` provides the same capability as an rvAgent middleware with upstream maintenance. However, `RvfToolAdapter::invoke()` is currently a stub — full execution requires `rvf-runtime` integration (the `rvf-compat` feature), which is not yet complete.

**Options.**
- **Option A — Replace now (with stub)**: Adopt `RvfManifestMiddleware` as the skill loader immediately. The stub response is acceptable during development; actual execution follows when `rvf-runtime` integration lands.
- **Option B — Wait for `rvf-runtime`**: Keep `openfang-skills`' existing loader until `rvf-runtime` integration is complete and `invoke()` produces real results. Avoids a broken execution path in production builds.
- **Option C — Complement**: Run both loaders in parallel. Skills loaded via the existing path continue to execute normally; `RvfManifestMiddleware` handles new RVF-format packages only.

**Decision needed before beginning any work that depends on skill invocation via the rvAgent middleware pipeline.**

**Resolution (2026-03-17): Option C — Complement, permanently.**
This is not a transitional choice pending `rvf-runtime` maturity — it is the permanent architectural split. `openfang-skills` and `rvf:<name>` tool injection serve fundamentally different use cases: skills are local, file-based, hot-reloadable prompt templates sourced from the OpenClaw marketplace; RVF tools are externally-packaged, cryptographically signed, immutable compiled capability packages distributed via the RVF App Gallery. Conflating them would be a design mistake even at full `rvf-runtime` maturity. When `rvf-runtime` invocation becomes real, `rvf:<name>` tools become the distribution path for signed binary capabilities — a second tier above the skill system, not a replacement for it. `openfang-skills` is untouched.
