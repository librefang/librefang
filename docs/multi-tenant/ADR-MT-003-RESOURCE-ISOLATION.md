# ADR-MT-003: Resource Isolation Strategy

**Status:** Proposed
**Date:** 2026-04-06
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

## Alternatives Considered

### Alt 1: Separate router per account
**Rejected.** Axum's Router is built once at startup. Dynamic per-account routers
would require a router factory and lose compile-time route checking. Filtering at
the handler level is simpler and proven in openfang-ai.

### Alt 2: Middleware-level account rejection
**Rejected.** A blanket middleware that rejects AccountId(None) would break
desktop mode and public health endpoints. Handler-level guards with tiered
policies are more flexible.

## Consequences

- **Positive:** Every tenant-visible endpoint is scoped. Cross-tenant access returns 404.
- **Negative:** ~252 handler signatures change. Mechanical but high-volume.
- **Phase 3 debt:** Memory recall filtering (account_id on memories table) and session scoping.
