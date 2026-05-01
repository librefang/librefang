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
