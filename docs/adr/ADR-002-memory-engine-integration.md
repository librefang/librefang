# ADR-002: Memory Engine Integration

**Status**: Conditionally Accepted — pending ADR-003 promotion to Accepted
**Date**: 2026-03-14
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-14 | Daniel Alberttis | Initial draft |
| 0.2 | 2026-03-14 | Daniel Alberttis | Scope restricted to local memory integration only — no deployment, no SaaS, no cloud |
| 0.3 | 2026-03-14 | Daniel Alberttis | Pivoted from pi brain pattern (individual RVF files + redb) to RvfStore pattern (persistent HNSW + embedded metadata). Dropped ruvector-core/filter/redb; replaced with rvf-runtime. Simplified file structure. ADR-003 and ADR-004 superseded by this revision. |
| 0.4 | 2026-03-14 | Daniel Alberttis | Locked dual-layer storage design: `shared.rvf` (org brain) + `{agent_id}.rvf` (private). Mirrors OpenFang's `shared_memory_agent_id()` architecture and Ruv's example set. Updated §1, §3, §4, §5. |
| 0.5 | 2026-03-14 | Daniel Alberttis | Corrected scope: rusqlite is retained for all operational stores. Only SemanticStore (vector path) is replaced. Six stores (StructuredStore, SessionStore, UsageStore, PairedDeviceStore, TaskQueueStore, AuditLog) are untouched. memory_store/memory_recall tools are KV ops on StructuredStore — not affected. |
| 0.6 | 2026-03-14 | Daniel Alberttis | Locked routing key: agent_id is the routing parameter for vector stores. remember_with_embedding(shared_memory_agent_id(), ...) routes to shared.rvf; all other agent IDs route to {agent_id}.rvf. memory_store/memory_recall tools confirmed KV-only — no involvement in vector path. Named internal store wrappers AgentSemanticStore and SharedSemanticStore (OpenFang {Domain}Store convention). Corrected Dual-Layer Design description: Phase 1 has separate recall/recall_shared methods; merge-and-rerank is Phase 2 only. |
| 0.7 | 2026-03-14 | Daniel Alberttis | Renamed SemanticStore → SemanticStore. Follows OpenFang {Domain}Store convention; preserves the same public name on MemorySubstrate. The implementation changes (SQLite → RvfStore); the contract name does not. |
| 0.8 | 2026-03-14 | Daniel Alberttis | Added North Star principle to Context: swap the storage engine, preserve the framework. OpenFang's scheduling, config, API contracts, and operational stores are unchanged. Daxiom's contribution is a better vector engine inside the existing boundary. |
| 0.9 | 2026-03-14 | Daniel Alberttis | Source-verified corrections: MetadataEntry/MetadataValue moved to rvf-runtime row (not rvf-types); §4 clarified F_SCOPE etc. are application-defined u16 constants with example declarations; §7 corrected MemorySource is not a new addition (already in openfang-types); §ADR-003 section reworded from "deleted" to "superseded in place" with accurate description of current ADR-003 and ADR-004 content. |
| 1.0 | 2026-03-15 | Daniel Alberttis | Design flaw F1 fixed: split single `meta.rs` into `agent_meta.rs` (per-agent store) and `shared_meta.rs` (shared store). Root cause: both modules declared `F_SCOPE: u16 = 0` and `F_AGENT_ID: u16 = 0` in the same flat namespace — different names, same value, no compile error, silent wrong-field filter at runtime if constants crossed stores. Separate modules enforce correct import at the call site. §4 code block updated; §5 file structure updated. Cascades to ADR-003 §4, SPEC-001 §4/§5, PLAN-001 T-04, ADR-009 §3, glossary. |
| 1.1 | 2026-03-15 | Daniel Alberttis | Status downgraded from Accepted to Conditionally Accepted — pending ADR-003 promotion to Accepted. ADR-003 is the implementation contract; accepting the design ADR without its implementation contract is premature (F6 audit finding). |
| 1.2 | 2026-03-15 | Daniel Alberttis | Alignment fixes from cross-doc audit: (1) §5 "Seven files" → "Eight files" — ADR-003 v1.1 split `meta.rs` into two, but the count in this doc was not updated. (2) `audit.rs` description extended with its Phase 1c second responsibility: `SessionAuditBuffer` for the per-agent session witness chain. |

---

## Context

### North Star: swap the storage engine, preserve the framework

OpenFang's framework is sound. Its scheduling model, configuration system, public API contracts, operational stores, and observability are all correct and worth keeping. The only problem is that `SemanticStore`'s vector search is O(n) and degrades silently at scale. This ADR and ADR-003 fix that specific problem by replacing the storage engine inside `SemanticStore`. Everything above that layer stays OpenFang.

This is the guiding principle for all subsequent decisions: if a choice preserves OpenFang's framework while improving vector search, it is correct. If a choice requires touching the kernel, changing API signatures, or modifying operational stores, it is out of scope.

---

OpenFang v0.4.0 ships with `openfang-memory`. Three stores are named structs: `StructuredStore`, `SemanticStore`, `KnowledgeStore`, and `SessionStore` / `UsageStore`. Three additional concerns (paired-device operations, task queue operations, and audit logging) are implemented as inline SQL methods on `MemorySubstrate` or live in a separate crate (`AuditLog` is in `openfang-runtime`, not `openfang-memory`). All non-vector concerns are working correctly and untouched by this ADR. `SemanticStore` handles vector memory (the `memories` table) and has two gaps that block production use at scale (see ADR-001 §3):

1. **Unindexed O(n) vector scan** — `SemanticStore::recall_with_embedding` fetches `(limit × 10).max(100)` rows from SQLite then re-ranks by cosine similarity in Rust. Latency grows linearly with memory count. The fetch cap silently degrades recall quality once total memories exceed the cap.
2. **No self-learning** — memories accumulate but the system never extracts patterns or adapts retrieval behaviour.

The ruvector ecosystem (`ruvector-upstream`) contains `RvfStore` — a persistent, file-backed HNSW vector store with built-in metadata filtering, audited queries, and a witness chain. It directly addresses both gaps in a single component. The six operational stores are not affected.

### The Pi Brain Pattern vs the RvfStore Pattern

Early drafts of this ADR (and an early ADR-003 that was subsequently deleted) derived the design from `mcp-brain-server` (pi brain) — Ruv's GCP-deployed community knowledge server. Pi brain uses individual `.rvf` files per memory entry with an in-memory HNSW index rebuilt at startup, plus Firestore/redb for structured metadata. That two-tier pattern was appropriate for pi brain's use case: a public server where thousands of human contributors submit knowledge, each with their own RVF container, and a Byzantine aggregation layer reconciles them.

That is not this use case.

The `examples/rvf/examples/openfang.rs` example in ruvector-upstream shows Ruv's actual design for the OpenFang ↔ RVF integration: a single `RvfStore` per agent namespace, where HNSW, metadata filtering, audited queries, witness chain, delete+compact lifecycle, and COW branching are all provided by `rvf-runtime` directly. The example was written against the OpenFang domain model (Hands, Tools, Channels) with the exact CLAUDE.md config (`claude-opus-4-6`, hierarchical topology, 15 agents) embedded in the AGI container.

The replacement design follows that example, not the pi brain pattern.

---

## Decision

### 1. What Changes — and What Doesn't

**`SemanticStore` is replaced. All six operational stores are untouched.**

`crates/openfang-memory/src/semantic.rs` is rewritten. `rusqlite` is **retained** — the six operational stores stay on SQLite exactly as they are:

| Store / concern | What it stores | Why SQLite stays |
|-----------------|---------------|-----------------|
| `StructuredStore` | Agent KV pairs, agent registry (`save_agent`/`load_agent`) | Relational, not vector |
| `SessionStore` | Canonical sessions, channel sessions, LLM summaries | Sequential append, not vector |
| `UsageStore` | Token metering — 9 query types | Aggregation queries, not vector |
| `KnowledgeStore` | Entity/relation graph (`knowledge.rs`) | Graph queries, not vector |
| Paired-device ops | Device pairing — inline SQL on `MemorySubstrate` (no named struct) | Simple KV, not vector |
| Task queue ops | Task post/claim/complete — inline SQL on `MemorySubstrate` (no named struct) | Queue semantics, not vector |
| `AuditLog` | Non-memory audit entries — lives in `openfang-runtime`, not `openfang-memory` | Append-only log, not vector |

Note: the `memory_store` and `memory_recall` **tools** in `kernel.rs` call `structured_set` / `structured_get` on `StructuredStore`. Those are key-value operations, not semantic search. They are not affected by this ADR.

`MemorySubstrate` keeps its SQLite connection. It gains an `SemanticStore` field alongside the existing connection. Externally its API is unchanged — `remember_with_embedding`, `recall_with_embedding`, `update_embedding`, `forget` are rerouted to `SemanticStore` internally. All other 40+ methods delegate to SQLite as before.

```
openfang-ai binary
├── Agent runtime         ← OpenFang kernel, tool runner, MCP client, channels (unchanged)
└── openfang-memory
      ├── MemorySubstrate ← unchanged public API; internal routing changes for vector methods
      │     ├── SQLite    ← rusqlite (retained) — StructuredStore, SessionStore, UsageStore,
      │     │               PairedDeviceStore, TaskQueueStore, AuditLog — all unchanged
      │     └── SemanticStore (new) — vector path only
      │           ├── shared.rvf   ← org brain vector store
      │           ├── {agent}.rvf  ← per-agent private vector store
      │           └── RvfStore API ← rvf-runtime
      └── SONA loop       ← Phase 2
```

### Dual-Layer Design

OpenFang uses a fixed UUID (`00000000-0000-0000-0000-000000000001`) as the shared memory namespace (`shared_memory_agent_id()`). For KV memory this UUID is used by the `memory_store` / `memory_recall` tools via `StructuredStore` — those tools are unchanged. The same UUID is the routing key for the vector layer: calling `remember_with_embedding(shared_memory_agent_id(), ...)` routes to `shared.rvf`; any other `agent_id` routes to that agent's private store.

This maps directly to two `RvfStore` files:

| Layer | File | Store type | Writer | Reader | Ruv example |
|-------|------|-----------|--------|--------|-------------|
| **Org brain** | `shared.rvf` | `SharedSemanticStore` | Any agent via `remember_with_embedding(shared_memory_agent_id(), ...)` | All agents | `openfang.rs`, `swarm_knowledge.rs` |
| **Agent private** | `{agent_id}.rvf` | `AgentSemanticStore` | That agent only | That agent only | `agent_memory.rs` |

**`shared.rvf`** — metadata includes `F_AGENT_ID` (provenance: who wrote it). All memories are cross-agent readable. Equivalent to pi brain's aggregation layer, and to OpenFang's `shared_memory_agent_id()` namespace.

**`{agent_id}.rvf`** — no `F_AGENT_ID` field needed (the file is the scope). Contains episodic sessions, semantic facts, procedural knowledge private to that agent.

`SemanticStore` routes on `agent_id`: `shared_memory_agent_id()` → `SharedSemanticStore::store_shared` / `recall_shared`; any other ID → `AgentSemanticStore::store` / `recall`. In Phase 1 these are separate query paths. Cross-store merge-and-rerank is Phase 2 only (degraded-quality fallback in `recall_shared_with_quality`).

### 2. Why Not Fork the Brain Server

| Dimension | mcp-brain-server | openfang-memory needs |
|-----------|-----------------|----------------------|
| Identity model | Human contributors — external, adversarial | Agents — internal, trusted, owned |
| Auth | `AuthenticatedContributor` middleware | None — in-process call |
| Aggregation | `ByzantineAggregator` — untrusted inputs | Simple accuracy-weighted averaging |
| Reputation | Accuracy + uptime + stake | Accuracy only |
| Storage | Individual RVF per contributor + GCS | One RvfStore per agent namespace |
| Architecture | Standalone axum HTTP server | Library crate in the agent runtime |
| Cold start | HNSW hydration from GCS/disk | `RvfStore::open_readonly` — index already on disk |

The gap is wide enough that forking means rewriting the design center. Using `rvf-runtime` directly is cleaner and matches the integration design Ruv already built.

### 3. How RvfStore Replaces SemanticStore

The current `SemanticStore::recall_with_embedding` pipeline:
```
SELECT * FROM memories WHERE agent_id = ? LIMIT 100
→ sort by cosine(embedding, query_vec) in Rust
→ filter by scope, deleted
```

The replacement:
```rust
store.query(&query_embedding, K, &QueryOptions {
    filter: Some(FilterExpr::And(vec![
        FilterExpr::Eq(F_SCOPE,   FilterValue::U64(scope as u64)),
        FilterExpr::Eq(F_DELETED, FilterValue::U64(0)),
    ])),
    ..Default::default()
})
```

The per-agent store has no `F_AGENT_ID` field — the file path is the agent scope boundary. `F_AGENT_ID` only exists in `shared.rvf` (field ID 0) where it records provenance.

One call. O(log n) HNSW. No Rust-side sort. No row fetch limit.

For compliance-grade audit (replacing `AuditLog::append` after recall):
```rust
store.query_audited(&query_embedding, K, &QueryOptions { .. })
// auto-appends a SHAKE256 witness entry — no separate audit call needed
```

### 4. Metadata Field Schema

Two stores, two schemas. Both use the same field IDs where they overlap.

**Important**: `F_SCOPE`, `F_CONFIDENCE`, `F_DELETED`, `F_SOURCE`, and `F_AGENT_ID` are **application-defined `u16` constants** — they do not exist in `rvf-runtime` or any rvf crate. `FilterExpr::Eq` takes a raw `u16` field ID.

The constants **must be declared in two separate modules** — one per store. A single `meta.rs` with both stores' constants would create a silent collision: both stores assign field slot `0` to different constants (`F_SCOPE` in the per-agent store; `F_AGENT_ID` in the shared store). Since Rust cannot detect misuse of identically-valued constants across stores, a developer who imports the wrong constant gets wrong filter behaviour with no compile error and no panic.

```rust
// crates/openfang-memory/src/agent_meta.rs — per-agent store ONLY
// Use `use agent_meta::*` in all store.rs / engine.rs code that touches {agent_id}.rvf
pub const F_SCOPE:      u16 = 0;
pub const F_CONFIDENCE: u16 = 1;
pub const F_DELETED:    u16 = 2;
pub const F_SOURCE:     u16 = 3;
```

```rust
// crates/openfang-memory/src/shared_meta.rs — shared store ONLY
// Use `use shared_meta::*` in all store.rs / engine.rs code that touches shared.rvf
pub const F_AGENT_ID:   u16 = 0;  // provenance — who wrote it
pub const F_SCOPE:      u16 = 1;
pub const F_CONFIDENCE: u16 = 2;
pub const F_DELETED:    u16 = 3;
pub const F_SOURCE:     u16 = 4;
```

This is the standard rvf pattern — all rvf adapters (sona, agentdb) define their own `u16` constants privately. Separate modules make cross-store contamination an obvious import error rather than a silent runtime bug.

**Per-agent store (`{agent_id}.rvf`) — 4 fields:**

| Field ID | Constant | Name | Type | Values |
|:--------:|----------|------|------|--------|
| 0 | `F_SCOPE` | scope | U64 | 0=Episodic, 1=Semantic, 2=Procedural |
| 1 | `F_CONFIDENCE` | confidence | U64 | `confidence * 10_000` as u64 (0–10000) |
| 2 | `F_DELETED` | deleted | U64 | 0=active, 1=tombstoned |
| 3 | `F_SOURCE` | source | U64 | 0=Conversation, 1=Document, 2=Observation, 3=Inference, 4=UserProvided, 5=System |

**Shared store (`shared.rvf`) — 5 fields (adds `F_AGENT_ID` for provenance):**

| Field ID | Constant | Name | Type | Values |
|:--------:|----------|------|------|--------|
| 0 | `F_AGENT_ID` | agent_id | String | agent UUID — who wrote it |
| 1 | `F_SCOPE` | scope | U64 | 0=Episodic, 1=Semantic, 2=Procedural |
| 2 | `F_CONFIDENCE` | confidence | U64 | `confidence * 10_000` as u64 (0–10000) |
| 3 | `F_DELETED` | deleted | U64 | 0=active, 1=tombstoned |
| 4 | `F_SOURCE` | source | U64 | 0=Conversation, 1=Document, 2=Observation, 3=Inference, 4=UserProvided, 5=System |

No `F_NAMESPACE` field — namespace is determined by which store file is being queried, not by metadata.

Recall behaviour:
- `scope` filter prevents episodic fragments contaminating semantic/procedural searches
- `deleted = 0` on all searches; tombstone by writing `F_DELETED = 1` then calling `compact()`
- `confidence` supports range filters: `Ge(F_CONFIDENCE, 7000)` = confidence > 0.7
- Shared store: `F_AGENT_ID` is for audit/provenance only — no per-agent isolation filter needed since all entries are cross-agent readable by design

### 5. Target File Structure

```
crates/openfang-memory/src/
├── lib.rs           ← pub re-exports
├── engine.rs        ← SemanticStore — public API (store/search/delete/vote)
├── store.rs         ← AgentSemanticStore + SharedSemanticStore — RvfStore wrappers, open/create/compact
├── agent_meta.rs    ← per-agent field ID constants (F_SCOPE=0, F_CONFIDENCE=1, F_DELETED=2, F_SOURCE=3)
│                      AgentVectorMeta struct. No RVF imports.
├── shared_meta.rs   ← shared store field ID constants (F_AGENT_ID=0, F_SCOPE=1, F_CONFIDENCE=2, F_DELETED=3, F_SOURCE=4)
│                      No RVF imports.
├── ranking.rs       ← post-query scoring blend: similarity + access_count + last_accessed (ADR-011 Constraint 2 compliant; confidence used as pre-filter threshold via FilterExpr, not a blend input)
├── consolidation.rs ← confidence decay background task (Tokio); SONA hook point for Phase 2
└── audit.rs         ← (1) SharedAuditBridge: query_audited adapter → OpenFang AuditLog bridge (witness hash passthrough); (2) SessionAuditBuffer: session-scoped Vec<WitnessEntry> for the per-agent session witness chain (Phase 1c)
```

Eight files (seven new + updated `lib.rs`). No backend abstraction layer needed in Phase 1 — all storage is `RvfStore` on local disk. Backend abstraction (GCS, S3, Supabase) is deferred to a future ADR if multi-tenant SaaS becomes a requirement.

### 6. RVF Crates to Vendor

The `vendor/rvf/` directory was pre-populated from `ruvector-upstream/crates/rvf/` and currently contains the full rvf tree (18+ subdirectories including rvf-adapters, rvf-cli, rvf-ebpf, rvf-federation, rvf-index, rvf-kernel, etc.). **None of these are wired into `Cargo.toml` workspace dependencies yet.** The active set per phase is:

**Phase 1 — core memory replacement (to be wired in `Cargo.toml`):**

| Crate | Version | Purpose |
|-------|---------|---------|
| `rvf-runtime` | 0.2.0 | `RvfStore`, `FilterExpr`, `MembershipFilter`, `QueryOptions`, `MetadataEntry`, `MetadataValue`, `query_with_envelope`, `query_audited`, `BudgetTokenBucket`, `NegativeCache` |
| `rvf-types` | 0.2.0 | Segment type enums, `DerivationType`, format constants, header structs (`SegmentHeader`, `WitnessHeader`, etc.), `DataType`, `QuantType`, `CompressionAlgo`, quality types (`ResponseQuality`, `QualityPreference`) |
| `rvf-crypto` | 0.2.0 | `create_witness_chain`, `shake256_256`, `verify_witness_chain`, `WitnessEntry` |

`rvf-runtime` only depends on `rvf-types` — no C, no third-party runtime deps beyond `sha3` and optional `ed25519-dalek` in `rvf-crypto`. Compiles clean on all targets.

**Phase 2 — self-learning (vendored but not yet wired):**

| Crate | Purpose |
|-------|---------|
| `rvf-adapters/sona` | SONA self-optimizing loop wired to RvfStore |
| `rvf-federation` | Accuracy-weighted LoRA averaging across agents |

**Not in scope:**

All `ruvector-core`, `ruvector-filter`, `ruvector-nervous-system`, `ruvector-sona`, `ruvector-metrics` references from earlier drafts are superseded. Those are the primitives that `rvf-runtime` is built on top of — we use the higher-level API, not the primitives directly.

All other `ruvector-*` crates (raft, cluster, gnn, graph, attention, robotics, quantum, fpga, verified, wasm/node) remain out of scope.

### 7. Workspace Structure

```
openfang-ai/
├── Cargo.toml
├── crates/
│   ├── openfang-types/        ← minor addition: MemoryScope enum (MemorySource already exists)
│   ├── openfang-memory/       ← rebuilt (this ADR)
│   ├── openfang-runtime/      ← unchanged
│   ├── openfang-kernel/       ← unchanged
│   ├── openfang-api/          ← minor: /api/memory/* routes wired to new engine
│   ├── openfang-channels/     ← unchanged
│   ├── openfang-skills/       ← unchanged
│   ├── openfang-wire/         ← unchanged
│   ├── openfang-hands/        ← unchanged
│   ├── openfang-extensions/   ← unchanged
│   ├── openfang-migrate/      ← unchanged
│   ├── openfang-desktop/      ← unchanged
│   └── openfang-cli/          ← unchanged
├── vendor/
│   └── rvf/
│       ├── rvf-runtime/
│       ├── rvf-types/
│       └── rvf-crypto/
└── (runtime data layout — not in repo, created at first run)
    ~/.openfang/             ← Resolved: upstream path retained. Fork does not coexist with upstream.
        ├── shared.rvf              ← org brain (all agents read/write)
        └── agents/
              ├── {agent_id_1}.rvf  ← agent 1 private memory
              └── {agent_id_2}.rvf  ← agent 2 private memory
```

---

## Consequences

### Positive

- In-process memory calls — microsecond latency, no IPC or network hop
- O(log n) ANN search replaces O(n) linear scan — scales to millions of memories without limit change
- No cold-start HNSW hydration — `RvfStore::open_readonly` loads the persisted index directly
- Surgical change — only `SemanticStore` (4 methods) is replaced; 40+ operational methods unchanged
- No `redb` — vector metadata lives in the RvfStore per-vector as `MetadataEntry`; no extra storage layer
- Audited queries built-in — `query_audited` replaces the two-step recall + `AuditLog::append` for the vector path
- Tombstone lifecycle built-in — `delete` + `compact` replaces `deleted = 1` SQL flag
- Self-learning ready — SONA background task plugs into `consolidation.rs` in Phase 2 without structural changes
- Agent-scoped from day one — per-agent `.rvf` file is the scope boundary; `F_AGENT_ID` in `shared.rvf` records provenance but is not an access-control filter

### Negative

- `openfang-memory` rewrite is the critical path — nothing else in the fork moves until the local pipeline works
- Binary size increase — `rvf-runtime` + `rvf-crypto` add compile time (smaller than the previous plan's five crates)
- Vendored crates require manual sync with ruvector-upstream when needed

### Neutral

- MCP client in OpenFang is unchanged — openfang-ai still connects to external MCP tool servers
- `rusqlite` is retained — the C build dependency remains, but is now scoped only to the operational stores (not the vector path)
- `memory_store` / `memory_recall` tools are unaffected — they call `StructuredStore` KV methods, not vector recall

---

## ADR-003 and ADR-009

Early drafts of ADR-003 (pi brain pattern: individual RVF files + in-memory HNSW + redb + DashMap) and the original ADR-004 (MemoryBackend/KvBackend trait abstraction — now superseded and renumbered ADR-009) were written before the `openfang.rs` integration reference was identified. Those drafts described the wrong architecture and were **superseded in place** — the filenames were reused for replacement content. Both files exist on disk with entirely different content.

`ADR-003-memory-store-implementation.md` is now the implementation contract for the SemanticStore → RvfStore migration: exact method mappings, vector ID scheme, access side-store (SQLite), compact trigger policy, and Phase 2 shared store capabilities.

`ADR-009-memory-intelligence.md` is now the implementation contract for Phase 2 SONA self-learning (`TrajectoryStore`, `ExperienceReplayBuffer`, `NeuralPatternStore` via `consolidation.rs`) and `rvf-federation` cross-agent knowledge export/import.

---

## References

- `ADR-001-openfang-baseline.md` — upstream state being forked
- `ADR-003-memory-store-implementation.md` — implementation contract for both stores
- `examples/rvf/examples/openfang.rs` in ruvector-upstream — reference implementation
- `examples/rvf/examples/assets/openfang-README.md` — 24-capability capability map
- `examples/rvf/examples/agent_memory.rs` — per-agent store pattern
- `examples/rvf/examples/swarm_knowledge.rs` — shared store pattern
- `examples/rvf/examples/filtered_search.rs` — full FilterExpr API
- [ruvector-upstream](https://github.com/ruvnet/ruvector-upstream)
