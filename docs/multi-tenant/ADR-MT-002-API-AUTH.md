# ADR-MT-002: API Authentication & Account Resolution

**Status:** Partially Implemented (Phase 1 shipped, Phase 2 in progress)
**Date:** 2026-04-06 (updated 2026-04-07)
**Author:** Engineering
**Related:** ADR-MT-001 (Account Model), SPEC-MT-001 (Account Data Model)
**Epic:** Multi-Tenant Architecture

---

## Problem Statement

librefang currently authenticates requests via bearer token + dashboard sessions
(`middleware.rs` auth middleware). There is no concept of "which account is making
this request." Every authenticated request has identical access to all agents,
channels, config, and memory.

For multi-tenant SaaS via Qwntik, we need:
1. Account identification — which tenant is calling?
2. Signature verification — is the account header authentic?
3. Backward compatibility — desktop/CLI mode works without account headers

## Blast Radius Scan

```bash
# Current auth middleware stack (server.rs lines 1056-1088):
$ grep -c "from_fn\|from_fn_with_state" crates/librefang-api/src/server.rs
# 7 middleware layers: auth, accept_language, oidc, rate_limit,
#   api_version_headers, security_headers, request_logging

# Current AuthState:
$ grep -A5 "pub struct AuthState" crates/librefang-api/src/middleware.rs
# api_key_lock: Arc<RwLock<String>>
# active_sessions: Arc<RwLock<HashMap<String, SessionToken>>>

# Files that will need account context:
$ grep -c "pub async fn" crates/librefang-api/src/routes/*.rs | sort -t: -k2 -rn
# 317 total handlers need account context eventually
# Phase 1: 76 handlers (agents.rs:50, channels.rs:11, config.rs:15)

# Existing extractors in handlers:
# State(state): State<Arc<AppState>> — every handler
# Path(id): Path<String> — ID-based handlers
# Extension<RequestLanguage> — i18n handlers
# No custom AccountId extractor exists
```

**Scope decision:** Add AccountId extraction + HMAC verification as new middleware
layers. Do NOT modify existing auth flow — stack on top of it.

## Decision

### Three-layer account resolution (matches openfang-ai ADR-026)

```
Request arrives
    │
    ▼
Layer 1: Existing auth (bearer token / session)  ← unchanged
    │
    ▼
Layer 2: AccountId extraction (X-Account-Id header)  ← NEW
    │  - Header present + non-empty → AccountId(Some(value))
    │  - Header absent/empty → AccountId(None) [legacy mode]
    │  - Infallible — never rejects
    │
    ▼
Layer 3: HMAC signature verification  ← NEW
    │  - If HMAC_SECRET env set AND X-Account-Id present:
    │    verify X-Account-Sig = HMAC-SHA256(secret, account_id)
    │  - If HMAC_SECRET not set: skip (dev mode)
    │  - If sig invalid: 401 Unauthorized
    │  - Constant-time comparison (verify_slice, not ==)
    │
    ▼
Handler receives AccountId via Axum FromRequestParts
```

### Why infallible extractor (not middleware rejection)

The `AccountId` extractor returns `AccountId(None)` when no header is present,
rather than rejecting the request. This means:

1. **Desktop/CLI mode works** — no X-Account-Id header → legacy behavior
2. **Handlers decide policy** — some handlers require accounts (`validate_account!`),
   others allow system access (`account_or_system!`)
3. **No middleware ordering bugs** — extractor is a simple `FromRequestParts`, not
   a middleware that can short-circuit before auth

### Why HMAC not JWT (Phase 1)

| Factor | HMAC | JWT |
|--------|------|-----|
| Complexity | Shared secret, 3 lines of code | Key management, rotation, verification |
| Qwntik integration | Qwntik signs header, librefang verifies | Need token issuer service |
| Revocation | N/A (stateless header per request) | Need revocation list or short expiry |
| Phase 1 scope | Minimal viable auth | Over-engineered for internal service-to-service |

JWT is the right choice for Phase 4 (external API keys, customer-facing auth).
HMAC is the right choice for Phase 1 (Qwntik → librefang service-to-service).

## Pattern Definition

Every request that carries account context MUST follow this pattern:

```
X-Account-Id: <opaque-string>
X-Account-Sig: <hex-encoded-hmac-sha256>
```

The signature is computed as:
```
HMAC-SHA256(HMAC_SECRET, account_id_bytes)
```

Verification:
```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn verify_signature(secret: &[u8], account_id: &str, signature_hex: &str) -> bool {
    let Ok(sig_bytes) = hex::decode(signature_hex) else { return false };
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).unwrap();
    mac.update(account_id.as_bytes());
    mac.verify_slice(&sig_bytes).is_ok()  // constant-time
}
```

## Implementation Scope

| Component | File | Change |
|-----------|------|--------|
| AccountId extractor | `crates/librefang-api/src/extractors.rs` (NEW) | `impl FromRequestParts` — infallible, reads X-Account-Id header |
| HMAC verification | `crates/librefang-api/src/middleware.rs` | New `hmac_verify` middleware function |
| HMAC secret in state | `crates/librefang-api/src/routes/mod.rs` | Add `account_hmac_secret: Option<String>` to `AppState` |
| Server wiring | `crates/librefang-api/src/server.rs` | Add HMAC middleware layer after auth |
| validate_account! macro | `crates/librefang-api/src/macros.rs` (NEW) | Returns 400 if AccountId(None) |
| account_or_system! macro | `crates/librefang-api/src/macros.rs` (NEW) | Defaults to "system" |
| Config | `crates/librefang-types/src/config/types.rs` | Add `hmac_secret: Option<String>` to API config |
| Env var | `.env.example` | Add `LIBREFANG_HMAC_SECRET` |

## Verification Gate

```bash
# Gate: HMAC middleware exists and is wired
grep -q "hmac_verify\|hmac_auth" crates/librefang-api/src/middleware.rs
grep -q "account_hmac_secret" crates/librefang-api/src/routes/mod.rs
grep -q "AccountId" crates/librefang-api/src/extractors.rs

# Gate: all 18 auth tests pass
cargo test -p librefang-api -- account
cargo test -p librefang-api -- hmac
```

## Alternatives Considered

### Alt 1: JWT from the start
**Rejected.** Over-engineered for Phase 1. Qwntik is the only client that sends
account headers. JWT adds key management complexity with no benefit for
service-to-service auth. Reconsidered in Phase 4.

### Alt 2: Account resolution in middleware (not extractor)
**Rejected.** Middleware that rejects requests without X-Account-Id would break
desktop/CLI mode. Infallible extractor + handler-level macros gives per-route
control over account requirements.

### Alt 3: Account in bearer token payload
**Rejected.** Would require changing the existing auth system. Stacking a new
header is additive — zero changes to existing auth flow.

## ⚠️ HMAC Replay Protection — Status Update (2026-04-07)

### What shipped (Phase 1, commit `7637f863`)

The replay-protected HMAC is **live** in `account_sig_check` middleware. The
signature now binds to request context:

```
X-Account-Id: acc_abc123
X-Account-Timestamp: 1712444400          # Unix seconds
X-Account-Sig: hmac-sha256(secret, account_id + "|" + method + "|" + path + "|" + timestamp)
```

- Timestamp tolerance: ±5 minutes (configurable via `replay_window_secs`)
- Method + path binding prevents cross-endpoint replay
- This is materially stronger than openfang-ai's simple `HMAC(secret, account_id)`

### What is still pending

1. **Nonce / replay cache:** No server-side deduplication within the time window.
   A captured request can still be replayed within the ±5-minute window to the
   same endpoint. Add an in-memory LRU nonce cache (~10K entries) to close this.

2. **Legacy HMAC fallback (`ValidLegacy`):** The `account_sig_check` middleware
   still accepts the simple `HMAC(secret, account_id)` format (no timestamp) for
   backward compatibility with existing Qwntik deployments. This path is
   indefinitely replayable.

   **Sunset deadline: Phase 2 completion (before customer-facing deployment).**
   - Add `X-Deprecation-Warning` response header when legacy sig is accepted
   - Log legacy sig usage at WARN level with request metadata
   - Remove `ValidLegacy` acceptance path by end of Phase 2

3. **Header contract documentation:** Update client examples and Qwntik
   integration guide to use the new 3-header format (Id + Timestamp + Sig).

### Original Phase 1 risk assessment (preserved for context)

**Original risk:** The Phase 1 HMAC signature was computed over `account_id` alone —
static and trivially replayable. This was accepted for Phase 1 because:
1. Server-to-server only (never exposed to browsers)
2. Qwntik computes signatures on every request
3. Intercepting server-to-server traffic implies deeper access
4. Internal deployment only

**Current status:** Partially mitigated by timestamp+method+path binding. Full
mitigation requires nonce cache and legacy fallback removal.

| Risk | Impact | Likelihood | Current Status | Remaining Work |
|------|--------|------------|----------------|----------------|
| HMAC replay (same endpoint, within window) | Low — 5-min window, same method+path | Low | Timestamp binding shipped | Add nonce cache |
| HMAC replay (cross-endpoint) | Medium | N/A | **Eliminated** — method+path in sig | Done |
| Legacy HMAC replay (indefinite) | Medium | Low (server-to-server) | `ValidLegacy` still accepted | Sunset by Phase 2 end |

---

## Consequences

### Positive
- Desktop/CLI mode unchanged — no `X-Account-Id` header = `AccountId(None)` = legacy behavior
- Qwntik gets verified account identity with minimal implementation complexity
- HMAC is simple to implement (3 lines of code) and test (5 policy matrix tests)
- Infallible extractor prevents middleware ordering bugs

### Negative
- HMAC secret must be shared between Qwntik and librefang (acceptable for internal services)
- Legacy HMAC fallback (`ValidLegacy`) still accepted — sunset by Phase 2 end
- `AccountId(None)` sees all data by default (mitigated: `require_account_id` middleware now rejects missing header in multi-tenant mode)

### Phase 2 Remaining Remediation
- ~~Add timestamp + nonce to HMAC signature (eliminates replay)~~ → **Timestamp+method+path shipped.** Nonce cache still pending.
- ~~Add `require_account_header` config toggle (closes system-sees-all bypass)~~ → **Shipped** in `require_account_id` middleware (`14e00fef`)
- Sunset `ValidLegacy` HMAC acceptance path (new — see Known Risk section)
- Document the 3-header contract (`X-Account-Id`, `X-Account-Timestamp`, `X-Account-Sig`)

### Phase 4 Debt
- JWT for external API keys, token rotation, key management
- HMAC is an internal implementation detail that Phase 4 wraps in a proper auth service
- Per-account API key management for direct daemon access without Qwntik
