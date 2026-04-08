# LibreFang Multi-Tenant Architecture — Master Plan

> **Date:** 2026-04-06  
> **Status:** DRAFT  
> **Author:** Daniel Alberttis  
> **Scope:** Add organization/account-level isolation to LibreFang, enabling SaaS deployment via Qwntik  
> **Origin:** Porting proven patterns from openfang-ai (ADR-026/027) onto fresh LibreFang upstream  

---

## Executive Summary

LibreFang ships with **user-level RBAC** (Viewer/User/Admin/Owner) and **peer_id-scoped memory recall**, but has no concept of organizations or accounts. All agents, channels, skills, and integrations live in a single global namespace.

Qwntik needs **account-level isolation** — each customer organization gets its own agents, credentials, memory, and configuration, running on a shared LibreFang daemon.

This plan defines **6 ADRs**, **6 SPECs**, **3 implementation PLANs** (5 phases across 3 plans), **1 migration guide**, **1 testing strategy**, and **1 quality audit** — 19 documents total.

The memory backend (Supabase + RuVector PostgreSQL extension via HTTP) was previously treated as out-of-scope, but the 7 ruvector Rust crates are already ported and proven in the openfang-ai workspace (ADR-033, SPEC-033, PLAN-033). Since LibreFang already has the `VectorStore` trait + `HttpVectorStore` implementation, the crate port is mechanical and the runtime wiring is plug-and-play.

---

## Document Set

| # | Type | Name | Purpose | Depends On |
|---|------|------|---------|------------|
| 1 | ADR | ADR-MT-001: Account Model | Defines the Account concept, isolation boundaries, backward compat | — |
| 2 | ADR | ADR-MT-002: API Authentication & Account Resolution | How requests carry account context, HMAC signing, middleware | ADR-MT-001 |
| 3 | ADR | ADR-MT-003: Resource Isolation Strategy | How agents, channels, skills, integrations are scoped | ADR-MT-001 |
| 4 | ADR | ADR-MT-004: Data & Memory Isolation | Schema changes, memory namespacing, session scoping | ADR-MT-001 |
| 5 | ADR | ADR-MT-005: Event Bus Tenant Isolation | Dispatch-side filtering, subscribe_account(), history_for_account() | ADR-MT-001, ADR-MT-003 |
| 6 | ADR | ADR-RV-001: RuVector PostgreSQL Extension Port | Port 7 ruvector crates into librefang workspace, Docker image, feature flags | — |
| 7 | SPEC | SPEC-MT-001: Account Data Model & Storage | Exact Rust types, extractors, guards, macros, v18 migration, 33 tests | ADR-MT-001 |
| 8 | SPEC | SPEC-MT-002: API Route Changes | 4-tier scoping for all 317 handlers with code examples | ADR-MT-002, ADR-MT-003 |
| 9 | SPEC | SPEC-MT-003: Database Migration | v19 migration for 14 remaining tables, FTS5 rebuild, rollback strategy | ADR-MT-004 |
| 10 | SPEC | SPEC-MT-004: Supabase RLS Policies | Copy-pasteable SQL for RLS policies on qwntik Supabase tables | SPEC-MT-001, ADR-RV-001 |
| 11 | SPEC | SPEC-RV-001: Extension Port Acceptance Criteria | 26 acceptance criteria across 6 groups (adapted from openfang SPEC-033) | ADR-RV-001 |
| 12 | SPEC | SPEC-RV-002: Supabase Vector Store Integration | PostgresVectorStore impl, RPC functions, kernel integration | ADR-RV-001, SPEC-MT-004 |
| 19 | SPEC | SPEC-RV-003: Supabase Memory Wiring | SupabaseVectorStore (HTTP/PostgREST), config, substrate wiring, 12 ACs | Phase 0 verified, ADR-RV-001 |
| 13 | PLAN | PLAN-MT-001: Phase 1 Implementation | Foundation: types, middleware, extractors, guards, v18 migration, 76 handlers | All MT ADRs, SPEC-MT-001 |
| 14 | PLAN | PLAN-MT-002: Phases 2–4 Implementation | Resource isolation, data isolation, hardening — 241 remaining handlers | PLAN-MT-001 |
| 15 | PLAN | PLAN-RV-001: Phase 0 RuVector Port | 7-crate port, Docker image, Supabase integration, acceptance tests | ADR-RV-001, SPEC-RV-001 |
| 16 | GUIDE | MIGRATION-GUIDE: Single → Multi-Tenant | Operator guide for upgrading existing installations | PLAN-MT-001 |
| 17 | TEST | TESTING-STRATEGY: Multi-Tenant Test Plan | 128 planned tests across 7 categories, CI gates | All SPECs |
| 18 | AUDIT | BHR-AUDIT-2026-04-07: Quality Audit | Post-hoc verification of all docs against source code | All docs |

---

## ADR-MT-001: Account Model

### Status
IMPLEMENTED (AccountId type, extractor, registry scoping, agent ownership — all shipped in Phase 1)

### Context
LibreFang operates as a single-tenant daemon. All resources (agents, skills, channels, integrations, memory, workflows) exist in one flat namespace under `~/.librefang/`. The existing `UserRole` RBAC controls what a user can do, but not what they can see — every authenticated user sees every agent.

Qwntik wraps LibreFang as a multi-tenant SaaS. Each Qwntik customer ("account") must have isolated resources. The openfang fork proved this with `account_id` scoping across registry, routes, kernel, and storage.

### Decision

**Introduce `AccountId` as a first-class isolation boundary.**

1. **Account** is an opaque string identifier (e.g., UUID or slug) provided by the upstream SaaS layer (Qwntik/Supabase). LibreFang does not manage account lifecycle — it receives `account_id` on requests and partitions resources accordingly.

2. **Isolation model: shared daemon, isolated namespaces.** One LibreFang process serves all accounts. Resources are tagged with `account_id` and filtered at query time. This is the same model as openfang's proven implementation.

3. **Default account: `AccountId(None)` / `"system"`.** When no account is specified (desktop mode, legacy API calls, CLI), the extractor returns `AccountId(None)`. Storage uses `DEFAULT 'system'` for backward compatibility. `AccountId(None)` sees all data (admin/legacy mode).

4. **Account is NOT a user.** Users belong to accounts. A user's `UserIdentity` gains an `account_id` field. The existing `UserRole` applies within the account context.

5. **MakerKit personal account alignment.** In Qwntik (MakerKit), `account_id = user_id` is auto-created on signup. LibreFang's `AccountId(None)` / `"system"` concept has no equivalent in MakerKit. Resolution: Qwntik ALWAYS sends `X-Account-Id: <user_uuid>` — the `system` string only appears in LibreFang's SQLite for legacy/desktop/CLI mode and is never sent from Qwntik.

### Consequences
- Every resource type gains an `account_id` field (agents, sessions, memories, skills, integrations, channels, workflows, goals)
- All list/get/update/delete operations filter by account
- Storage layout changes from flat to account-partitioned
- API middleware must extract account context from every request
- Existing single-tenant installations migrate seamlessly (everything → `system` account)

### Affected Crates
| Crate | Change |
|-------|--------|
| `librefang-types` | Add `AccountId` type alias, add `account_id` to config types |
| `librefang-kernel` | Account-scoped agent registry, `bind_agent_account()` in event bus |
| `librefang-api` | Middleware extraction, all route handlers |
| `librefang-memory` | Schema migration, query filters |
| `librefang-runtime` | Context engine account propagation |
| `librefang-channels` | Account-scoped channel loading |
| `librefang-skills` | Account-scoped skill access |
| `librefang-extensions` | Account-scoped integration registry |

### Key Files
```
crates/librefang-types/src/config/types.rs    → Add AccountId, account_id fields
crates/librefang-kernel/src/auth.rs           → UserIdentity gains account_id
crates/librefang-kernel/src/kernel.rs         → Thread AccountId through spawn_agent(), list_agents()
crates/librefang-kernel/src/registry.rs       → Account-filtered agent queries
```

---

## ADR-MT-002: API Authentication & Account Resolution

### Status
PARTIALLY IMPLEMENTED (Phase 1 shipped, Phase 2 in progress)

### Context
LibreFang's current auth model:
- **API key**: Single global bearer token (`api_key` in config)
- **Dashboard**: Single password → session token (no user identity beyond "logged in")
- **Channels**: Platform identity (Telegram user ID, Discord user ID) mapped to `peer_id`

None of these carry account context.

### Decision

**Account resolution via `X-Account-Id` header + HMAC signature validation.**

1. **SaaS-mediated auth (primary path):** Qwntik's server actions compute an HMAC signature over the request using a shared secret (`ACCOUNT_HMAC_SECRET`). LibreFang validates:
   ```
   X-Account-Id: acc_abc123
   X-Account-Sig: hmac-sha256(secret, account_id)
   ```
   This is the proven pattern from qwntik's `openfang-account-options.ts`. Note: the actual qwntik implementation uses only `X-Account-Id` and `X-Account-Sig` (no timestamp header).

2. **Direct API key auth (backward compat):** When only a bearer token is provided (no `X-Account-Id` header), the request is assigned to the `system` account. Existing integrations work unchanged.

3. **Channel-originated requests:** Channel bridges (Telegram, Discord) resolve account from the channel binding configuration. Each channel binding already has an `account_id: Option<String>` field in `AgentBinding` — this becomes required for multi-tenant deployments.

4. **Dashboard auth:** Dashboard login returns account context. Multi-account users see an account picker. Session tokens carry `account_id`.

### Middleware Design
```rust
// AccountId extractor (Axum FromRequestParts — infallible)
// See SPEC-MT-001 for the exact implementation.
impl<S: Send + Sync> FromRequestParts<S> for AccountId {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(header) = parts.headers.get("x-account-id") {
            if let Ok(s) = header.to_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Ok(AccountId(Some(trimmed.to_owned())));
                }
            }
        }
        Ok(AccountId(None)) // Legacy/desktop — sees all data
    }
}
```

> **Note:** The openfang implementation uses a `FromRequestParts` extractor (returning
> `AccountId(Option<String>)`) rather than a standalone middleware function. The actual
> qwntik implementation sends only `X-Account-Id` and `X-Account-Sig` headers — there
> is no `X-Account-Timestamp` header in the production implementation.

### Security Properties
- **404 not 403**: Wrong account returns 404 (prevents enumeration) — proven in openfang ADR-027
- **Timing-safe comparison**: HMAC validation uses `verify_slice()` (constant-time compare)
- **Replay-protected HMAC (shipped 2026-04-07)**: Signature binds `account_id + method + path + timestamp` with ±5-min window. Cross-endpoint replay eliminated. Remaining: nonce cache pending, `ValidLegacy` fallback sunset by Phase 2 end.
- **`require_account_id` middleware (shipped)**: Rejects missing `X-Account-Id` when multi-tenant mode is enabled.
- **No credential leakage**: Account credentials never appear in API responses
- **⚠️ ~190 handlers accept `_account: AccountId` but do not filter**: 139 of 330 handlers (42%) are tenant-isolated — 108 data-filtered + 28 admin-guarded + 3 Tier 4 Public. Cross-tenant data access remains possible on the ~190 unfiltered endpoints (system.rs, skills.rs, workflows.rs, channels.rs, providers.rs, plugins.rs, goals.rs, media.rs, inbox.rs). Config, budget, and network are now fully guarded. See ADR-MT-003 and PENDING-WORK.md for per-file breakdown.
- **⚠️ Upload cross-tenant read**: `UploadMeta` lacks `account_id` field; `serve_upload` does not check ownership. See PENDING-WORK.md item #3.

### Affected Files
```
crates/librefang-api/src/middleware.rs         → HMAC sig verification (verify_account_sig)
crates/librefang-api/src/extractors.rs         → AccountId FromRequestParts (infallible)
crates/librefang-api/src/server.rs             → Wire HMAC middleware into router
crates/librefang-types/src/config/types.rs     → hmac_secret config field
```

---

## ADR-MT-003: Resource Isolation Strategy

### Status
IN PROGRESS — 6 of 15 route files fully tenant-isolated (agents, memory, prompts data-filtered; config, budget, network admin-guarded via `require_admin`); 3 partially filtered; 6 extractor-only with zero filtering. 139 of 330 handlers (42%) genuinely isolated. See PENDING-WORK.md for per-file breakdown.

### Context
LibreFang manages 8 resource types that need account isolation:
1. **Agents** — spawned, configured, listed
2. **Channels** — Telegram, Discord, Slack, etc.
3. **Skills** — installed, invoked by agents
4. **Integrations** — MCP servers, OAuth connections
5. **Workflows** — automation sequences
6. **Goals** — agent objectives
7. **Hands** — pre-built autonomous agent bundles
8. **Prompts** — system prompt templates

### Decision

**Tag-and-filter isolation with account-partitioned filesystem storage.**

Every resource gains an `account_id: String` field. All CRUD operations include account in their query. Resources belonging to other accounts are invisible (not forbidden — invisible).

#### Per-Resource Strategy

| Resource | Current Key | New Key | Storage Change |
|----------|------------|---------|----------------|
| **Agents** | `AgentId` (UUID) | `(AccountId, AgentId)` | Manifests: `accounts/{id}/agents/` |
| **Channels** | channel name | `(AccountId, channel)` | Config: `accounts/{id}/channels/` |
| **Skills** | skill name | Global registry + per-account allowlist | Skills dir stays global; accounts get `skill_allowlist` |
| **Integrations** | integration ID | `(AccountId, integration)` | Per-account: `accounts/{id}/integrations.toml` |
| **Workflows** | workflow ID | `(AccountId, workflow)` | Per-account: `accounts/{id}/workflows/` |
| **Goals** | goal ID | `(AccountId, goal)` | In-memory, tagged |
| **Hands** | hand name | Global catalog + per-account instances | Definitions global; instances per-account |
| **Prompts** | prompt name | `(AccountId, prompt)` | Per-account: `accounts/{id}/prompts/` |

#### Filesystem Layout
```
~/.librefang/
├── config.toml                          # Global defaults
├── accounts/
│   ├── system/                          # Default account (backward compat)
│   │   ├── config.toml                  # Account-specific overrides
│   │   ├── agents/
│   │   │   ├── assistant.toml
│   │   │   └── researcher.toml
│   │   ├── channels/
│   │   ├── integrations.toml
│   │   ├── workflows/
│   │   ├── prompts/
│   │   └── workspaces/
│   │       └── {agent_id}/              # Sandboxed agent workspace
│   ├── acc_abc123/                      # Tenant A
│   │   ├── config.toml
│   │   ├── agents/
│   │   └── ...
│   └── acc_def456/                      # Tenant B
│       └── ...
├── skills/                              # Global skill marketplace
├── hands/                               # Global hand definitions
└── data/
    └── librefang.db                     # SQLite (all accounts, filtered by account_id)
```

#### Skills: Global Marketplace + Per-Account Allowlist
Skills are expensive to duplicate and rarely contain tenant-specific logic. Strategy:
- Skills installed globally in `~/.librefang/skills/`
- Each account config has `skill_allowlist: ["web_search", "code_exec", ...]`
- Empty allowlist = all skills available (backward compat)
- Agent manifests reference skills by name; kernel checks allowlist at invocation time

#### Agent Registry Changes
```rust
// Current
pub struct AgentEntry {
    pub id: AgentId,
    pub manifest: AgentManifest,
    pub status: AgentStatus,
}

// New
pub struct AgentEntry {
    pub id: AgentId,
    pub account_id: AccountId,       // ← NEW
    pub manifest: AgentManifest,
    pub status: AgentStatus,
}

// Registry queries
impl AgentRegistry {
    pub fn list_for_account(&self, account_id: &str) -> Vec<&AgentEntry>;
    pub fn get_for_account(&self, account_id: &str, agent_id: &AgentId) -> Option<&AgentEntry>;
    pub fn find_by_name_for_account(&self, account_id: &str, name: &str) -> Option<&AgentEntry>;
}
```

#### Event Bus Isolation
Tag events with `account_id`. Subscribers receive only events for their account:
```rust
pub struct Event {
    pub id: EventId,
    pub account_id: AccountId,       // ← NEW
    pub source: EventSource,
    pub target: EventTarget,
    pub payload: EventPayload,
}
```

The event bus filters on dispatch rather than maintaining per-account channels (simpler, less memory).

### Affected Files
```
crates/librefang-kernel/src/registry.rs       → AgentEntry + account-filtered queries
crates/librefang-kernel/src/kernel.rs         → spawn_agent takes account_id
crates/librefang-kernel/src/event_bus.rs      → Event gains account_id, filtered dispatch
crates/librefang-channels/src/router.rs       → Route resolution within account
crates/librefang-channels/src/bridge.rs       → SenderContext gains account_id
crates/librefang-skills/src/registry.rs       → Allowlist check at invocation
crates/librefang-extensions/src/registry.rs   → Per-account integration state
crates/librefang-api/src/routes/agents.rs     → Filter by account from middleware
crates/librefang-api/src/routes/channels.rs   → Filter by account
crates/librefang-api/src/routes/skills.rs     → Filter by account allowlist
crates/librefang-api/src/routes/workflows.rs  → Filter by account
crates/librefang-api/src/routes/goals.rs      → Filter by account
crates/librefang-api/src/routes/plugins.rs    → Filter by account
crates/librefang-api/src/routes/prompts.rs    → Filter by account
```

---

## ADR-MT-004: Data & Memory Isolation

### Status
PROPOSED

### Context
LibreFang stores runtime data in SQLite (`~/.librefang/data/librefang.db`). Tables include:
- `memories` — semantic memory fragments (agent_id, content, embedding, peer_id)
- `sessions` — conversation sessions (agent_id, messages)
- `kv_store` — structured key-value data (agent_id, key, value)

The recent #2062 fix added `peer_id` filtering to memory recall to prevent cross-user context leaks. This is the exact pattern we extend to account-level isolation.

### Decision

**Add `account_id TEXT NOT NULL DEFAULT 'system'` to all data tables. All queries include account_id in WHERE clause.**

#### Schema Changes
```sql
-- memories table
ALTER TABLE memories ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
CREATE INDEX idx_memories_account ON memories(account_id);
CREATE INDEX idx_memories_account_agent ON memories(account_id, agent_id);

-- sessions table  
ALTER TABLE sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
CREATE INDEX idx_sessions_account ON sessions(account_id);
CREATE INDEX idx_sessions_account_agent ON sessions(account_id, agent_id);

-- kv_store table
ALTER TABLE kv_store ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
CREATE INDEX idx_kv_account ON kv_store(account_id);
```

#### Memory Filter Extension
```rust
// Current (from #2062)
pub struct MemoryFilter {
    pub agent_id: Option<AgentId>,
    pub peer_id: Option<String>,
    pub scope: Option<String>,
}

// New
pub struct MemoryFilter {
    pub account_id: Option<String>,  // ← NEW (required in multi-tenant mode)
    pub agent_id: Option<AgentId>,
    pub peer_id: Option<String>,
    pub scope: Option<String>,
}
```

#### Query Pattern
Every memory operation gains an account_id parameter:
```rust
// Recall
SELECT * FROM memories
WHERE account_id = ?1
  AND agent_id = ?2
  AND (peer_id = ?3 OR peer_id IS NULL)
ORDER BY relevance DESC
LIMIT ?4;

// Insert
INSERT INTO memories (account_id, agent_id, peer_id, content, embedding, ...)
VALUES (?1, ?2, ?3, ?4, ?5, ...);
```

#### Context Engine Propagation
```rust
// context_engine.rs ingest signature change
pub async fn ingest(
    &self,
    account_id: &str,           // ← NEW
    agent_id: AgentId,
    user_message: &str,
    peer_id: Option<&str>,
) -> LibreFangResult<IngestResult>
```

#### Proactive Memory
The proactive memory system (mem0-style `POST /api/memory`) must extract `account_id` from the request context and include it in all store/recall operations.

#### HTTP Vector Store (Supabase + RuVector)
The existing `http_vector_store.rs` sends queries to an external HTTP endpoint. For multi-tenant:
- Include `account_id` in the request payload
- The Supabase side enforces RLS policies keyed on `account_id`
- This is already implemented in qwntik's Supabase schema — plug and play

### Migration Strategy
1. Add columns with `DEFAULT 'system'` — all existing data migrates automatically
2. Add indexes — no data loss
3. Update queries — backward compatible (system account = legacy behavior)
4. No downtime required

### Affected Files
```
crates/librefang-memory/src/semantic.rs       → All recall/insert queries add account_id
crates/librefang-memory/src/session.rs        → Session CRUD adds account_id
crates/librefang-memory/src/structured.rs     → KV store adds account_id
crates/librefang-memory/src/migration.rs      → Schema migration script
crates/librefang-memory/src/http_vector_store.rs → Include account_id in HTTP payload
crates/librefang-runtime/src/context_engine.rs   → Propagate account_id to memory calls
crates/librefang-api/src/routes/memory.rs        → Extract account from middleware
```

---

## ADR-RV-001: RuVector PostgreSQL Extension Port

### Status
ACCEPTED — Phase 0 tasks 0.1–0.6 verified 2026-04-07 (adapted from openfang-ai ADR-033)

### Context
The openfang-ai fork already ported 7 Rust crates from the ruvector upstream into its workspace. These crates compile into a PostgreSQL extension (`ruvector.so`) that provides 225 `#[pg_extern]` functions in source — of which 57 are behind optional feature flags (graph, learning, attention, gnn, routing, solver, gated_transformer), yielding 168 in the default build. The SQL extension file lists 197 `CREATE FUNCTION` statements; 161 are verified live in `pg_proc` (the delta accounts for overloaded signatures and upgrade-path stubs).

LibreFang's memory system already has:
- `VectorStore` trait in `librefang-types/src/memory.rs` — abstract interface with `insert()`, `search()`, `delete()`, `get_embeddings()`
- `HttpVectorStore` in `librefang-memory/src/http_vector_store.rs` — HTTP client that talks to external vector databases
- `SemanticStore.set_vector_store()` — hot-swappable backend

The extension runs **inside Supabase's PostgreSQL** (self-hosted Docker). LibreFang talks to it via HTTP through the existing `HttpVectorStore`. No direct Postgres driver needed in the LibreFang binary.

### Decision

**Port the 7 ruvector crates into the librefang workspace as optional workspace members. Build the PG extension via Docker. Connect at runtime via existing HttpVectorStore.**

#### Crates to Port

| Crate | Version | Purpose | Dependencies |
|-------|---------|---------|-------------|
| `ruvector-postgres` | 0.3.0 | PG extension hub — 225 `#[pg_extern]` in source (57 feature-gated), 197 in SQL, 161 live via pgrx 0.12.6 | All below (optional) |
| `ruvector-solver` | 2.0.4 | Sublinear sparse linear system solvers | None |
| `ruvector-math` | 2.0.4 | Optimal transport, information geometry, manifolds | None |
| `ruvector-attention` | 2.0.4 | 39 attention mechanisms | ruvector-math (optional) |
| `ruvector-sona` | 0.1.9 | SONA engine — LoRA, EWC++, ReasoningBank | None |
| `ruvector-domain-expansion` | 2.0.4 | Cross-domain transfer learning | None |
| `ruvector-mincut-gated-transformer` | 0.1.0 | Ultra-low latency transformer | None |

#### Integration Architecture
```
┌─────────────────────────┐     HTTP      ┌──────────────────────────────┐
│   LibreFang Daemon      │ ◄──────────► │  Supabase PostgreSQL 17      │
│                         │              │  + ruvector extension v0.3   │
│  HttpVectorStore ───────┤  /insert     │  + RLS policies              │
│  (existing, unchanged)  │  /search     │  + HNSW indexes              │
│                         │  /delete     │  + 161 live SQL functions     │
│                         │  /embeddings │  + Local embeddings (384-dim)│
└─────────────────────────┘              └──────────────────────────────┘
```

#### Feature Flags (workspace Cargo.toml)
```toml
# Workspace members — ruvector crates are optional, only needed for PG extension builds
[workspace]
members = [
    "crates/librefang-*",
    # Uncomment to build PG extension locally:
    # "crates/ruvector-*",
]

# Or use a feature flag:
[workspace.features]
ruvector-build = []  # Enables ruvector workspace members for extension compilation
```

The ruvector crates are **not compiled into the LibreFang binary**. They exist in the workspace solely for building the PostgreSQL extension (`cargo pgrx package`). The extension ships as a Docker image.

#### Docker Image
```dockerfile
# Dockerfile.supabase-ruvector (adapted from openfang)
FROM supabase/postgres:17.6.1.095
# ... Rust toolchain, pgrx install, compile extension
# Result: ~1.86 GB image with ruvector.so + ONNX models
```

#### Known Limitations (from openfang PLAN-033)
1. `ruvector_embed()` returns `real[]` not `ruvector` — requires manual cast
2. `ruvector_embed_batch()` SQL stub missing
3. HNSW bitmap scan warning (cosmetic)
4. Image size 1.86 GB (ONNX Runtime is large)

### Consequences
- LibreFang workspace gains 7 crates (~15K LOC) but they don't affect the main binary
- Docker compose gains a `supabase-ruvector` service
- The `HttpVectorStore` config gets a Supabase-specific example in docs
- Local embeddings (384-dim, all-MiniLM-L6-v2) eliminate external API dependency
- Multi-tenant memory isolation enforced at Supabase RLS level (account_id in policies)

### Affected Files
```
Cargo.toml                                    → Add ruvector workspace members (commented)
crates/ruvector-postgres/                     → Port from openfang-ai (7 crates)
crates/ruvector-solver/
crates/ruvector-math/
crates/ruvector-attention/
crates/ruvector-sona/
crates/ruvector-domain-expansion/
crates/ruvector-mincut-gated-transformer/
docker/Dockerfile.supabase-ruvector           → New: PG extension Docker build
docker/docker-compose.yml                     → Add supabase-ruvector service
docs/src/app/configuration/page.mdx           → Document vector_store HTTP config
```

---

## SPEC-RV-001: Extension Port Acceptance Criteria

Adapted from openfang-ai SPEC-033. All 26 criteria were verified on 2026-04-06 in the openfang workspace. Re-verification required after port to librefang.

### Group 1: Crate Port (5 criteria)
| # | Criterion | Verification |
|---|-----------|-------------|
| 1.1 | All 7 crates compile with `cargo check` | `cargo check -p ruvector-postgres --features all-features-v3` |
| 1.2 | No circular dependencies | `cargo tree -p ruvector-postgres` shows clean DAG |
| 1.3 | Path dependencies resolve within workspace | All `path = "../ruvector-*"` entries valid |
| 1.4 | Feature flags gate optional modules | `--no-default-features` compiles base only |
| 1.5 | Crate versions match upstream (solver 2.0.4, sona 0.1.9, etc.) | `Cargo.toml` version fields |

### Group 2: Naming Standardization (4 criteria)
| # | Criterion | Verification |
|---|-----------|-------------|
| 2.1 | No references to "openfang" in ruvector crates | `grep -r openfang crates/ruvector-*` returns empty |
| 2.2 | SQL function prefix is `ruvector_` | `\df ruvector_*` in psql shows correct prefix |
| 2.3 | Extension name is `ruvector` | `SELECT extname FROM pg_extension` |
| 2.4 | GUC prefix is `ruvector.` | `SHOW ruvector.ef_search` works |

### Group 3: Docker Image (5 criteria)
| # | Criterion | Verification |
|---|-----------|-------------|
| 3.1 | `docker build -f Dockerfile.supabase-ruvector .` succeeds | Build completes without error |
| 3.2 | Extension loads on startup | `CREATE EXTENSION ruvector` succeeds |
| 3.3 | 161+ SQL functions registered (197 in SQL file) | `SELECT count(*) FROM pg_proc WHERE proname LIKE 'ruvector_%'` |
| 3.4 | SIMD detected correctly | `SELECT ruvector_simd_info()` returns architecture |
| 3.5 | Base image is supabase/postgres:17.x | `FROM` line in Dockerfile |

### Group 4: Local Embeddings (4 criteria)
| # | Criterion | Verification |
|---|-----------|-------------|
| 4.1 | `ruvector_embed('text')` returns float array | Returns `real[]` with 384 dimensions |
| 4.2 | Default model is all-MiniLM-L6-v2 | Model name in logs on first call |
| 4.3 | Embedding dimensions match model spec | `array_length(ruvector_embed('test'), 1) = 384` |
| 4.4 | No external API calls for embeddings | Network monitor shows zero outbound during embed |

### Group 5: Semantic Search (4 criteria)
| # | Criterion | Verification |
|---|-----------|-------------|
| 5.1 | HNSW index creates successfully | `CREATE INDEX ... USING hnsw` succeeds |
| 5.2 | Cosine similarity search returns ranked results | `ORDER BY embedding <=> query` returns correct order |
| 5.3 | Insert + search round-trip works | Insert document, search for it, verify top result |
| 5.4 | RLS policies enforce account isolation | Query as account A, verify account B data invisible |

### Group 6: Integration (4 criteria)
| # | Criterion | Verification |
|---|-----------|-------------|
| 6.1 | HttpVectorStore connects to Supabase endpoint | Config: `vector_store_url = "http://localhost:54321/rest/v1/rpc"` |
| 6.2 | LibreFang semantic search delegates to HTTP backend | Memory recall returns Supabase-stored results |
| 6.3 | Account-scoped queries include account_id | HTTP payload contains `account_id` field |
| 6.4 | Fallback to SQLite when HTTP unavailable | Connection failure gracefully falls back |

---

## SPEC-MT-001: Account Data Model & Storage

### Rust Types

```rust
// crates/librefang-types/src/account.rs (NEW FILE)

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tenant isolation boundary. Every resource belongs to exactly one account.
///
/// Uses `Option<String>` — NOT `Option<Uuid>` — matching openfang-ai's proven pattern.
/// This keeps a single representation across extractor, storage, migration, and comparison.
///
/// - `AccountId(Some("uuid-string"))` = multi-tenant request (SaaS, team isolation)
/// - `AccountId(None)` = legacy/desktop mode (admin, sees everything)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Option<String>);

impl AccountId {
    /// The implicit account string for single-tenant / backward-compatible storage.
    /// Matches the SQLite migration DEFAULT 'system' exactly.
    pub const SYSTEM: &'static str = "system";

    /// Create a new random account ID (UUID v4).
    pub fn new() -> Self {
        Self(Some(Uuid::new_v4().to_string()))
    }

    /// Returns true if this is a scoped (non-None) request.
    pub fn is_scoped(&self) -> bool {
        self.0.is_some()
    }

    /// Returns the inner string, or "system" for legacy/desktop.
    pub fn as_str_or_system(&self) -> &str {
        match &self.0 {
            Some(s) => s.as_str(),
            None => Self::SYSTEM,
        }
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self(None) // Legacy/desktop mode
    }
}

/// Account metadata. Minimal for Phase 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub status: AccountStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountStatus {
    Active,
    Suspended,
    Deleted,
}

/// Per-account configuration (overrides global config).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AccountConfig {
    /// Override default model for this account
    pub default_model: Option<String>,
    /// Provider credentials specific to this account
    pub providers: Option<HashMap<String, ProviderConfig>>,
    /// Skills this account can use (empty = all)
    pub skill_allowlist: Vec<String>,
    /// Rate limits for this account
    pub rate_limit_per_minute: Option<u32>,
    /// Custom system prompt prefix
    pub system_prompt_prefix: Option<String>,
}
```

> **Type alignment note:** The `AccountId(pub Option<String>)` type is the canonical
> representation across ALL documents. See SPEC-MT-001 for the full type definition,
> extractor, guards, and macros. ADR-MT-001 contains the decision rationale.

### Config Changes

```toml
# ~/.librefang/config.toml — ADDITIONS

[multi_tenant]
enabled = false                        # Default: single-tenant (backward compat)
hmac_secret = ""                       # Required when enabled = true
default_account = "system"             # Account for unauthenticated requests
accounts_dir = "accounts"              # Relative to home_dir
```

### Storage Layout
See ADR-MT-003 filesystem layout. Key constraint: `accounts/` directory is created lazily on first request for an account. No pre-provisioning required.

### Kernel Integration

Account resolution is handled at the middleware layer via Axum's `FromRequestParts` extractor
(see SPEC-MT-001 for the exact implementation). The kernel does NOT manage account lifecycle
— it receives `AccountId` from the API layer and threads it through all operations.

For filesystem-based deployments with per-account config overrides:

```rust
// crates/librefang-kernel/src/account_manager.rs (OPTIONAL — filesystem deployments only)
// Qwntik/Supabase deployments use Supabase for account config, not this file.

pub struct AccountManager {
    accounts_dir: PathBuf,
    configs: DashMap<String, AccountConfig>,
    hmac_secret: Vec<u8>,
}

impl AccountManager {
    /// Load account config from disk, or return defaults
    pub fn get_config(&self, account_id: &str) -> AccountConfig;
    
    /// Validate HMAC signature for request.
    /// Note: Phase 1 HMAC has no timestamp/nonce — see ADR-MT-002 Risk section.
    pub fn validate_signature(
        &self,
        account_id: &str,
        signature: &str,
    ) -> Result<(), AuthError>;
    
    /// Ensure account directory structure exists
    pub fn ensure_account_dir(&self, account_id: &str) -> Result<PathBuf>;
    
    /// List all known accounts
    pub fn list_accounts(&self) -> Vec<String>;
}
```

> **Note:** For Qwntik deployments, account config lives in Supabase (`account_features` table).
> AccountManager is only needed for standalone daemon deployments with filesystem-based config.

---

## SPEC-MT-002: API Route Changes

### Middleware Chain

```
Request → bearer_auth → extract_account → rate_limit(account) → route_handler
```

`extract_account` runs after bearer auth. It:
1. Checks for `X-Account-Id` header
2. If present + non-empty: returns `AccountId(Some(value))`, HMAC verified if secret configured
3. If absent/empty: returns `AccountId(None)` — legacy/desktop mode (sees all data)
4. `AccountId` injected via Axum `FromRequestParts` extractor (infallible — never rejects)

### Route-by-Route Changes

| Route | Method | Current | Change |
|-------|--------|---------|--------|
| `/api/agents` | GET | Returns all agents | Filter by `account_id` |
| `/api/agents` | POST | Spawns global agent | Tag with `account_id` |
| `/api/agents/:id` | GET | Lookup by ID | Must belong to account (else 404) |
| `/api/agents/:id` | DELETE | Kill by ID | Must belong to account (else 404) |
| `/api/agents/:id/chat` | POST | Send message | Must belong to account |
| `/api/sessions` | GET | All sessions | Filter by `account_id` |
| `/api/sessions/:id` | GET | Session by ID | Must belong to account |
| `/api/memory` | POST | Store memory | Tag with `account_id` |
| `/api/memory/search` | POST | Search all | Filter by `account_id` |
| `/api/channels` | GET | All channels | Filter by `account_id` |
| `/api/channels/:id/test` | POST | Test channel | Must belong to account |
| `/api/skills` | GET | All installed | Filter by account allowlist |
| `/api/skills/install` | POST | Install globally | Install globally, add to allowlist |
| `/api/integrations` | GET | All integrations | Filter by `account_id` |
| `/api/integrations/add` | POST | Install globally | Install for `account_id` |
| `/api/integrations/:id` | DELETE | Remove globally | Remove for `account_id` |
| `/api/workflows` | GET | All workflows | Filter by `account_id` |
| `/api/workflows` | POST | Create workflow | Tag with `account_id` |
| `/api/goals` | GET | All goals | Filter by `account_id` |
| `/api/budget` | GET | Global budget | Per-account budget |
| `/api/prompts` | GET | All prompts | Filter by `account_id` |
| `/api/plugins/doctor` | GET | System-wide | No change (diagnostic) |
| `/api/config` | GET | Global config | Account config merged with global |
| `/api/health` | GET | Health check | No change |

### New Endpoints

| Route | Method | Purpose |
|-------|--------|---------|
| `/api/accounts/current` | GET | Return current account context + config |
| `/api/accounts/current/config` | PUT | Update account-specific config overrides |

### Response Pattern
All list endpoints gain pagination awareness (future) but no breaking changes to response shape. The `account_id` is NOT included in responses (prevent leakage to untrusted frontends).

---

## SPEC-MT-003: Database Migration

### Migration Script

```sql
-- Migration: 001_add_account_isolation
-- Safe to run on existing databases. All existing data migrates to 'system' account.

PRAGMA foreign_keys = OFF;

-- 1. memories
ALTER TABLE memories ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
CREATE INDEX IF NOT EXISTS idx_memories_account_id ON memories(account_id);
CREATE INDEX IF NOT EXISTS idx_memories_account_agent ON memories(account_id, agent_id);

-- 2. sessions
ALTER TABLE sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
CREATE INDEX IF NOT EXISTS idx_sessions_account_id ON sessions(account_id);
CREATE INDEX IF NOT EXISTS idx_sessions_account_agent ON sessions(account_id, agent_id);

-- 3. kv_store (structured memory)
ALTER TABLE kv_store ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
CREATE INDEX IF NOT EXISTS idx_kv_account_id ON kv_store(account_id);

-- Note: proactive_memories table does not exist in librefang's migration.rs.
-- The proactive memory system uses the `memories` table with scope/level filtering.
-- No separate proactive_memories table migration is needed.

PRAGMA foreign_keys = ON;
```

### Rollback Script

```sql
-- Rollback: 001_add_account_isolation
-- SQLite doesn't support DROP COLUMN before 3.35.0.
-- For older versions, requires table rebuild.
-- For 3.35.0+:

DROP INDEX IF EXISTS idx_memories_account_id;
DROP INDEX IF EXISTS idx_memories_account_agent;
DROP INDEX IF EXISTS idx_sessions_account_id;
DROP INDEX IF EXISTS idx_sessions_account_agent;
DROP INDEX IF EXISTS idx_kv_account_id;
ALTER TABLE memories DROP COLUMN account_id;
ALTER TABLE sessions DROP COLUMN account_id;
ALTER TABLE kv_store DROP COLUMN account_id;
```

### Version Tracking
Add to LibreFang's migration table (or create one):
```sql
CREATE TABLE IF NOT EXISTS migrations (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

INSERT INTO migrations (name) VALUES ('001_add_account_isolation');
```

---

## PLAN-MT-001: Implementation Plan

### Phase 0: RuVector Crate Port (2-3 days) — ✅ VERIFIED 2026-04-07
**Goal:** 7 ruvector crates in workspace, Docker image builds, extension verified in Supabase.

| # | Task | Crate | Files | Tests | Status |
|---|------|-------|-------|-------|--------|
| 0.1 | Copy 7 ruvector crates from openfang-ai | workspace | `crates/ruvector-*/` | `cargo check` all crates | ✅ Done — 397 .rs files, 18 .sql files, 1:1 match with openfang-ai |
| 0.2 | Scrub openfang references (naming standardization) | ruvector-* | All `Cargo.toml`, `lib.rs` | `grep -r openfang crates/ruvector-*` returns empty | ✅ Done — 0 references |
| 0.3 | Add as workspace members (commented by default) | workspace | `Cargo.toml` | Workspace resolves with members uncommented | ✅ Done — all 7 commented in root Cargo.toml, inter-crate deps use correct relative paths |
| 0.4 | Port Dockerfile.supabase-ruvector | docker | `docker/Dockerfile.supabase-ruvector` | `docker build` succeeds | ✅ Done — multi-stage build, PG17, pgrx 0.12.6, supabase/postgres:17.6.1.095 base |
| 0.5 | Add supabase-ruvector to docker-compose | docker | `docker/docker-compose.yml` | `docker compose up` starts PG with extension | ✅ Done — `librefang-ruvector-db` container, port 54322 |
| 0.6 | Verify extension: 161+ functions (197 in file), SIMD, embeddings | — | — | SPEC-RV-001 Groups 3-5 pass | ✅ **Verified live 2026-04-07**: 161 functions, NEON SIMD (aarch64), 384-dim embeddings, HNSW search works |
| 0.7 | Configure HttpVectorStore → Supabase endpoint | librefang-memory | `src/http_vector_store.rs` config example | Semantic search round-trip via HTTP | ❌ Not started |

**Exit criteria:** `SELECT ruvector_embed('hello')` works in Supabase ✅ (verified). LibreFang's `HttpVectorStore` retrieves results from it ❌ (0.7 pending). SPEC-RV-001 Groups 3-5 pass ✅.

### Phase 1: Multi-Tenant Foundation (3-5 days)
**Goal:** Account types + middleware + backward-compatible defaults. Nothing breaks.

| # | Task | Crate | Files | Tests |
|---|------|-------|-------|-------|
| 1.1 | Create `AccountId(Option<String>)` type, `Account`, `AccountStatus` | librefang-types | `src/account.rs` (new) | Unit: type construction, as_str_or_system() |
| 1.2 | Add `[multi_tenant]` config section | librefang-types | `src/config/types.rs` | Unit: deserialization with/without section |
| 1.3 | Create `AccountId` Axum extractor (infallible `FromRequestParts`) | librefang-api | `src/extractors.rs` (new) | Unit: 6 extraction tests (header, empty, whitespace, UUID, absent, infallible) |
| 1.4 | Add HMAC sig verification + policy matrix | librefang-api | `src/middleware.rs` | Unit: 5 policy matrix tests |
| 1.5 | Create guards: `check_account()`, `validate_account!`, `account_or_system!` | librefang-api | `src/routes/shared.rs`, `src/macros.rs` (new) | Unit: 6 guard tests |
| 1.6 | Wire HMAC middleware + extractor into router | librefang-api | `src/server.rs` | Integration: all routes receive AccountId |
| 1.7 | Add `account_id` to `UserIdentity` + `AgentEntry` | librefang-kernel | `src/auth.rs`, `src/registry.rs` | Unit: identity construction, filtered queries |
| 1.7 | Run full existing test suite | all | — | Regression: 100% pass with `multi_tenant.enabled = false` |

**Exit criteria:** All existing tests pass. `curl` without `X-Account-Id` header behaves identically to pre-change.

### Phase 2: Resource Isolation (5-7 days)
**Goal:** Agents, channels, integrations, skills scoped by account.

| # | Task | Crate | Files | Tests |
|---|------|-------|-------|-------|
| 2.1 | Add `account_id` to `AgentEntry` | librefang-kernel | `src/registry.rs` | Unit: list_for_account, get_for_account |
| 2.2 | Update `spawn_agent` to accept `account_id` | librefang-kernel | `src/kernel.rs` | Integration: spawned agent tagged correctly |
| 2.3 | Update all agent routes to filter by account | librefang-api | `src/routes/agents.rs` | Integration: cross-account access returns 404 |
| 2.4 | Account-scoped integration registry | librefang-extensions | `src/registry.rs` | Unit: install_for_account, list_for_account |
| 2.5 | Update integration routes | librefang-api | `src/routes/plugins.rs` | Integration: cross-account returns 404 |
| 2.6 | Account-scoped channel loading | librefang-channels | `src/router.rs`, `src/bridge.rs` | Integration: channels resolved within account |
| 2.7 | Update channel routes | librefang-api | `src/routes/channels.rs` | Integration: filtered by account |
| 2.8 | Skill allowlist enforcement | librefang-skills | `src/registry.rs` | Unit: blocked skill returns error |
| 2.9 | Update remaining routes (workflows, goals, prompts, budget) | librefang-api | `src/routes/*.rs` | Integration: all filtered |
| 2.10 | Event bus account tagging | librefang-kernel | `src/event_bus.rs` | Unit: events filtered by account |
| 2.11 | Account-partitioned filesystem | librefang-kernel | `src/account_manager.rs` | Integration: agents dir created per account |

**Exit criteria:** Two accounts can operate simultaneously on same daemon with complete isolation. Cross-account access attempts return 404.

### Phase 3: Data & Memory Isolation (3-5 days)
**Goal:** All persistent data scoped by account. Supabase RLS enforces at DB level.

| # | Task | Crate | Files | Tests |
|---|------|-------|-------|-------|
| 3.1 | SQLite migration script (account_id columns) | librefang-memory | `src/migration.rs` | Integration: migration runs idempotently |
| 3.2 | Update semantic memory (recall/insert) | librefang-memory | `src/semantic.rs` | Unit: cross-account recall returns nothing |
| 3.3 | Update session storage | librefang-memory | `src/session.rs` | Unit: sessions isolated by account |
| 3.4 | Update structured KV store | librefang-memory | `src/structured.rs` | Unit: KV isolated by account |
| 3.5 | Update proactive memory | librefang-memory | `src/proactive.rs` (if exists) | Unit: proactive memories isolated |
| 3.6 | Context engine propagation | librefang-runtime | `src/context_engine.rs` | Integration: ingest scopes to account |
| 3.7 | HTTP vector store: include account_id in payload | librefang-memory | `src/http_vector_store.rs` | Unit: payload includes account_id |
| 3.8 | Supabase RLS policies for account isolation | docker/sql | `docker/sql/rls_policies.sql` (new) | Integration: cross-account query returns empty |
| 3.9 | Memory routes | librefang-api | `src/routes/memory.rs` | Integration: memory API scoped |

**Exit criteria:** Memory from Account A is invisible to Account B. Existing `system` account data accessible without changes.

### Phase 4: Hardening & Integration (3-5 days)
**Goal:** Security validation, performance, documentation.

| # | Task | Crate | Files | Tests |
|---|------|-------|-------|-------|
| 4.1 | Security audit: enumerate all query paths | all | — | Checklist: every SQL/HTTP query includes account_id |
| 4.2 | Timing-safe HMAC comparison | librefang-kernel | `src/account_manager.rs` | Unit: constant-time validation |
| 4.3 | Cross-account penetration tests | librefang-testing | new test file | E2E: systematic cross-account access attempts |
| 4.4 | Cross-account Supabase RLS penetration tests | docker/sql | — | E2E: direct SQL as wrong account returns nothing |
| 4.5 | Performance baseline | — | — | Benchmark: overhead of account filtering vs. baseline |
| 4.6 | Dashboard account context | librefang-api | `dashboard/src/` | Dashboard shows account-scoped data |
| 4.7 | Migration guide for operators | docs | `multi-tenant/MIGRATION-GUIDE.md` | — |
| 4.8 | Update API documentation (en + zh) | docs | `src/app/configuration/page.mdx` + zh | — |
| 4.9 | Update CONTRIBUTING.md | root | `CONTRIBUTING.md` | — |

**Exit criteria:** Security audit passes. Performance overhead < 5%. Docs updated.

---

## MIGRATION-GUIDE: Single-Tenant → Multi-Tenant

### For Existing LibreFang Operators

#### Step 1: Update LibreFang
```bash
# Pull latest release with multi-tenant support
librefang update
```

#### Step 2: Enable Multi-Tenant Mode
```toml
# ~/.librefang/config.toml
[multi_tenant]
enabled = true
hmac_secret = "your-256-bit-secret-here"  # Generate: openssl rand -hex 32
```

#### Step 3: Restart
```bash
librefang restart
```

#### What Happens Automatically
1. Database migration runs on startup — adds `account_id` columns, all existing data tagged as `system`
2. `accounts/system/` directory created, existing agent manifests moved into it
3. API requests without `X-Account-Id` header continue working (assigned to `system` account)
4. CLI commands continue working (always `system` account)
5. Desktop app continues working (always `system` account)

#### What Doesn't Change
- Config format (additive only)
- CLI behavior
- Desktop app behavior
- Channel bridges without account bindings (default to `system`)
- Skill installations (remain global)

#### For Qwntik Integration
Qwntik's `@kit/openfang` package sends `X-Account-Id` + `X-Account-Sig` headers on all requests via `getAccountOptions()`. Once multi-tenant is enabled on LibreFang, Qwntik's existing account isolation flows through end-to-end.

Required Qwntik config:
```env
OPENFANG_ACCOUNT_HMAC_SECRET=same-secret-as-librefang-config
```

---

## Estimated Timeline

| Phase | Duration | Cumulative | Status |
|-------|----------|------------|--------|
| Phase 0: RuVector Crate Port | 2-3 days | Week 1 | ✅ 0.1–0.6 verified; 0.7 remaining |
| Phase 1: Multi-Tenant Foundation | 3-5 days | Week 1-2 | ⬜ Not started (account_id in channels done; extractor/HMAC/guards pending) |
| Phase 2: Resource Isolation | 5-7 days | Week 2-3 | ⬜ Blocked on Phase 1 |
| Phase 3: Data & Memory Isolation | 3-5 days | Week 3-4 | ⬜ Blocked on Phase 2 |
| Phase 4: Hardening & Integration | 3-5 days | Week 4-5 | ⬜ Blocked on Phase 3 |
| **Total** | **18-25 days** | **~5-6 weeks** |

## Risk Register

| Risk | Impact | Mitigation |
|------|--------|------------|
| Upstream LibreFang ships conflicting multi-tenant | High | Monitor PRs. Our ADRs are designed to be upstreamable. |
| Performance regression from account filtering | Medium | Indexed queries. Benchmark in Phase 3. |
| SQLite migration on large databases is slow | Low | Migration is additive (ALTER + INDEX). No table rebuilds. |
| Missed query path leaks cross-account data | High | Phase 3 security audit + penetration tests. |
| Channel bridges break with account scoping | Medium | `system` default ensures backward compat. |

## Test Strategy

### Per-Phase Testing
- **Phase 0:** Existing test suite passes 100% (regression gate)
- **Phase 1:** New tests: 2 accounts, each with agents/channels/skills. Cross-access returns 404.
- **Phase 2:** New tests: memory/session/KV isolation. Cross-account recall returns empty.
- **Phase 3:** Penetration tests: systematic attempt to access Account B resources from Account A context.

### Contract Tests (Port from Qwntik)
Qwntik has 369 contract tests against the openfang API. These become the acceptance criteria:
- Port relevant tests to run against LibreFang + multi-tenant
- Each test runs with account context headers
- Validates that qwntik's `@kit/openfang` SDK works unchanged

### Continuous Integration
- All phases gate on: `cargo test --workspace`
- Phase 1+ gates on: multi-tenant integration test suite
- Phase 3 gates on: security audit checklist + penetration tests

---

## Document Authoring Order

Write and approve in this sequence:

1. **ADR-RV-001** → crate port is Phase 0, no dependencies
2. **SPEC-RV-001** → acceptance criteria for extension verification
3. **ADR-MT-001** → foundational multi-tenant decision, everything else depends on it
4. **SPEC-MT-001** → exact types needed before any code
5. **ADR-MT-002** → auth mechanism locks the middleware design
6. **ADR-MT-003** → resource strategy locks the registry changes  
7. **ADR-MT-004** → data isolation locks the memory changes
8. **SPEC-MT-003** → migration script needed early for dev environments
9. **SPEC-MT-002** → route changes are the largest surface area, write last
10. **PLAN-MT-001** → this document IS the plan
11. **MIGRATION-GUIDE** → write during Phase 4 from real experience

---

## Upstream Contribution Strategy

After Phase 4 validation:
1. Open RFC issue on librefang/librefang describing multi-tenant support
2. Submit Phase 0 as first PR (types + middleware, no behavior change)
3. Submit Phase 1-2 as second PR (resource + data isolation)
4. If rejected: our fork delta stays at ~15-20 files, easily maintained
5. If accepted: we're on vanilla LibreFang + Qwntik. Zero fork tax.

### RuVector Upstream Strategy
The 7 ruvector crates are independently valuable to LibreFang (vector search, local embeddings, learning systems). Submit as a separate PR from multi-tenant work:
1. PR: "feat(memory): add ruvector PostgreSQL extension for self-hosted vector search"
2. Positioned as optional workspace members — zero impact on default builds
3. High acceptance probability: LibreFang already has `HttpVectorStore` and the extension provides the backend
4. If rejected: ship as separate `ruvector-postgres` crate on crates.io, reference from Docker build only
