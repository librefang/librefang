# ADR-017: openfang-knowledge — Dynamic Domain Knowledge Layer

**Status**: Accepted
**Phase**: 1
**Date**: 2026-03-21
**Authors**: Daniel Alberttis

## Version History

| Version | Date | Author | Changes |
|---------|------|--------|---------|
| 0.1 | 2026-03-21 | Daniel Alberttis | Replaced earlier repo-centric draft with the adopted architecture: `openfang-knowledge` as a second memory galaxy, dynamic runtime-configured domains, adapter-based indexing, privacy-tier-aware federated search, tribal intelligence, kernel wiring, dashboard scope, and four-phase implementation plan. |

---

## Context

OpenFang already has a memory system, but that system is optimized for agent runtime state, not for indexed external knowledge. The project needs a first-party knowledge layer that can ingest codebases, documents, messages, structured exports, and live feeds, then make them searchable with provenance and privacy controls.

The earlier DAXIOM direction hardcoded six knowledge domains as Rust enums:

- `work`
- `projects`
- `business`
- `personal`
- `mind`
- `comms`

That design was rejected. It encoded product policy into the type system and forced code changes every time a new user or deployment needed a new category. In practice, knowledge domains are user-defined organizational containers, not compile-time capabilities.

At the same time, extending `openfang-memory` directly was also the wrong boundary. Runtime memory and indexed knowledge differ on writer, lifecycle, freshness model, and citation contract:

| Axis | `openfang-memory` | Knowledge layer |
|------|-------------------|-----------------|
| Purpose | Agent lived experience | Indexed domain knowledge |
| Writers | Agents at runtime | Indexing pipelines only |
| Backing files | `agents/{id}.rvf`, `shared.rvf` | `knowledge/domains/{id}.rvf`, `knowledge/shared.rvf` |
| Staleness model | Session/runtime evolution, TTL | File hash, sync timestamp, adapter freshness rules |
| Retrieval contract | Memory references | Provenance-backed document references |
| Mutation pattern | Continuous runtime writes | Batch or incremental indexing |

There is also a naming conflict: `KnowledgeStore` already has meaning inside `openfang-memory` internals. Reusing that term for a new indexed knowledge subsystem inside the same crate would blur an important architectural distinction.

The adopted design therefore introduces a second memory galaxy, `openfang-knowledge`, with dynamic domains configured at runtime rather than fixed in Rust enums.

---

## Decision

OpenFang introduces a new crate, `openfang-knowledge`, as the indexed knowledge galaxy. It is separate from `openfang-memory`, and its organizational unit is a dynamic `DomainConfig` loaded from runtime registry data rather than a hardcoded domain enum.

### 1. Two-Galaxy Memory Model

OpenFang will operate two distinct but queryable memory galaxies:

| Galaxy | Crate | Backing files | Purpose | Writers |
|--------|-------|---------------|---------|---------|
| Galaxy 1 — Agent Memory | `openfang-memory` | `agents/{id}.rvf`, `shared.rvf` | Agent lived experience, conversation state, runtime recall | Agents and runtime systems |
| Galaxy 2 — Knowledge | `openfang-knowledge` | `knowledge/domains/{id}.rvf`, `knowledge/shared.rvf` | Indexed domain knowledge and cross-domain tribal intelligence | Indexing pipelines only |

The boundary is hard:

- Agents write runtime memory into Galaxy 1.
- Agents do not write domain indexes in Galaxy 2.
- Galaxy 2 is produced by indexing and sync workflows.
- Search may query both galaxies, but their storage and semantics remain separate.

### 2. Dynamic Domains, Not Hardcoded Domain Enums

The critical architectural decision is that domains are runtime data, not compile-time types.

The system will not define a Rust enum such as:

```rust
enum Domain {
    Work,
    Projects,
    Business,
    Personal,
    Mind,
    Comms,
}
```

Instead:

- `AdapterKind` is the finite Rust enum that expresses capability type.
- `DomainConfig` is runtime data loaded from `registry.json`.
- Users create, edit, and delete domains from the dashboard or API.
- New domains require zero code changes.

This preserves a stable type system where it belongs, at the capability layer, while keeping user organization flexible and deployment-specific.

### 3. `DomainConfig` Is Runtime Data

Each domain is represented by runtime configuration, not by a Rust enum variant:

```rust
pub struct DomainConfig {
    pub id: Uuid,
    pub name: String,
    pub label: String,
    pub adapter: AdapterKind,
    pub privacy_tier: PrivacyTier,
    pub sources: Vec<DomainSource>,
    pub created_at: DateTime<Utc>,
    pub last_indexed: Option<DateTime<Utc>>,
}
```

`DomainConfig` is stored in `~/.openfang/knowledge/registry.json` and is created by user action. A domain is therefore an indexed knowledge namespace with a user-visible label, an adapter type, a privacy tier, and a set of sources.

### 4. `AdapterKind` Is the Finite Enum

`AdapterKind` is the Rust enum that remains intentionally finite because it captures indexing capability classes:

- `Code`
- `Documents`
- `Messages`
- `Structured`
- `Feed`

These variants define ingestion and freshness behavior:

| AdapterKind | Sources | Retrieval/provenance emphasis |
|-------------|---------|-------------------------------|
| `Code` | Git repositories, source trees | AST-aware chunks, `file:line` pointers, codebase-memory-mcp backend |
| `Documents` | PDF, Markdown, Word docs, notes | Page and offset provenance |
| `Messages` | Email threads, WhatsApp, chat channels | Thread and timestamp provenance |
| `Structured` | CSV, JSON, DB exports | Row, record, or field provenance |
| `Feed` | Health wearables, finance feeds, calendar streams | Stream timestamp and freshness windows |

This split is intentional: adapters are product capabilities; domains are user-defined containers.

### 5. Privacy Tiers Are Enforced by the Knowledge Layer

Knowledge domains carry an explicit `PrivacyTier`:

- `Open`
- `Internal`
- `Private`
- `Sensitive`
- `Encrypted`

Access control is enforced at query time. Agents querying `Sensitive` or `Encrypted` domains must hold explicit capability grants.

The no-grant behavior is:

- return no results from that domain
- do not raise an access error
- do not leak that protected results existed

This avoids turning the search layer into a side channel for protected knowledge discovery.

### 6. `DocRef` Generalizes `FileRef`

The knowledge layer returns a generalized document reference rather than a code-only file reference:

```rust
pub struct DocRef {
    pub domain_id: Uuid,
    pub adapter: AdapterKind,
    pub source: String,
    pub excerpt: String,
    pub provenance: Provenance,
    pub privacy_tier: PrivacyTier,
}
```

`Provenance` is adapter-specific:

- `Code` uses file path and line information
- `Documents` uses page and offset metadata
- `Messages` uses thread identifier and timestamp
- `Structured` uses row/record provenance
- `Feed` uses event timestamp or window metadata

This keeps the query contract consistent across adapters while preserving precise source grounding.

### 7. `DomainRegistry` Owns Domain Lifecycle

`DomainRegistry` is the CRUD authority for `DomainConfig` entries stored at `~/.openfang/knowledge/registry.json`.

Its responsibilities are:

1. Create, update, list, and delete domains
2. Persist runtime domain metadata
3. Validate adapter and source configuration
4. Create `knowledge/domains/{id}.rvf` on registration
5. Create a SQLite sidecar for domain metadata and provenance bookkeeping

Dashboard actions and API routes delegate to `DomainRegistry`. They do not manipulate the filesystem directly.

### 8. `FreshnessResolver` Is Adapter-Specific

Knowledge freshness is resolved per adapter type rather than through a single global rule:

| AdapterKind | Freshness rule |
|-------------|----------------|
| `Code` | File hash plus commit identity |
| `Documents` | File modified time plus content hash |
| `Messages` | Last sync timestamp against source system |
| `Structured` | Export timestamp plus content hash or row-count signature |
| `Feed` | Data staleness window relative to event time |

`FreshnessResolver` is responsible for deciding whether an indexed chunk remains valid for retrieval or whether the source must be re-synced or re-indexed first.

### 9. `FederatedKnowledge` Fans Out Across Registered Domains

`FederatedKnowledge` is the query surface over Galaxy 2. It fans out a search across:

- every registered `knowledge/domains/{id}.rvf`
- `knowledge/shared.rvf`

It merges ranked results by score and returns `DocRef` values. Query fan-out is privacy-tier-aware:

- inaccessible domains are skipped
- skipped domains do not produce leakage signals
- callers may optionally constrain search to `domain_ids`

The system does not require a global monolithic knowledge index. Per-domain indexes remain isolated, while `FederatedKnowledge` provides the unified query experience.

### 10. `knowledge/shared.rvf` Stores Tribal Intelligence

`knowledge/shared.rvf` is reserved for cross-domain tribal intelligence:

- how to do things
- reusable tool idioms
- workflow patterns
- cross-domain operational practices

It is readable by all agents. It is written only by the system when cross-domain patterns are detected. It does not store domain-specific raw content or protected source excerpts. Its role is derived meta-knowledge, not another dumping ground for indexed documents.

### 11. Kernel Wiring

`openfang-kernel` will hold both galaxies explicitly:

```rust
Arc<MemoryEngine>
Arc<KnowledgeIndex>
```

The kernel surface will add:

```rust
knowledge_search(query, domain_ids: Option<Vec<Uuid>>) -> Vec<DocRef>
unified_search(query) -> UnifiedResults
```

With:

```rust
pub struct UnifiedResults {
    pub memory: Vec<MemoryRef>,
    pub knowledge: Vec<DocRef>,
}
```

`unified_search` fans out to both galaxies but does not collapse their result types. Runtime memory and indexed knowledge remain distinct classes even when returned together.

### 12. Dashboard Knowledge Tab

The dashboard will expose knowledge management directly. The Knowledge tab will support:

- create domain
- edit and delete domain
- choose adapter and privacy tier
- add and remove sources per domain
- trigger domain sync and re-index
- inspect per-domain stats: chunks, last indexed, source count
- search across registered domains

The user workflow is:

1. name the domain
2. select the adapter kind
3. choose the privacy tier
4. attach sources
5. sync the domain

This keeps domain creation in product space rather than in Rust source code.

### 13. Why This Does Not Belong in `openfang-memory`

The knowledge layer remains a separate crate because the differences are architectural, not cosmetic:

| Concern | `openfang-memory` | `openfang-knowledge` |
|---------|-------------------|----------------------|
| Lifecycle | Runtime recall and agent session state | Index-time ingestion and synchronization |
| Writers | Agents and runtime components | Indexing pipelines only |
| Freshness | TTL, runtime mutation, session semantics | Source hash, sync timestamps, staleness windows |
| Citation model | Memory references | Provenance-backed document references plus live source follow-up |
| Storage identity | Agent-centric | Domain-centric |

Keeping the crates separate preserves clearer ownership, clearer semantics, and avoids overloading `openfang-memory` with a second incompatible operating model.

---

## Consequences

### Positive

- New knowledge domains can be added without modifying Rust code.
- The architecture cleanly separates agent experience from indexed world knowledge.
- Adapter-specific provenance gives more reliable citations across heterogeneous source types.
- Privacy-tier-aware query fan-out lets the system hide protected domains without leaking their existence.
- The dashboard becomes the operational control plane for knowledge ingestion.
- Per-domain indexes preserve isolation while still allowing federated search.
- `knowledge/shared.rvf` creates a place for system-derived tribal intelligence without polluting agent runtime memory.

### Negative

- The system now has two memory galaxies to operate, reason about, and test.
- Dynamic domain configuration adds registry migration and validation complexity.
- Adapter-specific freshness logic increases implementation surface area.
- Privacy enforcement must be correct across all query paths or the search layer becomes a leakage risk.
- Federated ranking across multiple domain indexes is harder than single-index retrieval.
- SQLite sidecars plus RVF indexes create more local state to manage and repair.

### Neutral

- `AdapterKind` remains a finite Rust enum; the design is dynamic only at the domain layer.
- `openfang-memory` is not replaced or deprecated; it remains the runtime memory system of record.
- Code search is only one adapter specialization inside the broader knowledge layer.
- Some deployments may start with a single domain and still use the same architecture.

---

## Implementation

See [PLAN-004](../plans/PLAN-004-knowledge-layer-phase1.md) for the current execution plan.

