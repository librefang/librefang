# SPEC-MT-002: API Route Changes — Phase 2

**ADR:** ADR-MT-002 (API Auth), ADR-MT-003 (Resource Isolation)
**Date:** 2026-04-06
**Author:** Engineering

---

## Purpose

Scope all remaining 241 tenant-visible API handlers with AccountId extraction
and appropriate guards. Phase 1 covered 76 handlers (agents, channels extractor,
config). This SPEC covers the remaining 12 route files.

## Scope (from ADR-MT-003 Blast Radius Scan)

### Tier 1: Full Ownership (check_account on every op)

| File | Handlers | Guard |
|------|----------|-------|
| `routes/skills.rs` | 53 | `check_account()` — skills are user-created content |
| `routes/workflows.rs` | 30 | `check_account()` — workflow definitions are account-owned |
| `routes/goals.rs` | 7 | `check_account()` — goals are account-owned |
| `routes/inbox.rs` | 1 | `check_account()` — inbox messages are per-account |
| `routes/media.rs` | 6 | `check_account()` — uploaded media is account-owned |

**Subtotal:** 97 handlers

### Tier 2: Account-Filtered (validate on writes, system fallback on reads)

| File | Handlers | Guard |
|------|----------|-------|
| `routes/providers.rs` | 19 | Reads: `account_or_system!` (system providers visible to all). Writes: `validate_account!` |
| `routes/budget.rs` | 10 | Reads: `account_or_system!`. Writes: `validate_account!` |
| `routes/plugins.rs` | 8 | Reads: `account_or_system!` (system plugins shared). Writes: `validate_account!` |

**Subtotal:** 37 handlers

### Tier 3: Shared + Overlay

| File | Handlers | Guard |
|------|----------|-------|
| `routes/network.rs` | 19 | `account_or_system!` — network peers may span accounts |
| `routes/memory.rs` | 25 | `account_or_system!` — memory recall filters by account, system memories visible |
| `routes/channels.rs` | 11 | Full scoping (extractor added Phase 1, guard logic this phase) |

**Subtotal:** 55 handlers

### Tier 4: Public (no guard)

| File | Handlers | Public Endpoints |
|------|----------|-----------------|
| `routes/system.rs` | 63 | ~10 public (health, version, ready, well-known). ~53 need `account_or_system!` |

**Subtotal:** ~53 scoped + ~10 public

### Phase 2 Total

| Category | Handlers |
|----------|----------|
| Tier 1 (full ownership) | 97 |
| Tier 2 (account-filtered) | 37 |
| Tier 3 (shared + overlay) | 55 |
| Tier 4 (system, scoped subset) | ~53 |
| **Total scoped** | **~242** |
| Public (unscoped by design) | ~10 |

## Acceptance Criteria

### AC-1: All Tier 1 handlers enforce ownership
- **Given:** Agent/skill/workflow owned by account-A
- **When:** Account-B calls any Tier 1 endpoint for that resource
- **Then:** 404 Not Found (NOT 403)
- **And NOT:** Response body contains account-A's ID or resource details

### AC-2: Tier 2 reads show system + own resources
- **Given:** System provider "openai" exists + account-A provider "custom-llm" exists
- **When:** Account-A lists providers
- **Then:** Both "openai" and "custom-llm" returned
- **And NOT:** Account-B's "custom-llm-2" visible to account-A

### AC-3: Tier 2 writes require account
- **Given:** Request with `AccountId(None)` (no header)
- **When:** POST to create a new provider override
- **Then:** 400 Bad Request ("Account required")
- **And NOT:** Provider created under system account silently

### AC-4: Tier 3 memory recall filters by account
- **Given:** Account-A stored memory "my secret plan", Account-B stored "their plan"
- **When:** Account-A recalls memories
- **Then:** Only "my secret plan" returned (+ any system memories)
- **And NOT:** Account-B's memories visible

### AC-5: Tier 4 public endpoints work without account
- **Given:** Request with no X-Account-Id header
- **When:** GET /health, GET /version
- **Then:** 200 OK with valid response
- **And NOT:** 400 or 401 due to missing account

### AC-6: Channel full scoping
- **Given:** Channel configured by account-A
- **When:** Account-B lists channels
- **Then:** Account-A's channel not visible to account-B
- **And NOT:** Channel messages from account-A routed to account-B's agents

## Claims Requiring Verification

| Claim | Verification | Test Name |
|-------|-------------|-----------|
| All Tier 1 handlers have check_account | Pattern gate | `test_tier1_all_check_account` |
| All Tier 2 reads use account_or_system | Pattern gate | `test_tier2_reads_system_fallback` |
| All Tier 2 writes use validate_account | Pattern gate | `test_tier2_writes_require_account` |
| Cross-tenant skill access → 404 | Integration test | `test_cross_tenant_skill_404` |
| Cross-tenant workflow access → 404 | Integration test | `test_cross_tenant_workflow_404` |
| System provider visible to all | Integration test | `test_system_provider_visible_all` |
| Memory recall scoped by account | Integration test | `test_memory_recall_scoped` |
| Health endpoint works without account | Integration test | `test_health_no_account` |
| Channel listing scoped | Integration test | `test_channel_listing_scoped` |

## Exit Gate

```bash
#!/bin/bash
set -e

echo "=== SPEC-MT-002 Pattern Coverage Gate ==="

FAIL=0
for f in skills.rs workflows.rs goals.rs inbox.rs media.rs \
         providers.rs budget.rs plugins.rs network.rs memory.rs \
         channels.rs; do
  TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/$f")
  SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/$f" || echo 0)
  UNSCOPED=$((TOTAL - SCOPED))
  if [ "$UNSCOPED" -gt 0 ]; then
    echo "FAIL: $f: $SCOPED/$TOTAL scoped ($UNSCOPED remaining)"
    FAIL=1
  else
    echo "PASS: $f: $SCOPED/$TOTAL scoped"
  fi
done

# system.rs: check non-public handlers
TOTAL=$(grep -c "pub async fn" "crates/librefang-api/src/routes/system.rs")
PUBLIC=$(grep -c "// PUBLIC" "crates/librefang-api/src/routes/system.rs" || echo 0)
SCOPED=$(grep -c "account: AccountId" "crates/librefang-api/src/routes/system.rs" || echo 0)
EXPECTED=$((TOTAL - PUBLIC))
if [ "$SCOPED" -lt "$EXPECTED" ]; then
  echo "FAIL: system.rs: $SCOPED/$EXPECTED non-public scoped"
  FAIL=1
else
  echo "PASS: system.rs: $SCOPED/$EXPECTED non-public scoped ($PUBLIC public)"
fi

[ "$FAIL" -eq 0 ] || { echo "=== GATE FAILED ==="; exit 1; }
echo "=== ALL ROUTE FILES SCOPED ==="
```

## Out of Scope

- Phase 1 handlers (agents.rs, config.rs) — already scoped
- Database schema changes — see SPEC-MT-003
- Vector store account namespacing — see ADR-MT-004 Phase 3
- WebSocket channel scoping — Phase 4
