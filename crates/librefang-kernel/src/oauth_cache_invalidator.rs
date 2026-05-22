//! Trait for flushing OAuth/OIDC discovery and JWKS caches at runtime.
//!
//! `librefang-kernel` does not own these caches — they live as
//! module-level `LazyLock`s in `librefang-api::oauth` (the crate that
//! actually performs OIDC discovery and JWT validation). When
//! `[external_auth]` is hot-reloaded with a different identity provider
//! (different `issuer_url` / `jwks_uri`), the kernel cannot reach the
//! cache directly without inverting the crate dependency graph.
//!
//! The API layer therefore implements this trait and injects it into
//! the kernel after boot via
//! [`crate::LibreFangKernel::set_oauth_cache_invalidator`]. The
//! hot-reload path
//! ([`crate::config_reload::HotAction::ReloadExternalAuth`]) then calls
//! [`OauthCacheInvalidator::invalidate`] to drop the cached OIDC
//! discovery document and JWKS keysets. Without this, tokens issued by
//! the newly configured IdP get validated against the previous IdP's
//! JWKS until natural TTL (1h) → 401 / silent denial.
//!
//! Mirrors the [`crate::log_reload::LogLevelReloader`] pattern: the
//! kernel exposes a `OnceLock` slot; binaries that own the relevant
//! global state register their implementation once at startup.

use std::sync::Arc;

/// Drop all in-process OAuth/OIDC caches.
///
/// Called by the hot-reload pipeline when `[external_auth]` changes in
/// a way that affects IdP identity (issuer URL, JWKS URI, providers
/// list). Implementations must clear:
///   - the OIDC discovery document cache (keyed by issuer URL), and
///   - the JWKS keyset cache (keyed by JWKS URI).
///
/// The method is intentionally non-async: callers fire it from inside
/// `apply_hot_actions_inner`, which runs synchronously under the
/// config-reload write lock. Implementations should `block_in_place` or
/// otherwise avoid awaiting external I/O — invalidation is a memory
/// drop, not a network call.
pub trait OauthCacheInvalidator: Send + Sync {
    /// Clear all cached OIDC discovery + JWKS entries. Idempotent.
    fn invalidate(&self);
}

pub type OauthCacheInvalidatorArc = Arc<dyn OauthCacheInvalidator>;
