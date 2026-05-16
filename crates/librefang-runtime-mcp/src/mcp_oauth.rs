//! MCP OAuth discovery and authentication support.
//!
//! Implements RFC 8414 (OAuth Authorization Server Metadata) discovery
//! for MCP Streamable HTTP connections, with WWW-Authenticate header parsing,
//! PKCE support, and three-tier metadata resolution.

use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use librefang_types::config::McpOAuthConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::{debug, info, warn};
use url::Url;

// Canonical OAuth token type lives in `librefang-types`.  Re-export so existing
// callers can keep their `runtime::mcp_oauth::OAuthTokens` import path.
pub use librefang_types::oauth::OAuthTokens;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// Resolved OAuth metadata for an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthMetadata {
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub client_id: Option<String>,
    /// RFC 7591 Dynamic Client Registration endpoint.
    /// Used to obtain a `client_id` when none is configured.
    #[serde(default)]
    pub registration_endpoint: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Slack-style user scopes. Appended to the authorization URL as
    /// `&user_scope=...` when non-empty.
    #[serde(default)]
    pub user_scopes: Vec<String>,
    pub server_url: String,
}

/// Current authentication state for an MCP connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum McpAuthState {
    NotRequired,
    Authorized {
        #[serde(default)]
        expires_at: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<OAuthTokens>,
    },
    /// Server requires OAuth but the user hasn't started the flow yet.
    /// Set at daemon boot when a 401 is detected.
    NeedsAuth,
    /// OAuth flow is in progress — user clicked Authorize.
    PendingAuth {
        auth_url: String,
    },
    Expired,
    Error {
        message: String,
    },
}

/// Shared map of per-server MCP OAuth authentication states.
pub type McpAuthStates = tokio::sync::Mutex<std::collections::HashMap<String, McpAuthState>>;

// ---------------------------------------------------------------------------
// WWW-Authenticate parsing
// ---------------------------------------------------------------------------

/// Split a parameter string on commas, respecting quoted values.
///
/// For example: `realm="OAuth", error="invalid_token"` splits into
/// `["realm=\"OAuth\"", "error=\"invalid_token\""]`.
fn split_auth_params(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in s.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            ',' if !in_quotes => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    parts.push(trimmed);
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        parts.push(trimmed);
    }
    parts
}

/// Parse a `WWW-Authenticate: Bearer ...` header into key-value pairs.
///
/// Strips the "Bearer " prefix (case-insensitive), splits on commas respecting
/// quoted strings, and parses `key=value` / `key="value"` pairs.
pub fn parse_www_authenticate(header: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let body = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "));
    let body = match body {
        Some(b) => b,
        None => return map,
    };

    for param in split_auth_params(body) {
        if let Some((key, value)) = param.split_once('=') {
            let key = key.trim().to_lowercase();
            let value = value.trim().trim_matches('"').to_string();
            map.insert(key, value);
        }
    }
    map
}

/// Extract the `resource_metadata` URL from parsed WWW-Authenticate parameters.
///
/// Validates with three layered checks before returning:
/// 1. **HTTPS only** — RFC 8414 requires TLS; `http://` is rejected.
/// 2. **Same-origin** — the metadata URL must share scheme+host+port with `server_url`
///    to prevent a rogue MCP server from redirecting OAuth discovery cross-domain.
/// 3. **No loopback / link-local / private IPs** — belt-and-braces defence-in-depth
///    even in the (unlikely) case same-origin passes on a private-range server.
pub fn extract_metadata_url(params: &HashMap<String, String>, server_url: &str) -> Option<String> {
    let url_str = params.get("resource_metadata")?;

    // Layer 1: HTTPS only
    if !url_str.starts_with("https://") {
        return None;
    }

    let metadata_url = Url::parse(url_str).ok()?;
    let server_parsed = Url::parse(server_url).ok()?;

    // Layer 2: Same-origin — compares scheme, host, and port
    if metadata_url.origin() != server_parsed.origin() {
        return None;
    }

    // Layer 3: Block loopback / link-local / private addresses
    let host = metadata_url.host_str()?;
    if is_ssrf_blocked_host(host) {
        return None;
    }

    Some(url_str.clone())
}

/// Return `true` when the given host string is a literal IP address or
/// known internal hostname that must not be reachable via OAuth metadata
/// fetches or MCP transport connections (SSRF defence-in-depth).  No DNS
/// resolution is performed — only the literal value is matched.  A public
/// hostname that DNS-rebinds to an internal IP at fetch time is out of
/// scope; mitigate at the network layer.
///
/// Blocked values:
/// * Exact hostnames:   `localhost`, `ip6-localhost`, `metadata.google.internal`,
///   `metadata.aws.internal`, `instance-data`
/// * IPv4 loopback      127.0.0.0/8
/// * IPv4 unspecified   0.0.0.0
/// * IPv4 private       10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16
/// * IPv4 CGNAT         100.64.0.0/10 (incl. Alibaba Cloud IMDS 100.100.100.200)
/// * IPv4 link-local    169.254.0.0/16 (covers IMDS 169.254.169.254)
/// * IPv4 Azure IMDS    192.0.0.192 (legacy Azure metadata endpoint)
/// * IPv6 loopback      ::1
/// * IPv6 unique-local  fc00::/7
/// * IPv6 link-local    fe80::/10
/// * IPv4-mapped IPv6   `::ffff:x.x.x.x` when the embedded v4 is blocked
/// * NAT64              `64:ff9b::x.x.x.x` when the embedded v4 is blocked
///
/// Shared with the MCP transport SSRF guard (`McpConnection::check_ssrf`)
/// via [`is_ssrf_blocked_url`] so OAuth discovery, token exchange, and the
/// SSE / Streamable-HTTP / HTTP-compat connect paths cannot diverge.
fn is_ssrf_blocked_host(host: &str) -> bool {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    fn blocked_v4(v4: Ipv4Addr) -> bool {
        let o = v4.octets();
        // 0.0.0.0 unspecified — connects to every interface incl. loopback
        (o[0] == 0 && o[1] == 0 && o[2] == 0 && o[3] == 0)
        // 127.0.0.0/8 loopback
        || o[0] == 127
        // 10.0.0.0/8
        || o[0] == 10
        // 172.16.0.0/12
        || (o[0] == 172 && (o[1] & 0xf0) == 16)
        // 192.168.0.0/16
        || (o[0] == 192 && o[1] == 168)
        // 100.64.0.0/10 CGNAT (also Alibaba Cloud IMDS at 100.100.100.200)
        || (o[0] == 100 && (o[1] & 0xc0) == 64)
        // 169.254.0.0/16 link-local (incl. cloud IMDS 169.254.169.254)
        || (o[0] == 169 && o[1] == 254)
        // 192.0.0.192 — Azure IMDS alternative endpoint. Public IANA-
        // assigned (192.0.0.0/24 IETF reserved) but used by Azure for
        // legacy metadata access; web_fetch::check_ssrf blocks it and
        // this helper must stay aligned.
        || (o[0] == 192 && o[1] == 0 && o[2] == 0 && o[3] == 192)
    }

    /// IPv4 embedded in an IPv6 address through one of the two forms
    /// that route packets to an IPv4 endpoint on the wire:
    ///   * IPv4-mapped: `::ffff:x.x.x.x` (RFC 4291 §2.5.5.2)
    ///   * NAT64:       `64:ff9b::x.x.x.x` (RFC 6052)
    ///
    /// Without these, `http://[::ffff:7f00:0001]/` bypasses the V4
    /// loopback check entirely — the daemon happily connects to
    /// 127.0.0.1 over an IPv6 socket.
    fn ipv6_embedded_ipv4(v6: Ipv6Addr) -> Option<Ipv4Addr> {
        if let Some(v4) = v6.to_ipv4_mapped() {
            return Some(v4);
        }
        let s = v6.segments();
        if s[0] == 0x0064 && s[1] == 0xff9b && s[2..6].iter().all(|seg| *seg == 0) {
            return Some(Ipv4Addr::new(
                (s[6] >> 8) as u8,
                (s[6] & 0xff) as u8,
                (s[7] >> 8) as u8,
                (s[7] & 0xff) as u8,
            ));
        }
        None
    }

    // Strip a trailing dot ("localhost." is the same host as "localhost"
    // to a resolver) before the hostname comparison.
    let lower = host.trim_end_matches('.').to_lowercase();
    // Block the names every major cloud provider uses for IMDS plus a
    // couple of loopback aliases. Keep this list aligned with
    // `librefang_runtime::web_fetch::check_ssrf`.
    if matches!(
        lower.as_str(),
        "localhost"
            | "ip6-localhost"
            | "ip6-loopback"
            | "metadata.google.internal"
            | "metadata.aws.internal"
            | "instance-data"
    ) {
        return true;
    }

    // `Url::host_str()` returns IPv6 as "[::1]"; strip brackets before IpAddr::from_str rejects them.
    let ip_str = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);

    if let Ok(ip) = ip_str.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => blocked_v4(v4),
            IpAddr::V6(v6) => {
                if let Some(v4) = ipv6_embedded_ipv4(v6) {
                    if blocked_v4(v4) {
                        return true;
                    }
                }
                let segs = v6.segments();
                // ::1 loopback
                v6.is_loopback()
                // fc00::/7 unique-local
                || (segs[0] & 0xfe00) == 0xfc00
                // fe80::/10 link-local
                || (segs[0] & 0xffc0) == 0xfe80
            }
        };
    }

    false
}

/// Validate a full URL string against the SSRF block list (#3623).
///
/// In order, the function:
/// 1. Parses the URL.
/// 2. Rejects non-`http`/`https` schemes — `file://`, `ftp://`, etc. would
///    otherwise reach `reqwest` and be served from the local filesystem or
///    a bare-TCP gateway.
/// 3. Rejects URLs with a userinfo component (`user[:pass]@host`).  The
///    `url` crate's `host_str()` correctly returns the part after `@`, so
///    the IMDS-literal form `http://allowed.com@169.254.169.254/` is
///    already caught by the host check in step 4.  Userinfo is rejected
///    separately because RFC 6749 never sanctions it on OAuth endpoints,
///    and a metadata document that contains it is anomalous input —
///    likely phishing-shape (`http://user@public.example.com/`, where
///    the host check passes), a credential-smuggling attempt, or
///    accidental pollution that would leak into logs and reqwest's
///    connection-pool key.  (PR #4099 / #3527 closed a related but
///    distinct host-extraction bug on the wasm fetch path.)
/// 4. Delegates the host check to [`is_ssrf_blocked_host`].
///
/// Returns `Ok(())` when the URL is safe, or `Err(reason)` when blocked.
///
/// Public so callers outside this module (e.g. the kernel OAuth provider's
/// `try_refresh`, the API-layer token exchange) can re-validate stored
/// endpoint URLs before making outbound requests against values written
/// before policy tightened.
pub fn is_ssrf_blocked_url(url_str: &str) -> Result<(), String> {
    let parsed = Url::parse(url_str).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("scheme '{scheme}' is not allowed")),
    }
    // Reject userinfo on OAuth endpoints — see doc comment.
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("URLs with userinfo are not permitted".to_string());
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;
    if is_ssrf_blocked_host(host) {
        return Err(format!("host '{host}' is a blocked address"));
    }
    Ok(())
}

/// Construct the `.well-known/oauth-authorization-server` URL for a given server URL.
///
/// Parses the URL, extracts the origin, and appends the well-known path.
/// Returns `None` when the origin's host is a literal private / loopback /
/// link-local address or a known internal hostname (SSRF guard — see
/// `is_ssrf_blocked_host`).  No DNS resolution is performed.
pub fn well_known_url(server_url: &str) -> Option<String> {
    let parsed = Url::parse(server_url).ok()?;
    // Block SSRF before constructing the well-known URL.
    let host = parsed.host_str()?;
    if is_ssrf_blocked_host(host) {
        return None;
    }
    let origin = parsed.origin().unicode_serialization();
    Some(format!("{}/.well-known/oauth-authorization-server", origin))
}

/// Returns true when `endpoint_host` and `server_host` resolve to the same
/// registrable domain (eTLD+1) under the Public Suffix List.
///
/// IP literals are rejected: PSL has no opinion on them, and its default
/// rule for an unknown TLD label produces shared-tail false matches
/// (`psl::domain_str("10.0.0.1") == Some("0.1")` would let `10.0.0.1` and
/// `10.0.0.2` collide). They must only ever match via Rule 1 (strict
/// origin equality) — see [`validate_metadata_endpoints`].
///
/// Mirrors the policy of
/// `librefang-api::routes::mcp_auth::token_endpoint_host_matches`'s Rule 2
/// (#4779, #4665).
fn shares_registrable_domain(endpoint_host: &str, server_host: &str) -> bool {
    use std::net::IpAddr;

    // Strip IPv6 brackets — `Url::host_str` keeps them.
    fn strip_brackets(h: &str) -> &str {
        h.trim_start_matches('[').trim_end_matches(']')
    }
    let eh = strip_brackets(endpoint_host);
    let sh = strip_brackets(server_host);

    if eh.parse::<IpAddr>().is_ok() || sh.parse::<IpAddr>().is_ok() {
        return false;
    }

    match (psl::domain_str(eh), psl::domain_str(sh)) {
        (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
        _ => false,
    }
}

/// Verify that every OAuth endpoint URL returned by a metadata document
/// shares the same origin (scheme + host + port) as `server_url`, or
/// at minimum the same registrable domain (eTLD+1) **AND** the same
/// scheme as `server_url`.
///
/// Two-rule acceptance policy (mirrors api-side
/// `token_endpoint_host_matches`):
/// - **Rule 1** (#3713): strict origin equality.
/// - **Rule 2** (#4665, #4779): both endpoint host and server host are
///   DNS names sharing the same eTLD+1, and `parsed.scheme() ==
///   server_parsed.scheme()`.
///
/// The scheme floor on Rule 2 is load-bearing: the api-side
/// `token_endpoint_host_matches` is host-only, so without this, a rogue
/// metadata document could declare `http://<sibling>.<eTLD+1>/token`
/// against an `https` MCP server, pass Rule 2, and the discovered
/// metadata would later be POSTed to over cleartext (the SSRF guard
/// `is_ssrf_blocked_url` allows both http and https). Pinning the
/// scheme keeps the #3713 floor on the dimension Rule 2 must not loosen.
///
/// **Port is not pinned on Rule 2** — only scheme and registrable
/// domain. A metadata document on `https://example.com:8443/token` is
/// accepted against `https://example.com/mcp` (origin differs by port,
/// so Rule 1 fails; eTLD+1 + scheme match, so Rule 2 accepts). The
/// reasoning is that the same registrable domain implies the same
/// org, and an org running an OAuth endpoint on a non-default port
/// within its own domain is legitimate. If a future threat model
/// requires per-port pinning, tighten Rule 2 to compare
/// `parsed.port_or_known_default()` as well — see the
/// `accepts_same_etld1_different_port` test for the contract this
/// documents.
///
/// **Maintenance:** this Rule 1 + Rule 2 + IP-carve-out policy is
/// duplicated in api-side `librefang_api::routes::mcp_auth::
/// token_endpoint_host_matches` (host-only there — no scheme floor).
/// Both sides must change together, otherwise discovery and
/// token-exchange will disagree on what's an acceptable endpoint.
pub fn validate_metadata_endpoints(
    metadata: &OAuthMetadata,
    server_url: &str,
) -> Result<(), String> {
    let server_parsed = Url::parse(server_url).map_err(|e| format!("Invalid server URL: {e}"))?;
    let server_origin = server_parsed.origin();
    let server_scheme = server_parsed.scheme();

    let check = |endpoint: &str, label: &str| -> Result<(), String> {
        let parsed =
            Url::parse(endpoint).map_err(|e| format!("Invalid {label} URL '{endpoint}': {e}"))?;

        // Rule 1 (#3713): strict origin equality.
        if parsed.origin() == server_origin {
            return Ok(());
        }

        // Rule 2 (#4665, #4779): same registrable domain, same scheme.
        if parsed.scheme() == server_scheme {
            let endpoint_host = parsed.host_str().unwrap_or("");
            let server_host = server_parsed.host_str().unwrap_or("");
            if !endpoint_host.is_empty()
                && !server_host.is_empty()
                && shares_registrable_domain(endpoint_host, server_host)
            {
                let registrable = psl::domain_str(endpoint_host).unwrap_or("");
                info!(
                    endpoint = %endpoint,
                    endpoint_host = %endpoint_host,
                    server_url = %server_url,
                    server_host = %server_host,
                    registrable_domain = %registrable,
                    label = %label,
                    "accepted OAuth metadata endpoint on sibling subdomain within same registrable domain (#4665)"
                );
                return Ok(());
            }
        }

        Err(format!(
            "OAuth metadata endpoint domain mismatch: {label} '{endpoint}' \
             does not share the same scheme+host as the MCP server '{server_url}'"
        ))
    };

    check(&metadata.authorization_endpoint, "authorization_endpoint")?;
    check(&metadata.token_endpoint, "token_endpoint")?;
    if let Some(ref reg) = metadata.registration_endpoint {
        check(reg, "registration_endpoint")?;
    }
    Ok(())
}

/// Generate a unique OAuth flow ID.
///
/// Returns 12 random bytes encoded as lowercase hex (24 chars), which is
/// short enough to fit comfortably in a URL `state` parameter while providing
/// ~96 bits of entropy — sufficient to prevent cross-flow confusion.
pub fn generate_flow_id() -> String {
    let mut buf = [0u8; 12];
    rand::fill(&mut buf);
    buf.iter().fold(String::with_capacity(24), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

// ---------------------------------------------------------------------------
// PKCE helpers
// ---------------------------------------------------------------------------

/// Generate a PKCE code verifier and challenge pair.
///
/// Returns `(verifier, challenge)` where:
/// - `verifier` is 32 random bytes encoded as base64url (no padding)
/// - `challenge` is SHA-256 of verifier encoded as base64url (no padding)
pub fn generate_pkce() -> (String, String) {
    let mut buf = [0u8; 32];
    rand::fill(&mut buf);
    let verifier = URL_SAFE_NO_PAD.encode(buf);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

/// Generate a random state parameter for OAuth flows.
///
/// Returns 16 random bytes encoded as base64url (no padding).
pub fn generate_state() -> String {
    let mut buf = [0u8; 16];
    rand::fill(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

// ---------------------------------------------------------------------------
// Metadata merge
// ---------------------------------------------------------------------------

/// Merge discovered OAuth metadata with user-provided config overrides.
///
/// Config values take precedence over discovered values. Empty scopes in
/// config means use discovered scopes.
pub fn merge_metadata_with_config(
    discovered: OAuthMetadata,
    config: &McpOAuthConfig,
) -> OAuthMetadata {
    OAuthMetadata {
        authorization_endpoint: config
            .auth_url
            .clone()
            .unwrap_or(discovered.authorization_endpoint),
        token_endpoint: config
            .token_url
            .clone()
            .unwrap_or(discovered.token_endpoint),
        client_id: config.client_id.clone().or(discovered.client_id),
        registration_endpoint: discovered.registration_endpoint,
        scopes: if config.scopes.is_empty() {
            discovered.scopes
        } else {
            config.scopes.clone()
        },
        user_scopes: if config.user_scopes.is_empty() {
            discovered.user_scopes
        } else {
            config.user_scopes.clone()
        },
        server_url: discovered.server_url,
    }
}

// ---------------------------------------------------------------------------
// Auth flow handle + provider trait
// ---------------------------------------------------------------------------

/// Structured error type for `McpOAuthProvider` storage operations (#3750).
///
/// Replaces the prior `Result<_, String>` so callers can distinguish:
/// - **VaultLocked** — vault exists but no master key available (env var
///   missing / OS keyring inaccessible). Recovery: prompt the operator to
///   set `LIBREFANG_VAULT_KEY` or unlock via `librefang vault init`.
/// - **KeyNotFound** — vault file does not exist yet, or the requested
///   per-server entry is absent. Recovery: run the OAuth flow.
/// - **Io** — filesystem I/O failed (disk full, permission denied, etc).
/// - **Crypto** — decryption / parse failure (corrupt vault, wrong key
///   that decoded but failed AEAD, schema mismatch). Recovery: investigate
///   `~/.librefang/vault.enc`; do NOT auto-recreate (data loss risk).
///
/// `load_token` returns `Ok(None)` for "no token stored" (a normal state
/// requiring the OAuth flow); only `Err` indicates an underlying storage
/// problem that the UI should surface distinctly from re-auth.
#[derive(Debug, thiserror::Error)]
pub enum McpOAuthError {
    #[error("vault is locked — set LIBREFANG_VAULT_KEY or unlock via `librefang vault init`")]
    VaultLocked,
    #[error("vault key not found: {0}")]
    KeyNotFound(String),
    #[error("vault I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("vault crypto/format error: {0}")]
    Crypto(String),
}

/// Trait for OAuth token storage and management.
///
/// Implementors handle persistence of tokens (e.g., encrypted vault on disk).
/// The actual OAuth flow (PKCE, browser redirect) is driven by the API layer,
/// not by the provider — the provider only handles token CRUD.
///
/// Errors are reported via [`McpOAuthError`] so callers can distinguish
/// "vault locked" (prompt for key) from "no token stored" (run OAuth flow)
/// from "I/O failure" (operational alert) — see #3750.
#[async_trait]
pub trait McpOAuthProvider: Send + Sync {
    /// Load a cached access token for the given server URL.
    ///
    /// Returns `Ok(None)` when no token is stored (normal pre-auth state) and
    /// `Err(...)` only for underlying storage problems (locked vault, I/O
    /// failure, decryption failure).
    async fn load_token(&self, server_url: &str) -> Result<Option<String>, McpOAuthError>;

    /// Store tokens received from the token endpoint.
    async fn store_tokens(
        &self,
        server_url: &str,
        tokens: OAuthTokens,
    ) -> Result<(), McpOAuthError>;

    /// Persist the discovery-derived OAuth metadata (`token_endpoint` and the
    /// optional `client_id`) under the durable per-server namespace.
    ///
    /// This is the bridge from the per-flow vault namespace (where the
    /// authorization handler stages discovery output keyed by `flow_id`) to
    /// the bare per-server namespace that the refresh path reads from. It
    /// must be called from the OAuth callback **after** `store_tokens`
    /// succeeds and **before** the per-flow PKCE values are cleaned up — see
    /// `librefang-api/src/routes/mcp_auth.rs::auth_callback`.
    ///
    /// Required (no default impl) so that any new provider must consciously
    /// decide how it persists this metadata. A silent no-op default would
    /// silently re-introduce the refresh bug this method exists to fix
    /// (refresh fails on the first access-token expiry with
    /// `No token_endpoint stored for refresh`).
    async fn store_oauth_metadata(
        &self,
        server_url: &str,
        token_endpoint: &str,
        client_id: Option<&str>,
    ) -> Result<(), McpOAuthError>;

    /// Clear stored tokens for the given server URL.
    async fn clear_tokens(&self, server_url: &str) -> Result<(), McpOAuthError>;
}

// ---------------------------------------------------------------------------
// .well-known metadata discovery
// ---------------------------------------------------------------------------

/// Raw OAuth Authorization Server Metadata (RFC 8414) response.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AuthorizationServerMetadata {
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    code_challenge_methods_supported: Vec<String>,
}

/// Parse a JSON body into `OAuthMetadata`.
///
/// Expects the body to be a valid OAuth Authorization Server Metadata document
/// (RFC 8414). Extracts the required endpoints and converts to our internal type.
///
/// SECURITY (#3623): All discovered endpoint URLs are validated through the
/// SSRF guard (`is_ssrf_blocked_url`) before being returned.  This rejects
/// non-http/https schemes, URLs with a userinfo component, and any literal
/// IP or known internal hostname (loopback, link-local, RFC 1918, IMDS,
/// IPv4-mapped IPv6, NAT64, …) — a malicious MCP server could otherwise
/// point an endpoint at an internal service or attach userinfo that leaks
/// into logs and reqwest's connection-pool key.  DNS resolution is out
/// of scope; mitigate rebinding at the network layer.
pub fn parse_authorization_server_metadata(
    body: &str,
    server_url: &str,
) -> Result<OAuthMetadata, String> {
    let raw: AuthorizationServerMetadata =
        serde_json::from_str(body).map_err(|e| format!("Failed to parse metadata JSON: {e}"))?;

    // SSRF guard — validate every discovered endpoint URL. Use the shared
    // `is_ssrf_blocked_url` so scheme + userinfo + host checks stay in sync
    // across every callsite (parser, kernel try_refresh, API token-exchange).
    for (label, url_str) in [
        (
            "authorization_endpoint",
            raw.authorization_endpoint.as_str(),
        ),
        ("token_endpoint", raw.token_endpoint.as_str()),
    ] {
        if let Err(reason) = is_ssrf_blocked_url(url_str) {
            return Err(format!("SSRF: {label} rejected: {reason}"));
        }
    }
    if let Some(reg_ep) = raw.registration_endpoint.as_deref() {
        if let Err(reason) = is_ssrf_blocked_url(reg_ep) {
            return Err(format!("SSRF: registration_endpoint rejected: {reason}"));
        }
    }

    Ok(OAuthMetadata {
        authorization_endpoint: raw.authorization_endpoint,
        token_endpoint: raw.token_endpoint,
        client_id: None,
        registration_endpoint: raw.registration_endpoint,
        scopes: Vec::new(),
        user_scopes: Vec::new(),
        server_url: server_url.to_string(),
    })
}

/// Discover OAuth metadata for an MCP server using three-tier resolution.
///
/// 1. **Tier 1**: Parse `www_authenticate` header -> extract `resource_metadata` URL -> fetch -> parse.
/// 2. **Tier 2**: Construct `.well-known/oauth-authorization-server` URL from server_url -> fetch -> parse.
/// 3. **Tier 3**: Fall back to config (requires both `auth_url` and `token_url`).
///
/// If config is provided, it is merged with discovery results (config values take precedence).
/// Returns an error if all tiers fail.
pub async fn discover_oauth_metadata(
    server_url: &str,
    www_authenticate: Option<&str>,
    config: Option<&McpOAuthConfig>,
) -> Result<OAuthMetadata, String> {
    // OAuth metadata discovery must respect [proxy] config (#3577) — corporate
    // networks routinely require a proxy and OAuth was a primary failure case.
    let client = librefang_http::proxied_client_builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    // Tier 1: WWW-Authenticate header -> resource_metadata URL
    if let Some(header) = www_authenticate {
        let params = parse_www_authenticate(header);
        if let Some(metadata_url) = extract_metadata_url(&params, server_url) {
            debug!(url = %metadata_url, "Tier 1: fetching metadata from WWW-Authenticate resource_metadata");
            match client.get(&metadata_url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(body) = resp.text().await {
                        match parse_authorization_server_metadata(&body, server_url) {
                            Ok(meta) => {
                                // #3713: Verify discovered endpoints share the server's origin.
                                if let Err(e) = validate_metadata_endpoints(&meta, server_url) {
                                    warn!(error = %e, "Tier 1: endpoint domain mismatch — rejecting metadata");
                                } else {
                                    let meta = if let Some(cfg) = config {
                                        merge_metadata_with_config(meta, cfg)
                                    } else {
                                        meta
                                    };
                                    return Ok(meta);
                                }
                            }
                            Err(e) => {
                                warn!(error = %e, "Tier 1: failed to parse metadata");
                            }
                        }
                    }
                }
                Ok(resp) => {
                    warn!(status = %resp.status(), "Tier 1: metadata fetch returned non-success");
                }
                Err(e) => {
                    warn!(error = %e, "Tier 1: metadata fetch failed");
                }
            }
        }
    }

    // Tier 2: .well-known URL
    // well_known_url() already guards against SSRF (private/loopback hosts) — #3592.
    if let Some(wk_url) = well_known_url(server_url) {
        debug!(url = %wk_url, "Tier 2: fetching .well-known metadata");
        match client.get(&wk_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    match parse_authorization_server_metadata(&body, server_url) {
                        Ok(meta) => {
                            // #3713: Verify discovered endpoints share the server's origin.
                            if let Err(e) = validate_metadata_endpoints(&meta, server_url) {
                                warn!(error = %e, "Tier 2: endpoint domain mismatch — rejecting metadata");
                            } else {
                                let meta = if let Some(cfg) = config {
                                    merge_metadata_with_config(meta, cfg)
                                } else {
                                    meta
                                };
                                return Ok(meta);
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Tier 2: failed to parse .well-known metadata");
                        }
                    }
                }
            }
            Ok(resp) => {
                warn!(status = %resp.status(), "Tier 2: .well-known fetch returned non-success");
            }
            Err(e) => {
                warn!(error = %e, "Tier 2: .well-known fetch failed");
            }
        }
    }

    // Tier 3: Config fallback
    if let Some(cfg) = config {
        if let (Some(auth_url), Some(token_url)) = (&cfg.auth_url, &cfg.token_url) {
            debug!("Tier 3: using config fallback");
            return Ok(OAuthMetadata {
                authorization_endpoint: auth_url.clone(),
                token_endpoint: token_url.clone(),
                client_id: cfg.client_id.clone(),
                registration_endpoint: None,
                scopes: cfg.scopes.clone(),
                user_scopes: cfg.user_scopes.clone(),
                server_url: server_url.to_string(),
            });
        }
    }

    Err(format!(
        "OAuth metadata discovery failed for {server_url}: \
         no resource_metadata in WWW-Authenticate, .well-known fetch failed, \
         and no config fallback (auth_url + token_url) provided"
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- split_auth_params tests --

    #[test]
    fn test_split_auth_params_simple() {
        let parts = split_auth_params(r#"realm="OAuth", error="invalid_token""#);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0], r#"realm="OAuth""#);
        assert_eq!(parts[1], r#"error="invalid_token""#);
    }

    #[test]
    fn test_split_auth_params_commas_in_quotes() {
        let parts = split_auth_params(r#"realm="OAuth, v2", error="bad""#);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("OAuth, v2"));
    }

    #[test]
    fn test_split_auth_params_empty() {
        let parts = split_auth_params("");
        assert!(parts.is_empty());
    }

    // -- parse_www_authenticate tests --

    #[test]
    fn test_parse_www_authenticate_basic() {
        let map = parse_www_authenticate(
            r#"Bearer realm="OAuth", error="invalid_token", error_description="Token expired""#,
        );
        assert_eq!(map.get("realm").unwrap(), "OAuth");
        assert_eq!(map.get("error").unwrap(), "invalid_token");
        assert_eq!(map.get("error_description").unwrap(), "Token expired");
    }

    #[test]
    fn test_parse_www_authenticate_with_resource_metadata() {
        let map = parse_www_authenticate(
            r#"Bearer realm="mcp", resource_metadata="https://auth.example.com/.well-known/oauth-authorization-server""#,
        );
        assert_eq!(map.get("realm").unwrap(), "mcp");
        assert_eq!(
            map.get("resource_metadata").unwrap(),
            "https://auth.example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_parse_www_authenticate_no_bearer_prefix() {
        let map = parse_www_authenticate("Basic realm=\"test\"");
        assert!(map.is_empty());
    }

    #[test]
    fn test_parse_www_authenticate_case_insensitive_prefix() {
        let map = parse_www_authenticate(r#"bearer realm="test""#);
        assert_eq!(map.get("realm").unwrap(), "test");
    }

    // -- extract_metadata_url tests --

    #[test]
    fn test_extract_metadata_url_present() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://example.com/.well-known/oauth-authorization-server".to_string(),
        );
        // Same origin: metadata and server both on example.com
        let url = extract_metadata_url(&params, "https://example.com/mcp");
        assert_eq!(
            url.unwrap(),
            "https://example.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_extract_metadata_url_missing() {
        let params = HashMap::new();
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn test_extract_metadata_url_invalid_scheme() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "ftp://bad.example.com".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    // -- B2: SSRF hardening tests for extract_metadata_url --

    #[test]
    fn extract_metadata_url_rejects_http() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "http://example.com/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_rejects_cross_origin() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://evil.com/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_accepts_same_origin_https() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://example.com/meta".to_string(),
        );
        let result = extract_metadata_url(&params, "https://example.com/mcp");
        assert_eq!(result.unwrap(), "https://example.com/meta");
    }

    #[test]
    fn extract_metadata_url_rejects_loopback_literal() {
        // Same-origin already rejects this (different hosts), but layer 3 also blocks
        // loopback IPs as defence-in-depth.
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://127.0.0.1/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_rejects_link_local() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "https://169.254.169.254/latest/meta-data/".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    #[test]
    fn extract_metadata_url_rejects_missing_scheme() {
        let mut params = HashMap::new();
        params.insert(
            "resource_metadata".to_string(),
            "example.com/meta".to_string(),
        );
        assert!(extract_metadata_url(&params, "https://example.com/mcp").is_none());
    }

    // -- well_known_url tests --

    #[test]
    fn test_well_known_url_basic() {
        let url = well_known_url("https://my-server.com/mcp").unwrap();
        assert_eq!(
            url,
            "https://my-server.com/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_well_known_url_with_port() {
        let url = well_known_url("https://my-server.com:8443/mcp/v1").unwrap();
        assert_eq!(
            url,
            "https://my-server.com:8443/.well-known/oauth-authorization-server"
        );
    }

    #[test]
    fn test_well_known_url_invalid() {
        assert!(well_known_url("not-a-url").is_none());
    }

    #[test]
    fn test_well_known_url_http() {
        let url = well_known_url("http://mcp.example.com:3000/mcp").unwrap();
        assert_eq!(
            url,
            "http://mcp.example.com:3000/.well-known/oauth-authorization-server"
        );
    }

    // -- PKCE tests --

    #[test]
    fn test_generate_pkce_length() {
        let (verifier, challenge) = generate_pkce();
        // 32 bytes -> 43 base64url chars (no padding)
        assert_eq!(verifier.len(), 43);
        // SHA-256 -> 32 bytes -> 43 base64url chars
        assert_eq!(challenge.len(), 43);
    }

    #[test]
    fn test_generate_pkce_uniqueness() {
        let (v1, c1) = generate_pkce();
        let (v2, c2) = generate_pkce();
        assert_ne!(v1, v2);
        assert_ne!(c1, c2);
    }

    #[test]
    fn test_generate_pkce_challenge_is_sha256_of_verifier() {
        let (verifier, challenge) = generate_pkce();
        let digest = Sha256::digest(verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(digest);
        assert_eq!(challenge, expected);
    }

    // -- state generation tests --

    #[test]
    fn test_generate_state_length() {
        let state = generate_state();
        // 16 bytes -> 22 base64url chars (no padding)
        assert_eq!(state.len(), 22);
    }

    #[test]
    fn test_generate_state_uniqueness() {
        let s1 = generate_state();
        let s2 = generate_state();
        assert_ne!(s1, s2);
    }

    // -- metadata merge tests --

    #[test]
    fn test_merge_metadata_config_overrides_endpoints() {
        let discovered = OAuthMetadata {
            authorization_endpoint: "https://discovered.com/auth".to_string(),
            token_endpoint: "https://discovered.com/token".to_string(),
            client_id: Some("discovered-client".to_string()),
            registration_endpoint: None,
            scopes: vec!["read".to_string()],
            user_scopes: Vec::new(),
            server_url: "https://server.com/mcp".to_string(),
        };
        let config = McpOAuthConfig {
            auth_url: Some("https://override.com/auth".to_string()),
            token_url: Some("https://override.com/token".to_string()),
            client_id: Some("override-client".to_string()),
            scopes: vec!["admin".to_string()],
            user_scopes: Vec::new(),
        };
        let merged = merge_metadata_with_config(discovered, &config);
        assert_eq!(merged.authorization_endpoint, "https://override.com/auth");
        assert_eq!(merged.token_endpoint, "https://override.com/token");
        assert_eq!(merged.client_id.unwrap(), "override-client");
        assert_eq!(merged.scopes, vec!["admin"]);
        assert_eq!(merged.server_url, "https://server.com/mcp");
    }

    #[test]
    fn test_merge_metadata_empty_config_keeps_discovered() {
        let discovered = OAuthMetadata {
            authorization_endpoint: "https://discovered.com/auth".to_string(),
            token_endpoint: "https://discovered.com/token".to_string(),
            client_id: Some("discovered-client".to_string()),
            registration_endpoint: None,
            scopes: vec!["read".to_string(), "write".to_string()],
            user_scopes: Vec::new(),
            server_url: "https://server.com/mcp".to_string(),
        };
        let config = McpOAuthConfig::default();
        let merged = merge_metadata_with_config(discovered, &config);
        assert_eq!(merged.authorization_endpoint, "https://discovered.com/auth");
        assert_eq!(merged.token_endpoint, "https://discovered.com/token");
        assert_eq!(merged.client_id.unwrap(), "discovered-client");
        assert_eq!(merged.scopes, vec!["read", "write"]);
    }

    // -- parse_authorization_server_metadata tests --

    #[test]
    fn test_parse_authorization_server_metadata_success() {
        let body = r#"{
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "registration_endpoint": "https://auth.example.com/register",
            "code_challenge_methods_supported": ["S256"]
        }"#;
        let meta = parse_authorization_server_metadata(body, "https://server.com/mcp").unwrap();
        assert_eq!(
            meta.authorization_endpoint,
            "https://auth.example.com/authorize"
        );
        assert_eq!(meta.token_endpoint, "https://auth.example.com/token");
        assert!(meta.client_id.is_none());
        assert!(meta.scopes.is_empty());
        assert_eq!(meta.server_url, "https://server.com/mcp");
    }

    #[test]
    fn test_parse_authorization_server_metadata_missing_fields() {
        let body = r#"{"authorization_endpoint": "https://auth.example.com/authorize"}"#;
        let result = parse_authorization_server_metadata(body, "https://server.com/mcp");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Failed to parse metadata JSON"));
    }

    #[test]
    fn test_parse_authorization_server_metadata_invalid_json() {
        let result = parse_authorization_server_metadata("not json", "https://server.com/mcp");
        assert!(result.is_err());
    }

    // -- #3592: well_known_url SSRF guard tests --

    #[test]
    fn well_known_url_blocks_loopback_ipv4() {
        // A server_url pointing to 127.x must not yield a well-known fetch URL.
        assert!(well_known_url("http://127.0.0.1:8080/mcp").is_none());
    }

    #[test]
    fn well_known_url_blocks_private_10_range() {
        assert!(well_known_url("http://10.0.0.1/mcp").is_none());
    }

    #[test]
    fn well_known_url_blocks_private_172_range() {
        assert!(well_known_url("http://172.16.0.1/mcp").is_none());
    }

    #[test]
    fn well_known_url_blocks_private_192_168_range() {
        assert!(well_known_url("http://192.168.1.1/mcp").is_none());
    }

    #[test]
    fn well_known_url_blocks_link_local() {
        assert!(well_known_url("http://169.254.169.254/mcp").is_none());
    }

    #[test]
    fn well_known_url_blocks_localhost_hostname() {
        assert!(well_known_url("http://localhost/mcp").is_none());
    }

    #[test]
    fn well_known_url_allows_public_host() {
        let url = well_known_url("https://my-mcp-server.example.com/mcp").unwrap();
        assert_eq!(
            url,
            "https://my-mcp-server.example.com/.well-known/oauth-authorization-server"
        );
    }

    /// Regression: `::ffff:x.x.x.x` (IPv4-mapped IPv6) used to bypass
    /// the V4 loopback check.  Packets to this address are delivered to
    /// the V4 endpoint on the wire, so it must be classified by the V4
    /// rules.
    #[test]
    fn well_known_url_blocks_ipv4_mapped_ipv6_loopback() {
        assert!(well_known_url("http://[::ffff:7f00:0001]/mcp").is_none());
        assert!(well_known_url("http://[::ffff:127.0.0.1]/mcp").is_none());
    }

    #[test]
    fn well_known_url_blocks_ipv4_mapped_ipv6_imds() {
        // 169.254.169.254 — AWS / Azure IMDS — delivered via mapped V6.
        assert!(well_known_url("http://[::ffff:a9fe:a9fe]/mcp").is_none());
    }

    /// NAT64 prefix `64:ff9b::x.x.x.x` (RFC 6052) is the second wire
    /// path that delivers packets to a V4 endpoint over a V6 socket.
    #[test]
    fn well_known_url_blocks_nat64_loopback() {
        assert!(well_known_url("http://[64:ff9b::7f00:1]/mcp").is_none());
    }

    /// Trailing-dot variants of `localhost` resolve to the same host;
    /// the lookup must be case- and dot-insensitive.
    #[test]
    fn well_known_url_blocks_localhost_with_trailing_dot() {
        assert!(well_known_url("http://localhost./mcp").is_none());
        assert!(well_known_url("http://LOCALHOST/mcp").is_none());
    }

    // -- #3713: validate_metadata_endpoints domain-mismatch tests --

    #[test]
    fn validate_metadata_endpoints_accepts_same_origin() {
        let meta = OAuthMetadata {
            authorization_endpoint: "https://example.com/auth".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
            client_id: None,
            registration_endpoint: Some("https://example.com/register".to_string()),
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://example.com/mcp".to_string(),
        };
        assert!(validate_metadata_endpoints(&meta, "https://example.com/mcp").is_ok());
    }

    #[test]
    fn validate_metadata_endpoints_rejects_cross_domain_token_endpoint() {
        let meta = OAuthMetadata {
            authorization_endpoint: "https://example.com/auth".to_string(),
            token_endpoint: "https://evil.com/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://example.com/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://example.com/mcp").unwrap_err();
        assert!(
            err.contains("domain mismatch"),
            "error should mention domain mismatch: {err}"
        );
        assert!(
            err.contains("token_endpoint"),
            "error should name the field: {err}"
        );
    }

    #[test]
    fn validate_metadata_endpoints_rejects_cross_domain_auth_endpoint() {
        let meta = OAuthMetadata {
            authorization_endpoint: "https://evil.com/auth".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://example.com/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://example.com/mcp").unwrap_err();
        assert!(err.contains("domain mismatch"), "{err}");
        assert!(err.contains("authorization_endpoint"), "{err}");
    }

    #[test]
    fn validate_metadata_endpoints_rejects_cross_domain_registration_endpoint() {
        let meta = OAuthMetadata {
            authorization_endpoint: "https://example.com/auth".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
            client_id: None,
            registration_endpoint: Some("https://attacker.net/register".to_string()),
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://example.com/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://example.com/mcp").unwrap_err();
        assert!(err.contains("domain mismatch"), "{err}");
        assert!(err.contains("registration_endpoint"), "{err}");
    }

    #[test]
    fn validate_metadata_endpoints_no_registration_endpoint_ok() {
        let meta = OAuthMetadata {
            authorization_endpoint: "https://example.com/auth".to_string(),
            token_endpoint: "https://example.com/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://example.com/mcp".to_string(),
        };
        assert!(validate_metadata_endpoints(&meta, "https://example.com/mcp").is_ok());
    }

    // -- #4665, #4779: Rule 2 eTLD+1 acceptance with scheme floor --

    #[test]
    fn validate_metadata_endpoints_accepts_cross_subdomain_within_registrable_domain() {
        // Slack-shaped delegation: server URL on mcp.slack.com, metadata
        // declares endpoints on slack.com. Both share eTLD+1 = slack.com,
        // schemes match — Rule 2 accepts (#4665, #4779).
        let meta = OAuthMetadata {
            authorization_endpoint: "https://slack.com/oauth/v2/authorize".to_string(),
            token_endpoint: "https://slack.com/api/oauth.v2.access".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://mcp.slack.com/mcp".to_string(),
        };
        assert!(
            validate_metadata_endpoints(&meta, "https://mcp.slack.com/mcp").is_ok(),
            "expected legitimate cross-subdomain delegation to be accepted"
        );
    }

    #[test]
    fn validate_metadata_endpoints_accepts_deeper_subdomain_within_registrable_domain() {
        let meta = OAuthMetadata {
            authorization_endpoint: "https://c.foo.com/auth".to_string(),
            token_endpoint: "https://c.foo.com/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://a.b.foo.com/mcp".to_string(),
        };
        assert!(validate_metadata_endpoints(&meta, "https://a.b.foo.com/mcp").is_ok());
    }

    #[test]
    fn validate_metadata_endpoints_accepts_multilabel_psl_pair() {
        // mcp.bbc.co.uk and bbc.co.uk share eTLD+1 = bbc.co.uk under the
        // PSL multi-label public suffix. Pin against a naive `split('.')`
        // implementation that would treat `co.uk` as the eTLD+1.
        let meta = OAuthMetadata {
            authorization_endpoint: "https://bbc.co.uk/auth".to_string(),
            token_endpoint: "https://bbc.co.uk/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://mcp.bbc.co.uk/mcp".to_string(),
        };
        assert!(validate_metadata_endpoints(&meta, "https://mcp.bbc.co.uk/mcp").is_ok());
    }

    #[test]
    fn validate_metadata_endpoints_accepts_same_etld1_different_port() {
        // Server URL on the default https port, endpoint on a non-default
        // port within the same registrable domain. Url::origin() normalises
        // default ports to the scheme's default, so origins differ — Rule 1
        // fails. eTLD+1 + scheme match — Rule 2 accepts.
        //
        // Pins the policy from the validate_metadata_endpoints doc comment:
        // port is NOT compared on Rule 2. If a future threat model demands
        // per-port pinning, that change must update this test (and the
        // matching api-side `token_endpoint_host_matches`) together.
        let meta = OAuthMetadata {
            authorization_endpoint: "https://example.com/auth".to_string(),
            token_endpoint: "https://example.com:8443/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://example.com/mcp".to_string(),
        };
        assert!(
            validate_metadata_endpoints(&meta, "https://example.com/mcp").is_ok(),
            "expected same-eTLD+1 different-port to be accepted under Rule 2"
        );
    }

    #[test]
    fn validate_metadata_endpoints_rejects_co_uk_neighbour() {
        // attacker.co.uk and bbc.co.uk live under the SAME public suffix
        // (`co.uk`) but are different registrable domains — must reject.
        let meta = OAuthMetadata {
            authorization_endpoint: "https://attacker.co.uk/auth".to_string(),
            token_endpoint: "https://attacker.co.uk/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://mcp.bbc.co.uk/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://mcp.bbc.co.uk/mcp").unwrap_err();
        assert!(err.contains("domain mismatch"), "{err}");
    }

    #[test]
    fn validate_metadata_endpoints_rejects_under_public_suffix_boundary() {
        // *.github.io is a PSL private suffix; userA and userB are
        // different registrable domains and must not trust each other.
        let meta = OAuthMetadata {
            authorization_endpoint: "https://userB.github.io/auth".to_string(),
            token_endpoint: "https://userA.github.io/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://userA.github.io/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://userA.github.io/mcp").unwrap_err();
        assert!(err.contains("domain mismatch"), "{err}");
        assert!(err.contains("authorization_endpoint"), "{err}");
    }

    #[test]
    fn validate_metadata_endpoints_rejects_http_downgrade_within_same_registrable_domain() {
        // Server URL is https; metadata declares an http token endpoint
        // on the same eTLD+1. Without the scheme floor on Rule 2, this
        // would pass (eTLD+1 = slack.com matches), the SSRF guard
        // `is_ssrf_blocked_url` allows http, and credentials would be
        // POSTed in cleartext. Rule 2 must refuse.
        let meta = OAuthMetadata {
            authorization_endpoint: "https://mcp.slack.com/oauth/v2/authorize".to_string(),
            token_endpoint: "http://slack.com/api/oauth.v2.access".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://mcp.slack.com/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://mcp.slack.com/mcp").unwrap_err();
        assert!(err.contains("domain mismatch"), "{err}");
        assert!(err.contains("token_endpoint"), "{err}");
    }

    #[test]
    fn validate_metadata_endpoints_rejects_ip_literal_with_shared_psl_default_rule() {
        // `psl::domain_str`'s unknown-TLD default rule produces shared
        // trailing labels for unrelated IPv4 literals
        // (`psl::domain_str("10.0.0.1") == Some("0.1")`,
        // `psl::domain_str("10.0.0.2") == Some("0.2")` ... but
        // `psl::domain_str("10.0.0.1") == psl::domain_str("11.0.0.1") == "0.1"`).
        // The IP carve-out in `shares_registrable_domain` must reject all
        // IP-literal pairs that fail Rule 1 (strict origin equality), no
        // matter how the PSL default rule labels them. Mirrors #4779's
        // `token_endpoint_ip_host_requires_exact_match`.
        let meta = OAuthMetadata {
            authorization_endpoint: "https://10.0.0.1/auth".to_string(),
            token_endpoint: "https://11.0.0.1/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://10.0.0.1/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://10.0.0.1/mcp").unwrap_err();
        assert!(err.contains("domain mismatch"), "{err}");
        assert!(err.contains("token_endpoint"), "{err}");
    }

    #[test]
    fn validate_metadata_endpoints_rejects_bracketed_ipv6_pair() {
        // `Url::host_str` returns IPv6 in bracketed form (`[::1]`); the
        // IP carve-out must strip brackets before parsing or both sides
        // would fail the IpAddr parse and slip past into the PSL path,
        // where they'd fail again — but we want the failure to be
        // unambiguous and on the IP carve-out (so a future PSL upgrade
        // that started recognising IPv6 literals doesn't regress).
        let meta = OAuthMetadata {
            authorization_endpoint: "https://[2001:db8::1]/auth".to_string(),
            token_endpoint: "https://[2001:db8::2]/token".to_string(),
            client_id: None,
            registration_endpoint: None,
            scopes: Vec::new(),
            user_scopes: Vec::new(),
            server_url: "https://[2001:db8::1]/mcp".to_string(),
        };
        let err = validate_metadata_endpoints(&meta, "https://[2001:db8::1]/mcp").unwrap_err();
        assert!(err.contains("domain mismatch"), "{err}");
    }

    // -- #3727: generate_flow_id tests --

    #[test]
    fn generate_flow_id_is_24_hex_chars() {
        let id = generate_flow_id();
        assert_eq!(id.len(), 24, "expected 24 hex chars, got {id}");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "expected all hex digits: {id}"
        );
    }

    #[test]
    fn generate_flow_id_is_unique() {
        let ids: Vec<String> = (0..10).map(|_| generate_flow_id()).collect();
        let unique: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();
        assert_eq!(unique.len(), ids.len(), "duplicate flow IDs generated");
    }

    // -- #3623: is_ssrf_blocked_url scheme + userinfo coverage --

    #[test]
    fn is_ssrf_blocked_url_rejects_userinfo() {
        // The first two URLs are also caught by the host check (host_str()
        // returns the IMDS literal / loopback after the @, per RFC 3986),
        // so they exercise belt-and-suspenders defense.  The third case is
        // the unique regression point for this guard: host_str() is
        // "example.com" (public), the host check passes, and *only* the
        // userinfo guard rejects.  If anyone removes the guard in a future
        // refactor, this assertion is what fails.
        assert!(is_ssrf_blocked_url("http://allowed.com@169.254.169.254/").is_err());
        assert!(is_ssrf_blocked_url("http://user:pass@127.0.0.1/x").is_err());
        assert!(is_ssrf_blocked_url("http://user@example.com/").is_err());
    }

    #[test]
    fn is_ssrf_blocked_url_rejects_non_http_schemes() {
        assert!(is_ssrf_blocked_url("file:///etc/passwd").is_err());
        assert!(is_ssrf_blocked_url("ftp://example.com/x").is_err());
        assert!(is_ssrf_blocked_url("gopher://example.com/").is_err());
    }

    #[test]
    fn is_ssrf_blocked_url_allows_plain_public() {
        assert!(is_ssrf_blocked_url("https://auth.example.com/token").is_ok());
        assert!(is_ssrf_blocked_url("http://auth.example.com:8443/authorize").is_ok());
    }

    #[test]
    fn parse_authorization_server_metadata_rejects_userinfo_endpoint() {
        // Userinfo on OAuth metadata is anomalous (RFC 6749 doesn't
        // sanction it).  The post-`@` host here is public — `host_str()`
        // returns "auth.example.com" and the host check passes — so this
        // test fails ONLY if the userinfo guard is removed.  Pairs with
        // `is_ssrf_blocked_url_rejects_userinfo` to lock the parser-level
        // wiring at the same isolated regression point.
        let body = r#"{
            "authorization_endpoint": "https://user@auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token"
        }"#;
        let result = parse_authorization_server_metadata(body, "https://server.com/mcp");
        assert!(
            result.is_err(),
            "userinfo authorization_endpoint must be rejected"
        );
    }

    #[test]
    fn parse_authorization_server_metadata_rejects_imds_registration_endpoint() {
        let body = r#"{
            "authorization_endpoint": "https://auth.example.com/authorize",
            "token_endpoint": "https://auth.example.com/token",
            "registration_endpoint": "http://169.254.169.254/latest/api/token"
        }"#;
        let result = parse_authorization_server_metadata(body, "https://server.com/mcp");
        assert!(
            result.is_err(),
            "registration_endpoint pointing at IMDS must be rejected"
        );
    }
}
