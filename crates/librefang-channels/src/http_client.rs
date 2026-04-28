//! Shared HTTP client builder with fallback CA roots and SSRF guard.

use std::net::IpAddr;
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

/// Validate that a URL from a channel payload is safe to fetch server-side.
///
/// Rejects:
/// - Non-http/https schemes
/// - IP literals or hostnames that are private/loopback/link-local/metadata-service
///   addresses (prevents SSRF)
///
/// This is a best-effort guard against *obvious* SSRF vectors. It checks the
/// host string directly without performing DNS resolution (DNS-rebind attacks
/// must be mitigated at the network layer or with a resolving SSRF proxy).
pub fn validate_url_for_fetch(url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("scheme '{scheme}' is not allowed; only http/https")),
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    // If the host is an IP literal, check directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_private_ip(ip) {
            return Err(format!("host '{host}' resolves to a private/reserved address"));
        }
        return Ok(());
    }

    // Reject obviously private hostnames without DNS resolution.
    let host_lower = host.to_ascii_lowercase();
    if is_private_hostname(&host_lower) {
        return Err(format!("host '{host}' is a reserved or private hostname"));
    }

    Ok(())
}

/// Returns `true` if `ip` is in any private, loopback, link-local, or
/// cloud-metadata address range.
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // Loopback: 127.0.0.0/8
            if o[0] == 127 {
                return true;
            }
            // Private: 10.0.0.0/8
            if o[0] == 10 {
                return true;
            }
            // Private: 172.16.0.0/12
            if o[0] == 172 && (16..=31).contains(&o[1]) {
                return true;
            }
            // Private: 192.168.0.0/16
            if o[0] == 192 && o[1] == 168 {
                return true;
            }
            // Link-local: 169.254.0.0/16  (includes AWS/GCP/Azure metadata at 169.254.169.254)
            if o[0] == 169 && o[1] == 254 {
                return true;
            }
            // Unspecified: 0.0.0.0/8
            if o[0] == 0 {
                return true;
            }
            false
        }
        IpAddr::V6(v6) => {
            // Loopback ::1
            if v6.is_loopback() {
                return true;
            }
            let segs = v6.segments();
            // Link-local fe80::/10
            if (segs[0] & 0xffc0) == 0xfe80 {
                return true;
            }
            // Unique local fc00::/7  (covers fd00::/8 etc.)
            if (segs[0] & 0xfe00) == 0xfc00 {
                return true;
            }
            // Unspecified ::
            if v6.is_unspecified() {
                return true;
            }
            false
        }
    }
}

/// Returns `true` for hostnames that are obviously private without requiring DNS.
fn is_private_hostname(host: &str) -> bool {
    // localhost and variants
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    // .local mDNS names
    if host.ends_with(".local") {
        return true;
    }
    // Common internal hostname patterns
    if host == "metadata.google.internal"
        || host == "metadata"
        || host == "169.254.169.254"
    {
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_allows_public_https() {
        assert!(validate_url_for_fetch("https://example.com/image.png").is_ok());
        assert!(validate_url_for_fetch("http://cdn.example.org/file").is_ok());
    }

    #[test]
    fn test_validate_url_rejects_bad_scheme() {
        assert!(validate_url_for_fetch("ftp://example.com/file").is_err());
        assert!(validate_url_for_fetch("file:///etc/passwd").is_err());
    }

    #[test]
    fn test_validate_url_rejects_loopback() {
        assert!(validate_url_for_fetch("http://127.0.0.1/admin").is_err());
        assert!(validate_url_for_fetch("http://[::1]/admin").is_err());
        assert!(validate_url_for_fetch("http://localhost/admin").is_err());
    }

    #[test]
    fn test_validate_url_rejects_private_ranges() {
        assert!(validate_url_for_fetch("http://10.0.0.1/").is_err());
        assert!(validate_url_for_fetch("http://172.16.0.1/").is_err());
        assert!(validate_url_for_fetch("http://172.31.255.255/").is_err());
        assert!(validate_url_for_fetch("http://192.168.1.1/").is_err());
    }

    #[test]
    fn test_validate_url_rejects_metadata_service() {
        assert!(validate_url_for_fetch("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn test_validate_url_rejects_ipv6_unique_local() {
        assert!(validate_url_for_fetch("http://[fd00::1]/").is_err());
        assert!(validate_url_for_fetch("http://[fe80::1]/").is_err());
    }

    #[test]
    fn test_validate_url_rejects_private_hostname() {
        assert!(validate_url_for_fetch("http://metadata.google.internal/").is_err());
        assert!(validate_url_for_fetch("http://myserver.local/").is_err());
    }

    #[test]
    fn test_172_boundary() {
        // 172.15.x is public; 172.16.x – 172.31.x is private; 172.32.x is public
        assert!(validate_url_for_fetch("http://172.15.0.1/").is_ok());
        assert!(validate_url_for_fetch("http://172.16.0.1/").is_err());
        assert!(validate_url_for_fetch("http://172.31.0.1/").is_err());
        assert!(validate_url_for_fetch("http://172.32.0.1/").is_ok());
    }
}
