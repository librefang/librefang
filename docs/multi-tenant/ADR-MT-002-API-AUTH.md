# ADR-MT-002: API Authentication & Account Resolution

**Status:** Proposed
**Date:** 2026-04-06
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

## ⚠️ Known Risk: HMAC Replay Vulnerability (Accepted — Phase 1)

**Risk:** The Phase 1 HMAC signature is computed over `account_id` alone:
```
HMAC-SHA256(secret, account_id)
```
This signature is **static** — the same account always produces the same signature.
An attacker who captures one valid `X-Account-Id` + `X-Account-Sig` pair can replay
it indefinitely.

**Why accepted for Phase 1:**
1. The HMAC secret is shared between Qwntik server and LibreFang daemon — both are
   server-side, never exposed to browsers or clients
2. Qwntik's server actions compute the signature on every request — it never leaves
   the server-to-server boundary
3. An attacker who can intercept server-to-server traffic already has deeper access
   than HMAC replay provides
4. Phase 1 scope is internal deployment only — not customer-facing

**Phase 2 remediation plan:**
```
X-Account-Id: acc_abc123
X-Account-Timestamp: 1712444400          # Unix seconds
X-Account-Nonce: a1b2c3d4e5f6            # Random hex, 12+ bytes
X-Account-Sig: hmac-sha256(secret, account_id + "|" + timestamp + "|" + nonce)
```
- Timestamp tolerance: ±5 minutes (reject stale requests)
- Nonce: deduplicate within the tolerance window (in-memory LRU cache, ~10K entries)
- This eliminates replay without requiring JWT infrastructure

| Risk | Impact | Likelihood | Phase 1 Mitigation | Phase 2 Fix |
|------|--------|------------|--------------------|-----------|
| HMAC replay | Medium — attacker impersonates account | Low — requires server-to-server interception | Server-side only, not client-facing | Timestamp + nonce in signature |

---

## Consequences

### Positive
- Desktop/CLI mode unchanged — no `X-Account-Id` header = `AccountId(None)` = legacy behavior
- Qwntik gets verified account identity with minimal implementation complexity
- HMAC is simple to implement (3 lines of code) and test (5 policy matrix tests)
- Infallible extractor prevents middleware ordering bugs

### Negative
- HMAC secret must be shared between Qwntik and librefang (acceptable for internal services)
- Phase 1 HMAC has no replay protection (accepted risk — see Known Risk section above)
- `AccountId(None)` sees all data by default (system-sees-all — toggle added in Phase 2)

### Phase 2 Remediation
- Add timestamp + nonce to HMAC signature (eliminates replay)
- Add `require_account_header` config toggle (closes system-sees-all bypass)

### Phase 4 Debt
- JWT for external API keys, token rotation, key management
- HMAC is an internal implementation detail that Phase 4 wraps in a proper auth service
- Per-account API key management for direct daemon access without Qwntik
