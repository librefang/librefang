# ADR-MT-001: Account Model

**Status:** Proposed
**Date:** 2026-04-06
**Author:** Engineering
**Related:** MASTER-PLAN.md, ADR-MT-002 (Auth), ADR-MT-003 (Resource Isolation), ADR-MT-004 (Data Isolation)
**Epic:** Multi-Tenant Architecture

---

## Problem Statement

LibreFang is single-tenant. One daemon = one owner = one flat namespace for agents,
sessions, memories, skills, channels, workflows, goals, plugins, and media.

There is no concept of an "account." The kernel holds a single `AgentRegistry`
(`DashMap<AgentId, AgentEntry>`), the SQLite schema has no `account_id` column in
any table, the auth middleware validates a single global API key, and all 297 HTTP
handlers assume they operate on the one global dataset.

This blocks:
- SaaS hosting (multiple customers on one instance)
- Team isolation (separate agent pools within an org)
- Supabase RLS integration (Phase 0 RuVector extension expects `account_id` for row-level security)

### Source files verified (2026-04-06):

| Component | File | Key Finding |
|-----------|------|-------------|
| Kernel | `librefang-kernel/src/kernel.rs` (12,492 lines) | Singleton, no account partitioning |
| Agent Registry | `librefang-kernel/src/registry.rs` (570 lines) | `DashMap<AgentId, AgentEntry>` — flat, global |
| Auth Middleware | `librefang-api/src/middleware.rs` (531 lines) | Single API key + session tokens, no tenant extraction |
| Agent types | `librefang-types/src/agent.rs` | `AgentEntry` has no `account_id` field |
| Memory types | `librefang-types/src/memory.rs` | `MemoryFragment` has `agent_id` but no `account_id` |
| Session store | `librefang-memory/src/session.rs` | `Session { id, agent_id, messages }` — no account |
| Config | `librefang-types/src/config/types.rs` | `UserConfig` has role but no account association |
| DB migrations | `librefang-memory/src/migration.rs` (1,552 lines, 17 versions) | Zero tables have `account_id` |

---

## Blast Radius Scan

### 1. API Route Handlers (297 total, 0 account-scoped)

Scan: count all `pub async fn` handlers in each route file.

| File | Total Handlers | Currently Account-Scoped | Gap |
|------|---------------|-------------------------|-----|
| `routes/system.rs` | 63 | 0 | 63 |
| `routes/skills.rs` | 53 | 0 | 53 |
| `routes/agents.rs` | 50 | 0 | 50 |
| `routes/workflows.rs` | 30 | 0 | 30 |
| `routes/memory.rs` | 25 | 0 | 25 |
| `routes/providers.rs` | 19 | 0 | 19 |
| `routes/network.rs` | 19 | 0 | 19 |
| `routes/config.rs` | 15 | 0 | 15 |
| `routes/channels.rs` | 11 | 0 | 11 |
| `routes/budget.rs` | 10 | 0 | 10 |
| `routes/plugins.rs` | 8 | 0 | 8 |
| `routes/goals.rs` | 7 | 0 | 7 |
| `routes/media.rs` | 6 | 0 | 6 |
| `routes/inbox.rs` | 1 | 0 | 1 |
| **Total** | **297** | **0** | **297** |

### 2. Database Tables (15+ tables, 0 with account_id)

| Table | Current Key | Needs account_id | Phase |
|-------|------------|-------------------|-------|
| `agents` | `id TEXT PK` | Yes | 1 |
| `sessions` | `id TEXT PK` | Yes | 1 |
| `memories` | `id TEXT PK` | Yes | 3 |
| `usage_events` | `id TEXT PK` | Yes | 1 |
| `approval_audit` | `id TEXT PK` | Yes | 2 |
| `kv_store` | `(agent_id, key) PK` | Yes | 2 |
| `task_queue` | `id TEXT PK` | Yes | 2 |
| `events` | `id TEXT PK` | Yes | 2 |
| `entities` | `(agent_id, name) PK` | Yes | 3 |
| `relations` | `(agent_id, source, target, rel_type)` | Yes | 3 |
| `canonical_sessions` | `agent_id PK` | Yes | 3 |
| `paired_devices` | `device_id PK` | Yes | 2 |
| `audit_entries` | auto-increment | Yes | 2 |
| `prompt_versions` | `id TEXT PK` | Yes | 2 |
| `prompt_experiments` | `id TEXT PK` | Yes | 2 |

### 3. Kernel & Runtime (0 account context)

| Component | File | Lines | Issue |
|-----------|------|-------|-------|
| Kernel | `kernel.rs` | 12,492 | Singleton, no account in `spawn_agent()` |
| Agent Registry | `registry.rs` | 570 | `DashMap<AgentId, AgentEntry>` — flat global |
| Agent Loop | `agent_loop.rs` | 6,465 | No account context in execution |
| Memory Substrate | `substrate.rs` | ~2,000 | `recall()` / `remember()` take agent_id only |
| Session Store | `session.rs` | 445 | Queries by agent_id, no account filter |
| Semantic Store | `semantic.rs` | ~1,000 | Vector search by agent_id only |
| Proactive Memory | `proactive.rs` | ~800 | `stats()` takes user_id, no account |

### 4. Config & Auth (single-tenant)

| Item | Current State | Gap |
|------|---------------|-----|
| API key | Single global `api_key` in config | Need per-account keys or JWT |
| Dashboard creds | Single `dashboard_user` / `dashboard_pass` | Need per-account login |
| Session tokens | `HashMap<String, SessionToken>` in memory | Need account association |
| Users | `Vec<UserConfig>` with roles but no account | Need account_id field |
| CORS | Single `cors_origin` list | May need per-account origins |

**Scope decision:** ALL 297 handlers and ALL 15+ tables require `account_id`. The pattern
is universal — there are zero existing account-scoped paths to preserve.

---

## Decision

Introduce `AccountId` as a first-class type threaded through every layer of the stack.
Use a **backward-compatible "system" default account** so existing single-tenant
deployments continue working without configuration changes.

### Core Design

```rust
// librefang-types/src/account.rs (NEW FILE)

/// Tenant isolation boundary. Every resource belongs to exactly one account.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Uuid);

impl AccountId {
    /// The implicit account for single-tenant / backward-compatible deployments.
    pub const SYSTEM: AccountId = AccountId(Uuid::from_u128(0));

    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AccountId {
    fn default() -> Self {
        Self::SYSTEM
    }
}

/// Account metadata. Minimal for Phase 1 — extended in Phase 2+.
pub struct Account {
    pub id: AccountId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub status: AccountStatus,  // Active, Suspended, Deleted
}

pub enum AccountStatus {
    Active,
    Suspended,
    Deleted,
}
```

### Backward Compatibility Contract

1. If no account context is provided in a request, `AccountId::SYSTEM` is used
2. Existing API keys map to `AccountId::SYSTEM`
3. Existing SQLite data migrated with `account_id = 'system'` default
4. Single-tenant deployments never see account_id in API responses unless opted in
5. Channel bridges default to `AccountId::SYSTEM` when no tenant routing configured

### Account Resolution Chain

```
Request → middleware.rs
  1. Check Authorization header for JWT → extract account_id claim
  2. Check X-API-Key → lookup account from api_keys table
  3. Check session token → lookup account from active_sessions
  4. Fallback → AccountId::SYSTEM
  → Insert AccountId into axum request Extensions
```

Every handler extracts `AccountId` from Extensions (via a custom Axum extractor),
never from query params or path segments.

---

## Pattern Definition

**The structural rule (grepable):**

Every handler in a tenant-scoped route file MUST:
1. Extract `account_id: AccountId` from the request (via Axum extractor)
2. Pass `account_id` to every kernel/storage call

Every SQL query on tenant-scoped tables MUST:
1. Include `WHERE account_id = ?` (or `AND account_id = ?`)
2. Use parameterized queries (no string interpolation)

```rust
// PATTERN: Every route handler signature includes AccountId extractor
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    account_id: AccountId,  // <-- Axum FromRequestParts extractor
) -> impl IntoResponse {
    let agents = state.kernel.list_agents(&account_id).await?;
    // ...
}

// ANTI-PATTERN: Handler without AccountId
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    // missing account_id — VIOLATION
) -> impl IntoResponse
```

**SQL pattern:**
```sql
-- PATTERN: Every query includes account_id filter
SELECT * FROM agents WHERE account_id = ?1 AND id = ?2;
INSERT INTO agents (id, account_id, ...) VALUES (?1, ?2, ...);

-- ANTI-PATTERN: Query without account_id
SELECT * FROM agents WHERE id = ?1;  -- VIOLATION: no tenant filter
```

---

## Implementation Scope

### Phase 1 (This ADR): Foundation (3-5 days)

| File | Change | Handlers Affected |
|------|--------|-------------------|
| `librefang-types/src/account.rs` | NEW: `AccountId`, `Account`, `AccountStatus` | — |
| `librefang-types/src/agent.rs` | Add `account_id: AccountId` to `AgentEntry` | — |
| `librefang-types/src/lib.rs` | Add `pub mod account;` | — |
| `librefang-api/src/middleware.rs` | Add `AccountId` extraction from JWT/key/session → Extensions | — |
| `librefang-api/src/extractors.rs` | NEW: `impl FromRequestParts for AccountId` | — |
| `librefang-kernel/src/registry.rs` | Add `account_id` filter to `list()`, `get()`, `spawn()` | — |
| `librefang-kernel/src/kernel.rs` | Thread `account_id` through `spawn_agent()`, `list_agents()` | — |
| `librefang-memory/src/migration.rs` | v18: `ALTER TABLE agents ADD COLUMN account_id TEXT DEFAULT 'system'` | — |
| `librefang-api/src/routes/agents.rs` | Add `account_id: AccountId` extractor to all handlers | 50 |
| `librefang-api/src/routes/channels.rs` | Add `account_id: AccountId` extractor | 11 |
| `librefang-api/src/routes/config.rs` | Add `account_id: AccountId` extractor | 15 |

**Phase 1 total: 76 handlers + 6 new/modified infra files**

### Phase 2: Resource Isolation (5-7 days) — ADR-MT-003

All remaining route files get `AccountId` + database tables get column:
- `routes/system.rs` (63), `routes/skills.rs` (53), `routes/workflows.rs` (30),
  `routes/budget.rs` (10), `routes/providers.rs` (19), `routes/network.rs` (19),
  `routes/plugins.rs` (8), `routes/goals.rs` (7), `routes/media.rs` (6), `routes/inbox.rs` (1)
- Tables: `approval_audit`, `kv_store`, `task_queue`, `events`, `paired_devices`,
  `prompt_versions`, `prompt_experiments`

**Phase 2 total: 216 handlers + 7 tables**

### Phase 3: Data & Memory Isolation (3-5 days) — ADR-MT-004

Memory substrate + semantic store + knowledge graph:
- `substrate.rs`, `semantic.rs`, `session.rs`, `proactive.rs`
- Tables: `memories`, `entities`, `relations`, `canonical_sessions`
- `routes/memory.rs` (25 handlers)

**Phase 3 total: 25 handlers + 4 tables + 4 core memory files**

### Phase 4: Hardening (3-5 days)

- Security audit: systematic cross-account access testing
- Channel bridge: account-aware routing for all 50+ adapters
- Performance: benchmark account_id indexed queries
- Supabase RLS: wire `account_id` to Phase 0 PostgreSQL policies

---

## Verification Gate

```bash
#!/usr/bin/env bash
set -euo pipefail

ROUTES_DIR="crates/librefang-api/src/routes"

echo "=== ADR-MT-001 Verification Gate ==="

# Gate 1: AccountId type exists
grep -q "pub struct AccountId" crates/librefang-types/src/account.rs \
  || { echo "FAIL: AccountId type not found"; exit 1; }
echo "✅ AccountId type exists"

# Gate 2: AccountId extractor exists
grep -q "impl.*FromRequestParts.*for AccountId" crates/librefang-api/src/extractors.rs \
  || { echo "FAIL: AccountId extractor not found"; exit 1; }
echo "✅ AccountId extractor exists"

# Gate 3: Middleware extracts account
grep -q "AccountId" crates/librefang-api/src/middleware.rs \
  || { echo "FAIL: Middleware does not reference AccountId"; exit 1; }
echo "✅ Middleware references AccountId"

# Gate 4: AgentEntry has account_id
grep -q "account_id.*AccountId" crates/librefang-types/src/agent.rs \
  || { echo "FAIL: AgentEntry missing account_id field"; exit 1; }
echo "✅ AgentEntry has account_id"

# Gate 5: Migration adds account_id to agents table
grep -q "account_id" crates/librefang-memory/src/migration.rs \
  || { echo "FAIL: Migration does not add account_id"; exit 1; }
echo "✅ Migration references account_id"

# Gate 6: Phase-specific handler coverage
# Phase 1 target: agents.rs, channels.rs, config.rs
for f in agents.rs channels.rs config.rs; do
  TOTAL=$(grep -c "pub async fn" "$ROUTES_DIR/$f" 2>/dev/null || echo 0)
  SCOPED=$(grep -c "account_id.*AccountId\|AccountId.*account_id\|account_id: AccountId" "$ROUTES_DIR/$f" 2>/dev/null || echo 0)
  UNSCOPED=$((TOTAL - SCOPED))
  echo "$f: $SCOPED/$TOTAL scoped ($UNSCOPED remaining)"
  if [ "$UNSCOPED" -gt 0 ]; then
    echo "FAIL: $f has $UNSCOPED unscoped handlers"
    exit 1
  fi
done

# Gate 7: Compilation
cargo check -p librefang-types -p librefang-api -p librefang-kernel
echo "✅ Compilation clean"

echo ""
echo "=== ADR-MT-001 Gate: ALL PASSED ==="
```

### Full-scope gate (runs after Phase 4 — all 297 handlers):

```bash
for f in "$ROUTES_DIR"/*.rs; do
  TOTAL=$(grep -c "pub async fn" "$f" 2>/dev/null || echo 0)
  [ "$TOTAL" -eq 0 ] && continue
  SCOPED=$(grep -c "account_id" "$f" 2>/dev/null || echo 0)
  BASENAME=$(basename "$f")
  echo "$BASENAME: $SCOPED/$TOTAL"
  [ "$SCOPED" -ge "$TOTAL" ] || { echo "FAIL: $BASENAME has unscoped handlers"; exit 1; }
done
```

---

## Alternatives Considered

### Alternative A: Path-based tenant isolation (`/api/v1/{account_id}/agents`)

**Pros:** Explicit in URL, easy to route at load balancer level, cacheable by path.
**Cons:** Breaks backward compat (every client URL changes), account_id in URL is a
security anti-pattern (IDOR risk, leaks in logs/referers), doubles route definitions.

**Rejected:** Breaking backward compat is unacceptable for Phase 1. The `Extensions`
approach is invisible to existing clients.

### Alternative B: Separate database per account

**Pros:** Perfect isolation, simple queries (no WHERE account_id), easy to delete an account.
**Cons:** Connection pool explosion, cross-account queries impossible, migration complexity
multiplied by N accounts, SQLite file-per-account means N open file handles.

**Rejected:** LibreFang uses embedded SQLite — separate files per account creates
operational complexity without meaningful security benefit (SQLite has no network attack
surface). Use `account_id` column + index instead.

### Alternative C: Account context via HTTP header only (`X-Account-Id`)

**Pros:** Simple, no JWT infrastructure needed.
**Cons:** Trivially spoofable (client sets any account_id), no cryptographic binding
between auth token and account, violates zero-trust principles.

**Rejected:** Header-only is insufficient for production multi-tenant. JWT claims
provide cryptographic binding. However, during Phase 1 development, the middleware
CAN fall back to `X-Account-Id` header for testing convenience, gated behind a
`allow_header_account_override: true` config flag (disabled by default).

---

## Consequences

### Positive
- Every resource has a clear owner (AccountId)
- Backward-compatible: existing deployments see zero API changes
- `AccountId::SYSTEM` default means zero migration friction
- Additive schema changes only (`ALTER TABLE ADD COLUMN ... DEFAULT`)
- Pattern is grepable — mechanical verification possible
- Enables Supabase RLS integration (Phase 0 `account_id` column matches)
- Foundation for SaaS, team isolation, and enterprise features

### Negative
- 297 handler signatures change (large diff, multi-phase rollout)
- Every SQL query must include `AND account_id = ?` (easy to miss)
- Kernel becomes account-aware — adds complexity to `spawn_agent()`, `list_agents()`
- Testing surface area multiplies (single-tenant + multi-tenant + cross-tenant negative tests)
- Index overhead: every `account_id` column needs a composite index

### Phase 3 Debt (intentionally deferred)
- **Memory isolation:** `memories`, `entities`, `relations` tables deferred to Phase 3 because
  memory queries are performance-critical and need benchmarking before adding the filter
- **Channel bridge routing:** 50+ adapters deferred to Phase 4 because each adapter has
  unique account-mapping semantics (Discord server ≠ Slack workspace ≠ Telegram chat)
- **Proactive memory consolidation:** Account-scoped consolidation deferred to Phase 3
  because the consolidation scheduler needs redesign for multi-tenant fairness
- **Knowledge graph:** Entity/relation scoping deferred to Phase 3 alongside memory

---

## Integration Points

| Trigger | Existing Code | New Behavior |
|---------|--------------|-------------|
| HTTP request arrives | `middleware::auth()` validates API key | Must also resolve `AccountId` from token/key/session and inject into Extensions |
| Agent spawned | `kernel.spawn_agent(manifest)` | Must accept `account_id` parameter, store in `AgentEntry`, persist to DB |
| Agent listed | `kernel.list_agents()` returns all | Must filter by `account_id` from request context |
| Session created | `session_store.create(agent_id)` | Must include `account_id` — agent's account propagates to session |
| Memory stored | `substrate.remember(agent_id, ...)` | Must include `account_id` (Phase 3) |
| Channel message | `channel_bridge.route(msg)` | Must resolve account from channel config binding (Phase 4) |
| Usage tracked | `budget.record_usage(agent_id, ...)` | Must include `account_id` for per-account billing |
| Config loaded | `ConfigSnapshot` from TOML | Must support per-account config overrides (Phase 2) |

---

## Quality Checks

- [x] Blast radius scan is present with actual numbers (297 handlers, 15+ tables)
- [x] Scope covers ALL affected code in touched files, not just known symptoms
- [x] Verification gate is a runnable command, not prose
- [x] Pattern definition is structural (grepable), not a list of function names
- [x] Phase 3 debt section exists with specific items and rationale
- [x] Alternatives considered (3) with trade-offs
- [x] Integration points listed with existing code references
- [x] Backward compatibility contract defined
