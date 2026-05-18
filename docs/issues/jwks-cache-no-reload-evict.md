# [Low] JWKS / discovery caches lack eviction on config reload

**Severity:** Low · **Domain:** Performance · **Source:** `audit-05-performance.md`

**Location:** `crates/librefang-api/src/oauth.rs:163-193, 1790-1828`

**Problem:** When `external_auth` config is hot-reloaded with a different IdP, the JWKS + discovery caches still hold the old provider's keys until natural expiry.

**Fix:** Invalidate on `replace_config` for relevant fields.
