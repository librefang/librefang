//! Enhanced web fetch with SSRF protection, HTML→Markdown extraction,
//! in-memory caching, and external content markers.
//!
//! Pipeline: SSRF check → cache lookup → HTTP GET → detect HTML →
//! html_to_markdown() → truncate → wrap_external_content() → cache → return

use crate::str_utils::safe_truncate_str;
use crate::web_cache::WebCache;
use crate::web_content::{html_to_markdown, wrap_external_content};
use librefang_types::config::WebFetchConfig;
use std::net::{IpAddr, ToSocketAddrs};
use std::sync::Arc;
use tracing::debug;

/// Enhanced web fetch engine with SSRF protection and readability extraction.
pub struct WebFetchEngine {
    config: WebFetchConfig,
    cache: Arc<WebCache>,
}

impl WebFetchEngine {
    /// Create a new fetch engine from config with a shared cache.
    pub fn new(config: WebFetchConfig, cache: Arc<WebCache>) -> Self {
        Self { config, cache }
    }

    /// Read-only access to the engine's config.
    /// Used by sibling modules (e.g. `web_fetch_to_file`) that share the
    /// SSRF / DNS-pin pipeline but enforce a different size cap.
    pub(crate) fn config(&self) -> &WebFetchConfig {
        &self.config
    }

    /// Build a per-request reqwest client pinned to the SSRF-validated IPs.
    ///
    /// Uses the resolved addresses from [`check_ssrf`] to configure DNS
    /// pinning on the builder, preventing DNS-rebinding TOCTOU attacks.
    ///
    /// Auto-redirects are **disabled** (`Policy::none`). A `Policy::custom`
    /// that re-runs `check_ssrf` on each hop is not enough: reqwest
    /// re-resolves the redirect target's hostname through its own connector,
    /// and the DNS pin only covers the *original* hostname — so a cross-host
    /// redirect to a TTL-0 rebinding name would be validated against one
    /// resolution and connected against another (the DNS-rebinding TOCTOU,
    /// #3563/#3782). Redirects are instead followed manually by
    /// [`Self::send_with_pinned_redirects`], which re-validates AND re-pins
    /// every hop so the IP we checked is the IP we connect to.
    pub(crate) fn pinned_client(&self, resolution: SsrfResolution) -> reqwest::Client {
        let builder = crate::http_client::proxied_client_builder()
            .timeout(std::time::Duration::from_secs(self.config.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .gzip(true)
            .deflate(true)
            .brotli(true);
        resolution
            .pin_dns(builder)
            .build()
            .expect("HTTP client build")
    }

    /// Send a request, following redirects manually with a fresh SSRF check +
    /// DNS pin on **every** hop.
    ///
    /// This closes the DNS-rebinding window that `Policy::custom` cannot:
    /// each hop resolves and validates its target, then connects to those
    /// exact validated IPs via [`SsrfResolution::pin_dns`]. The validated
    /// resolution and the connecting resolution are therefore identical for
    /// every hop, not just the first.
    pub(crate) async fn send_with_pinned_redirects(
        &self,
        method: &str,
        url: &str,
        headers: Option<&serde_json::Map<String, serde_json::Value>>,
        body: Option<&str>,
    ) -> Result<reqwest::Response, String> {
        const MAX_REDIRECTS: usize = 10;
        let mut current_url = url.to_string();
        let mut current_method = method.to_uppercase();
        let mut current_body = body.map(|s| s.to_string());
        // Latches true once any hop crosses origin (scheme / host / port).
        // Mirrors reqwest's `remove_sensitive_headers`: once a redirect leaves
        // the original origin, the caller's credential-bearing headers
        // (Authorization / Cookie / Proxy-Authorization) must never be
        // re-attached — including on any further same-origin hops on the new
        // host — or they leak to an attacker-controlled redirect target.
        let mut credentials_stripped = false;

        for _ in 0..=MAX_REDIRECTS {
            // SSRF check + DNS pin happen together per hop so the IP we
            // validated is the IP the pinned client connects to.
            let resolution = check_ssrf(&current_url, &self.config.ssrf_allowed_hosts)?;
            let client = self.pinned_client(resolution);
            let mut req = match current_method.as_str() {
                "POST" => client.post(&current_url),
                "PUT" => client.put(&current_url),
                "PATCH" => client.patch(&current_url),
                "DELETE" => client.delete(&current_url),
                "HEAD" => client.head(&current_url),
                _ => client.get(&current_url),
            };
            req = req.header(
                "User-Agent",
                format!("Mozilla/5.0 (compatible; {})", crate::USER_AGENT),
            );
            if let Some(hdrs) = headers {
                for (k, v) in hdrs {
                    if let Some(val) = v.as_str() {
                        if credentials_stripped && is_sensitive_redirect_header(k) {
                            continue;
                        }
                        req = req.header(k.as_str(), val);
                    }
                }
            }
            if let Some(ref b) = current_body {
                if b.trim_start().starts_with('{') || b.trim_start().starts_with('[') {
                    req = req.header("Content-Type", "application/json");
                }
                req = req.body(b.clone());
            }

            let resp = req
                .send()
                .await
                .map_err(|e| format!("HTTP request failed: {e}"))?;
            let status = resp.status();
            if !status.is_redirection() {
                return Ok(resp);
            }

            // Follow the redirect manually (the pinned client does not).
            let Some(location) = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
            else {
                // Redirect status with no usable Location — hand the response
                // back to the caller rather than loop forever.
                return Ok(resp);
            };
            let base =
                url::Url::parse(&current_url).map_err(|e| format!("invalid request URL: {e}"))?;
            let next = base
                .join(location)
                .map_err(|e| format!("invalid redirect Location '{location}': {e}"))?;
            // Strip credential-bearing headers on any cross-origin hop, exactly
            // like reqwest's built-in `remove_sensitive_headers` (which the
            // manual `Policy::none` loop bypasses). Compare against the hop we
            // just made (`base`); the flag latches so the headers stay gone for
            // the remainder of the chain even if a later hop returns to the
            // original origin.
            if crosses_origin(&base, &next) {
                credentials_stripped = true;
            }
            // 301/302/303 downgrade to GET and drop the body (browser
            // behaviour); 307/308 preserve method and body.
            if (301..=303).contains(&status.as_u16()) {
                current_method = "GET".to_string();
                current_body = None;
            }
            current_url = next.to_string();
        }
        Err(format!("Too many redirects (cap: {MAX_REDIRECTS})"))
    }

    /// Fetch a URL with full security pipeline (GET only, for backwards compat).
    pub async fn fetch(&self, url: &str) -> Result<String, String> {
        self.fetch_with_options(url, "GET", None, None).await
    }

    /// Fetch a URL with configurable HTTP method, headers, and body.
    pub async fn fetch_with_options(
        &self,
        url: &str,
        method: &str,
        headers: Option<&serde_json::Map<String, serde_json::Value>>,
        body: Option<&str>,
    ) -> Result<String, String> {
        let method_upper = method.to_uppercase();

        // Step 1: SSRF protection — resolve DNS once and validate IPs
        let resolution = check_ssrf(url, &self.config.ssrf_allowed_hosts)?;

        // Step 2: Cache lookup (only for GET)
        let cache_key = format!("fetch:{}:{}", method_upper, url);
        if method_upper == "GET" {
            if let Some(cached) = self.cache.get(&cache_key) {
                debug!(url, "Fetch cache hit");
                return Ok(cached);
            }
        }

        // Step 3: Send via the manual redirect loop, which re-runs the SSRF
        // check + DNS pin on every hop (the client built per hop connects
        // only to the IPs validated for that hop). The `resolution` from
        // step 1 was the early-fail / cache-key gate; the loop re-validates
        // the first hop too so there is a single source of truth.
        let _ = resolution;
        let resp = self
            .send_with_pinned_redirects(&method_upper, url, headers, body)
            .await?;

        let status = resp.status();

        // Check response size
        if let Some(len) = resp.content_length() {
            if len > self.config.max_response_bytes as u64 {
                return Err(format!(
                    "Response too large: {} bytes (max {})",
                    len, self.config.max_response_bytes
                ));
            }
        }

        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let resp_body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response body: {e}"))?;

        // Step 4: For GET requests, detect HTML and convert to Markdown.
        // For non-GET (API calls), return raw body — don't mangle JSON/XML responses.
        let processed = if method_upper == "GET"
            && self.config.readability
            && is_html(&content_type, &resp_body)
        {
            let markdown = html_to_markdown(&resp_body);
            if markdown.trim().is_empty() {
                resp_body
            } else {
                markdown
            }
        } else {
            resp_body
        };

        // Step 5: Truncate (char-boundary-safe to avoid panics on multi-byte UTF-8)
        let truncated = if processed.len() > self.config.max_chars {
            format!(
                "{}... [truncated, {} total chars]",
                safe_truncate_str(&processed, self.config.max_chars),
                processed.len()
            )
        } else {
            processed
        };

        // Step 6: Wrap with external content markers
        let result = format!(
            "HTTP {status}\n\n{}",
            wrap_external_content(url, &truncated)
        );

        // Step 7: Cache (only GET responses)
        if method_upper == "GET" {
            self.cache.put(cache_key, result.clone());
        }

        Ok(result)
    }
}

/// Detect if content is HTML based on Content-Type header or body sniffing.
fn is_html(content_type: &str, body: &str) -> bool {
    if content_type.contains("text/html") || content_type.contains("application/xhtml") {
        return true;
    }
    // Sniff: check if body starts with HTML-like content
    let trimmed = body.trim_start();
    trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<!doctype")
        || trimmed.starts_with("<html")
}

/// Whether following `from` → `to` leaves the original origin.
///
/// "Origin" is scheme + host + port, matching reqwest's
/// `remove_sensitive_headers` cross-host test. A change in any of the three
/// means caller-supplied credential headers must not be replayed to `to`.
fn crosses_origin(from: &url::Url, to: &url::Url) -> bool {
    from.host_str() != to.host_str()
        || from.port_or_known_default() != to.port_or_known_default()
        || from.scheme() != to.scheme()
}

/// Whether `name` is a credential-bearing header reqwest strips on a
/// cross-origin redirect (case-insensitive). Mirrors the set removed by
/// reqwest's `remove_sensitive_headers`.
fn is_sensitive_redirect_header(name: &str) -> bool {
    const SENSITIVE: [&str; 5] = [
        "authorization",
        "cookie",
        "cookie2",
        "proxy-authorization",
        "www-authenticate",
    ];
    SENSITIVE
        .iter()
        .any(|sensitive| name.eq_ignore_ascii_case(sensitive))
}

// ---------------------------------------------------------------------------
// SSRF Protection (replicates host_functions.rs logic for builtin tools)
// ---------------------------------------------------------------------------

/// Result of a successful SSRF check: the hostname and its resolved socket
/// addresses.  Callers should use [`SsrfResolution::pin_dns`] to build an
/// HTTP client that connects to the *already-validated* IPs, preventing
/// DNS-rebinding TOCTOU attacks.
pub struct SsrfResolution {
    /// The hostname extracted from the URL (without port).
    pub hostname: String,
    /// All resolved socket addresses (guaranteed to be non-private).
    pub resolved: Vec<std::net::SocketAddr>,
}

impl SsrfResolution {
    /// Apply the pinned DNS resolution to a [`reqwest::ClientBuilder`] so
    /// the actual HTTP request connects to the IPs we already validated.
    pub fn pin_dns(self, mut builder: reqwest::ClientBuilder) -> reqwest::ClientBuilder {
        for addr in &self.resolved {
            builder = builder.resolve(&self.hostname, *addr);
        }
        builder
    }
}

/// Check if a URL targets a private/internal network resource.
/// Blocks localhost, metadata endpoints, and private IPs.
/// Must run BEFORE any network I/O.
///
/// `allowed_hosts` is a list of CIDRs (e.g. `"10.0.0.0/8"`), glob hostname
/// patterns (e.g. `"*.internal.example.com"`), or literal IPs/hostnames that
/// are exempt from the private-IP block.  Cloud metadata ranges
/// (`169.254.0.0/16`, `100.64.0.0/10`) remain unconditionally blocked even
/// when an entry matches.
///
/// Returns the resolved addresses on success so that callers can pin DNS
/// and avoid TOCTOU / DNS-rebinding attacks.
pub fn check_ssrf(url: &str, allowed_hosts: &[String]) -> Result<SsrfResolution, String> {
    // Only allow http:// and https:// schemes
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("Only http:// and https:// URLs are allowed".to_string());
    }

    // Reject userinfo (`user[:pass]@host`) in the authority. Defense in
    // depth (issue #5141): `extract_host` splits on `://` then `/`, so
    // `http://allowed.example.com@127.0.0.1/` yields the host
    // `allowed.example.com@127.0.0.1` and today merely fails DNS — but
    // that is one parser tweak away from exploitable (the #3527 shape).
    // `is_ssrf_target` on the WASM path and the MCP-OAuth path both reject
    // userinfo explicitly; align this path so the policy is uniform and
    // the URL never reaches reqwest's connection-pool key with credentials
    // embedded. Scope the `@` scan to the authority only (a `?query=a@b`
    // or `#frag@x` is legitimate and must not be rejected).
    if let Some((_, rest)) = url.split_once("://") {
        let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        if rest[..authority_end].contains('@') {
            return Err("SSRF blocked: URLs with userinfo are not permitted".to_string());
        }
    }

    let host = extract_host(url);
    // For IPv6 bracket notation like [::1]:80, extract [::1] as hostname
    let hostname = if host.starts_with('[') {
        host.find(']').map(|i| &host[..=i]).unwrap_or(&host)
    } else {
        host.split(':').next().unwrap_or(&host)
    };

    // Hostname-based blocklist (catches metadata endpoints — always blocked,
    // even when the hostname appears in allowed_hosts).
    let blocked = [
        "localhost",
        "ip6-localhost",
        "metadata.google.internal",
        "metadata.aws.internal",
        "instance-data",
        "169.254.169.254",
        "100.100.100.200", // Alibaba Cloud IMDS
        "192.0.0.192",     // Azure IMDS alternative
        "0.0.0.0",
        "::1",
        "[::1]",
    ];
    if blocked.contains(&hostname) {
        return Err(format!("SSRF blocked: {hostname} is a restricted hostname"));
    }

    // Resolve DNS and check every returned IP
    let port = if url.starts_with("https") { 443 } else { 80 };
    let socket_addr = format!("{hostname}:{port}");
    let mut resolved = Vec::new();
    match socket_addr.to_socket_addrs() {
        Ok(addrs) => {
            for addr in addrs {
                // Canonicalise IPv4-mapped IPv6 (::ffff:X.X.X.X) before any
                // safety check. The OS transparently connects these to the
                // embedded IPv4 target, so leaving them as IPv6 lets an
                // attacker reach loopback / private / cloud-metadata IPs via
                // the IPv6 form (e.g. [::ffff:169.254.169.254]) which the
                // v6-only branches of is_private_ip / is_cloud_metadata_ip
                // do not recognise.
                let ip = canonical_ip(&addr.ip());
                if ip.is_loopback() || ip.is_unspecified() || is_private_ip(&ip) {
                    // Before rejecting, check the allowlist — but cloud metadata
                    // ranges are unconditionally blocked regardless of allowlist.
                    if !is_cloud_metadata_ip(&ip) && is_host_allowed(hostname, &ip, allowed_hosts) {
                        resolved.push(addr);
                        continue;
                    }
                    return Err(format!(
                        "SSRF blocked: {hostname} resolves to private IP {ip}"
                    ));
                }
                resolved.push(addr);
            }
        }
        Err(e) => {
            return Err(format!(
                "SSRF blocked: DNS resolution failed for {hostname}: {e}"
            ));
        }
    }
    if resolved.is_empty() {
        return Err(format!(
            "SSRF blocked: DNS resolution returned no addresses for {hostname}"
        ));
    }

    Ok(SsrfResolution {
        hostname: hostname.to_string(),
        resolved,
    })
}

/// Returns true if the IP is a cloud metadata or CGNAT range that must be
/// blocked unconditionally, even when the host appears in the allowlist.
///
/// Covers:
/// - `169.254.0.0/16` — link-local / AWS EC2 metadata
/// - `100.64.0.0/10`  — CGNAT (also used by Alibaba Cloud IMDS at 100.100.100.200)
pub(crate) fn is_cloud_metadata_ip(ip: &IpAddr) -> bool {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // 169.254.0.0/16
            o[0] == 169 && o[1] == 254
            // 100.64.0.0/10: first octet 100, second octet 64..=127
            || o[0] == 100 && (o[1] & 0xC0) == 64
        }
        IpAddr::V6(_) => false,
    }
}

/// Unwrap IPv4-mapped IPv6 (`::ffff:X.X.X.X`) and the NAT64 well-known
/// prefix (`64:ff9b::/96`, RFC 6052) to the IPv4 address the connection
/// will actually reach. All other addresses are returned unchanged.
///
/// IPv4-mapped is translated by the OS itself; NAT64 is translated by a
/// network gateway when one is deployed. Both forms must be unwrapped
/// before SSRF checks so an attacker can't smuggle loopback / RFC1918 /
/// cloud-metadata IPs through them.
///
/// Custom NAT64 prefixes (RFC 6052 §2.2) are NOT handled — those are
/// per-environment configuration and would need an explicit setting.
pub(crate) fn canonical_ip(ip: &IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return IpAddr::V4(v4);
            }
            if let Some(v4) = extract_nat64_well_known(v6) {
                return IpAddr::V4(v4);
            }
            IpAddr::V6(*v6)
        }
        IpAddr::V4(_) => *ip,
    }
}

/// Extract the embedded IPv4 from an address in the NAT64 well-known
/// prefix `64:ff9b::/96` (RFC 6052). Returns `None` for any other address.
pub(crate) fn extract_nat64_well_known(v6: &std::net::Ipv6Addr) -> Option<std::net::Ipv4Addr> {
    let segments = v6.segments();
    // 96-bit prefix: 0064:ff9b:0000:0000:0000:0000::/96
    if segments[0] != 0x0064
        || segments[1] != 0xff9b
        || segments[2] != 0
        || segments[3] != 0
        || segments[4] != 0
        || segments[5] != 0
    {
        return None;
    }
    // Embedded IPv4 lives in the low 32 bits (segments 6 and 7).
    Some(std::net::Ipv4Addr::new(
        (segments[6] >> 8) as u8,
        (segments[6] & 0xff) as u8,
        (segments[7] >> 8) as u8,
        (segments[7] & 0xff) as u8,
    ))
}

/// Check whether a hostname or resolved IP matches any entry in `allowed_hosts`.
///
/// Entry formats:
/// - `"10.0.0.0/8"`           — CIDR; matched against the resolved `ip`
/// - `"*.internal.example.com"` — glob prefix wildcard; matched against `hostname`
/// - `"10.1.2.3"` / `"svc.local"` — literal IP or hostname exact match
fn is_host_allowed(hostname: &str, ip: &IpAddr, allowed_hosts: &[String]) -> bool {
    for entry in allowed_hosts {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        // CIDR notation (contains '/')
        if entry.contains('/') {
            if let Ok(matched) = cidr_contains(entry, ip) {
                if matched {
                    return true;
                }
            }
            continue;
        }
        // Glob hostname pattern (starts with '*').
        // Bare "*" is rejected — it would match every hostname and silently
        // bypass all private-IP protection, which is almost certainly a
        // misconfiguration rather than intent.
        if let Some(suffix) = entry.strip_prefix('*') {
            if suffix.is_empty() {
                continue; // reject "*" — too broad
            }
            // "*.foo.com" -> suffix = ".foo.com"
            if hostname.ends_with(suffix) {
                return true;
            }
            continue;
        }
        // Literal IP match
        if let Ok(entry_ip) = entry.parse::<IpAddr>() {
            if entry_ip == *ip {
                return true;
            }
            continue;
        }
        // Literal hostname match
        if entry.eq_ignore_ascii_case(hostname) {
            return true;
        }
    }
    false
}

/// Parse a CIDR string like `"10.0.0.0/8"` and check if `ip` falls within it.
/// Only IPv4 CIDRs are supported; IPv4-in-IPv6 and pure IPv6 CIDRs are not.
fn cidr_contains(cidr: &str, ip: &IpAddr) -> Result<bool, ()> {
    let (addr_str, prefix_str) = cidr.split_once('/').ok_or(())?;
    let prefix_len: u32 = prefix_str.parse().map_err(|_| ())?;
    match (addr_str.parse::<IpAddr>(), ip) {
        (Ok(IpAddr::V4(net_addr)), IpAddr::V4(v4)) => {
            if prefix_len > 32 {
                return Err(());
            }
            let mask = if prefix_len == 0 {
                0u32
            } else {
                !0u32 << (32 - prefix_len)
            };
            Ok((u32::from_be_bytes(net_addr.octets()) & mask)
                == (u32::from_be_bytes(v4.octets()) & mask))
        }
        (Ok(IpAddr::V6(net_addr)), IpAddr::V6(v6)) => {
            if prefix_len > 128 {
                return Err(());
            }
            let net_bits = u128::from_be_bytes(net_addr.octets());
            let ip_bits = u128::from_be_bytes(v6.octets());
            let mask = if prefix_len == 0 {
                0u128
            } else {
                !0u128 << (128 - prefix_len)
            };
            Ok((net_bits & mask) == (ip_bits & mask))
        }
        _ => Ok(false),
    }
}

/// Check if an IP address is in a private range.
pub(crate) fn is_private_ip(ip: &IpAddr) -> bool {
    match canonical_ip(ip) {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            matches!(
                octets,
                [10, ..] | [172, 16..=31, ..] | [192, 168, ..] | [169, 254, ..]
            )
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            (segments[0] & 0xfe00) == 0xfc00 || (segments[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Extract host:port from a URL.
fn extract_host(url: &str) -> String {
    if let Some(after_scheme) = url.split("://").nth(1) {
        let host_port = after_scheme.split('/').next().unwrap_or(after_scheme);
        // Handle IPv6 bracket notation: [::1]:8080
        if host_port.starts_with('[') {
            // Extract [addr]:port or [addr]
            if let Some(bracket_end) = host_port.find(']') {
                let ipv6_host = &host_port[..=bracket_end]; // includes brackets
                let after_bracket = &host_port[bracket_end + 1..];
                if let Some(port) = after_bracket.strip_prefix(':') {
                    return format!("{ipv6_host}:{port}");
                }
                let default_port = if url.starts_with("https") { 443 } else { 80 };
                return format!("{ipv6_host}:{default_port}");
            }
        }
        if host_port.contains(':') {
            host_port.to_string()
        } else if url.starts_with("https") {
            format!("{host_port}:443")
        } else {
            format!("{host_port}:80")
        }
    } else {
        url.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::str_utils::safe_truncate_str;

    #[test]
    fn test_truncate_multibyte_no_panic() {
        // Simulate a gzip-decoded response containing multi-byte UTF-8
        // (Chinese, Japanese, emoji — common on international finance sites).
        // Old code: &s[..max] panics when max lands inside a multi-byte char.
        let content = "\u{4f60}\u{597d}\u{4e16}\u{754c}!"; // "你好世界!" = 13 bytes
                                                           // Truncate at byte 7 — lands inside the 3rd Chinese char (bytes 6..9).
                                                           // safe_truncate_str walks back to byte 6, returning "你好".
        let truncated = safe_truncate_str(content, 7);
        assert_eq!(truncated, "\u{4f60}\u{597d}");
        assert!(truncated.len() <= 7);
    }

    #[test]
    fn test_truncate_emoji_no_panic() {
        let content = "\u{1f4b0}\u{1f4c8}\u{1f4b9}"; // 💰📈💹 = 12 bytes
                                                     // Truncate at byte 5 — lands inside the 2nd emoji (bytes 4..8).
        let truncated = safe_truncate_str(content, 5);
        assert_eq!(truncated, "\u{1f4b0}"); // 4 bytes
    }

    #[test]
    fn test_ssrf_blocks_localhost() {
        assert!(check_ssrf("http://localhost/admin", &[]).is_err());
        assert!(check_ssrf("http://localhost:8080/api", &[]).is_err());
    }

    #[tokio::test]
    async fn test_manual_redirect_loop_follows_and_repins() {
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // wiremock binds 127.0.0.1; allow the loopback range so the per-hop
        // check_ssrf permits it (cloud-metadata stays blocked regardless).
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(ResponseTemplate::new(200).set_body_string("FINAL-OK"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/final"))
            .mount(&server)
            .await;

        let config = WebFetchConfig {
            ssrf_allowed_hosts: vec!["127.0.0.0/8".to_string()],
            ..Default::default()
        };
        let engine = WebFetchEngine::new(
            config,
            Arc::new(WebCache::new(std::time::Duration::from_secs(60))),
        );

        // The manual redirect loop must follow the 302 to /final, re-running
        // check_ssrf + pin on the second hop, and return the final body.
        let out = engine
            .fetch(&format!("{}/redirect", server.uri()))
            .await
            .expect("manual redirect loop should follow the 302");
        assert!(
            out.contains("FINAL-OK"),
            "expected final body after redirect, got: {out}"
        );
    }

    #[tokio::test]
    async fn test_cross_host_redirect_strips_credentials() {
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // Two distinct origins (different loopback ports). origin_a issues a
        // 302 to origin_b; origin_b must NOT see the caller's credential
        // headers — they would otherwise leak across the origin boundary.
        let origin_b = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(ResponseTemplate::new(200).set_body_string("FINAL-OK"))
            .mount(&origin_b)
            .await;

        let origin_a = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/final", origin_b.uri())),
            )
            .mount(&origin_a)
            .await;

        let config = WebFetchConfig {
            ssrf_allowed_hosts: vec!["127.0.0.0/8".to_string()],
            ..Default::default()
        };
        let engine = WebFetchEngine::new(
            config,
            Arc::new(WebCache::new(std::time::Duration::from_secs(60))),
        );

        let mut hdrs = serde_json::Map::new();
        hdrs.insert(
            "Authorization".to_string(),
            serde_json::json!("Bearer secret-token"),
        );
        hdrs.insert("Cookie".to_string(), serde_json::json!("session=abc123"));
        hdrs.insert(
            "Proxy-Authorization".to_string(),
            serde_json::json!("Basic xyz"),
        );
        hdrs.insert("X-Custom".to_string(), serde_json::json!("keepme"));

        let out = engine
            .fetch_with_options(
                &format!("{}/redirect", origin_a.uri()),
                "GET",
                Some(&hdrs),
                None,
            )
            .await
            .expect("cross-host redirect should still succeed");
        assert!(out.contains("FINAL-OK"), "got: {out}");

        let received = origin_b
            .received_requests()
            .await
            .expect("origin_b should record the request");
        assert_eq!(received.len(), 1, "origin_b hit exactly once");
        let h = &received[0].headers;
        assert!(
            !h.contains_key("authorization"),
            "Authorization must be stripped on cross-host redirect"
        );
        assert!(
            !h.contains_key("cookie"),
            "Cookie must be stripped on cross-host redirect"
        );
        assert!(
            !h.contains_key("proxy-authorization"),
            "Proxy-Authorization must be stripped on cross-host redirect"
        );
        // Non-sensitive caller headers are still forwarded.
        assert_eq!(
            h.get("x-custom").map(|v| v.to_str().unwrap()),
            Some("keepme"),
            "non-sensitive headers must survive the redirect"
        );
    }

    #[tokio::test]
    async fn test_same_host_redirect_preserves_credentials() {
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // A same-origin redirect (relative Location → same host:port) must keep
        // the caller's credentials — only cross-origin hops strip them.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/final"))
            .respond_with(ResponseTemplate::new(200).set_body_string("FINAL-OK"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(ResponseTemplate::new(302).insert_header("location", "/final"))
            .mount(&server)
            .await;

        let config = WebFetchConfig {
            ssrf_allowed_hosts: vec!["127.0.0.0/8".to_string()],
            ..Default::default()
        };
        let engine = WebFetchEngine::new(
            config,
            Arc::new(WebCache::new(std::time::Duration::from_secs(60))),
        );

        let mut hdrs = serde_json::Map::new();
        hdrs.insert(
            "Authorization".to_string(),
            serde_json::json!("Bearer secret-token"),
        );
        hdrs.insert("Cookie".to_string(), serde_json::json!("session=abc123"));

        let out = engine
            .fetch_with_options(
                &format!("{}/redirect", server.uri()),
                "GET",
                Some(&hdrs),
                None,
            )
            .await
            .expect("same-host redirect should succeed");
        assert!(out.contains("FINAL-OK"), "got: {out}");

        let received = server
            .received_requests()
            .await
            .expect("server should record requests");
        // Find the request to /final (the redirect target).
        let final_req = received
            .iter()
            .find(|r| r.url.path() == "/final")
            .expect("the /final hop must have been made");
        assert_eq!(
            final_req
                .headers
                .get("authorization")
                .map(|v| v.to_str().unwrap()),
            Some("Bearer secret-token"),
            "Authorization must survive a same-host redirect"
        );
        assert_eq!(
            final_req.headers.get("cookie").map(|v| v.to_str().unwrap()),
            Some("session=abc123"),
            "Cookie must survive a same-host redirect"
        );
    }

    #[tokio::test]
    async fn test_redirect_to_metadata_ip_blocked_on_later_hop() {
        use std::sync::Arc;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        // ATTACK: a public host 302s to the cloud-metadata endpoint. The
        // per-hop check_ssrf must block the redirect target on hop 2, even
        // though hop 1 (the wiremock loopback) was allowlisted.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/redirect"))
            .respond_with(ResponseTemplate::new(302).insert_header(
                "location",
                "http://169.254.169.254/latest/meta-data/iam/security-credentials/",
            ))
            .mount(&server)
            .await;

        let config = WebFetchConfig {
            ssrf_allowed_hosts: vec!["127.0.0.0/8".to_string()],
            ..Default::default()
        };
        let engine = WebFetchEngine::new(
            config,
            Arc::new(WebCache::new(std::time::Duration::from_secs(60))),
        );

        let err = engine
            .fetch(&format!("{}/redirect", server.uri()))
            .await
            .expect_err("redirect to metadata IP must be blocked");
        assert!(
            err.contains("169.254.169.254") || err.to_lowercase().contains("restricted"),
            "expected SSRF block on the metadata hop, got: {err}"
        );
    }

    #[test]
    fn test_crosses_origin_helper() {
        let a = url::Url::parse("http://example.com/a").unwrap();
        let same = url::Url::parse("http://example.com/b").unwrap();
        let other_host = url::Url::parse("http://evil.com/a").unwrap();
        let other_port = url::Url::parse("http://example.com:8443/a").unwrap();
        let other_scheme = url::Url::parse("https://example.com/a").unwrap();
        assert!(!crosses_origin(&a, &same));
        assert!(crosses_origin(&a, &other_host));
        assert!(crosses_origin(&a, &other_port));
        assert!(crosses_origin(&a, &other_scheme));
    }

    #[test]
    fn test_is_sensitive_redirect_header() {
        assert!(is_sensitive_redirect_header("Authorization"));
        assert!(is_sensitive_redirect_header("authorization"));
        assert!(is_sensitive_redirect_header("COOKIE"));
        assert!(is_sensitive_redirect_header("Proxy-Authorization"));
        assert!(!is_sensitive_redirect_header("X-Custom"));
        assert!(!is_sensitive_redirect_header("Content-Type"));
    }

    #[test]
    fn test_ssrf_blocks_private_ip() {
        use std::net::IpAddr;
        assert!(is_private_ip(&"10.0.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"172.16.0.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"192.168.1.1".parse::<IpAddr>().unwrap()));
        assert!(is_private_ip(&"169.254.169.254".parse::<IpAddr>().unwrap()));
    }

    #[test]
    fn test_ssrf_blocks_metadata() {
        assert!(check_ssrf("http://169.254.169.254/latest/meta-data/", &[]).is_err());
        assert!(check_ssrf("http://metadata.google.internal/computeMetadata/v1/", &[]).is_err());
    }

    #[test]
    fn test_ssrf_allows_public() {
        assert!(!is_private_ip(
            &"8.8.8.8".parse::<std::net::IpAddr>().unwrap()
        ));
        assert!(!is_private_ip(
            &"1.1.1.1".parse::<std::net::IpAddr>().unwrap()
        ));
    }

    #[test]
    fn test_ssrf_blocks_non_http() {
        assert!(check_ssrf("file:///etc/passwd", &[]).is_err());
        assert!(check_ssrf("ftp://internal.corp/data", &[]).is_err());
        assert!(check_ssrf("gopher://evil.com", &[]).is_err());
    }

    // --- #5141: userinfo rejection (defense-in-depth, #3527 shape) ---
    #[test]
    fn test_ssrf_rejects_userinfo() {
        // ATTACK: host-confusion via userinfo. `extract_host` splits on
        // `://` then `/`, so the host would be
        // `allowed.example.com@127.0.0.1` — today that merely fails DNS,
        // but it must be rejected explicitly so a future parser change
        // can't make it exploitable, and so credentials never reach
        // reqwest's connection-pool key.
        let err = match check_ssrf("http://allowed.example.com@127.0.0.1/", &[]) {
            Err(e) => e,
            Ok(_) => panic!("userinfo URL must be rejected"),
        };
        assert!(err.contains("userinfo"), "got: {err}");
        assert!(check_ssrf("http://user:pass@169.254.169.254/", &[]).is_err());
        assert!(check_ssrf("https://a@b@evil.example.com/", &[]).is_err());
    }

    #[test]
    fn test_ssrf_userinfo_check_does_not_false_positive_on_query_or_fragment() {
        // POSITIVE: an `@` in the path / query / fragment is legitimate
        // and must NOT be treated as userinfo. (DNS for example.com may
        // fail in CI sandboxes, so only assert it is not the userinfo
        // rejection.)
        for u in [
            "http://example.com/path?email=a@b.com",
            "http://example.com/#section@2",
            "http://example.com/users/a@b",
        ] {
            if let Err(e) = check_ssrf(u, &[]) {
                assert!(
                    !e.contains("userinfo"),
                    "URL {u} wrongly rejected as userinfo: {e}"
                );
            }
        }
    }

    #[test]
    fn test_ssrf_blocks_cloud_metadata() {
        // Alibaba Cloud IMDS
        assert!(check_ssrf("http://100.100.100.200/latest/meta-data/", &[]).is_err());
        // Azure IMDS alternative
        assert!(check_ssrf("http://192.0.0.192/metadata/instance", &[]).is_err());
    }

    #[test]
    fn test_ssrf_blocks_zero_ip() {
        assert!(check_ssrf("http://0.0.0.0/", &[]).is_err());
    }

    #[test]
    fn test_ssrf_blocks_ipv6_localhost() {
        assert!(check_ssrf("http://[::1]/admin", &[]).is_err());
        assert!(check_ssrf("http://[::1]:8080/api", &[]).is_err());
    }

    #[test]
    fn test_ssrf_blocks_ipv4_mapped_ipv6_loopback() {
        // OS transparently connects ::ffff:127.0.0.1 to 127.0.0.1.
        // The standard is_loopback() check on IpAddr::V6 returns false, so
        // without canonicalisation this slipped past SSRF protection.
        assert!(check_ssrf("http://[::ffff:127.0.0.1]/", &[]).is_err());
        assert!(check_ssrf("http://[::ffff:7f00:1]/", &[]).is_err());
    }

    #[test]
    fn test_ssrf_blocks_ipv4_mapped_ipv6_metadata() {
        // 169.254.169.254 expressed as an IPv4-mapped IPv6 address reaches
        // the AWS EC2 instance metadata service on real hosts.
        assert!(check_ssrf("http://[::ffff:169.254.169.254]/", &[]).is_err());
        assert!(check_ssrf("http://[::ffff:a9fe:a9fe]/", &[]).is_err());
        assert!(check_ssrf("http://[0:0:0:0:0:ffff:169.254.169.254]/", &[]).is_err());
    }

    #[test]
    fn test_ssrf_blocks_ipv4_mapped_ipv6_private() {
        assert!(check_ssrf("http://[::ffff:10.0.0.1]/", &[]).is_err());
        assert!(check_ssrf("http://[::ffff:192.168.1.1]/", &[]).is_err());
    }

    #[test]
    fn test_canonical_ip_unwraps_mapped() {
        use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
        let mapped: IpAddr = IpAddr::V6("::ffff:169.254.169.254".parse::<Ipv6Addr>().unwrap());
        assert_eq!(
            canonical_ip(&mapped),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))
        );
        // Real IPv6 is left alone.
        let real_v6: IpAddr = "2001:db8::1".parse().unwrap();
        assert_eq!(canonical_ip(&real_v6), real_v6);
    }

    #[test]
    fn test_is_private_ip_recognises_mapped_v6() {
        use std::net::IpAddr;
        let mapped_private: IpAddr = "::ffff:10.0.0.1".parse().unwrap();
        assert!(is_private_ip(&mapped_private));
        let mapped_link_local: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
        assert!(is_private_ip(&mapped_link_local));
    }

    #[test]
    fn test_is_cloud_metadata_ip_recognises_mapped_v6() {
        use std::net::IpAddr;
        let mapped_imds: IpAddr = "::ffff:169.254.169.254".parse().unwrap();
        assert!(is_cloud_metadata_ip(&mapped_imds));
        let mapped_cgnat: IpAddr = "::ffff:100.64.0.1".parse().unwrap();
        assert!(is_cloud_metadata_ip(&mapped_cgnat));
    }

    #[test]
    fn test_extract_nat64_well_known() {
        use std::net::{Ipv4Addr, Ipv6Addr};
        // 169.254.169.254 embedded → AWS IMDS via NAT64
        let nat64_imds: Ipv6Addr = "64:ff9b::a9fe:a9fe".parse().unwrap();
        assert_eq!(
            extract_nat64_well_known(&nat64_imds),
            Some(Ipv4Addr::new(169, 254, 169, 254))
        );
        // 10.0.0.1 embedded → RFC1918 via NAT64
        let nat64_priv: Ipv6Addr = "64:ff9b::0a00:0001".parse().unwrap();
        assert_eq!(
            extract_nat64_well_known(&nat64_priv),
            Some(Ipv4Addr::new(10, 0, 0, 1))
        );
        // Real IPv6 outside the prefix → None
        let real_v6: Ipv6Addr = "2001:db8::a9fe:a9fe".parse().unwrap();
        assert_eq!(extract_nat64_well_known(&real_v6), None);
    }

    #[test]
    fn test_canonical_ip_unwraps_nat64() {
        use std::net::{IpAddr, Ipv4Addr};
        let nat64_imds: IpAddr = "64:ff9b::a9fe:a9fe".parse().unwrap();
        assert_eq!(
            canonical_ip(&nat64_imds),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))
        );
    }

    #[test]
    fn test_ssrf_blocks_nat64_metadata() {
        // 169.254.169.254 reachable via NAT64 well-known prefix when a NAT64
        // gateway is on path (e.g. cloud VPC with IPv6 transition setup).
        assert!(check_ssrf("http://[64:ff9b::a9fe:a9fe]/", &[]).is_err());
    }

    #[test]
    fn test_ssrf_blocks_nat64_loopback() {
        // 127.0.0.1 = 7f00:0001 in the NAT64 low 32 bits.
        assert!(check_ssrf("http://[64:ff9b::7f00:1]/", &[]).is_err());
    }

    #[test]
    fn test_ssrf_blocks_nat64_private() {
        // 10.0.0.1 and 192.168.1.1 via NAT64.
        assert!(check_ssrf("http://[64:ff9b::a00:1]/", &[]).is_err());
        assert!(check_ssrf("http://[64:ff9b::c0a8:101]/", &[]).is_err());
    }

    #[test]
    fn test_is_private_ip_recognises_nat64_v6() {
        use std::net::IpAddr;
        let nat64_priv: IpAddr = "64:ff9b::a00:1".parse().unwrap();
        assert!(is_private_ip(&nat64_priv));
        let nat64_link_local: IpAddr = "64:ff9b::a9fe:a9fe".parse().unwrap();
        assert!(is_private_ip(&nat64_link_local));
    }

    #[test]
    fn test_is_cloud_metadata_ip_recognises_nat64_v6() {
        use std::net::IpAddr;
        let nat64_imds: IpAddr = "64:ff9b::a9fe:a9fe".parse().unwrap();
        assert!(is_cloud_metadata_ip(&nat64_imds));
        let nat64_alibaba: IpAddr = "64:ff9b::6464:64c8".parse().unwrap();
        assert!(is_cloud_metadata_ip(&nat64_alibaba));
    }

    #[test]
    fn test_extract_host_ipv6() {
        let h = extract_host("http://[::1]:8080/path");
        assert_eq!(h, "[::1]:8080");

        let h2 = extract_host("https://[::1]/path");
        assert_eq!(h2, "[::1]:443");

        let h3 = extract_host("http://[::1]/path");
        assert_eq!(h3, "[::1]:80");
    }

    #[test]
    fn test_cidr_contains() {
        use std::net::IpAddr;
        let ip_10: IpAddr = "10.1.2.3".parse().unwrap();
        let ip_192: IpAddr = "192.168.0.1".parse().unwrap();
        let ip_8: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(cidr_contains("10.0.0.0/8", &ip_10).unwrap());
        assert!(!cidr_contains("10.0.0.0/8", &ip_192).unwrap());
        assert!(!cidr_contains("10.0.0.0/8", &ip_8).unwrap());
        // /32 exact
        assert!(cidr_contains("10.1.2.3/32", &ip_10).unwrap());
        assert!(!cidr_contains("10.1.2.4/32", &ip_10).unwrap());
        // /0 matches all
        assert!(cidr_contains("0.0.0.0/0", &ip_8).unwrap());
    }

    #[test]
    fn test_is_host_allowed_cidr() {
        use std::net::IpAddr;
        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        let allowed = vec!["10.0.0.0/8".to_string()];
        assert!(is_host_allowed("svc.internal", &ip, &allowed));
        let not_allowed: IpAddr = "8.8.8.8".parse().unwrap();
        assert!(!is_host_allowed("dns.google", &not_allowed, &allowed));
    }

    #[test]
    fn test_is_host_allowed_glob() {
        use std::net::IpAddr;
        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        let allowed = vec!["*.internal.example.com".to_string()];
        assert!(is_host_allowed("svc.internal.example.com", &ip, &allowed));
        assert!(!is_host_allowed("evil.example.com", &ip, &allowed));
    }

    #[test]
    fn test_is_host_allowed_literal_ip() {
        use std::net::IpAddr;
        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        let allowed = vec!["10.1.2.3".to_string()];
        assert!(is_host_allowed("anything", &ip, &allowed));
        let other: IpAddr = "10.1.2.4".parse().unwrap();
        assert!(!is_host_allowed("anything", &other, &allowed));
    }

    #[test]
    fn test_cloud_metadata_blocked_even_when_allowlisted() {
        // 169.254.169.254 is in hostname blocklist so check_ssrf returns Err before
        // reaching IP resolution, but the is_cloud_metadata_ip guard also covers
        // cases where a hostname resolves to a link-local IP.
        use std::net::IpAddr;
        let link_local: IpAddr = "169.254.0.1".parse().unwrap();
        let cgnat: IpAddr = "100.64.0.1".parse().unwrap();
        assert!(is_cloud_metadata_ip(&link_local));
        assert!(is_cloud_metadata_ip(&cgnat));
        // Regular private IPs are NOT cloud metadata
        let priv_ip: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(!is_cloud_metadata_ip(&priv_ip));
    }

    #[test]
    fn test_bare_glob_star_is_rejected() {
        use std::net::IpAddr;
        let ip: IpAddr = "10.1.2.3".parse().unwrap();
        // Bare "*" must NOT match everything — it would bypass all private-IP protection.
        let allowed = vec!["*".to_string()];
        assert!(
            !is_host_allowed("any.internal.host", &ip, &allowed),
            "bare '*' must not be a universal allowlist entry"
        );
        // "*" with a dot suffix still works normally.
        let allowed_dot = vec!["*.internal.example.com".to_string()];
        assert!(is_host_allowed(
            "svc.internal.example.com",
            &ip,
            &allowed_dot
        ));
        assert!(!is_host_allowed("evil.com", &ip, &allowed_dot));
    }
}
