# ADR-016: Bi-Temporal Memory — Valid-Time and Transaction-Time Annotations

**Status**: Draft
**Phase**: 3 (Memory Layer)
**Date**: 2026-03-21
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-21 | Daniel Alberttis | Initial draft. Bi-temporal annotation (Phase 1) — three new nullable columns, corrected PIT query, explicit scope boundary. Codex peer review incorporated. |
| 0.2 | 2026-03-21 | Daniel Alberttis | Art of the Field fully source-verified via DeepWiki. Corrected Hindsight field names (`occurred_start`/`occurred_end`/`mentioned_at`; not `occurrence_time`/`mention_time`). Corrected memv fields (`valid_at`/`invalid_at`/`expired_at`/`superseded_by`; not `valid_from`/`valid_to`). Added memv SQLite schema, supersession logic, `KnowledgeStore.get_valid_at`. Corrected mem7 as uni-temporal (no `valid_at`/`invalid_at`); documented audit trail + Ebbinghaus decay. Added Graphiti full EntityEdge schema, `add_episode` signature, Cypher PIT query, `SearchFilters` API, index names, `group_id` isolation. Hindsight stack corrected: PostgreSQL/pg0, not SQLite. |
| 0.3 | 2026-03-21 | Daniel Alberttis | Third DeepWiki loop: added full `add_episode` 7-step pipeline + two-stage edge deduplication + community detection detail (Graphiti); full 5-stage retain pipeline + `ExtractedFact` struct + 5 extraction modes + Mental Models hierarchy (Hindsight); `Memory.add_exchange`/`Memory.retrieve` signatures + `ExtractedKnowledge` fields + `backfill_temporal_fields` + Predict-Calibrate phases (memv); 6-step write path + `VectorIndex`/`GraphStore` Rust traits + `tokio::join!` dual-path recall semantics (mem7). |
| 0.4 | 2026-03-21 | Daniel Alberttis | Agentic QE review (Codex) — 4 FAILs corrected: (1) `forget()` execution order corrected (RVF delete first, conditional on `has_embedding != 0`, then SQL update, then `content_map.remove`); (2) migration version corrected (current highest is v8, next is v9, not M-007/version 7); (3) migration mechanism corrected (explicit `migrate_v9()` function, not `MigrationStep` struct); (4) current-state "unchanged behavior" claim corrected (current `recall()` uses only `deleted = 0` — adding `invalid_at`/`expires_at` predicates IS a behavior change); (5) `consolidation.rs` claim corrected (only confidence decay today, `memories_merged: 0`, no conflict resolution). API compat note: adding `valid_at` param to `remember_with_embedding()` requires call-site updates. |
| 0.5 | 2026-03-21 | Daniel Alberttis | Final DeepWiki + Codex precision pass — 8 corrections: (1) Graphiti `ComparisonOperator` enum names corrected (`greater_than`/`less_than`/`greater_than_equal`/`less_than_equal`); (2) `add_episode_bulk` note: `EpisodicNode.valid_at` IS set from `reference_time` — only `EntityEdge.valid_at/invalid_at` remain null; (3) Hindsight `fact_type`: removed `opinion` (deprecated/removed, current set: `world`, `experience`, `observation`, `mental_model`); (4) Hindsight temporal anchor: API param is `query_timestamp` (str), not `question_date`; (5) Hindsight dedup: no `DUPLICATE_SIMILARITY_THRESHOLD = 0.95` — actual thresholds are 0.7 (semantic linking) and 0.6 (entity resolution); (6) `migrate_v9()` guard: separate `column_exists` per column, not one guard for all; (7) current-state query: `datetime('now')` replaced with bound `:now` RFC3339 param; (8) consequences section: corrected "unchanged/zero-impact" claim to reflect that adding predicates IS a behavior change with query-cost impact. |

---

## Context

### The Problem

OpenFang agents make decisions grounded in memory. Those decisions may later be subject to audit, compliance review, or debugging. Today there is no way to answer the question:

> "What did agent X believe at time T?"

The `memories` table (managed by `RvfSemanticStore` in `crates/openfang-memory/src/rvf_store.rs`) is a **mutable, uni-temporal store**. It tracks only one clock: `created_at` — when a row was inserted. Invalidation is a flag flip (`deleted = 1`) with no timestamp recording _when_ the deletion occurred. Row fields including `confidence`, `access_count`, `accessed_at`, and embeddings are updated in-place with no version history. The result is that after any mutation or soft-delete, the prior state is permanently lost.

This matters for:

- **Compliance and audit** — a financial or medical agent cannot reconstruct the information state that existed when a specific recommendation was made.
- **Multi-agent debugging** — when Agent B corrects a fact that Agent A acted on, there is no way to trace which version of the fact Agent A held at decision time.
- **Conflict resolution** — when a corrected memory supersedes an existing one (today this must be done manually or via a future consolidation upgrade — `consolidation.rs` currently only performs confidence decay), the superseded belief vanishes rather than being preserved with a bounded validity window.
- **Regulatory accountability** — any domain where "what did you know, and when did you know it?" is a legal question.

### Current Schema (Source of Truth)

Taken verbatim from `rvf_store.rs:140` (`init_schema`):

```sql
CREATE TABLE IF NOT EXISTS memories (
    rvf_id        INTEGER NOT NULL UNIQUE,
    uuid          TEXT    PRIMARY KEY,
    agent_id      TEXT    NOT NULL,
    content       TEXT    NOT NULL,
    source        TEXT    NOT NULL DEFAULT '"system"',
    scope         TEXT    NOT NULL DEFAULT 'episodic',
    confidence    REAL    NOT NULL DEFAULT 1.0,
    metadata      TEXT    NOT NULL DEFAULT '{}',
    created_at    TEXT    NOT NULL,        -- RFC3339, insert time (only timestamp)
    accessed_at   TEXT    NOT NULL,        -- RFC3339, mutated in-place on recall
    access_count  INTEGER NOT NULL DEFAULT 0,
    deleted       INTEGER NOT NULL DEFAULT 0,  -- 0/1 flag, no deletion timestamp
    has_embedding INTEGER NOT NULL DEFAULT 0,
    importance    REAL    NOT NULL DEFAULT 0.5,
    expires_at    INTEGER DEFAULT NULL     -- Unix seconds (TTL, not world-time invalidity)
);
```

**Temporal fields audit:**
- `created_at` — single clock, conflates "when recorded by system" and "when true in world"
- `accessed_at` — mutated in-place on every recall; not temporal in the bi-temporal sense
- `deleted` — boolean flag; `forget()` at line 743 sets this with no `deleted_at` timestamp
- `expires_at` — INTEGER Unix seconds; TTL-based eviction policy, not world-time invalidity

Current `forget()` flow (lines 718–755) — verified execution order:
1. SELECT `(rvf_id, has_embedding)` for the target UUID
2. If `has_embedding != 0`: deletes vector from RVF bitmap (`rvf.delete(&[rvf_id as u64])`) — **conditional, not always executed**
3. SQL `UPDATE memories SET deleted = 1 WHERE uuid = ?1`
4. `content_map.remove(&(rvf_id as u64))` — removes from in-memory content map
5. `compact_if_needed()` — RVF compaction check
6. **No `deleted_at` timestamp is ever written at any step**

### Why This Is Not Bi-Temporal

A bi-temporal model requires two independent time axes per fact:

| Axis | Name | Meaning |
|------|------|---------|
| World time | Valid time (`valid_at` / `invalid_at`) | When the fact was true in reality |
| System time | Transaction time (`created_at` / `deleted_at`) | When the system learned/invalidated it |

OpenFang currently has neither axis cleanly separated. `created_at` serves as a partial transaction-time start but is not tracked per-version (rows are mutated in-place). Valid time does not exist at all. `expires_at` is a third concept — scheduled TTL eviction — and must not be conflated with either axis.

---

## Art of the Field — Reference Implementations

### Graphiti (`github.com/getzep/graphiti`)

**DeepWiki**: Indexed — `deepwiki.com/getzep/graphiti` §3.2 "Temporal Awareness and Bi-Temporal Model"

The most complete open-source bi-temporal implementation for agent memory. Bi-temporal tracking applies exclusively to **edges** (`EntityEdge` / `RELATES_TO`). `EntityNode` objects carry only `created_at`. `EpisodicNode` carries `valid_at` (non-null, required) + `created_at`.

#### EntityEdge — full schema (source: `graphiti_core/edges.py`)

| Field | Type | Default | Axis | Meaning |
|-------|------|---------|------|---------|
| `uuid` | `str` | generated | — | Unique identifier |
| `group_id` | `str` | required | — | Multi-agent / multi-tenant namespace |
| `name` | `str` | required | — | Relation name |
| `fact` | `str` | required | — | Natural language fact description |
| `fact_embedding` | `list[float] \| None` | `None` | — | Semantic embedding of `fact` |
| `episodes` | `list[str]` | `[]` | — | Source episode UUIDs |
| `created_at` | `datetime` | `utcnow()` | Tx time start | When ingested; never updated |
| `expired_at` | `datetime \| None` | `None` | Tx time end | Set to `utcnow()` by conflict resolution; NULL = current |
| `valid_at` | `datetime \| None` | `None` | Valid time start | LLM-extracted or caller-supplied; NULL = unknown/ongoing |
| `invalid_at` | `datetime \| None` | `None` | Valid time end | LLM-extracted or set by conflict resolution; NULL = still true |
| `attributes` | `dict` | `{}` | — | Custom edge attributes |

#### How valid_at / invalid_at are set (source: `graphiti_core/prompts/extract_edges.py`)
`EpisodicNode.valid_at` → `REFERENCE_TIME` in LLM prompt. LLM `DATETIME RULES`:
- Ongoing fact (present tense) → `valid_at = REFERENCE_TIME`, `invalid_at = null`
- Termination expressed → `invalid_at = relevant timestamp`
- No explicit time → both `null`
- Relative expressions ("last week") → resolved via `REFERENCE_TIME`
- `add_episode_bulk` does **not** run date extraction — `valid_at`/`invalid_at` remain null

#### add_episode — caller signature
```python
async def add_episode(
    self,
    name: str,
    episode_body: str,
    source_description: str,
    reference_time: datetime,   # REQUIRED; no default; omitting raises TypeError
    source: EpisodeType = EpisodeType.message,  # message | json | text
    group_id: str | None = None,
    ...
) -> AddEpisodeResults
```

#### Conflict resolution — resolve_edge_contradictions (exact logic)
```python
def resolve_edge_contradictions(
    resolved_edge: EntityEdge,
    invalidation_candidates: list[EntityEdge]
) -> list[EntityEdge]
```
For each candidate `edge`:
- **Not invalidated**: `edge.invalid_at <= resolved_edge.valid_at` OR `resolved_edge.invalid_at <= edge.valid_at` (no temporal overlap)
- **Invalidated**: `edge.valid_at < resolved_edge.valid_at` →
  - `edge.invalid_at = resolved_edge.valid_at`
  - `edge.expired_at = utcnow()` (if not already set)
- `resolved_edge` may also be immediately expired if a candidate has `valid_at` after `resolved_edge.valid_at`

#### Point-in-time Cypher query (generated by `edge_search_filter_query_constructor`)
```cypher
WHERE (e.valid_at <= $T) AND (e.expired_at > $T OR e.expired_at IS NULL)
```

#### SearchFilters temporal API
```python
# DateFilter.comparison_operator enum member names:
# equals | not_equals | greater_than | less_than | greater_than_equal | less_than_equal | is_null | is_not_null
# (these map to Cypher operators =, <>, >, <, >=, <=, IS NULL, IS NOT NULL respectively)
# Outer list = OR; inner list = AND
SearchFilters(
    valid_at=[[DateFilter(date=T, comparison_operator=ComparisonOperator.less_than_equal)]],
    expired_at=[[DateFilter(date=T, comparison_operator=ComparisonOperator.greater_than)],
                [DateFilter(date=None, comparison_operator=ComparisonOperator.is_null)]]
)
```

#### Database indexes (Neo4j `RELATES_TO` edges)
`created_at_edge_index`, `expired_at_edge_index`, `valid_at_edge_index`, `invalid_at_edge_index` — all range indexes. FalkorDB mirrors these.

#### Multi-agent isolation
`group_id` on all nodes and edges. `retrieve_episodes(group_ids, reference_time)` is the group-scoped PIT entry point.

**Reference**: Zep paper arXiv:2501.13956v1; DeepWiki `getzep/graphiti` §3.2, §4.3, §5.3, §7.3

#### add_episode Pipeline — 7 Steps (source: `graphiti_core/graphiti.py`)

The single-episode processing pipeline runs seven async steps in order:

1. **Retrieve Previous Episodes** — `retrieve_episodes()` fetches relevant prior episodes to provide temporal context for entity/edge extraction.
2. **Get or Create Episode** — `EpisodicNode` is retrieved by UUID if provided, or created from `name`, `episode_body`, `source_description`, and `reference_time`.
3. **Extract and Resolve Nodes** — `extract_nodes()` identifies entities; `resolve_extracted_nodes()` deduplicates against existing graph nodes.
4. **Extract and Resolve Edges** — `_extract_and_resolve_edges()` calls `extract_edges()` + `resolve_extracted_edges()`; this is where bi-temporal annotation and conflict resolution occur.
5. **Extract Node Attributes** — `extract_attributes_from_nodes()` enriches entity nodes with additional LLM-extracted properties.
6. **Process and Save** — Episode, hydrated nodes, and entity edges are persisted to the graph; saga association applied if specified.
7. **Update Communities (Optional)** — If `update_communities=True`, runs community cluster update (label propagation). **Not run in `add_episode_bulk`.**

**`add_episode_bulk` exclusions:** batch path explicitly skips LLM date extraction and edge invalidation — `EntityEdge.valid_at`/`invalid_at` remain `null` for bulk-ingested episodes. `EpisodicNode.valid_at` IS still set from `RawEpisode.reference_time` even in bulk mode.

#### Two-Stage Edge Deduplication (source: `graphiti_core/utils/maintenance/edge_operations.py`)

Handled by `resolve_extracted_edges()` + `resolve_extracted_edge()`:

**Stage 1 — In-Memory Exact Match (fast path)**
Before any LLM calls: `fact` text is normalized; edges with identical `source_node_uuid`, `target_node_uuid`, and normalized `fact` are deduplicated and only one retained. Also applied in `resolve_extracted_edge()` to reuse verbatim-matching existing edges.

**Stage 2 — LLM-Based Deduplication + Contradiction Detection**
For edges that survive Stage 1: `resolve_extracted_edge(extracted_edge, related_edges, existing_edges)` prepares indexed context, calls `dedupe_edges.resolve_edge` LLM prompt, receives `EdgeDuplicate(duplicate_facts: list[int], contradicted_facts: list[int])`, validates indices, and invalidates contradicted edges by setting their `invalid_at` timestamp.

#### Community Detection — No Temporal Fields (source: `graphiti_core/utils/maintenance/graph_data_operations.py`)

`label_propagation()` operates on a `projection` (dict of node UUID → neighbors + edge counts). It does **not** use temporal fields from `EntityEdge` — `valid_at`, `invalid_at`, `expired_at` are ignored.

`CommunityNode` fields: `name`, `summary`, `name_embedding`, `uuid`, `group_id`, `labels`, `created_at`. **No `valid_at`, `invalid_at`, or `expired_at` — communities are not bi-temporally tracked.**

Label propagation steps: (1) each node starts in its own community; (2) each node adopts the plurality community of its neighbors; (3) tie-broken by community size; (4) iterate until convergence.

`update_community()` — called when a new entity is added — **does not re-run full label propagation**; only updates `summary` and embedding of the existing `CommunityNode`.

---

### Hindsight (`github.com/vectorize-io/hindsight`)

**DeepWiki**: Indexed — `deepwiki.com/vectorize-io/hindsight` §3.3 "Data Model and Schema", §4.7 "Observations and Consolidation"

Python + PostgreSQL engine. MIT license. 91.4% on LongMemEval. Rust component (`hindsight-cli`) is a CLI client only; core storage is PostgreSQL (not SQLite). Embedded mode (`hindsight-embed`) uses `pg0` — embedded PostgreSQL, not SQLite.

#### memory_units table — temporal columns (source: Alembic migrations)

| Column | Type | Nullable | Axis | Meaning |
|--------|------|----------|------|---------|
| `occurred_start` | `TIMESTAMP(tz)` | Yes | Valid time start | LLM-extracted from content; null if no explicit time |
| `occurred_end` | `TIMESTAMP(tz)` | Yes | Valid time end | LLM-extracted; null for open-ended/point-in-time facts |
| `mentioned_at` | `TIMESTAMP(tz)` | Yes | Tx time | Automatically set to ingestion time at `retain`; caller cannot override |
| `created_at` | `TIMESTAMP(tz)` | No | Row creation | Set at INSERT; `not null`; default `now()` |
| `updated_at` | `TIMESTAMP(tz)` | No | Last mutation | Updated on every write |

Note: `confidence_score` (`Float`, nullable, 0.0–1.0) exists in the DB but is **not exposed** in the `RecallResult` API response (`_fact_to_result` does not map it). `fact_type` values: `world | experience | observation | mental_model`. Note: `opinion` was a valid type historically but has been **deprecated and removed** — the database `CHECK` constraint no longer includes it.

#### Canonical example
"Alice got married in June 2024" retained in January 2025:
- `occurred_start = June 2024`, `occurred_end = June 2024` (LLM-extracted)
- `mentioned_at = January 2025` (auto-set at ingestion)

#### Temporal partial indexes (source: `b3c4d5e6f7g8_add_temporal_date_indexes.py`)
```sql
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_units_bank_occurred_start
    ON memory_units(bank_id, fact_type, occurred_start) WHERE occurred_start IS NOT NULL;
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_units_bank_occurred_end
    ON memory_units(bank_id, fact_type, occurred_end)   WHERE occurred_end IS NOT NULL;
CREATE INDEX CONCURRENTLY IF NOT EXISTS idx_memory_units_bank_mentioned_at
    ON memory_units(bank_id, fact_type, mentioned_at)   WHERE mentioned_at IS NOT NULL;
```

#### Conflict resolution — three strategies (no hard deletes, no soft-delete timestamps)
Run asynchronously after `retain`. LLM classifies new fact against existing observations:
1. **Redundant** — same info, different wording: observation `text` updated in-place
2. **Contradiction** — conflict: both states preserved with temporal narrative ("used to X, now Y")
3. **State update** — replacement: transition captured ("changed from X to Y")

Change recorded in `history` JSONB array:
```json
[{
  "previous_text": "Alex loves pizza.",
  "changed_at": "2026-03-20T10:00:00.000000+00:00",
  "reason": "Contradiction resolved",
  "new_source_memory_ids": ["uuid-of-source-fact-2"]
}]
```
History accessible via `GET /memories/{id}/history`.

#### TEMPR retrieval (4 strategies → RRF → cross-encoder)
Semantic (vector) + Keyword (BM25) + Graph traversal + Temporal spreading activation. Fusion: `score = Σ 1/(k + rank)` where **`k = 60`** (default in `hindsight_api/engine/search/fusion.py::reciprocal_rank_fusion`). After RRF: cross-encoder reranking; `recency_boost` (linear decay on `occurred_start`) + `temporal_boost`.

#### Temporal recall parameter
`query_timestamp: str | None` — the public API parameter name for temporal anchoring (ISO 8601 string). Inside the HTTP handler it is parsed to `datetime` and passed to `recall_async` as the local variable `question_date`. The core `MemoryEngine.recall_async()` accepts `question_date: datetime | None`. No direct `occurred_start`/`occurred_end` range filter in the API.

**Note on naming**: Early web summaries describe fields as `occurrence_time`/`mention_time`. Verified names: `occurred_start`, `occurred_end`, `mentioned_at`.

#### Full retain Pipeline — 5 Stages (source: `hindsight-api-slim/hindsight_api/engine/retain/orchestrator.py`)

The `retain_batch` orchestrator runs five stages in sequence:

1. **Fact Extraction** — LLM extracts facts in a five-dimension model (`what`, `when`, `where`, `who`, `why`); classifies into `world`, `experience`, `opinion`, `observation`; handles large content via chunking.
2. **Embedding Generation** — `embedding_processing.generate_embeddings_batch()` generates semantic vectors for each extracted fact; enables similarity search during recall.
3. **Deduplication** — facts grouped by event date; cosine similarity computed between new and existing facts. Note: there is no named `DUPLICATE_SIMILARITY_THRESHOLD = 0.95` constant in the Hindsight codebase. Actual thresholds: `0.7` for semantic link creation (`create_semantic_links_batch`), `0.6` for entity resolution (`EntityResolver._resolve_from_candidates`).
4. **Entity Resolution** — `EntityResolver` canonicalizes entity mentions: exact + fuzzy matching against existing canonical IDs; creates new canonical IDs for novel entities.
5. **Link Creation** — establishes four link types: temporal (event dates), semantic (embedding similarity), entity (same entity mentions), causal (cause-and-effect triples extracted by LLM).

#### ExtractedFact Structure (source: `hindsight-api-slim/hindsight_api/engine/retain/types.py`)

```python
@dataclass
class ExtractedFact:
    fact_text:        str                    # extracted fact text
    fact_type:        str                    # "world" | "experience" | "opinion" | "observation"
    entities:         list[str]             # entity names mentioned
    occurred_start:   str | None            # ISO 8601; LLM-resolved from event_date
    occurred_end:     str | None            # ISO 8601; LLM-resolved; None = point-in-time/open
    where:            str | None            # location
    causal_relations: list[CausalRelation]  # cause/effect pairs
    content_index:    int
    chunk_index:      int
    context:          str | None
    mentioned_at:     datetime              # auto-set to ingestion time
    metadata:         dict
    tags:             list[str]
```

`occurred_start`/`occurred_end` are populated **directly by the LLM** during extraction. `extract_facts_from_text()` passes the content and a `reference event_date` to anchor relative expressions — e.g. "last night" + `event_date=2023-08-14` → `occurred_start=2023-08-13`. After extraction, these values are carried over verbatim to `ProcessedFact` and are also used to augment fact text for embedding (enabling temporal matching during recall).

#### Extraction Modes — 5 Modes (source: `hindsight_api/config.py`, `RETAIN_EXTRACTION_MODES`)

| Mode | Behavior |
|------|----------|
| `concise` (default) | Selective — extracts only facts deemed important for long-term memory. Fast, general-purpose. |
| `verbose` | Detailed — extracts full context and relationships. Slower, more tokens. |
| `custom` | `HINDSIGHT_API_RETAIN_CUSTOM_INSTRUCTIONS` completely replace built-in extraction rules. |
| `verbatim` | Each chunk stored as-is; LLM still extracts entities, temporal info, and location for indexing. Useful for RAG where original text is preferred over LLM summaries. |
| `chunks` | Each chunk stored with no LLM extraction calls — embeddings only; user-provided entities only. Maximum ingestion speed/cost efficiency. |

#### Mental Models Hierarchy (source: `reflect` agent)

The `reflect` operation checks knowledge in priority order: **Mental Models → Observations → Raw Facts**.

- **Mental Models** (highest) — user-curated summaries on specific topics; if fresh and relevant, may fully answer a query.
- **Observations** (middle) — auto-consolidated from raw facts; capture patterns with evidence tracking back to source raw facts.
- **Raw Facts** (ground truth) — individual `world`/`experience` memories; used when upper tiers are absent, stale, or when specific detail is needed.

**Reference**: arXiv:2512.12818v1; DeepWiki `vectorize-io/hindsight` §3.3, §4.1, §4.7, §3.4

---

### memv (`github.com/vstorm-co/memv`)

**DeepWiki**: Indexed — `deepwiki.com/vstorm-co/memv` §2.3 "Data Models", §3.4 "Knowledge Deduplication and Supersession", §4.1 "KnowledgeStore"

Python. SQLite single-file (`memory.db`). All five stores — MessageStore, EpisodeStore, KnowledgeStore, VectorIndex (sqlite-vec), TextIndex (FTS5) — share one database. Closest SQLite-based stack to OpenFang's architecture.

#### SemanticKnowledge — full schema (source: `BiTemporalValidity` mixin + `SemanticKnowledge` model)

```python
class SemanticKnowledge(BaseModel):
    id: UUID                            # PK
    user_id: str | None                 # user/agent namespace
    statement: str                      # declarative fact text
    source_episode_id: UUID | None      # episode that generated this
    created_at: datetime                # tx time start; auto-set to utcnow()
    importance_score: float | None
    embedding: list[float] | None
    valid_at: datetime | None           # valid time start; None = unknown/always-true
    invalid_at: datetime | None         # valid time end; None = still true
    expired_at: datetime | None         # tx time end; None = current record
    superseded_by: UUID | None          # FK to replacing entry; creates audit chain
```

`is_valid_at(event_time: datetime) -> bool` — checks `valid_at <= event_time < invalid_at`.
`is_current() -> bool` — checks `expired_at is None`.

#### SQLite schema (source: `KnowledgeStore._create_table`)
```sql
CREATE TABLE IF NOT EXISTS semantic_knowledge (
    id              TEXT PRIMARY KEY,
    user_id         TEXT,
    statement       TEXT,
    source_episode_id TEXT,
    created_at      INTEGER,    -- Unix timestamp (NOT RFC3339)
    importance_score REAL,
    embedding       TEXT,       -- JSON array
    valid_at        INTEGER,    -- Unix timestamp; NULL = unknown/always-true
    invalid_at      INTEGER,    -- Unix timestamp; NULL = still true
    expired_at      INTEGER,    -- Unix timestamp; NULL = current record
    superseded_by   TEXT        -- UUID of replacement entry
);
CREATE INDEX idx_sk_valid_at   ON semantic_knowledge(valid_at);
CREATE INDEX idx_sk_expired_at ON semantic_knowledge(expired_at);
CREATE INDEX idx_sk_user_id    ON semantic_knowledge(user_id);
CREATE INDEX idx_sk_episode    ON semantic_knowledge(source_episode_id);
```
**Note**: temporal fields stored as INTEGER Unix timestamps, not TEXT RFC3339. OpenFang uses TEXT RFC3339 — a schema difference to maintain when drawing on memv patterns.

#### Supersession mechanism (source: `_pipeline.py._handle_supersedes`)
LLM classifies extracted fact as `new`, `update`, or `contradiction`. On `update`/`contradiction`:
1. LLM provides `supersedes` index into numbered list of existing knowledge
2. `KnowledgeStore.invalidate_with_successor(old_id, new_id)` → sets `old.expired_at = utcnow()`, `old.superseded_by = new_id`
3. Fallback: if index invalid, vector search with `contradiction_threshold = 0.7` cosine similarity

Old entry is **never deleted**. `memory.list_knowledge(include_expired=True)` retrieves full history.

#### Point-in-time query
`KnowledgeStore.get_valid_at(event_time, include_expired=False)` — filters by `valid_at`/`invalid_at` range. `include_expired=False` (default) implicitly excludes `expired_at IS NOT NULL`.

#### Public API — Memory.add_exchange() / Memory.retrieve() (source: `memv/memory/_api.py`)

```python
# Write path — records a conversation exchange
Memory.add_exchange(
    user_id:           str,
    user_message:      str,
    assistant_message: str,
    timestamp:         datetime | None = None,  # defaults to utcnow()
) -> None
# Internally creates two Message objects, appends to lifecycle.messages.
# If auto_process enabled and batch_threshold reached → schedules background processing for user_id.

# Read path — semantic retrieval with bi-temporal filter
Memory.retrieve(
    query:          str,
    user_id:        str,           # required; enforces per-user privacy
    top_k:          int   = 10,
    vector_weight:  float = 0.5,   # 1.0 = pure vector, 0.0 = pure text search
    at_time:        datetime | None = None,   # valid-time filter: only knowledge valid at this datetime
    include_expired: bool = False,             # if True, includes superseded records (full history)
) -> list[RetrievedKnowledge]
```

`at_time` maps directly to `KnowledgeStore.get_valid_at(event_time)` — the PIT query on `valid_at`/`invalid_at`. `include_expired=False` implicitly excludes rows where `expired_at IS NOT NULL`.

#### ExtractedKnowledge — Exact Fields (source: `memv/pipeline/types.py`)

```python
class ExtractedKnowledge(BaseModel):
    statement:      str                                    # concrete, self-contained declarative fact
    knowledge_type: Literal["new", "update", "contradiction"]  # storage handling
    temporal_info:  str                                    # human-readable time expression ("since Jan 2024")
    valid_at:       datetime | None                        # world-time start; None = unknown/always-true
    invalid_at:     datetime | None                        # world-time end; None = still valid
    confidence:     float                                  # 0.0–1.0; items < 0.7 filtered out
    supersedes:     int | None                             # index into existing knowledge list to replace
```

Extraction quality tests applied to every item: **Persistence, Specificity, Utility, Independence**. Atomization rules: no pronouns, no relative time, third person, coreference resolved.

#### backfill_temporal_fields (source: `memv/pipeline/temporal.py`)

Runs inside `Pipeline._process_episode()`, **after** `PredictCalibrateExtractor` returns. For each item where `temporal_info` is set, resolves relative time expressions in `temporal_info` to absolute `datetime` values using `episode.end_time` as the reference point. Populates or corrects `valid_at` / `invalid_at`. Example: `temporal_info="since January 2024"` + `end_time=2025-03-21` → `valid_at=2024-01-01`.

#### Predict-Calibrate Extraction Pipeline (source: `memv/pipeline/extract.py`)

**Phase 1 — Prediction**
1. Retrieve existing knowledge relevant to the episode via `_lc.retriever.retrieve` (bounded by `max_statements_for_prediction`).
2. LLM called with `prediction_prompt(existing_knowledge, episode_title)` to generate a prediction of what the conversation will contain.
3. Cold-start: if no prior knowledge exists, LLM call is skipped entirely; `_predict()` returns empty string.

**Phase 2 — Calibration (Gap Extraction)**
1. LLM given its own prediction + `original_messages` of the episode (not a narrative summary — prevents hallucination).
2. `extraction_prompt_with_prediction` instructs LLM to extract **only gaps** — information missing, misrepresented, or providing new detail beyond the prediction.
3. Optional inputs: `reference_timestamp`, `existing_knowledge_numbered`.
4. Each extracted item classified as `new`, `update`, or `contradiction`.
5. `_validate_extraction` drops any item with `confidence < 0.7`.
6. `backfill_temporal_fields` resolves any remaining relative `temporal_info` to absolute timestamps.

**Reference**: DeepWiki `vstorm-co/memv` §2.3, §3.4, §4.1

---

### XTDB

**DeepWiki**: N/A (not a GitHub repo)

Purpose-built immutable SQL database with first-class bitemporality. Design principle: "Bitemporality has to be baked in. It should work automatically, across all your records. By default you shouldn't even know it exists; a good bitemporal system assumes you're talking about the state of the world as-of-now unless you specify otherwise."

Not embeddable (JVM runtime). Relevant as the theoretical reference for the default-PIT-query design principle that OpenFang should aim toward.

---

### mem7 (`github.com/mem7ai/mem7`)

**DeepWiki**: Indexed — `deepwiki.com/mem7ai/mem7` §2.3 "Memory Decay", §3.3 "History Store and Audit Trail"

Rust-native agent memory. **Purely uni-temporal** — no `valid_at` or `invalid_at` fields. Relevant to OpenFang for two specific patterns: the audit trail design and the Ebbinghaus decay model.

#### MemoryItem — core struct (source: `src/mem7-core/src/types.rs`)
```rust
pub struct MemoryItem {
    pub id: Uuid,
    pub text: String,
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub run_id: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: String,          // ISO 8601
    pub updated_at: String,          // ISO 8601; mutated in-place on update
    pub score: Option<f32>,          // confidence/relevance score
    pub last_accessed_at: Option<String>,  // ISO 8601; drives decay
    pub access_count: u32,           // rehearsal count; drives decay
    pub memory_type: Option<String>, // factual | preference | procedural | episodic
}
```

#### History Store — audit trail (source: `mem7-history` crate, `mem7_history.db`)
Every `ADD`, `UPDATE`, `DELETE` triggers a `MemoryEvent` stored in SQLite via `tokio-rusqlite`:
```rust
pub struct MemoryEvent {
    pub id: String,
    pub memory_id: String,
    pub old_value: Option<String>,   // prior text value; None on ADD
    pub new_value: Option<String>,   // new text value; None on DELETE
    pub action: MemoryAction,        // Add | Update | Delete
    pub created_at: String,          // ISO 8601 event timestamp
}
```
`MemoryEngine.history(memory_id)` returns the full event log for a memory.

#### Ebbinghaus decay (source: `src/mem7-store/src/decay.rs`)
```
S = base_half_life × (1 + rehearsal_factor × ln(1 + access_count))
R = exp(-(age / S)^decay_shape)
score_final = raw_score × (min_retention + (1 - min_retention) × R)
```
Default config: `base_half_life_secs = 604800.0f64 (7d)`, `decay_shape = 0.8`, `min_retention = 0.1`, `rehearsal_factor = 0.5`. Decay is **disabled by default** in the core engine (must be explicitly enabled in config); decay logic in `src/mem7-store/src/decay.rs`.

This decay model is architecturally analogous to openfang-memory's `consolidate_decay` / confidence decay, but implemented in Rust with a precise mathematical formula.

**Storage backends**: SQLite (`tokio-rusqlite`) for history; Kuzu (embedded, `mem7_graph.kuzu`) or Neo4j for graph; FlatIndex (in-memory) or Upstash for vectors.

#### Full Write Path — 6 Steps (source: `src/mem7-store/src/add.rs`, `src/mem7-store/src/pipeline.rs`)

Entry point: `MemoryEngine::add()` — dispatches to `add_with_inference` (LLM mode) or `add_raw` (direct storage).

1. **Vision Description** — if vision enabled, `describe_images()` scans messages for image URLs and calls `VISION_DESCRIBE_PROMPT` to produce text descriptions appended to message content before LLM processing.
2. **LLM Fact Extraction** — `extract_facts()` selects `USER_FACT_EXTRACTION_PROMPT` or `AGENT_FACT_EXTRACTION_PROMPT` based on role + `agent_id`; LLM categorizes facts as `factual`, `preference`, `procedural`, or `episodic`; returns JSON.
3. **Deduplication with ADD/UPDATE/DELETE Decisions** — `decide_memory_updates()` performs vector search for related existing memories; sends new facts + retrieved memories to LLM; LLM returns one of four actions per fact: `ADD`, `UPDATE`, `DELETE`, or `NONE`. Supersession is entirely LLM-driven.
4. **Graph Relation Extraction (concurrent)** — `GraphPipeline` runs `extract_entities()` (nodes) and `extract_relations()` (triples) concurrently with vector processing; conflict detection performs soft-delete (`valid = false`) on contradicted relations.
5. **Vector Indexing** — for ADD/UPDATE facts: `build_memory_payload()` constructs payload; embedding generated; `VectorIndex::insert()` called. UPDATE: retrieves existing entry, modifies, upserts via `VectorIndex::update()`.
6. **History Recording** — every ADD/UPDATE/DELETE action calls `HistoryStore::add_event()` with `MemoryAction` + old/new values → permanent audit trail.

#### Rust Traits: VectorIndex + GraphStore (source: `src/mem7-core/src/traits.rs`)

**`VectorIndex` trait** — all methods async:
```rust
async fn insert(&self, id: Uuid, vector: &[f32], payload: serde_json::Value) -> Result<()>
async fn search(&self, query: &[f32], limit: usize, filters: Option<&MemoryFilter>) -> Result<Vec<VectorSearchResult>>
async fn delete(&self, id: &Uuid) -> Result<()>
async fn update(&self, id: &Uuid, vector: Option<&[f32]>, payload: Option<serde_json::Value>) -> Result<()>
async fn get(&self, id: &Uuid) -> Result<Option<(Vec<f32>, serde_json::Value)>>
async fn list(&self, filters: Option<&MemoryFilter>, limit: Option<usize>) -> Result<Vec<(Uuid, serde_json::Value)>>
```
Known impl: `UpstashVectorIndex`.

**`GraphStore` trait** — all methods async:
```rust
async fn add_entities(&self, entities: &[Entity], filter: &MemoryFilter) -> Result<()>
async fn add_relations(&self, relations: &[Relation], entities: &[Entity], filter: &MemoryFilter) -> Result<()>
async fn search(&self, query: &str, filter: &MemoryFilter, limit: usize) -> Result<Vec<GraphSearchResult>>
async fn search_by_embedding(&self, embedding: &[f32], filter: &MemoryFilter, threshold: f32, limit: usize) -> Result<Vec<GraphSearchResult>>
async fn invalidate_relations(&self, triples: &[(String, String, String)], filter: &MemoryFilter) -> Result<()>
async fn rehearse_relations(&self, triples: &[(String, String, String)], filter: &MemoryFilter, now: &str) -> Result<()>
async fn delete_all(&self, filter: &MemoryFilter) -> Result<()>
async fn reset(&self) -> Result<()>
```
Known impls: `FlatGraph` (in-memory), `KuzuGraphStore` (embedded Cypher), `Neo4jGraphStore` (Bolt).

#### Dual-Path Recall — tokio::join! (source: `src/mem7-store/src/search.rs`)

`MemoryEngine::search_with_options()` fires two async tasks concurrently:

```rust
let (vector_results, graph_results) = tokio::join!(
    vector_search(query_embedding, filters),
    graph_search(query_text, filters)
);
```

| Path | Returns | Post-processing |
|------|---------|-----------------|
| Vector | `Vec<MemoryItem>` — factual memories with content, metadata, similarity score | Optional reranking → Ebbinghaus decay (score × `R(last_accessed_at, access_count)`) → context-aware scoring (`memory_type` + `task_type` coefficient) → threshold filter |
| Graph | `Vec<GraphRelation>` — subject-predicate-object triples (e.g. `USER -[loves_playing]-> tennis`) | Optional context-aware score adjustment |

Results are **not interleaved or deduplicated** against each other. Final `SearchResult` struct has two separate fields: `memories: Vec<MemoryItem>` and `relations: Vec<GraphRelation>`. After both paths complete, rehearsal is triggered asynchronously to update `last_accessed_at` + `access_count` for all retrieved items.

**Reference**: DeepWiki `mem7ai/mem7` §2.3, §3.3

---

## Decision

### Scope — What This ADR Covers (Phase 1)

This ADR delivers **bi-temporal annotation** on the existing mutable row store. It does NOT deliver full bi-temporal correctness, which requires append-only row versioning (Phase 2). The distinction is explicit:

**Phase 1 (this ADR):** Add three nullable columns and fix `forget()` to record `deleted_at`. Enables point-in-time reconstruction for _deletion events_. Provides world-time validity bounds for _new_ memories created after migration. Does not retroactively reconstruct mutable field history (`confidence`, `access_count`, `accessed_at`) — those fields are still mutated in-place.

**Phase 2 (future ADR):** Append-only row versioning. Every mutation to `confidence`, `metadata`, or `content` creates a new row version with a new `tx_from`. Enables full point-in-time reconstruction of all fields. This is the design target of memv's "Messages → Episodes → Knowledge" layering.

This boundary is explicit because adding temporal annotations to a mutable store without acknowledging the limitations is architecturally misleading. As Codex review identified: "Adding `valid_at`/`invalid_at`/`deleted_at` to a mutable row does not make it bi-temporal."

### New Columns

Three nullable columns are added to the `memories` table via a versioned migration (not `init_schema`):

```sql
-- Migration v9 (current highest is v8; SCHEMA_VERSION = 8 in migration.rs)
ALTER TABLE memories ADD COLUMN valid_at   TEXT DEFAULT NULL;
ALTER TABLE memories ADD COLUMN invalid_at TEXT DEFAULT NULL;
ALTER TABLE memories ADD COLUMN deleted_at TEXT DEFAULT NULL;
```

All three are `TEXT` RFC3339 strings, consistent with the existing `created_at` and `accessed_at` columns. The existing `expires_at` INTEGER column is intentionally retained as a separate concept (TTL-based eviction policy) and is not merged into `invalid_at`.

#### Column Semantics

| Column | Axis | Nullable meaning | Set by |
|--------|------|-----------------|--------|
| `valid_at` | Valid time start | `NULL` = same as `created_at`; fact was true from the moment it was recorded | Caller on `remember()` / `remember_with_embedding()` |
| `invalid_at` | Valid time end | `NULL` = still true in the world | Conflict resolution / consolidation engine |
| `deleted_at` | Transaction time end | `NULL` = not yet invalidated by the system | `forget()` |

**`valid_at` semantics clarified:** `NULL` does not mean "unknown start". It means "this fact was true starting from `created_at`." Callers with explicit world-time knowledge (e.g., a document with a known effective date) supply `valid_at` explicitly. Callers without it (conversational memory, tool responses) leave it `NULL`. This is backward-compatible: all existing rows have `valid_at = NULL`, interpreted as `COALESCE(valid_at, created_at)`.

**`invalid_at` semantics:** Set when a fact is superseded by a correction, not when it is deleted. A memory can be `invalid_at = T` (the world-state changed at T) while still present and `deleted = 0`. This is the Graphiti pattern: contradicted facts are invalidated but not deleted.

**`deleted_at` semantics:** Transaction time. Set by `forget()` to `now()`. Records precisely when the system decided to remove this fact. Pre-migration rows with `deleted = 1` and `deleted_at = NULL` are an explicit known gap — their deletion timestamp is permanently unknown.

**`expires_at` is NOT `invalid_at`:** `expires_at` is an INTEGER Unix timestamp set by the eviction policy for working-memory TTL. It drives `expired_ids()` at line 1058 of `rvf_store.rs`. It represents scheduled policy-based eviction, not world-time truth invalidity. Do not conflate.

### `forget()` Change

```rust
// Before (line 743, rvf_store.rs):
"UPDATE memories SET deleted = 1 WHERE uuid = ?1"

// After:
"UPDATE memories SET deleted = 1, deleted_at = ?2 WHERE uuid = ?1"
// where ?2 = Utc::now().to_rfc3339()
```

This is the only behavioral change to `forget()`. The RVF bitmap deletion and `content_map` removal are unchanged. The `deleted = 1` flag is retained as a fast-path exclusion for current-state queries.

### Conflict Resolution — New Pattern

**Current state of `consolidation.rs`:** The consolidation engine only performs confidence decay (`memories_merged: 0` — verified at `consolidation.rs:81`). There is no active conflict-resolution logic today. `invalid_at` invalidation on conflict is a **new capability** — not a wiring of existing consolidation behavior. The `invalidate()` method proposed here is the mechanism that future conflict resolution (or an external caller) will invoke.

When a corrected memory supersedes an existing one, the correct bi-temporal operation is:

```sql
-- Step 1: Bound the superseded fact's world-time validity
UPDATE memories
   SET invalid_at = :now
 WHERE uuid = :superseded_id;

-- Step 2: Insert the corrected fact with explicit valid_at
INSERT INTO memories
    (uuid, agent_id, content, source, scope, confidence, metadata,
     created_at, accessed_at, access_count, deleted, has_embedding,
     importance, valid_at, invalid_at, deleted_at)
VALUES
    (:new_uuid, :agent_id, :corrected_content, ...,
     :now, :now, 0, 0, ..., 1.0,
     :valid_at_override_or_null, NULL, NULL);
```

This is append-only at the knowledge level (new row for correction) while the superseded row is invalidated but not deleted. This matches the Graphiti edge-invalidation pattern adapted for flat SQLite rows.

### Point-in-Time Reconstruction Query

The following query reconstructs "what did agent X believe was true in the world at world-time `vt`, as known to the system at system-time `st`":

```sql
SELECT *
FROM memories
WHERE agent_id          = :agent_id
  AND created_at        <= :st                                  -- system saw it by st
  AND (deleted_at IS NULL OR deleted_at > :st)                 -- system hadn't deleted it yet
  AND COALESCE(valid_at, created_at) <= :vt                    -- fact was valid by vt
  AND (invalid_at IS NULL OR invalid_at > :vt)                 -- fact hadn't been superseded yet
ORDER BY COALESCE(valid_at, created_at) DESC;
```

**Parameters:**
- `:vt` — valid time: the world-time moment being reconstructed ("what was true on Feb 15?")
- `:st` — system time: the system's knowledge horizon ("as the system knew it on March 1?")
- For "what does the agent know right now?" pass both as `now()`.

**Critical corrections from Codex review (incorporated):**

1. `COALESCE(valid_at, created_at) <= :vt` — not `(valid_at IS NULL OR valid_at <= :vt)`. The latter allows rows inserted after `:vt` to appear when `valid_at IS NULL`. The `COALESCE` correctly interprets NULL as "fact started at creation time."

2. `AND deleted = 0` is **excluded from the historical query**. The `deleted = 1` flag is a current-state convenience; applying it to historical queries incorrectly excludes records that were alive at time T but deleted afterward. The `deleted_at > :st` predicate handles this correctly.

3. Two parameters (`:vt`, `:st`) are required for a genuine bi-temporal query. Single-parameter "as of T" queries collapse both axes to the same value and are valid for the common case but must not be confused with full bi-temporal semantics.

**Current-state query (fast path):**

```sql
SELECT * FROM memories
WHERE agent_id = :agent_id
  AND deleted = 0
  AND (invalid_at IS NULL OR invalid_at > :now)
  AND (expires_at IS NULL OR expires_at > :now_unix);
-- :now         = Utc::now().to_rfc3339()   (RFC3339 TEXT, matches invalid_at format)
-- :now_unix    = Utc::now().timestamp()     (INTEGER, matches expires_at format)
-- NOTE: do NOT use datetime('now') — SQLite's format differs from RFC3339 and
--       lexicographic comparison across "YYYY-MM-DD HH:MM:SS" vs RFC3339 is unsafe.
```

**Important:** The current `recall()` text path uses only `WHERE deleted = 0` with no `invalid_at` or `expires_at` filters (`rvf_store.rs:563`). Adding `invalid_at` and `expires_at` predicates to the current-state query **is a behavior change** — it will hide memories where `invalid_at` has been set (superseded facts) and memories past their TTL. This is the correct target behavior but callers must be made aware. The `expires_at` filter in particular deduplicates the work already done by `expired_ids()`/eviction; this should be coordinated with the eviction policy during Phase 1 implementation.

### Required Indexes

```sql
-- Migration v9 (add inside migrate_v9() alongside the ALTER TABLE statements)
CREATE INDEX IF NOT EXISTS idx_memories_temporal_valid
    ON memories (agent_id, created_at, deleted_at);

CREATE INDEX IF NOT EXISTS idx_memories_temporal_world
    ON memories (agent_id, valid_at, invalid_at);
```

Note: `COALESCE(valid_at, created_at)` in the PIT query prevents SQLite from using `idx_memories_temporal_world` directly when `valid_at IS NULL`. For workloads where PIT reconstruction is frequent, a computed column or a partial index on `valid_at IS NOT NULL` may be warranted. This is deferred to Phase 2.

### Migration Correctness

**Use `migration.rs`, not `init_schema`.** The existing versioned migration mechanism at `crates/openfang-memory/src/migration.rs` must be used. `init_schema()` runs on every open and silently ignores `ALTER TABLE` failures. This is unacceptable for temporal columns — a partial migration leaves the store in a mixed semantic state.

**Current highest migration version: v8** (`SCHEMA_VERSION: u32 = 8`, `migrate_v8()` in `migration.rs:42`). The Phase 1 migration is **v9**.

The migration mechanism uses explicit `migrate_vN()` functions gated by `if current_version < N` in `run_migrations()`. There is no `MigrationStep` struct. The correct implementation:

```rust
// In migration.rs — add to run_migrations() after the migrate_v8 block:
if current_version < 9 {
    migrate_v9(conn)?;
}

// Update constant:
const SCHEMA_VERSION: u32 = 9;

// Add the function:
fn migrate_v9(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Per-column guards (pattern from existing migrations in migration.rs).
    // Each column must be guarded separately: a partial failure on one ALTER TABLE
    // would leave valid_at added but invalid_at/deleted_at absent, and a rerun
    // guarded only on valid_at would incorrectly skip the remaining columns.
    if !column_exists(conn, "memories", "valid_at") {
        conn.execute("ALTER TABLE memories ADD COLUMN valid_at TEXT DEFAULT NULL", [])?;
    }
    if !column_exists(conn, "memories", "invalid_at") {
        conn.execute("ALTER TABLE memories ADD COLUMN invalid_at TEXT DEFAULT NULL", [])?;
    }
    if !column_exists(conn, "memories", "deleted_at") {
        conn.execute("ALTER TABLE memories ADD COLUMN deleted_at TEXT DEFAULT NULL", [])?;
    }
    // CREATE INDEX IF NOT EXISTS is idempotent — no guard needed.
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_temporal_valid
             ON memories (agent_id, created_at, deleted_at);
         CREATE INDEX IF NOT EXISTS idx_memories_temporal_world
             ON memories (agent_id, valid_at, invalid_at);",
    )?;
    Ok(())
}
```

**Backfill policy for existing `deleted = 1` rows:**

Rows already at `deleted = 1` before migration will have `deleted_at = NULL` after migration. This is an explicit known gap — their deletion timestamp is permanently unknown. Setting `deleted_at = migration_timestamp` would fabricate history. The correct policy is:

- `deleted = 1` AND `deleted_at IS NULL` → pre-migration tombstone; deletion time unknown
- `deleted = 1` AND `deleted_at IS NOT NULL` → post-migration soft-delete; deletion time known
- Any historical PIT query that treats `deleted_at IS NULL` as "never deleted" will incorrectly resurrect pre-migration tombstones into past snapshots

**Mitigation:** The PIT query above uses `deleted_at > :st`, which handles this correctly only if `:st` is a time after migration. For `:st` before migration, pre-migration tombstones will appear in the result set (incorrect). This is an unavoidable consequence of adding temporal tracking after data exists. Document this limitation explicitly in the PIT query API.

### `remember()` / `remember_with_embedding()` API Change

Both functions gain an optional `valid_at: Option<DateTime<Utc>>` parameter:

```rust
pub fn remember_with_embedding(
    &self,
    agent_id: AgentId,
    content: &str,
    source: MemorySource,
    scope: &str,
    metadata: HashMap<String, serde_json::Value>,
    embedding: Option<&[f32]>,
    valid_at: Option<DateTime<Utc>>,  // NEW — None = same as created_at
) -> OpenFangResult<MemoryId>
```

The `valid_at` is stored as RFC3339 or `NULL`. **This is a Rust API signature change** — all existing call sites to `remember_with_embedding()` and `remember()` must be updated to pass `valid_at: None`. The behavioral default is backward-compatible (existing memories behave as before) but the Rust compiler will require all callers to be updated. Search for all call sites before implementing: `grep -rn "remember_with_embedding\|\.remember(" crates/`.

### RVF Vector Layer Limitation

**The RVF bitmap is not historically reconstructable.** `forget()` calls `rvf.delete(&[rvf_id])` which permanently removes the vector from the HNSW index and removes it from `content_map`. Even after adding `deleted_at` to SQLite, the vector for a deleted memory is gone. This means:

- **Text-path recall** (SQL-based): fully supports historical PIT queries via the new temporal columns.
- **Vector-path recall** (RVF HNSW + SQL post-filter): returns only vectors that are currently alive in the index. Historical vector recall requires a separate read-only HNSW snapshot — deferred to Phase 2.

PIT queries must use the text path (`recall` without embedding) to guarantee historical correctness. This is an explicit architectural constraint until Phase 2 addresses RVF snapshotting.

---

## Consequences

### Positive

- `forget()` now records when a deletion occurred — all post-migration soft-deletes carry a `deleted_at` timestamp
- Conflict resolution has a first-class pattern: invalidate old, insert new, preserving both beliefs
- Point-in-time reconstruction is possible for deletion events and world-time validity from the migration date forward
- The four-column model (`valid_at`, `invalid_at`, `created_at`, `deleted_at`) exactly matches Graphiti's proven schema, adapted to SQLite
- Migration is backward-compatible: all existing callers pass `valid_at = None`, and all existing rows have `NULL` for the new columns with correct `COALESCE` semantics
- The `deleted = 0` fast path guard is retained in the current-state query, but adding `invalid_at` and `expires_at` predicates to that query **is a behavior change with query-cost impact** (two additional column checks per row, two new nullable columns checked). Callers that relied on `recall()` returning soft-deleted-as-invalid records will see different results. This is intentional and correct, but must be communicated as a breaking change.

### Negative

- **Phase 1 is partial bi-temporal, not full.** Fields mutated in-place (`confidence`, `access_count`, `accessed_at`) have no version history. PIT queries reconstruct the existence and validity bounds of a memory but not its full mutable state at time T. This must be communicated clearly to any callers building audit features on top of this layer.
- **Pre-migration tombstones are a known gap.** Rows with `deleted = 1` before migration have `deleted_at = NULL`. Historical queries crossing the migration boundary will produce incorrect results for those rows.
- **Vector-path historical recall is not supported.** RVF bitmap deletions are destructive. PIT queries are restricted to the SQL text path.
- Two new indexes add write overhead on every `INSERT` and `UPDATE` that touches the indexed columns.

### Neutral

- `expires_at` semantics are unchanged — TTL-based eviction continues via `expired_ids()` as before
- The `MemoryFilter` struct (`openfang-types/src/memory.rs`) does not need changes for Phase 1; `before`/`after` filters continue to operate on `created_at`. A `as_of_valid_time` filter field is a Phase 2 addition.

---

## Phase 2 — Append-Only Versioning (Future ADR)

Phase 2 upgrades from bi-temporal annotation to full bi-temporal correctness. The key change: every mutation creates a new row version rather than updating in-place.

```sql
-- Phase 2 table shape (sketch)
ALTER TABLE memories ADD COLUMN tx_from  TEXT NOT NULL DEFAULT (datetime('now'));
ALTER TABLE memories ADD COLUMN tx_to    TEXT DEFAULT NULL;  -- NULL = current version
ALTER TABLE memories ADD COLUMN prev_uuid TEXT DEFAULT NULL; -- links version chain
```

With `tx_to` tracking when each row version was superseded by a new version, full PIT queries over all fields become correct. `confidence` at time T, `metadata` at time T, and `content` at time T are all reconstructable.

This also unlocks proper RVF snapshotting: rather than deleting from the HNSW index on `forget()`, Phase 2 would maintain a separate "deleted set" per snapshot epoch, enabling vector recall as-of-time.

Phase 2 is not in scope for this ADR and requires its own implementation plan, migration strategy, and performance analysis. memv's architecture (layered: Messages → Episodes → Knowledge) is the reference design.

---

## Implementation Order

### Prerequisites

- `cargo test -p openfang-memory` passing (Sherlock baseline before any schema change)
- Current highest migration is v8 (`SCHEMA_VERSION = 8`). Phase 1 is v9 — verified against `migration.rs`

### Steps

1. **Verify `migration.rs`** — current highest is v8 (`SCHEMA_VERSION = 8`). Phase 1 migration is v9. Update `SCHEMA_VERSION` to `9`, add `if current_version < 9 { migrate_v9(conn)?; }` in `run_migrations()`.
2. **Write `migrate_v9()`** with the three `ALTER TABLE` statements (guarded by `column_exists()`) and two `CREATE INDEX` statements.
3. **Update `forget()`** — add `deleted_at = ?2` binding with `Utc::now().to_rfc3339()`.
4. **Update `remember_with_embedding()`** — add `valid_at: Option<DateTime<Utc>>` parameter; pass through to `INSERT`.
5. **Update `remember()`** — delegate to `remember_with_embedding` with `valid_at: None`.
6. **Implement `invalidate()`** — new method on `RvfSemanticStore` that sets `invalid_at = now()` on a memory by ID (for conflict resolution use).
7. **Add PIT query function** — `recall_as_of(agent_id, valid_time, system_time, limit)` on the text path only. Document RVF restriction explicitly.

### Test Requirements (RED before GREEN)

- `test_forget_records_deleted_at` — after `forget()`, row has `deleted_at IS NOT NULL`
- `test_pit_query_excludes_post_deletion` — a record deleted at T+1 appears in PIT query at T, absent at T+2
- `test_pit_query_excludes_future_insert` — a record created at T+1 does not appear in PIT query at T
- `test_pit_query_coalesce_null_valid_at` — record with `valid_at = NULL` uses `created_at` as world-time start
- `test_pit_query_respects_invalid_at` — a record with `invalid_at = T` appears before T, absent at T and after
- `test_remember_explicit_valid_at` — record stored with `valid_at = T-7d` retrieves correctly in PIT query at T-3d
- `test_invalidate_sets_invalid_at` — calling the new `invalidate(id)` method sets `invalid_at IS NOT NULL` on the target row; PIT query at T (before) returns the record, PIT query at T+ε (after `invalid_at`) does not. Note: `consolidation.rs` has no conflict resolution today (`memories_merged: 0`) — this test exercises `invalidate()` directly, not via consolidation.
- `test_pre_migration_tombstone_gap` — documents (not hides) the known gap: pre-migration `deleted = 1` rows have `deleted_at = NULL`
- `test_current_state_query_excludes_invalidated` — after Phase 1 current-state predicates are in place, a record with `invalid_at` set to a past time is absent from `recall()`. This IS a behavior change vs pre-migration `recall()` (which only filtered `deleted = 0`); the test explicitly asserts the new behavior and documents the break.

### Acceptance Criteria

```bash
cargo test -p openfang-memory                              # All tests pass including new PIT tests
cargo clippy --workspace --all-targets -- -D warnings      # Zero warnings
cargo build --workspace --lib                              # Clean build
```

PIT query manual verification:
```sql
-- Verify: row exists in past, excluded after deletion
SELECT uuid, content, created_at, deleted_at
FROM memories
WHERE agent_id = '<test_id>'
  AND created_at <= '<T>'
  AND (deleted_at IS NULL OR deleted_at > '<T>');
```

---

## References

### Reference Implementations

| Project | URL | DeepWiki | Relevance |
|---------|-----|----------|-----------|
| Graphiti | `github.com/getzep/graphiti` | Indexed — §3.2, §4.3, §5.3, §7.3 | Full bi-temporal `EntityEdge` schema; `resolve_edge_contradictions` logic; Cypher PIT query; `SearchFilters` temporal API; Neo4j/FalkorDB indexes |
| Zep paper | arXiv:2501.13956 | N/A | Formal description of Graphiti's temporal edge schema and PIT query semantics |
| Hindsight | `github.com/vectorize-io/hindsight` | Indexed — §3.3, §4.1, §4.7, §3.4 | `occurred_start`/`occurred_end`/`mentioned_at`; PostgreSQL storage; TEMPR 4-strategy retrieval; `history` JSONB audit trail; 91.4% LongMemEval |
| Hindsight paper | arXiv:2512.12818 | N/A | §2.1 temporal metadata; separation of world-time interval from ingestion time |
| memv | `github.com/vstorm-co/memv` | Indexed — §2.3, §3.4, §4.1 | `valid_at`/`invalid_at`/`expired_at`/`superseded_by`; SQLite INTEGER timestamps; `BiTemporalValidity` mixin; supersession via `invalidate_with_successor`; `get_valid_at` PIT query |
| mem7 | `github.com/mem7ai/mem7` | Indexed — §2.3, §3.3 | Rust-native; uni-temporal; `MemoryEvent` audit trail (SQLite); Ebbinghaus decay formula; audit trail pattern reference |
| XTDB | `xtdb.com` | N/A | Gold standard bi-temporal theory; "baked in" design principle |
| JUXT bitemporality | `juxt.pro/blog/bitemporality-more-than-a-design-pattern` | N/A | Design philosophy: bi-temporal as a default assumption, not an opt-in feature |

### Prior Art in This Codebase

- `ADR-004-content-durability.md` — durability guarantees for the existing memory store
- `ADR-006-episodic-eviction.md` — `expires_at` TTL semantics (distinct from `invalid_at`)
- `ADR-007-witness-chain.md` — session-level audit chain; bi-temporal memory is the row-level analogue
- `ADR-009-memory-intelligence.md` — SONA consolidation engine; conflict resolution is a consumer of `invalidate()`
- `crates/openfang-memory/src/rvf_store.rs` — `init_schema` (line 138), `forget` (line 718), `remember_with_embedding` (line 240)
- `crates/openfang-memory/src/migration.rs` — versioned migration mechanism that Phase 1 must use

### Literature

- Martin Fowler, "Temporal Patterns" — canonical definition of valid-time vs. transaction-time
- SQL:2011 — standardised `PERIOD FOR SYSTEM_TIME` and `PERIOD FOR APPLICATION_TIME` clauses
- André Lindenberg, "The Memory Problem No One Talks About: Why AI Agents Need Two Clocks" (March 2026) — the article that prompted this ADR

### Peer Review

Codex peer review conducted 2026-03-21 before finalising this ADR. Nine issues identified and incorporated:
1. Query predicate `(valid_at IS NULL OR valid_at <= T)` corrected to `COALESCE(valid_at, created_at) <= T`
2. Two-parameter query (`:vt`, `:st`) required for genuine bi-temporal semantics
3. `AND deleted = 0` removed from historical PIT query (breaks historical reconstruction)
4. Row mutation scope acknowledged — Phase 1 is annotation, not full bi-temporal
5. `created_at` as transaction-time start is insufficient for mutated rows — deferred to Phase 2
6. `expires_at` is a third concept (TTL policy) — not collapsed into `invalid_at`
7. RFC3339 / INTEGER timestamp mixing flagged — all new columns use TEXT RFC3339
8. Migration must use `migration.rs` versioned mechanism, not `init_schema`
9. Pre-migration tombstone backfill is lossy — documented as explicit known gap
