# SPEC-MT-001: Account Data Model & Storage — Phase 1

**ADR:** ADR-MT-001 (Account Model)
**Date:** 2026-04-06 (v2: post-BHR review)
**Author:** Engineering

---

## Purpose

Define the exact Rust types, Axum extractors, guard functions, validation macros,
SQLite migration, and test suite for introducing `AccountId` into librefang. Phase 1
covers the foundation layer (types + middleware + extractors + macros + guards) and
the first 76 handlers (agents, channels, config).

## Source of Truth: openfang-ai Reference Implementation

The openfang-ai codebase completed Phase 1+2 of an identical multi-tenant conversion.
All patterns, macros, guards, and test structures in this SPEC are adapted from the
verified openfang-ai implementation (ADR-025, ADR-026, ADR-027).

### Proven Patterns to Port

| Component | openfang-ai File | Pattern | Adaptation for librefang |
|-----------|-----------------|---------|------------------------|
| AccountId extractor | `openfang-api/src/middleware.rs:347-372` | `FromRequestParts`, infallible, `X-Account-Id` header | Port directly — `AccountId(Option<String>)` |
| check_account guard | `openfang-api/src/routes/shared.rs` | Returns 404 (not 403), generic error body | Port directly — same `Option<String>` type |
| validate_account! macro | `openfang-api/src/macros.rs` | Returns 400 if `AccountId(None)` | Port directly |
| account_or_system! macro | `openfang-api/src/macros.rs` | Defaults to `"system"` | Port directly — keep `"system"` string |
| HMAC sig verification | `openfang-api/src/middleware.rs` | `HMAC-SHA256(secret, account_id)` constant-time | Port directly |
| Handler pattern | `openfang-api/src/routes/agents.rs` (30+ handlers) | `account: AccountId` 2nd param, `check_account()` before op | Same pattern, 317 handlers |
| 33 account tests | Multiple test files | Extraction, HMAC, ownership guard, info disclosure | Port all |

### openfang-ai Key Design Decisions (proven correct)

| Decision | Rationale | Evidence |
|----------|-----------|---------|
| `Option<String>` everywhere | ONE type across extractor, storage, migration, comparison — no conversion bugs | Zero breaking changes in production |
| 404 not 403 on cross-tenant | Prevents existence leaking | `check_account()` returns generic "Agent not found" |
| Infallible extractor | Never panics, never rejects request | 6 extraction tests prove edge cases |
| Two macros (validate vs or_system) | Separates "must have" from "default" | Used 50+ times across routes |
| HMAC constant-time compare | Prevents timing attacks | `verify_slice()` not `==` |
| Error body never leaks account | Prevents fishing for valid account IDs | `test_error_body_is_generic` |

---

## Scope (from ADR-MT-001 Blast Radius Scan)

### Phase 1 Files to Create

| File | Purpose |
|------|---------|
| `crates/librefang-types/src/account.rs` | `AccountId`, `Account`, `AccountStatus` types |
| `crates/librefang-api/src/extractors.rs` | `impl FromRequestParts for AccountId` |
| `crates/librefang-api/src/macros.rs` | `validate_account!`, `account_or_system!` |
| `crates/librefang-api/src/routes/shared.rs` | `check_account()` guard function |
| `crates/librefang-api/tests/account_tests.rs` | All 33 account-related tests |

### Phase 1 Files to Modify

| File | Handlers to Change | Type |
|------|--------------------|------|
| `crates/librefang-types/src/agent.rs` | — | Add `account_id: Option<String>` to `AgentEntry` |
| `crates/librefang-types/src/lib.rs` | — | Add `pub mod account;` |
| `crates/librefang-api/src/middleware.rs` | — | Add `AccountId` header extraction + HMAC sig guard |
| `crates/librefang-api/src/server.rs` | — | Wire extractors + HMAC middleware into router |
| `crates/librefang-kernel/src/registry.rs` | — | Add `account_id` filter to `list()`, `get()` |
| `crates/librefang-kernel/src/kernel.rs` | — | Thread `account_id` through `spawn_agent()`, `list_agents()` |
| `crates/librefang-memory/src/migration.rs` | — | v18: `ALTER TABLE agents ADD COLUMN account_id TEXT DEFAULT 'system'` |
| `crates/librefang-api/src/routes/agents.rs` | 50 handlers | Add `account: AccountId`, `check_account()` guard |
| `crates/librefang-api/src/routes/channels.rs` | 11 handlers | Add `account: AccountId` extractor (full scoping Phase 4) |
| `crates/librefang-api/src/routes/config.rs` | 15 handlers | Add `account: AccountId`, `account_or_system!` for reads |

**Total Phase 1: 5 new files + 10 modified files + 76 handlers + 33 tests**

### New Cargo Dependencies (Phase 1)

```toml
# crates/librefang-api/Cargo.toml — add:
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
```

---

## Exact Type Definitions

### AccountId — `Option<String>` everywhere (matches openfang-ai)

```rust
// crates/librefang-types/src/account.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Tenant isolation boundary. Every resource belongs to exactly one account.
///
/// Uses `Option<String>` — NOT `Option<Uuid>` — matching openfang-ai's proven pattern.
/// This keeps a single representation across extractor, storage, migration, and comparison.
///
/// - `AccountId(Some("uuid-string"))` = multi-tenant request (SaaS, team isolation)
/// - `AccountId(None)` = legacy/desktop mode (admin, sees everything)
///
/// The string is opaque to the type system. Callers may use UUIDs, slugs, or any
/// format — the only invariant is: trimmed, non-empty, case-sensitive equality.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountId(pub Option<String>);

impl AccountId {
    /// The implicit account for single-tenant / backward-compatible deployments.
    /// Matches the migration DEFAULT 'system' exactly.
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
    pub id: String,         // matches AccountId inner type
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
```

### AgentEntry Modification

```rust
// crates/librefang-types/src/agent.rs — ADD field to existing struct

pub struct AgentEntry {
    pub id: AgentId,
    pub account_id: Option<String>,  // NEW: None = legacy/system, Some("uuid") = tenant
    pub name: String,
    pub manifest: AgentManifest,
    pub state: AgentState,
    pub mode: AgentMode,
    // ... existing fields unchanged
}
```

**Why `Option<String>` everywhere (extractor AND storage):**
- Matches openfang-ai's proven pattern exactly — zero type conversion bugs
- SQLite stores as TEXT — no serialization boundary
- `check_account()` compares `&str == &str` — no UUID parsing on every request
- Migration DEFAULT `'system'` matches `AccountId::SYSTEM` exactly
- ONE representation across all layers: header → extractor → guard → storage → migration

---

## Axum Extractor (port from openfang-ai middleware.rs:347-372)

```rust
// crates/librefang-api/src/extractors.rs

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use librefang_types::account::AccountId;

/// Extracts AccountId from the request. Resolution chain:
/// 1. X-Account-Id header (opaque string, trimmed)
/// 2. JWT claim (Phase 2 — if present in Extensions from auth middleware)
/// 3. Default: AccountId(None) — legacy/desktop mode
///
/// INFALLIBLE: never rejects a request. Returns AccountId(None) at worst.
/// This matches openfang-ai's proven behavior (6 tests confirm edge cases).
impl<S: Send + Sync> FromRequestParts<S> for AccountId {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        // 1. Try X-Account-Id header
        if let Some(header) = parts.headers.get("x-account-id") {
            if let Ok(s) = header.to_str() {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    return Ok(AccountId(Some(trimmed.to_owned())));
                }
            }
        }

        // 2. Try JWT claim from auth middleware (set via Extensions)
        // Phase 2: check parts.extensions.get::<JwtClaims>()

        // 3. Default: unscoped (legacy/desktop)
        Ok(AccountId(None))
    }
}
```

---

## Validation Macros (port from openfang-ai macros.rs)

```rust
// crates/librefang-api/src/macros.rs

/// REQUIRE a concrete account ID. Returns the inner &str or short-circuits
/// with 400. Use for mutations and data access that MUST be scoped.
///
/// Port of openfang-ai validate_account! — identical semantics.
#[macro_export]
macro_rules! validate_account {
    ($account:expr) => {
        match $account.0 {
            Some(ref s) => s.as_str(),
            None => {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    axum::Json(serde_json::json!({
                        "error": "X-Account-Id header is required"
                    })),
                ).into_response();
            }
        }
    };
}

/// DEFAULT to "system" if AccountId(None).
/// Use for info-only endpoints (templates, health) that don't leak tenant data.
///
/// Port of openfang-ai account_or_system! — identical semantics.
#[macro_export]
macro_rules! account_or_system {
    ($account:expr) => {
        $account.as_str_or_system()
    };
}
```

---

## Guard Function (port from openfang-ai shared.rs)

```rust
// crates/librefang-api/src/routes/shared.rs

use axum::http::StatusCode;
use axum::Json;
use librefang_types::account::AccountId;
use librefang_types::agent::AgentEntry;

/// Check if the requesting account owns the agent.
///
/// Returns 404 (NOT 403) on cross-tenant access to prevent existence leaking.
/// Returns Ok(()) for AccountId(None) — legacy/desktop admin sees everything.
///
/// Ported from openfang-ai shared.rs with identical security semantics.
///
/// Security properties (each backed by a test):
/// - Matching owner → Ok
/// - Mismatching owner → 404 (not 403)
/// - No header (admin) → Ok (sees everything)
/// - Scoped request vs unowned agent → 404 (prevents info disclosure)
/// - Error body never contains real account_id
pub fn check_account(
    entry: &AgentEntry,
    account: &AccountId,
) -> Result<(), (StatusCode, Json<serde_json::Value>)> {
    if let Some(ref request_account) = account.0 {
        let owns = entry
            .account_id
            .as_deref()
            .map(|a| a == request_account.as_str())
            .unwrap_or(false);
        if !owns {
            return Err((
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": "Agent not found"})),
                //                       ^^^ Generic message — never leak real account_id
            ));
        }
    }
    // AccountId(None) = admin/legacy → sees everything (backward compat)
    Ok(())
}
```

---

## HMAC Signature Verification (port from openfang-ai middleware.rs)

```rust
// crates/librefang-api/src/middleware.rs (add to existing)

use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verify HMAC-SHA256 signature: sig = HMAC(secret, account_id).
/// Uses constant-time comparison to prevent timing attacks.
///
/// Ported from openfang-ai middleware.rs verify_account_sig().
pub fn verify_account_sig(secret: &str, account_id: &str, sig_hex: &str) -> bool {
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(account_id.as_bytes());
    mac.verify_slice(&sig_bytes).is_ok() // constant-time comparison
}

/// Policy matrix for account signature enforcement.
/// 5 cases (each backed by a test):
///
/// | secret  | account_id | sig     | Result                          |
/// |---------|------------|---------|--------------------------------|
/// | absent  | any        | any     | None (pass — no secret configured) |
/// | any     | absent     | any     | None (pass — no account to verify) |
/// | present | present    | absent  | Some("Missing X-Account-Sig")  |
/// | present | present    | invalid | Some("Invalid account signature") |
/// | present | present    | valid   | None (pass)                    |
pub fn account_sig_policy(
    secret: Option<&str>,
    account_id: Option<&str>,
    sig: Option<&str>,
) -> Option<&'static str> {
    let secret = match secret {
        Some(s) if !s.is_empty() => s,
        _ => return None, // No secret configured → pass
    };
    let account_id = match account_id {
        Some(a) if !a.is_empty() => a,
        _ => return None, // No account_id → pass (legacy/desktop)
    };
    match sig {
        None | Some("") => Some("Missing X-Account-Sig header"),
        Some(s) => {
            if verify_account_sig(secret, account_id, s) {
                None // Valid signature → pass
            } else {
                Some("Invalid account signature")
            }
        }
    }
}
```

---

## SQLite Migration (v18)

```rust
// crates/librefang-memory/src/migration.rs — ADD new version

// Migration v18: Add account_id to Phase 1 tables
fn migrate_v18(conn: &rusqlite::Connection) -> rusqlite::Result<()> {
    // Agents table
    conn.execute_batch("
        ALTER TABLE agents ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
        CREATE INDEX idx_agents_account ON agents(account_id);
        CREATE INDEX idx_agents_account_id ON agents(account_id, id);
    ")?;

    // Sessions table
    conn.execute_batch("
        ALTER TABLE sessions ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
        CREATE INDEX idx_sessions_account ON sessions(account_id);
        CREATE INDEX idx_sessions_account_agent ON sessions(account_id, agent_id);
    ")?;

    // Usage events table
    conn.execute_batch("
        ALTER TABLE usage_events ADD COLUMN account_id TEXT NOT NULL DEFAULT 'system';
        CREATE INDEX idx_usage_account ON usage_events(account_id);
    ")?;

    Ok(())
}
```

**Why `DEFAULT 'system'`:** Existing rows become "system" account — no data loss,
backward compatible, matches `AccountId::SYSTEM` string constant exactly. SQLite ALTER TABLE
ADD COLUMN with DEFAULT is instantaneous (no table rewrite).

### Migration Rollback Strategy

SQLite ≥ 3.35.0 supports `ALTER TABLE DROP COLUMN`. For older versions:

```sql
-- Rollback v18: recreate tables without account_id
BEGIN;
CREATE TABLE agents_backup AS SELECT id, name, manifest, state, created_at, updated_at FROM agents;
DROP TABLE agents;
ALTER TABLE agents_backup RENAME TO agents;
-- Repeat for sessions, usage_events (preserve all original columns)
COMMIT;
```

Test the rollback script BEFORE running the forward migration in production.

---

## Acceptance Criteria

### Group 1: Type Definitions (4 criteria)

#### AC-1.1: AccountId Type Exists
- **Given:** `crates/librefang-types/src/account.rs` created
- **When:** `cargo check -p librefang-types`
- **Then:** Compiles with `AccountId`, `Account`, `AccountStatus` types
- **And NOT:** No compilation errors, no circular dependencies

#### AC-1.2: AccountId SYSTEM Constant Matches Migration
- **Given:** `AccountId::SYSTEM` defined as `"system"`
- **When:** `AccountId(None).as_str_or_system()`
- **Then:** Returns `"system"` — matching migration DEFAULT exactly
- **And NOT:** Does not return a UUID string, empty string, or anything other than `"system"`

#### AC-1.3: AgentEntry Has account_id Field
- **Given:** `AgentEntry` struct in `agent.rs`
- **When:** Construct `AgentEntry` with `account_id: Some("user-1".into())`
- **Then:** Field accessible, serializable, defaults to `None`
- **And NOT:** No breaking change to existing `AgentEntry` construction (Option means backward compat)

#### AC-1.4: Account Module Exported
- **Given:** `pub mod account;` in `lib.rs`
- **When:** `use librefang_types::account::AccountId;` from another crate
- **Then:** Resolves correctly
- **And NOT:** No visibility issues

### Group 2: Extractor & Middleware (6 criteria)

#### AC-2.1: Extractor Parses Valid Header
- **Given:** HTTP request with `X-Account-Id: user-abc-123`
- **When:** `AccountId::from_request_parts()`
- **Then:** Returns `AccountId(Some("user-abc-123".to_string()))`
- **And NOT:** Does not return `None` for non-empty header

#### AC-2.2: Extractor Returns None for Missing Header
- **Given:** HTTP request without `X-Account-Id` header
- **When:** `AccountId::from_request_parts()`
- **Then:** Returns `AccountId(None)`
- **And NOT:** Does not panic, does not reject request

#### AC-2.3: Extractor Returns None for Empty/Whitespace Header
- **Given:** HTTP request with `X-Account-Id: "   "` or `X-Account-Id: ""`
- **When:** `AccountId::from_request_parts()`
- **Then:** Returns `AccountId(None)`
- **And NOT:** Does not return `Some("")` or `Some("   ")`

#### AC-2.4: Extractor Is Infallible
- **Given:** Any possible HTTP request (malformed headers, garbage data)
- **When:** `AccountId::from_request_parts()`
- **Then:** Always returns `Ok(...)` — never `Err`
- **And NOT:** Never panics, never rejects

#### AC-2.5: HMAC Signature Verification Works
- **Given:** Secret `"test-secret"`, account `"acc-123"`, valid HMAC-SHA256 hex signature
- **When:** `verify_account_sig("test-secret", "acc-123", &sig_hex)`
- **Then:** Returns `true`
- **And NOT:** Does not return `true` for wrong secret, wrong account, malformed hex, or empty sig

#### AC-2.6: Signature Policy Matrix Correct
- **Given:** 5 combinations of (secret, account_id, sig) per policy table above
- **When:** `account_sig_policy(secret, account_id, sig)`
- **Then:** Returns correct `Option<&str>` for each row
- **And NOT:** No false passes (signature present but invalid → must return error)

### Group 3: Guard & Macros (5 criteria)

#### AC-3.1: check_account Allows Matching Owner
- **Given:** `AgentEntry { account_id: Some("user-1") }`, `AccountId(Some("user-1"))`
- **When:** `check_account(&entry, &account)`
- **Then:** Returns `Ok(())`
- **And NOT:** Does not return 404 for matching owner

#### AC-3.2: check_account Returns 404 for Mismatching Owner
- **Given:** `AgentEntry { account_id: Some("user-1") }`, `AccountId(Some("user-2"))`
- **When:** `check_account(&entry, &account)`
- **Then:** Returns `Err((404, "Agent not found"))`
- **And NOT:** Does NOT return 403 (prevents existence leaking)

#### AC-3.3: check_account Allows Admin (None) to See Everything
- **Given:** `AgentEntry { account_id: Some("user-1") }`, `AccountId(None)`
- **When:** `check_account(&entry, &account)`
- **Then:** Returns `Ok(())`
- **And NOT:** Does not return 404 for admin/legacy mode

#### AC-3.4: check_account Hides Unowned Agents from Scoped Requests
- **Given:** `AgentEntry { account_id: None }`, `AccountId(Some("user-1"))`
- **When:** `check_account(&entry, &account)`
- **Then:** Returns `Err((404, ...))`
- **And NOT:** Scoped tenant must NOT see legacy/system agents with no owner

#### AC-3.5: check_account Error Body Never Leaks Real Account
- **Given:** `AgentEntry { account_id: Some("secret-owner") }`, `AccountId(Some("attacker"))`
- **When:** `check_account(&entry, &account)` returns error
- **Then:** Error JSON body contains `"Agent not found"` only
- **And NOT:** Body does NOT contain `"secret-owner"` string (information disclosure)

### Group 4: Database Migration (4 criteria)

#### AC-4.1: Migration Adds account_id to agents Table
- **Given:** Existing SQLite database at v17
- **When:** Run v18 migration
- **Then:** `agents` table has `account_id TEXT NOT NULL DEFAULT 'system'` column
- **And NOT:** No existing rows lost, no table rewrite

#### AC-4.2: Migration Adds Compound Indexes
- **Given:** v18 migration complete
- **When:** `EXPLAIN QUERY PLAN SELECT * FROM agents WHERE account_id = ? AND id = ?`
- **Then:** Uses `idx_agents_account_id` index
- **And NOT:** No full table scan

#### AC-4.3: Existing Data Defaults to 'system'
- **Given:** 10 existing agents in database before migration
- **When:** Run v18 migration, then `SELECT account_id FROM agents`
- **Then:** All 10 rows have `account_id = 'system'`
- **And NOT:** No NULLs, no empty strings

#### AC-4.4: Migration Covers All Phase 1 Tables
- **Given:** v18 migration script
- **When:** Inspect ALTER TABLE statements
- **Then:** Covers `agents`, `sessions`, `usage_events` (3 tables)
- **And NOT:** Does NOT touch Phase 2/3 tables (kv_store, memories, entities, etc.)

### Group 5: Handler Scoping — agents.rs (5 criteria)

#### AC-5.1: spawn_agent Stores account_id
- **Given:** POST /api/agents with `X-Account-Id: user-1`
- **When:** Agent spawned
- **Then:** `AgentEntry.account_id == Some("user-1")`
- **And NOT:** account_id is not stored with `None` or `"system"` when header present

#### AC-5.2: list_agents Filters by Account
- **Given:** 3 agents: A (user-1), B (user-2), C (system)
- **When:** GET /api/agents with `X-Account-Id: user-1`
- **Then:** Returns only agent A
- **And NOT:** Does NOT return agent B (user-2) or agent C (system/no-owner)

#### AC-5.3: get_agent Returns 404 for Cross-Tenant
- **Given:** Agent A owned by user-1
- **When:** GET /api/agents/{A.id} with `X-Account-Id: user-2`
- **Then:** Returns 404
- **And NOT:** Does NOT return 403, does NOT return agent data

#### AC-5.4: delete_agent Returns 404 for Cross-Tenant
- **Given:** Agent A owned by user-1
- **When:** DELETE /api/agents/{A.id} with `X-Account-Id: user-2`
- **Then:** Returns 404, agent NOT deleted
- **And NOT:** Agent is NOT deleted by cross-tenant request

#### AC-5.5: No Header = Admin Mode (Backward Compat)
- **Given:** 3 agents across different accounts
- **When:** GET /api/agents with NO `X-Account-Id` header
- **Then:** Returns all 3 agents (legacy/admin behavior)
- **And NOT:** Does NOT filter — backward compatibility preserved

### Group 6: Handler Scoping — channels.rs, config.rs (3 criteria)

#### AC-6.1: Channel Handlers Accept AccountId Extractor
- **Given:** Handler signature for GET /api/channels
- **When:** Request with `X-Account-Id: user-1`
- **Then:** `account: AccountId` extractor populates `Some("user-1")`
- **And NOT:** Does NOT crash or ignore the header
- **Note:** Full channel-to-account routing is Phase 4 (50+ adapters need per-adapter scoping). Phase 1 only adds the extractor parameter so the type signature is ready.

#### AC-6.2: Config Reads Default to System
- **Given:** GET /api/config/status with NO header
- **When:** Handler uses `account_or_system!`
- **Then:** Returns system-level config (backward compat)
- **And NOT:** Does NOT return 400 or require header

#### AC-6.3: Config Mutations Require Account
- **Given:** POST /api/config/... with NO `X-Account-Id` header
- **When:** Handler uses `validate_account!`
- **Then:** Returns 400 `{"error": "X-Account-Id header is required"}`
- **And NOT:** Does NOT allow unscoped mutation

---

## Claims Requiring Verification

| # | Claim | Method | Test Name |
|---|-------|--------|-----------|
| C-1 | AccountId extractor parses valid header | Unit test | `test_account_id_present` |
| C-2 | AccountId returns None for missing header | Unit test | `test_account_id_absent` |
| C-3 | AccountId returns None for whitespace | Unit test | `test_account_id_whitespace_only_treated_as_absent` |
| C-4 | AccountId returns None for empty string | Unit test | `test_account_id_empty_string` |
| C-5 | AccountId extraction is infallible | Unit test | `test_account_id_extraction_is_infallible` |
| C-6 | AccountId parses UUID format | Unit test | `test_account_id_uuid_style` |
| C-7 | HMAC verify valid sig | Unit test | `test_verify_account_sig_valid` |
| C-8 | HMAC reject wrong account | Unit test | `test_verify_account_sig_wrong_account_id` |
| C-9 | HMAC reject wrong secret | Unit test | `test_verify_account_sig_wrong_secret` |
| C-10 | HMAC reject malformed hex | Unit test | `test_verify_account_sig_malformed_hex` |
| C-11 | HMAC reject empty sig | Unit test | `test_verify_account_sig_empty_sig` |
| C-12 | Policy: no secret → pass | Unit test | `test_policy_no_secret_passes_through` |
| C-13 | Policy: no account → pass | Unit test | `test_policy_no_account_id_passes_through` |
| C-14 | Policy: sig absent → error | Unit test | `test_policy_sig_absent_returns_error` |
| C-15 | Policy: sig invalid → error | Unit test | `test_policy_sig_invalid_returns_error` |
| C-16 | Policy: valid sig → pass | Unit test | `test_policy_valid_sig_passes_through` |
| C-17 | Guard: matching owner → Ok | Unit test | `test_check_account_matching_owner` |
| C-18 | Guard: mismatching owner → 404 | Unit test | `test_check_account_mismatching_owner_returns_404` |
| C-19 | Guard: no header → admin sees all | Unit test | `test_check_account_no_header_allows_all` |
| C-20 | Guard: no header + unowned → Ok | Unit test | `test_check_account_no_header_allows_unowned` |
| C-21 | Guard: scoped vs unowned → 404 | Unit test | `test_check_account_scoped_request_vs_unowned_agent_returns_404` |
| C-22 | Guard: error body generic | Unit test | `test_check_account_error_body_is_generic` |
| C-23 | Migration adds account_id to agents | Migration test | `test_migration_v18_agents_account_id` |
| C-24 | Migration defaults to 'system' | Migration test | `test_migration_v18_default_system` |
| C-25 | spawn_agent stores account_id | Integration test | `test_spawn_agent_stores_account_id` |
| C-26 | list_agents filters by account | Integration test | `test_list_agents_filters_by_account` |
| C-27 | get_agent returns 404 cross-tenant | Integration test | `test_get_agent_cross_tenant_404` |
| C-28 | delete_agent returns 404 cross-tenant | Integration test | `test_delete_agent_cross_tenant_404` |
| C-29 | No header = admin sees all | Integration test | `test_no_header_admin_sees_all_agents` |
| C-30 | account_or_system! defaults | Unit test | `test_account_or_system_defaults_to_system` |
| C-31 | channels accept AccountId | Integration test | `test_channels_accept_account_id_extractor` |
| C-32 | config reads default to system | Integration test | `test_config_reads_default_to_system` |
| C-33 | config mutations require account | Integration test | `test_config_mutation_requires_account` |

---

## Test Suite (33 tests — ported from openfang-ai)

### File: `crates/librefang-api/tests/account_tests.rs`

```rust
use axum::extract::FromRequestParts;
use axum::http::{Request, StatusCode};
use librefang_types::account::AccountId;
use librefang_types::agent::{AgentEntry, AgentId};
use serde_json::json;

// === Test Helpers ===

/// Build axum Parts with a single header set.
fn test_parts_with_header(name: &str, value: &str) -> axum::http::request::Parts {
    let req = Request::builder().header(name, value).body(()).unwrap();
    let (parts, _) = req.into_parts();
    parts
}

fn test_parts_no_headers() -> axum::http::request::Parts {
    let (parts, _) = Request::builder().body(()).unwrap().into_parts();
    parts
}

fn test_entry(id: &str, account_id: Option<&str>) -> AgentEntry {
    AgentEntry {
        id: AgentId::new(),
        account_id: account_id.map(String::from),
        name: id.to_string(),
        ..Default::default()
    }
}

// === Category 1: AccountId Extraction (6 tests) ===

#[tokio::test]
async fn test_account_id_present() {
    let mut parts = test_parts_with_header("x-account-id", "user-abc-123");
    let account = AccountId::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(account.0, Some("user-abc-123".to_string()));
}

#[tokio::test]
async fn test_account_id_absent() {
    let mut parts = test_parts_no_headers();
    let account = AccountId::from_request_parts(&mut parts, &()).await.unwrap();
    assert!(account.0.is_none());
}

#[tokio::test]
async fn test_account_id_empty_string() {
    let mut parts = test_parts_with_header("x-account-id", "");
    let account = AccountId::from_request_parts(&mut parts, &()).await.unwrap();
    assert!(account.0.is_none());
}

#[tokio::test]
async fn test_account_id_whitespace_only_treated_as_absent() {
    let mut parts = test_parts_with_header("x-account-id", "   ");
    let account = AccountId::from_request_parts(&mut parts, &()).await.unwrap();
    assert!(account.0.is_none());
}

#[tokio::test]
async fn test_account_id_uuid_style() {
    let mut parts = test_parts_with_header("x-account-id", "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
    let account = AccountId::from_request_parts(&mut parts, &()).await.unwrap();
    assert_eq!(account.0, Some("a1b2c3d4-e5f6-7890-abcd-ef1234567890".to_string()));
}

#[tokio::test]
async fn test_account_id_extraction_is_infallible() {
    let mut parts = test_parts_with_header("x-account-id", "literally-anything");
    let result = AccountId::from_request_parts(&mut parts, &()).await;
    assert!(result.is_ok()); // Never Err — infallible
    // Non-empty string → Some (opaque, not validated as UUID)
    assert!(result.unwrap().0.is_some());
}

// === Category 2: HMAC Signature Verification (5 tests) ===

fn valid_sig_for(secret: &str, account_id: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(account_id.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

#[test]
fn test_verify_account_sig_valid() {
    let sig = valid_sig_for("test-secret", "acc-uuid-123");
    assert!(verify_account_sig("test-secret", "acc-uuid-123", &sig));
}

#[test]
fn test_verify_account_sig_wrong_account_id() {
    let sig = valid_sig_for("test-secret", "acc-uuid-123");
    assert!(!verify_account_sig("test-secret", "different-account", &sig));
}

#[test]
fn test_verify_account_sig_wrong_secret() {
    let sig = valid_sig_for("correct-secret", "acc-uuid-123");
    assert!(!verify_account_sig("wrong-secret", "acc-uuid-123", &sig));
}

#[test]
fn test_verify_account_sig_malformed_hex() {
    assert!(!verify_account_sig("secret", "account", "not-valid-hex!"));
}

#[test]
fn test_verify_account_sig_empty_sig() {
    assert!(!verify_account_sig("secret", "account", ""));
}

// === Category 3: Signature Policy Matrix (5 tests) ===

#[test]
fn test_policy_no_secret_passes_through() {
    assert_eq!(account_sig_policy(None, Some("acc-123"), Some("any")), None);
    assert_eq!(account_sig_policy(None, Some("acc-123"), None), None);
    assert_eq!(account_sig_policy(None, None, None), None);
}

#[test]
fn test_policy_no_account_id_passes_through() {
    assert_eq!(account_sig_policy(Some("secret"), None, Some("any")), None);
    assert_eq!(account_sig_policy(Some("secret"), None, None), None);
}

#[test]
fn test_policy_sig_absent_returns_error() {
    assert_eq!(
        account_sig_policy(Some("secret"), Some("acc-123"), None),
        Some("Missing X-Account-Sig header")
    );
}

#[test]
fn test_policy_sig_invalid_returns_error() {
    assert_eq!(
        account_sig_policy(Some("secret"), Some("acc-123"), Some("deadbeef")),
        Some("Invalid account signature")
    );
}

#[test]
fn test_policy_valid_sig_passes_through() {
    let sig = valid_sig_for("my-secret", "acc-uuid-456");
    assert_eq!(
        account_sig_policy(Some("my-secret"), Some("acc-uuid-456"), Some(&sig)),
        None
    );
}

// === Category 4: Ownership Guard (6 tests) ===

#[test]
fn test_check_account_matching_owner() {
    let entry = test_entry("a", Some("user-1"));
    let account = AccountId(Some("user-1".to_string()));
    assert!(check_account(&entry, &account).is_ok());
}

#[test]
fn test_check_account_mismatching_owner_returns_404() {
    let entry = test_entry("a", Some("user-1"));
    let account = AccountId(Some("user-2".to_string()));
    let err = check_account(&entry, &account).unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND); // 404, NOT 403
}

#[test]
fn test_check_account_no_header_allows_all() {
    let entry = test_entry("a", Some("user-1"));
    let account = AccountId(None); // admin/legacy
    assert!(check_account(&entry, &account).is_ok());
}

#[test]
fn test_check_account_no_header_allows_unowned() {
    let entry = test_entry("a", None); // no owner
    let account = AccountId(None);
    assert!(check_account(&entry, &account).is_ok());
}

#[test]
fn test_check_account_scoped_request_vs_unowned_agent_returns_404() {
    let entry = test_entry("a", None); // no owner
    let account = AccountId(Some("user-1".to_string())); // scoped request
    let err = check_account(&entry, &account).unwrap_err();
    assert_eq!(err.0, StatusCode::NOT_FOUND);
}

#[test]
fn test_check_account_error_body_is_generic() {
    let entry = test_entry("a", Some("secret-owner-id"));
    let account = AccountId(Some("attacker".to_string()));
    let err = check_account(&entry, &account).unwrap_err();
    let body = serde_json::to_string(&err.1 .0).unwrap();
    assert!(!body.contains("secret-owner-id")); // MUST NOT leak real account
    assert!(body.contains("Agent not found"));   // Generic message only
}

// === Category 5: Macro & Type Tests (1 test) ===

#[test]
fn test_account_or_system_defaults_to_system() {
    let account = AccountId(None);
    assert_eq!(account.as_str_or_system(), "system");

    let account = AccountId(Some("my-account-id".to_string()));
    assert_eq!(account.as_str_or_system(), "my-account-id");
}

// === Category 6: Migration Tests (2 tests) ===

#[test]
fn test_migration_v18_agents_account_id() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    // Run migrations v1-v17
    run_migrations_up_to(&conn, 17).unwrap();
    // Insert test agents BEFORE v18
    conn.execute(
        "INSERT INTO agents (id, name, manifest, state, created_at, updated_at) \
         VALUES ('a1', 'test', X'00', 'idle', '2026-01-01', '2026-01-01')",
        [],
    ).unwrap();
    // Run v18
    migrate_v18(&conn).unwrap();
    // Verify: account_id column exists with default 'system'
    let account: String = conn.query_row(
        "SELECT account_id FROM agents WHERE id = 'a1'", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(account, "system");
    // Verify: index exists
    let plan: String = conn.query_row(
        "EXPLAIN QUERY PLAN SELECT * FROM agents WHERE account_id = 'x' AND id = 'y'",
        [], |r| r.get(3)
    ).unwrap();
    assert!(plan.contains("idx_agents_account"), "Expected index, got: {}", plan);
}

#[test]
fn test_migration_v18_default_system() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    run_migrations_up_to(&conn, 17).unwrap();
    // Insert 5 agents before v18
    for i in 0..5 {
        conn.execute(
            &format!("INSERT INTO agents (id, name, manifest, state, created_at, updated_at) \
                       VALUES ('a{}', 'agent-{}', X'00', 'idle', '2026-01-01', '2026-01-01')", i, i),
            [],
        ).unwrap();
    }
    migrate_v18(&conn).unwrap();
    // ALL 5 agents must have account_id = 'system'
    let count: i64 = conn.query_row(
        "SELECT count(*) FROM agents WHERE account_id = 'system'", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(count, 5);
    // No NULLs
    let nulls: i64 = conn.query_row(
        "SELECT count(*) FROM agents WHERE account_id IS NULL", [], |r| r.get(0)
    ).unwrap();
    assert_eq!(nulls, 0);
}

// === Category 7: Integration Tests (5 tests) ===
// These use the TestServer harness (same pattern as openfang-ai api_integration_test.rs)

/// Test harness — boots real kernel with temp dir, builds axum router.
struct TestServer {
    client: axum_test::TestClient,
    _tmp: tempfile::TempDir,
}

impl TestServer {
    async fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let kernel = LibreFangKernel::boot_test(tmp.path()).await.unwrap();
        let state = Arc::new(AppState::new(kernel));
        let app = build_router(state);
        Self { client: axum_test::TestClient::new(app), _tmp: tmp }
    }

    async fn post_agent(&self, manifest: &str, account: &str) -> axum_test::TestResponse {
        self.client.post("/api/agents")
            .header("x-account-id", account)
            .json(&json!({"manifest_toml": manifest}))
            .await
    }

    async fn get_agents(&self, account: Option<&str>) -> axum_test::TestResponse {
        let mut req = self.client.get("/api/agents");
        if let Some(a) = account { req = req.header("x-account-id", a); }
        req.await
    }

    async fn get_agent(&self, id: &str, account: &str) -> axum_test::TestResponse {
        self.client.get(&format!("/api/agents/{}", id))
            .header("x-account-id", account)
            .await
    }

    async fn delete_agent(&self, id: &str, account: &str) -> axum_test::TestResponse {
        self.client.delete(&format!("/api/agents/{}", id))
            .header("x-account-id", account)
            .await
    }
}

#[tokio::test]
async fn test_spawn_agent_stores_account_id() {
    let server = TestServer::new().await;
    let resp = server.post_agent(TEST_MANIFEST, "user-1").await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value = resp.json().await;
    let agent_id = body["agent_id"].as_str().unwrap();
    // Verify agent belongs to user-1
    let resp = server.get_agent(agent_id, "user-1").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let agent: serde_json::Value = resp.json().await;
    assert_eq!(agent["account_id"].as_str().unwrap(), "user-1");
}

#[tokio::test]
async fn test_list_agents_filters_by_account() {
    let server = TestServer::new().await;
    // Spawn agent-A as user-1
    let resp_a = server.post_agent(TEST_MANIFEST, "user-1").await;
    assert_eq!(resp_a.status(), StatusCode::CREATED);
    // Spawn agent-B as user-2
    let resp_b = server.post_agent(TEST_MANIFEST, "user-2").await;
    assert_eq!(resp_b.status(), StatusCode::CREATED);
    // List as user-1 → only sees own agents
    let resp = server.get_agents(Some("user-1")).await;
    let agents: Vec<serde_json::Value> = resp.json().await;
    assert!(agents.iter().all(|a| a["account_id"] == "user-1"));
    // List as user-2 → only sees own agents
    let resp = server.get_agents(Some("user-2")).await;
    let agents: Vec<serde_json::Value> = resp.json().await;
    assert!(agents.iter().all(|a| a["account_id"] == "user-2"));
}

#[tokio::test]
async fn test_get_agent_cross_tenant_404() {
    let server = TestServer::new().await;
    let resp = server.post_agent(TEST_MANIFEST, "user-1").await;
    let agent_id = resp.json::<serde_json::Value>().await["agent_id"]
        .as_str().unwrap().to_string();
    // Cross-tenant access → 404 (not 403)
    let resp = server.get_agent(&agent_id, "user-2").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body: serde_json::Value = resp.json().await;
    assert!(body["error"].as_str().unwrap().contains("not found"));
    assert!(!serde_json::to_string(&body).unwrap().contains("user-1")); // no leak
}

#[tokio::test]
async fn test_delete_agent_cross_tenant_404() {
    let server = TestServer::new().await;
    let resp = server.post_agent(TEST_MANIFEST, "user-1").await;
    let agent_id = resp.json::<serde_json::Value>().await["agent_id"]
        .as_str().unwrap().to_string();
    // Cross-tenant delete → 404, agent survives
    let resp = server.delete_agent(&agent_id, "user-2").await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    // Verify agent still exists for owner
    let resp = server.get_agent(&agent_id, "user-1").await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_no_header_admin_sees_all_agents() {
    let server = TestServer::new().await;
    server.post_agent(TEST_MANIFEST, "user-1").await;
    server.post_agent(TEST_MANIFEST, "user-2").await;
    // No X-Account-Id header → admin sees all
    let resp = server.get_agents(None).await;
    let agents: Vec<serde_json::Value> = resp.json().await;
    assert!(agents.len() >= 2); // at least the 2 we spawned
}
```

---

## Storage Dependency Check

| ADR Claims | Actual API | Match? |
|-----------|------------|--------|
| "account_id column in agents table" | SQLite `ALTER TABLE ADD COLUMN` with `DEFAULT` | ✅ Yes — instantaneous, no rewrite |
| "Compound index (account_id, id)" | SQLite `CREATE INDEX` | ✅ Yes — standard B-tree |
| "agent registry filtered by account" | `DashMap<AgentId, AgentEntry>` with `.iter().filter()` | ⚠️ Works but O(n) — acceptable for Phase 1, optimize in Phase 2 |

### Migration Rollback Strategy

SQLite ≥ 3.35.0 supports `ALTER TABLE DROP COLUMN`. For older versions:

```sql
-- Rollback v18: recreate tables without account_id
BEGIN;
CREATE TABLE agents_backup AS SELECT id, name, manifest, state, created_at, updated_at FROM agents;
DROP TABLE agents;
ALTER TABLE agents_backup RENAME TO agents;
-- Repeat for sessions, usage_events (preserve all original columns)
COMMIT;
```

Test the rollback script BEFORE running the forward migration in production.

---

## Integration Wiring

| Trigger | Existing Code | New Behavior |
|---------|--------------|-------------|
| HTTP request arrives | `middleware::auth()` in middleware.rs | Add `AccountId` extraction after auth, before route |
| Agent spawned | `kernel.spawn_agent(manifest)` in kernel.rs | Accept `&AccountId`, store `.as_str_or_system()` in `AgentEntry.account_id` |
| Agent listed | `kernel.list_agents()` in kernel.rs | Accept `&AccountId`, filter registry |
| Agent accessed | `registry.get(id)` in routes/agents.rs | Follow with `check_account(&entry, &account)?` |
| Session created | `session_store.create(agent_id)` in session.rs | Propagate agent's `account_id` to session row |
| Usage recorded | `budget.record(agent_id, ...)` in budget.rs | Include `account_id` from agent's entry |
| Router construction | `build_router()` in server.rs | Add HMAC sig guard middleware layer |

---

## Exit Gate (from ADR-MT-001)

```bash
#!/usr/bin/env bash
set -euo pipefail

ROUTES_DIR="crates/librefang-api/src/routes"

# Gate 1: Types compile
cargo check -p librefang-types

# Gate 2: AccountId extractor exists
grep -q "impl.*FromRequestParts.*for AccountId" crates/librefang-api/src/extractors.rs

# Gate 3: check_account guard exists
grep -q "fn check_account" crates/librefang-api/src/routes/shared.rs

# Gate 4: Macros exist
grep -q "validate_account" crates/librefang-api/src/macros.rs
grep -q "account_or_system" crates/librefang-api/src/macros.rs

# Gate 5: Migration adds account_id
grep -q "account_id" crates/librefang-memory/src/migration.rs

# Gate 6: Phase 1 handlers scoped
for f in agents.rs channels.rs config.rs; do
  TOTAL=$(grep -c "pub async fn" "$ROUTES_DIR/$f" 2>/dev/null || echo 0)
  SCOPED=$(grep -c "account.*AccountId\|AccountId.*account" "$ROUTES_DIR/$f" 2>/dev/null || echo 0)
  [ "$SCOPED" -ge "$TOTAL" ] || { echo "FAIL: $f has unscoped handlers"; exit 1; }
done

# Gate 7: All tests pass
cargo test -p librefang-types -p librefang-api

echo "✅ SPEC-MT-001 Phase 1: ALL PASSED"
```

## Out of Scope

| Item | Deferred To | Reason |
|------|------------|--------|
| Routes: system.rs (63), skills.rs (53), workflows.rs (30) | Phase 2 (SPEC-MT-002) | Phase 1 covers foundation + agents/channels/config only |
| Routes: budget.rs, providers.rs, network.rs, plugins.rs, goals.rs, media.rs, inbox.rs | Phase 2 | Same pattern, lower priority |
| Memory isolation (memories, entities, relations tables) | Phase 3 (ADR-MT-004) | Performance-critical, needs benchmarking |
| Channel bridge account routing (50+ adapters) | Phase 4 | Each adapter has unique tenant semantics |
| JWT claim extraction (Phase 2 of extractor) | Phase 2 | X-Account-Id header sufficient for Phase 1 |
| Per-account config overrides | Phase 2 | Global config works for Phase 1 |
| Account CRUD API endpoints | Phase 2 | Manual account creation acceptable for Phase 1 |
