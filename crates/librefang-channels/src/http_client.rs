//! Shared HTTP client builder, SSRF guard, and constant-time HMAC compare.

use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

static TLS_CONFIG: OnceLock<rustls::ClientConfig> = OnceLock::new();

fn init_tls_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();
    let result = rustls_native_certs::load_native_certs();
    let (added, _) = root_store.add_parsable_certificates(result.certs);
    if added == 0 {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    rustls::ClientConfig::builder_with_provider(
        rustls::crypto::aws_lc_rs::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .expect("default protocol versions")
    .with_root_certificates(root_store)
    .with_no_client_auth()
}

pub fn client_builder() -> reqwest::ClientBuilder {
    let tls = TLS_CONFIG.get_or_init(init_tls_config).clone();
    reqwest::ClientBuilder::new().use_preconfigured_tls(tls)
}

pub fn new_client() -> reqwest::Client {
    client_builder()
        .build()
        .expect("HTTP client with bundled CA roots should always build")
}

/// Error returned when a per-channel `proxy = "…"` setting cannot be
/// turned into a working `reqwest::Client`.
///
/// Two variants by design — the bridge needs to distinguish them in
/// log output:
///
/// * [`Self::InvalidUrl`] — the operator typed `proxy = "not a url"`
///   or used a scheme reqwest does not support
///   (`reqwest::Proxy::all` accepts `http://`, `https://`, `socks5://`,
///   `socks5h://`). Surfaced at config-load / adapter-init time so the
///   error is obvious before the first send hits the wire.
/// * [`Self::Build`] — `reqwest::ClientBuilder::build` itself failed
///   after the proxy was attached (e.g., TLS setup). Carries the raw
///   reqwest error string.
#[derive(Debug)]
pub enum ChannelProxyError {
    /// Proxy URL string was not accepted by `reqwest::Proxy::all`.
    /// `value` is the offending string (not redacted — `proxy` strings
    /// can carry `user:pass@host` so the caller logs `redact_proxy_url`
    /// in the channel bridge before printing).
    InvalidUrl { value: String, source: String },
    /// `reqwest::ClientBuilder::build` failed after the proxy was set.
    Build(String),
}

impl std::fmt::Display for ChannelProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUrl { value, source } => write!(
                f,
                "invalid channel proxy URL {value:?}: {source} \
                 (accepted schemes: http, https, socks5, socks5h)"
            ),
            Self::Build(msg) => write!(f, "failed to build proxied HTTP client: {msg}"),
        }
    }
}

impl std::error::Error for ChannelProxyError {}

/// Build a [`reqwest::ClientBuilder`] that routes ALL traffic through
/// `proxy_url` (HTTP, HTTPS, and any other scheme reqwest dispatches),
/// on top of the same bundled-TLS configuration [`client_builder`]
/// uses.
///
/// `proxy_url = None` returns the plain [`client_builder()`] — identical
/// to today's behavior, so reqwest's default env-var fallback
/// (`HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`) still
/// applies. `Some(url)` *overrides* env vars: the per-channel setting
/// is authoritative.
///
/// Returns [`ChannelProxyError::InvalidUrl`] when `proxy_url` is not a
/// shape `reqwest::Proxy::all` understands. The accepted schemes are
/// `http://`, `https://`, `socks5://`, `socks5h://`. SOCKS support is
/// gated on reqwest's `socks` feature — enabled at the workspace level.
pub fn proxied_client_builder(
    proxy_url: Option<&str>,
) -> Result<reqwest::ClientBuilder, ChannelProxyError> {
    let mut builder = client_builder();
    if let Some(url) = proxy_url {
        let proxy = reqwest::Proxy::all(url).map_err(|e| ChannelProxyError::InvalidUrl {
            value: url.to_string(),
            source: e.to_string(),
        })?;
        builder = builder.proxy(proxy);
    }
    Ok(builder)
}

/// Build a [`reqwest::Client`] that routes traffic through `proxy_url`,
/// or behaves identically to [`new_client`] when `proxy_url` is `None`.
///
/// This is the function channel adapters call from their `with_proxy`
/// builders. See [`proxied_client_builder`] for accepted URL shapes and
/// the env-var interaction contract.
pub fn new_proxied_client(proxy_url: Option<&str>) -> Result<reqwest::Client, ChannelProxyError> {
    proxied_client_builder(proxy_url)?
        .build()
        .map_err(|e| ChannelProxyError::Build(e.to_string()))
}

/// Emit a one-shot WARN naming the WS-bypass limitation for an adapter
/// whose REST client honours the per-channel `proxy` setting (#4795) but
/// whose long-lived WebSocket connection does not. Operators who set
/// `proxy = "..."` because their corporate egress only allows the
/// corporate HTTP proxy would otherwise see REST go through the proxy
/// and WS go direct — silently failing on strict egress. Surfacing the
/// limitation in startup logs (in addition to the docstring on each
/// `with_proxy`) gives them a fighting chance to spot the misconfig
/// before they file a bug.
///
/// Called by `SlackAdapter::start` (Socket Mode), `DiscordAdapter::start`
/// (Gateway), and `MattermostAdapter::start` (WebSocket). Telegram is
/// pure REST long-polling and does not call this. The `#[allow]` keeps it
/// available when all three WS-using channel features happen to be off
/// (the lib still compiles without forcing a `cfg(any(...))` here).
#[allow(dead_code)]
pub(crate) fn warn_ws_proxy_bypass(adapter: &str) {
    tracing::warn!(
        "{adapter}: proxy is configured but the long-lived WebSocket bypasses it; \
         REST traffic only goes through the proxy (#4795 follow-up)"
    );
}

/// Process-wide HTTP client used by [`fetch_url_bytes`] for safe
/// outbound media fetches.
///
/// Configured with a `redirect::Policy::custom` that re-runs
/// [`validate_url_for_fetch`] on every redirect target, refusing the
/// hop if the new URL points at a private/loopback/metadata host or a
/// non-`http(s)` scheme. Without this, reqwest's default
/// follow-up-to-10-redirects behaviour would let a `302 Location:
/// http://169.254.169.254/...` smuggle past the entry-time guard.
///
/// `OnceLock`-backed so the redirect policy and bundled TLS roots are
/// initialised once per process; multiple concurrent media fetches
/// share the connection pool.
pub(crate) fn safe_fetch_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        client_builder()
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 5 {
                    return attempt.error("too many redirects (cap: 5)");
                }
                let url_str = attempt.url().as_str().to_string();
                if let Err(reason) = validate_url_for_fetch(&url_str) {
                    return attempt.error(format!(
                        "SSRF guard refused redirect to '{url_str}': {reason}"
                    ));
                }
                attempt.follow()
            }))
            .build()
            .expect("safe fetch client builds with bundled roots")
    })
}

// ---------------------------------------------------------------------------
// SSRF guard
// ---------------------------------------------------------------------------

/// Validate that a URL from a channel payload is safe to fetch server-side.
///
/// Rejects:
/// - Non-http/https schemes (`file://`, `ftp://`, …).
/// - IP literals — IPv4 or IPv6 — that fall in any loopback, private,
///   link-local, unique-local, broadcast, multicast, reserved, or
///   cloud-metadata range.
/// - IPv4 written in any non-canonical form (short form `127.1`, decimal
///   `2130706433`, octal `0177.0.0.1`, hex `0x7f.0.0.1`). The WHATWG URL
///   parser inside [`url::Url::host`] normalizes these to a 4-octet
///   `Ipv4Addr` before we run the private-range check.
/// - IPv4-mapped IPv6 (`::ffff:7f00:1`) and the RFC 6052 NAT64 well-known
///   prefix (`64:ff9b::7f00:1`) when the embedded IPv4 is private. Both
///   route to a `127.x.x.x` socket on the wire even though the literal is
///   IPv6.
/// - Hostnames that match a known internal pattern (`localhost`,
///   `*.localhost`, `*.local`, `metadata`, `metadata.google.internal`,
///   `169.254.169.254`). The trailing-dot FQDN form (`localhost.`) is
///   normalized away before comparison.
///
/// **Out of scope by design:** DNS resolution. A public hostname may
/// still resolve to `127.0.0.1` (DNS rebinding); mitigate that at the
/// network layer or with a resolving SSRF proxy.
pub fn validate_url_for_fetch(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("scheme '{scheme}' is not allowed; only http/https")),
    }

    // `host()` returns the WHATWG-normalized `Host` enum; using `host_str()`
    // would lose information (IPv4 short forms get a string back, IPv6
    // gets the bracketed form like `"[::1]"` which `IpAddr::from_str`
    // refuses) and route through the wrong branch.
    let host = parsed.host().ok_or_else(|| "URL has no host".to_string())?;

    match host {
        url::Host::Ipv4(v4) => {
            if is_private_ipv4(v4) {
                return Err(format!("host resolves to private/reserved IPv4 {v4}"));
            }
        }
        url::Host::Ipv6(v6) => {
            // IPv4-mapped (::ffff:x.x.x.x) and NAT64 (64:ff9b::x.x.x.x)
            // both deliver packets to an IPv4 endpoint on the wire. Check
            // the embedded IPv4 against the private table before falling
            // back to pure-IPv6 ranges.
            if let Some(v4) = ipv6_embedded_ipv4(v6) {
                if is_private_ipv4(v4) {
                    return Err(format!("host '{v6}' embeds private IPv4 {v4}"));
                }
            }
            if is_private_ipv6(v6) {
                return Err(format!("host resolves to private/reserved IPv6 {v6}"));
            }
        }
        url::Host::Domain(domain) => {
            // Strip the trailing dot of an FQDN so "localhost." doesn't
            // bypass the "localhost" comparison.
            let trimmed = domain.trim_end_matches('.').to_ascii_lowercase();
            if is_private_hostname(&trimmed) {
                return Err(format!("host '{domain}' is a reserved or private hostname"));
            }
        }
    }

    Ok(())
}

/// IPv4 ranges that are not safe to dial from a server-side fetch.
fn is_private_ipv4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    // First-octet rules cover four big blocks cleanly:
    //   0.0.0.0/8         — "this network" / unspecified
    //   10.0.0.0/8        — RFC 1918 private
    //   127.0.0.0/8       — loopback
    //   224.0.0.0/4       — multicast (224.x.x.x – 239.x.x.x)
    //   240.0.0.0/4       — reserved + 255.255.255.255 broadcast
    if matches!(o[0], 0 | 10 | 127) || matches!(o[0], 224..=255) {
        return true;
    }
    matches!(
        (o[0], o[1]),
        // 100.64.0.0/10 — RFC 6598 carrier-grade NAT (shared address)
        (100, 64..=127)
        // 169.254.0.0/16 — link-local (incl. cloud metadata 169.254.169.254)
        | (169, 254)
        // 172.16.0.0/12 — RFC 1918 private
        | (172, 16..=31)
        // 192.168.0.0/16 — RFC 1918 private
        | (192, 168)
    ) || (
        // 192.0.0.0/24 — IETF protocol assignments (deliberately /24, NOT /16)
        o[0] == 192 && o[1] == 0 && o[2] == 0
    )
}

/// IPv6 ranges that are not safe to dial.
fn is_private_ipv6(v6: Ipv6Addr) -> bool {
    if v6.is_loopback() || v6.is_unspecified() {
        return true;
    }
    let segs = v6.segments();
    // Link-local fe80::/10
    if (segs[0] & 0xffc0) == 0xfe80 {
        return true;
    }
    // Unique local fc00::/7  (covers fd00::/8)
    if (segs[0] & 0xfe00) == 0xfc00 {
        return true;
    }
    // Multicast ff00::/8
    if (segs[0] & 0xff00) == 0xff00 {
        return true;
    }
    false
}

/// Extract an IPv4 address embedded in an IPv6 in the two ways an
/// attacker can use to reach an IPv4 endpoint via an IPv6 host:
/// IPv4-mapped (`::ffff:x.x.x.x`, RFC 4291 §2.5.5.2) and NAT64
/// (`64:ff9b::x.x.x.x`, RFC 6052).
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

/// Hostnames that should be refused without resolution.
fn is_private_hostname(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    if host.ends_with(".local") {
        return true;
    }
    matches!(
        host,
        "metadata" | "metadata.google.internal" | "metadata.azure.com" | "169.254.169.254"
    )
}

// ---------------------------------------------------------------------------
// Bounded fetch
// ---------------------------------------------------------------------------

/// Errors returned by [`fetch_url_bytes`].
///
/// Three variants by design — callers usually need to distinguish:
///
/// * a pre-network rejection (SSRF guard, scheme),
/// * a size-cap hit (so the caller can phrase its own "too large"
///   message and surface the real or the limit byte count), and
/// * any other failure (transport, non-2xx status, mid-stream read).
///
/// Used by channel modules behind `channel-*` feature flags; the `#[allow]`
/// keeps it available without forcing a `cfg(any(...))` here.
#[allow(dead_code)]
#[derive(Debug)]
pub enum FetchError {
    /// `validate_url_for_fetch` refused the URL before any I/O.
    Rejected(String),
    /// Body would exceed `max_bytes`. `actual` carries the
    /// `Content-Length` value when the pre-check tripped, and is
    /// `None` when the mid-stream accumulator tripped (no honest
    /// total available). `limit` is the cap that was passed in.
    TooLarge { actual: Option<u64>, limit: usize },
    /// Anything else: transport error, non-2xx status, mid-stream
    /// read failure, ...
    Failed(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(s) => write!(f, "{s}"),
            Self::TooLarge {
                actual: Some(len),
                limit,
            } => write!(f, "Content-Length {len} exceeds cap of {limit} bytes"),
            Self::TooLarge {
                actual: None,
                limit,
            } => write!(f, "response exceeds cap of {limit} bytes mid-stream"),
            Self::Failed(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetch the body of an `http(s)` URL with both the SSRF guard and a
/// hard in-memory size cap, returning the body and the optional
/// `Content-Type` header.
///
/// Two layers of defense:
///
/// 1. `validate_url_for_fetch` is applied to the input URL before any
///    network I/O, rejecting non-`http(s)` schemes and any host that
///    resolves syntactically into a private/loopback/metadata range.
/// 2. The streamed body is rejected if either the advertised
///    `Content-Length` exceeds `max_bytes`, or the accumulated bytes
///    grow past `max_bytes` mid-stream. The streaming guard is the
///    load-bearing one — `Content-Length` can be missing, wrong, or a
///    chunked-transfer "lie".
///
/// Use this in any code path where the URL can come from an untrusted
/// source (LLM output, channel-supplied attachment, MCP server response).
/// Callers MUST pass an explicit `max_bytes` ceiling — there is no
/// default; security utilities should make the cap a deliberate decision
/// of the caller.
///
/// The returned `Option<String>` is the response's `Content-Type`
/// header (raw, including any `; charset=...` / `; boundary=...`
/// parameters). Callers that need to dispatch on media type should
/// strip parameters and validate the prefix themselves — see the image
/// download path in `bridge.rs` for an example.
///
/// **DNS rebinding is out of scope** — same as `validate_url_for_fetch`.
/// Mitigate at the network layer or with a resolving SSRF proxy if the
/// threat model requires it.
///
/// Used by channel modules behind `channel-*` feature flags; the `#[allow]`
/// keeps it available without forcing a `cfg(any(...))` here.
#[allow(dead_code)]
pub async fn fetch_url_bytes(
    url: &str,
    max_bytes: usize,
    extra_headers: &[(String, String)],
) -> Result<(Vec<u8>, Option<String>), FetchError> {
    validate_url_for_fetch(url).map_err(FetchError::Rejected)?;
    fetch_url_bytes_unchecked(safe_fetch_client(), url, max_bytes, extra_headers).await
}

/// Send the GET and apply the size cap. Skips the SSRF guard.
///
/// `extra_headers` is attached to the outgoing request — caller is
/// responsible for deciding whether the URL belongs to a trusted host
/// (auth tokens MUST NOT be sent to a model-controlled hostname); see
/// `ChannelAdapter::fetch_headers_for` for the gating contract that
/// guards Matrix's MSC3916 Bearer-token path.
///
/// **Do NOT call this directly from production code.** The only legitimate
/// callers are unit tests pointing at a `wiremock` server (which binds
/// `127.0.0.1`, refused by `validate_url_for_fetch`). Production paths
/// MUST go through [`fetch_url_bytes`].
#[allow(dead_code)]
pub(crate) async fn fetch_url_bytes_unchecked(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
    extra_headers: &[(String, String)],
) -> Result<(Vec<u8>, Option<String>), FetchError> {
    let mut req = client.get(url);
    for (name, value) in extra_headers {
        req = req.header(name.as_str(), value.as_str());
    }
    let resp = req
        .send()
        .await
        .map_err(|e| FetchError::Failed(format!("fetch failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(FetchError::Failed(format!(
            "fetch returned status {}",
            resp.status()
        )));
    }

    if let Some(len) = resp.content_length() {
        if len > max_bytes as u64 {
            return Err(FetchError::TooLarge {
                actual: Some(len),
                limit: max_bytes,
            });
        }
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let cap = max_bytes;
    let prealloc = resp
        .content_length()
        .map(|c| (c as usize).min(cap))
        .unwrap_or(0);
    let mut body = Vec::with_capacity(prealloc);
    let mut resp = resp;
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| FetchError::Failed(format!("read chunk: {e}")))?
    {
        if body.len().saturating_add(chunk.len()) > cap {
            return Err(FetchError::TooLarge {
                actual: None,
                limit: cap,
            });
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, content_type))
}

// ---------------------------------------------------------------------------
// Constant-time HMAC compare
// ---------------------------------------------------------------------------

/// Constant-time equality for HMAC digests.
///
/// Always compares full slices and returns `false` on length mismatch.
/// Backed by the `subtle` crate so the comparison is genuinely
/// constant-time (the hand-rolled `for ... |= a ^ b` form risks being
/// auto-vectorized into an early-exit `memcmp` by future compilers).
///
/// Callers (`messenger`, `line`, `teams`, `viber`) are all behind
/// `channel-*` feature flags, so this is dead code when none of them
/// are enabled — the `#[allow]` keeps it available for any channel
/// module that opts in without forcing a matching `cfg(any(...))` here.
#[allow(dead_code)]
pub(crate) fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    use subtle::ConstantTimeEq;
    if a.len() != b.len() {
        return false;
    }
    a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------- SSRF guard ---------------------------------------------------

    #[test]
    fn allows_public_https() {
        assert!(validate_url_for_fetch("https://example.com/image.png").is_ok());
        assert!(validate_url_for_fetch("http://cdn.example.org/file").is_ok());
    }

    #[test]
    fn rejects_bad_scheme() {
        assert!(validate_url_for_fetch("ftp://example.com/file").is_err());
        assert!(validate_url_for_fetch("file:///etc/passwd").is_err());
        assert!(validate_url_for_fetch("gopher://example.com/").is_err());
        assert!(validate_url_for_fetch("javascript:alert(1)").is_err());
    }

    #[test]
    fn rejects_canonical_loopback_and_private() {
        for url in [
            "http://127.0.0.1/admin",
            "http://[::1]/admin",
            "http://localhost/admin",
            "http://10.0.0.1/",
            "http://172.16.0.1/",
            "http://172.31.255.255/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
        ] {
            assert!(
                validate_url_for_fetch(url).is_err(),
                "expected reject for {url}"
            );
        }
    }

    /// All of these are common SSRF bypass tricks. The WHATWG URL parser
    /// normalizes IPv4 short forms before we ever get to the private-range
    /// check, so the guard catches them.
    #[test]
    fn rejects_ipv4_short_forms() {
        for url in [
            "http://127.1/",
            "http://2130706433/",
            "http://0177.0.0.1/",
            "http://0x7f.0.0.1/",
            "http://0/",
        ] {
            assert!(
                validate_url_for_fetch(url).is_err(),
                "expected reject for {url}"
            );
        }
    }

    /// IPv6 unspecified / IPv4-mapped / NAT64 — all reach an IPv4 endpoint
    /// on the wire even though the literal looks IPv6.
    #[test]
    fn rejects_ipv6_embedded_ipv4_paths_to_private() {
        for url in [
            "http://[::]/",
            "http://[::ffff:127.0.0.1]/",
            "http://[::ffff:7f00:1]/",
            "http://[::ffff:10.0.0.1]/",
            "http://[::ffff:169.254.169.254]/",
            "http://[64:ff9b::7f00:1]/",
            "http://[ff02::1]/", // multicast
        ] {
            assert!(
                validate_url_for_fetch(url).is_err(),
                "expected reject for {url}"
            );
        }
    }

    #[test]
    fn rejects_trailing_dot_fqdn() {
        assert!(validate_url_for_fetch("http://localhost./").is_err());
        assert!(validate_url_for_fetch("http://metadata.google.internal./").is_err());
    }

    #[test]
    fn rejects_metadata_hostnames() {
        assert!(validate_url_for_fetch("http://metadata.google.internal/").is_err());
        assert!(validate_url_for_fetch("http://metadata.azure.com/").is_err());
        assert!(validate_url_for_fetch("http://myserver.local/").is_err());
    }

    #[test]
    fn rejects_carrier_nat_and_protocol_ranges() {
        // 100.64.0.0/10
        assert!(validate_url_for_fetch("http://100.64.0.1/").is_err());
        assert!(validate_url_for_fetch("http://100.127.255.255/").is_err());
        assert!(validate_url_for_fetch("http://100.128.0.1/").is_ok()); // outside CGN
                                                                        // 192.0.0.0/24
        assert!(validate_url_for_fetch("http://192.0.0.1/").is_err());
        assert!(validate_url_for_fetch("http://192.0.1.1/").is_ok());
        // multicast / reserved
        assert!(validate_url_for_fetch("http://224.0.0.1/").is_err());
        assert!(validate_url_for_fetch("http://255.255.255.255/").is_err());
    }

    #[test]
    fn ipv4_172_16_boundary() {
        assert!(validate_url_for_fetch("http://172.15.0.1/").is_ok());
        assert!(validate_url_for_fetch("http://172.16.0.1/").is_err());
        assert!(validate_url_for_fetch("http://172.31.0.1/").is_err());
        assert!(validate_url_for_fetch("http://172.32.0.1/").is_ok());
    }

    /// Userinfo (`user@host`) does not change the host. The host is the
    /// part after the `@`.
    #[test]
    fn userinfo_does_not_fool_host_check() {
        assert!(validate_url_for_fetch("http://attacker.com@127.0.0.1/").is_err());
        assert!(validate_url_for_fetch("http://attacker.com@example.com/").is_ok());
    }

    // -------- fetch_url_bytes ---------------------------------------------

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn fetch_url_bytes_rejects_blocked_url_before_dialing() {
        // No mock server stood up — if the SSRF guard didn't fire we'd see a
        // connection error, not the scheme/host rejection.
        let err = fetch_url_bytes("http://127.0.0.1/", 1024, &[])
            .await
            .unwrap_err();
        match &err {
            FetchError::Rejected(msg) => assert!(
                msg.contains("private/reserved"),
                "expected SSRF guard message, got: {msg}"
            ),
            other => panic!("expected Rejected, got: {other:?}"),
        }

        let err = fetch_url_bytes("file:///etc/passwd", 1024, &[])
            .await
            .unwrap_err();
        match &err {
            FetchError::Rejected(msg) => {
                assert!(msg.contains("scheme"), "expected scheme reject, got: {msg}")
            }
            other => panic!("expected Rejected, got: {other:?}"),
        }
    }

    /// Regression for the redirect-bypass attack: a public URL that
    /// `validate_url_for_fetch` accepts at entry returns 302 to a
    /// private/loopback host. Without the `Policy::custom` revalidator
    /// reqwest's default 10-redirect follower would happily fetch the
    /// IMDS / loopback target. With it, the redirect is refused.
    #[tokio::test]
    async fn fetch_url_bytes_refuses_redirect_to_private_host() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        // Hand-rolled HTTP/1.1 server that 302s to http://127.0.0.1:1/
        // — a private IPv4 the entry-time guard would have refused had
        // it been the original URL. The listener binds on 127.0.0.1:0
        // (the test fixture is the redirect ORIGIN, not the redirect
        // TARGET — the target is what must be refused).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let resp = b"HTTP/1.1 302 Found\r\n\
                    Location: http://127.0.0.1:1/loot\r\n\
                    Content-Length: 0\r\n\
                    \r\n";
                let _ = sock.write_all(resp).await;
                let _ = sock.shutdown().await;
            }
        });

        // The original URL is a literal IPv4 that `validate_url_for_fetch`
        // would refuse — so we can't use it as the entry-point. Instead
        // we have to bypass the entry-time guard by calling the
        // unchecked-on-entry path with a private origin, but we still
        // want the redirect-policy revalidator to fire. Use the safe
        // client directly with a pre-validated URL skip.
        //
        // For the entry-point check: the test fixture itself binds
        // 127.0.0.1:0, so the entry-time guard would also refuse. To
        // exercise ONLY the redirect policy, drive the safe_fetch_client
        // directly with the bound listener URL.
        let client = safe_fetch_client();
        let url = format!("http://{addr}/start");
        let resp = client.get(&url).send().await;
        let _ = handle.await;
        match resp {
            Err(e) => {
                let msg = e.to_string();
                assert!(
                    msg.contains("SSRF") || msg.contains("redirect"),
                    "redirect rejection should mention SSRF or redirect, got: {msg}"
                );
            }
            Ok(r) => panic!(
                "expected redirect to be refused, got status {} (redirected to a private host!)",
                r.status()
            ),
        }
    }

    #[tokio::test]
    async fn fetch_url_bytes_returns_body_within_cap() {
        let server = MockServer::start().await;
        let body = b"hello world".to_vec();
        Mock::given(method("GET"))
            .and(path("/file"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "text/plain; charset=utf-8")
                    .set_body_bytes(body.clone()),
            )
            .mount(&server)
            .await;

        let client = new_client();
        let url = format!("{}/file", server.uri());
        let (got, content_type) = fetch_url_bytes_unchecked(&client, &url, 1024, &[])
            .await
            .unwrap();
        assert_eq!(got, body);
        // The Content-Type header is surfaced verbatim — caller is
        // responsible for stripping `;` parameters and validating the
        // prefix. Asserting raw form pins that contract.
        assert_eq!(content_type.as_deref(), Some("text/plain; charset=utf-8"));
    }

    #[tokio::test]
    async fn fetch_url_bytes_rejects_content_length_over_cap() {
        let server = MockServer::start().await;
        let body = vec![0u8; 4096];
        Mock::given(method("GET"))
            .and(path("/big"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
            .mount(&server)
            .await;

        let client = new_client();
        let url = format!("{}/big", server.uri());
        let err = fetch_url_bytes_unchecked(&client, &url, 1024, &[])
            .await
            .unwrap_err();
        match err {
            FetchError::TooLarge {
                actual: Some(len),
                limit,
            } => {
                assert_eq!(len, 4096);
                assert_eq!(limit, 1024);
            }
            other => panic!("expected TooLarge {{ actual: Some(..), .. }}, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_url_bytes_rejects_streamed_body_over_cap() {
        // Stand up a hand-rolled chunked-transfer server: no Content-Length
        // is emitted, so the pre-check is bypassed and the streaming
        // accumulator must catch the overrun. (wiremock is not used here
        // because hyper rejects responses whose advertised Content-Length
        // disagrees with the body bytes — there is no clean way through
        // wiremock to produce a "lying" length response.)
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                // Drain the request — anything until \r\n\r\n.
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let header = b"HTTP/1.1 200 OK\r\n\
                    Content-Type: application/octet-stream\r\n\
                    Transfer-Encoding: chunked\r\n\
                    \r\n";
                let _ = sock.write_all(header).await;
                // First chunk: 600 bytes of 'x'.
                let _ = sock.write_all(b"258\r\n").await; // 0x258 = 600
                let _ = sock.write_all(&vec![b'x'; 600]).await;
                let _ = sock.write_all(b"\r\n").await;
                // Second chunk: another 600 bytes (total 1200).
                let _ = sock.write_all(b"258\r\n").await;
                let _ = sock.write_all(&vec![b'x'; 600]).await;
                let _ = sock.write_all(b"\r\n").await;
                // End-of-chunks.
                let _ = sock.write_all(b"0\r\n\r\n").await;
                let _ = sock.shutdown().await;
            }
        });

        let client = new_client();
        let url = format!("http://{addr}/");
        // Cap = 800: pre-check has nothing to check (no Content-Length),
        // streaming accumulator sees the real 1200 bytes and rejects.
        let err = fetch_url_bytes_unchecked(&client, &url, 800, &[])
            .await
            .unwrap_err();
        match err {
            FetchError::TooLarge {
                actual: None,
                limit,
            } => assert_eq!(limit, 800),
            other => panic!("expected TooLarge {{ actual: None, .. }}, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_url_bytes_returns_status_error_on_4xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = new_client();
        let url = format!("{}/missing", server.uri());
        let err = fetch_url_bytes_unchecked(&client, &url, 1024, &[])
            .await
            .unwrap_err();
        match &err {
            FetchError::Failed(msg) => {
                assert!(msg.contains("404"), "expected 404 in error, got: {msg}")
            }
            other => panic!("expected Failed, got: {other:?}"),
        }
    }

    /// Regression for the Tier-4 dedup: confirm that `extra_headers`
    /// attached at the public `fetch_url_bytes_*` boundary actually
    /// reach the wire. Without this, the dedup would silently drop
    /// Matrix's MSC3916 `Authorization: Bearer …` and inbound media
    /// downloads would 401 on `enable_authenticated_media` homeservers.
    #[tokio::test]
    async fn fetch_url_bytes_unchecked_attaches_extra_headers() {
        use wiremock::matchers::header;

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/secret"))
            .and(header("Authorization", "Bearer t0p-s3cret"))
            .and(header("X-Extra", "yes"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .mount(&server)
            .await;

        let client = new_client();
        let url = format!("{}/secret", server.uri());
        let headers = vec![
            ("Authorization".to_string(), "Bearer t0p-s3cret".to_string()),
            ("X-Extra".to_string(), "yes".to_string()),
        ];
        let (got, _) = fetch_url_bytes_unchecked(&client, &url, 1024, &headers)
            .await
            .expect("auth'd request must succeed");
        assert_eq!(got, b"ok");
    }

    // -------- channel proxy override --------------------------------------

    #[test]
    fn proxied_client_none_builds_without_error() {
        // No proxy set — must behave identically to `new_client()`.
        let _client = new_proxied_client(None).expect("None proxy should always build");
    }

    #[test]
    fn proxied_client_accepts_http_scheme() {
        let _client = new_proxied_client(Some("http://127.0.0.1:8080"))
            .expect("http:// proxy URL should be accepted by reqwest");
    }

    #[test]
    fn proxied_client_accepts_https_scheme() {
        let _client = new_proxied_client(Some("https://proxy.example.com:3128"))
            .expect("https:// proxy URL should be accepted by reqwest");
    }

    #[test]
    fn proxied_client_accepts_socks5_scheme() {
        // Requires reqwest's `socks` feature — workspace Cargo.toml
        // enables it. If this regresses to "not enabled", the URL gets
        // refused at parse time with a clear "scheme not supported"
        // reqwest error.
        let _client = new_proxied_client(Some("socks5://127.0.0.1:1080"))
            .expect("socks5:// proxy URL requires the `socks` reqwest feature");
        let _client = new_proxied_client(Some("socks5h://127.0.0.1:1080"))
            .expect("socks5h:// proxy URL requires the `socks` reqwest feature");
    }

    #[test]
    fn proxied_client_accepts_proxy_with_userinfo() {
        // reqwest stores the auth out-of-band and uses it for Proxy-Authorization.
        let _client = new_proxied_client(Some("http://user:pass@proxy.example.com:3128"))
            .expect("user:pass@host should be accepted");
    }

    #[test]
    fn proxied_client_rejects_garbage_url() {
        let err = new_proxied_client(Some("not a url")).expect_err("garbage must error");
        match err {
            ChannelProxyError::InvalidUrl { value, .. } => {
                assert_eq!(value, "not a url");
            }
            other => panic!("expected InvalidUrl, got: {other:?}"),
        }
    }

    #[test]
    fn proxied_client_rejects_javascript_scheme() {
        // `javascript:` does not parse as an absolute URL with an
        // authority component — reqwest's `Proxy::all` rejects it at
        // parse time.
        let err = new_proxied_client(Some("javascript:alert(1)")).expect_err("must reject");
        match err {
            ChannelProxyError::InvalidUrl { value, source } => {
                assert_eq!(value, "javascript:alert(1)");
                // Operator-friendly error: source must say SOMETHING
                // useful. Don't pin the exact wording — it's reqwest's.
                assert!(
                    !source.is_empty(),
                    "InvalidUrl.source must carry the underlying reqwest reason"
                );
            }
            other => panic!("expected InvalidUrl, got: {other:?}"),
        }
    }

    /// Smoke test for the per-adapter WS-bypass WARN helper (#4795
    /// follow-up). The behavioural assertion lives in each adapter test
    /// (`*_with_proxy_some_sets_ws_bypass_warn_flag`) — this just pins
    /// that the helper itself does not panic and accepts the three
    /// adapter names the start() paths pass in.
    #[test]
    fn warn_ws_proxy_bypass_smoke() {
        warn_ws_proxy_bypass("slack");
        warn_ws_proxy_bypass("discord");
        warn_ws_proxy_bypass("mattermost");
    }

    #[test]
    fn channel_proxy_error_display_lists_accepted_schemes() {
        // Operators need to know which schemes are valid in the error
        // text — pin that contract so the message stays useful.
        let err = ChannelProxyError::InvalidUrl {
            value: "bogus".into(),
            source: "boom".into(),
        };
        let s = err.to_string();
        assert!(s.contains("http"), "schemes list must mention http: {s}");
        assert!(
            s.contains("socks5"),
            "schemes list must mention socks5: {s}"
        );
    }

    // -------- ct_eq --------------------------------------------------------

    #[test]
    fn ct_eq_matches_only_exact_bytes() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
        assert!(!ct_eq(b"abc", b""));
        assert!(ct_eq(b"", b""));
    }
}
