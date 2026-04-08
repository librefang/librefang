# ADR-MT-003: Resource Isolation Strategy

**Status:** In Progress — 6 of 15 route files fully isolated (3 data-filtered + 3 admin-guarded); 3 partially filtered; 6 extractor-only
**Date:** 2026-04-06 (updated 2026-04-07 — corrected after code audit)
**Author:** Engineering
**Related:** ADR-MT-001 (Account Model), ADR-MT-002 (API Auth)
**Epic:** Multi-Tenant Architecture — Phase 2

---

## Problem Statement

After Phase 1 lands the AccountId type and scopes 76 handlers (agents, channels,
config), 241 handlers across 12 route files remain unscoped. Any of these can
leak data cross-tenant or allow one tenant to modify another's resources.

Phase 2 must scope ALL remaining tenant-visible resources: skills, workflows,
plugins, providers, budget, goals, inbox, media, memory, network, system (subset).

> **Event bus isolation** is covered separately in **ADR-MT-005** (Event Bus Tenant
> Isolation). This ADR covers API handler scoping; ADR-MT-005 covers the dispatch-side
> filtering, `subscribe_account()`, and `history_for_account()` changes to the
> EventBus struct.

## Blast Radius Scan

```bash
# Handler counts per route file (verified 2026-04-06):
# Phase 1 (already scoped by PLAN-MT-001):
#   agents.rs:    50 handlers  ← DONE in Phase 1
#   channels.rs:  11 handlers  ← extractor added in Phase 1, full scoping Phase 2
#   config.rs:    15 handlers  ← DONE in Phase 1

# Phase 2 scope (this ADR):
$ grep -c "pub async fn" crates/librefang-api/src/routes/*.rs
#   skills.rs:      53 handlers
#   system.rs:      63 handlers (subset — health/version are public)
#   workflows.rs:   30 handlers
#   memory.rs:      25 handlers
#   network.rs:     19 handlers
#   providers.rs:   19 handlers
#   budget.rs:      10 handlers
#   plugins.rs:      8 handlers
#   goals.rs:        7 handlers
#   media.rs:        6 handlers
#   inbox.rs:        1 handler
#   channels.rs:    11 handlers (full scoping — extractor from Phase 1)
#   TOTAL Phase 2: ~252 handlers (minus public system endpoints)

# Public endpoints (NOT scoped — available to all):
#   GET /health, GET /version, GET /ready, GET /.well-known/agent.json
#   Estimated: ~10 system.rs handlers stay public
```

**Scope decision:** ALL handlers in Phase 2 files get `account: AccountId` param.
System health endpoints explicitly bypass with `account_or_system!`. Everything
else uses `check_account()` or `validate_account!`.

## Decision

### Tiered scoping by resource sensitivity

| Tier | Resources | Guard | Rationale |
|------|-----------|-------|-----------|
| **Tier 1: Full ownership** | skills, workflows, goals, inbox, media | `check_account()` on every read/write | These are user-created content, must be fully isolated |
| **Tier 2: Account-filtered** | providers, budget, plugins | `validate_account!` on writes, `account_or_system!` on reads | System defaults visible to all, tenant overrides scoped |
| **Tier 3: Shared + overlay** | network, memory | `account_or_system!` | Network peers may span accounts; memory recall filters by account but system memories visible |
| **Tier 4: Public** | system health/version/ready | No guard | Monitoring endpoints must work without auth |

### Registry changes

The DashMap-based `AgentRegistry` currently stores all agents in a single flat map.
Phase 2 does NOT change the registry data structure — filtering happens at the
API layer via `registry.list_by_account()` (added in Phase 1 Round 3).

For skills, workflows, goals, plugins — the kernel stores these in SQLite tables
that need `account_id` columns (see ADR-MT-004 for schema changes).

### Channel scoping (deferred from Phase 1)

Channels have a many-to-many relationship with agents. A channel (e.g., Slack
workspace) may serve multiple accounts. Phase 2 scoping:
- Channel **configuration** is account-scoped (who configured this Slack webhook)
- Channel **message routing** uses agent ownership (message goes to the agent's account)
- Channel **listing** shows only channels configured by the requesting account

## Pattern Definition

Same pattern as Phase 1 — every handler in scope gets:

```rust
pub async fn handler_name(
    State(state): State<Arc<AppState>>,
    account: AccountId,
    // ... other params
) -> impl IntoResponse {
    // Tier 1: check_account(&entry, &account)?;
    // Tier 2: validate_account!(account); or account_or_system!(account);
    // Tier 3: account_or_system!(account);
    // Tier 4: (no guard)
}
```

## Verification Gate

```bash
# Gate: zero unscoped handlers in ALL route files
for f in agents.rs channels.rs config.rs skills.rs workflows.rs \
         memory.rs network.rs providers.rs budget.rs plugins.rs \
         goals.rs media.rs inbox.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f")
  UNSCOPED=$((TOTAL - SCOPED))
  echo "$f: $SCOPED/$TOTAL scoped ($UNSCOPED remaining)"
done

# system.rs: count ONLY non-public handlers
PUBLIC=$(grep -c "// PUBLIC" "crates/librefang-api/src/routes/system.rs" || echo 0)
TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/system.rs")
SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/system.rs")
echo "system.rs: $SCOPED/$TOTAL scoped, $PUBLIC public (expected: $((TOTAL - PUBLIC)) scoped)"
```

## ⚠️ Known Gap: Upload Cross-Tenant Read (Discovered 2026-04-07)

**Gap:** `serve_upload` in `agents.rs` accepts `AccountId` but never checks it.
`UploadMeta` has no `account_id` field, so a tenant who knows a `file_id` can
fetch another tenant's uploaded file. The `require_account_id` middleware ensures
the header is present, but without ownership metadata on the upload record itself,
enforcement is impossible at the handler level.

**Root cause:** Uploads were exempted from account enforcement in early Phase 1 and
the metadata struct was never extended.

**Fix required:**
1. Add `account_id: Option<String>` to `UploadMeta` (or equivalent registration struct)
2. Populate `account_id` at upload time from the request's `AccountId`
3. Check ownership in `serve_upload` before returning the file
4. Add integration test: tenant A uploads file, tenant B attempts fetch → 404

**Files:**
- `crates/librefang-api/src/routes/agents.rs` (serve_upload handler)
- `crates/librefang-api/src/routes/media.rs` (UploadMeta struct)
- `crates/librefang-api/tests/account_tests.rs` (cross-tenant upload test)

**Tracked in:** PENDING-WORK.md item #3

---

## Alternatives Considered

### Alt 1: Separate router per account
**Rejected.** Axum's Router is built once at startup. Dynamic per-account routers
would require a router factory and lose compile-time route checking. Filtering at
the handler level is simpler and proven in openfang-ai.

### Alt 2: Middleware-level account rejection
**Rejected.** A blanket middleware that rejects AccountId(None) would break
desktop mode and public health endpoints. Handler-level guards with tiered
policies are more flexible.

## Implementation Progress (verified 2026-04-07)

> **Important:** "AccountId extractor in signature" ≠ "tenant-filtered." A handler
> with `_account: AccountId` provides zero isolation. Only handlers that use the
> value in a query or guard (`check_account`, `list_by_account`, `get_scoped`,
> `require_agent_access`, `require_admin`, or direct `account.0` checks) are genuinely scoped.

| Tier | File | Handlers | Data-filtered | Admin-guarded | Tier 4 Public | `_account` (unused) | No `AccountId` | Status |
|------|------|----------|--------------|--------------|--------------|--------------------|--------------------|--------|
| 1 | `agents.rs` | 50 | 48 | 0 | 0 | 1 | 1 (helper) | **Done** |
| 1 | `memory.rs` | 25 | 25 | 0 | 0 | 0 | 0 | **Done** |
| 1 | `prompts.rs` | 12 | 12 | 0 | 0 | 0 | 0 | **Done** |
| 2 | `config.rs` | 15 | 2 | 10 | 3 | 0 | 0 | **Done** |
| 2 | `budget.rs` | 10 | 5 | 5 | 0 | 0 | 0 | **Done** |
| 2 | `network.rs` | 20 | 7 | 13 | 0 | 0 | 0 | **Done** |
| 2 | `system.rs` | 63 | 6 | 0 | 0 | 57 | 0 | Partial — tier decision pending |
| 2 | `skills.rs` | 53 | 2 | 0 | 0 | 51 | 0 | Barely started |
| 2 | `workflows.rs` | 30 | 1 | 0 | 0 | 29 | 0 | Barely started |
| — | `channels.rs` | 11 | 0 | 0 | 0 | 11 | 0 | Extractor only |
| — | `providers.rs` | 19 | 0 | 0 | 0 | 19 | 0 | Extractor only |
| — | `plugins.rs` | 8 | 0 | 0 | 0 | 8 | 0 | Extractor only |
| — | `goals.rs` | 7 | 0 | 0 | 0 | 7 | 0 | Extractor only |
| — | `media.rs` | 6 | 0 | 0 | 0 | 6 | 0 | Extractor only |
| — | `inbox.rs` | 1 | 0 | 0 | 0 | 1 | 0 | Extractor only |
| | **Total** | **330** | **108** | **28** | **3** | **190** | **1** | **42% isolated** |

## Consequences

### Positive
- 6 of 15 route files are fully tenant-isolated: agents, memory, prompts (data-filtered), config, budget, network (admin-guarded). Cross-tenant access returns 404 or 403 on these endpoints.
- `require_admin()` guard added to `shared.rs` — blocks scoped tenants from system-level operations (shutdown, config mutation, global budget, peer registry, A2A management, MCP HTTP).
- Registry-backed scoping primitives (`get_scoped`, `list_by_account`) proven and reusable.
- `require_account_id` middleware enforces header presence in multi-tenant mode.
- All 15 config.rs handlers now have `AccountId` — no handlers without account context remain.

### Negative
- ~190 handlers accept `_account: AccountId` but ignore it. Cross-tenant access is possible on those endpoints.
- 1 handler (helper in agents.rs) has no `AccountId` parameter.
- Admin-guarded handlers return 403 to tenants rather than providing per-tenant views. Global budget/usage/metrics will need per-tenant kernel APIs in Phase 3.
- Store-level filtering not yet wired — handlers that do filter mostly rely on registry (agents) or in-handler match arms (memory). Other stores have no `account_id` parameter.
- Skills allowlist adds config complexity per account.

### Remaining Work
- Wire filtering in remaining 9 partially-scoped + unscoped route files — see PENDING-WORK.md items #1–#2
- Upload cross-tenant fix — see §Known Gap above and PENDING-WORK.md item #3
- Kernel/store migration (ADR-MT-004, v19) — ~76 store methods need `account_id`
- Event bus isolation — see ADR-MT-005
- Channel bridge per-adapter account routing — Phase 4
