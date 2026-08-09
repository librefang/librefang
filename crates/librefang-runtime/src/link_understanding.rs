//! Link understanding — auto-extract and summarize URLs from messages.

use tracing::warn;

/// Configuration for link understanding (re-exported from types).
pub use librefang_types::media::LinkConfig;

/// Summary of a fetched link.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LinkSummary {
    pub url: String,
    pub title: Option<String>,
    /// Content preview, max 2000 chars.
    pub content_preview: String,
    pub content_type: String,
}

/// Extract URLs from text, with a network-free SSRF prefilter.
///
/// Returns up to `max` valid, unique URLs that do not contain private literal targets or restricted hostnames.
/// Any later fetch must still use the shared WebFetch SSRF resolver and pinned transport to validate DNS at connect time.
pub fn extract_urls(text: &str, max: usize) -> Vec<String> {
    // Simple but effective URL regex
    let url_pattern = regex_lite::Regex::new(
        r#"https?://[^\s<>\[\](){}|\\^`"']+[^\s<>\[\](){}|\\^`"'.,;:!?\-)]"#,
    )
    .expect("URL regex is valid");

    let mut seen = std::collections::HashSet::new();
    let mut urls = Vec::new();

    for m in url_pattern.find_iter(text) {
        let url = m.as_str().to_string();

        // Deduplicate
        if !seen.insert(url.clone()) {
            continue;
        }

        // SECURITY: SSRF check — reject private IPs and metadata endpoints
        if is_private_url(&url) {
            warn!("Rejected private/SSRF URL: {}", url);
            continue;
        }

        urls.push(url);
        if urls.len() >= max {
            break;
        }
    }

    urls
}

/// Check URL syntax, userinfo, restricted hostnames, and literal IP ranges.
/// This intentionally performs no DNS resolution because link extraction is synchronous and does not initiate network I/O.
fn is_private_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => parsed,
        _ => return true,
    };
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return true;
    }

    match parsed.host() {
        Some(url::Host::Ipv4(ip)) => is_blocked_literal_ip(std::net::IpAddr::V4(ip)),
        Some(url::Host::Ipv6(ip)) => is_blocked_literal_ip(std::net::IpAddr::V6(ip)),
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local")
                || host.ends_with(".internal")
                || matches!(
                    host,
                    "metadata.google.internal" | "metadata.aws.internal" | "instance-data"
                )
        }
        None => true,
    }
}

fn is_blocked_literal_ip(ip: std::net::IpAddr) -> bool {
    if let std::net::IpAddr::V6(v6) = ip {
        let segments = v6.segments();
        if segments[0] == 0x2002 {
            let embedded = std::net::Ipv4Addr::new(
                (segments[1] >> 8) as u8,
                segments[1] as u8,
                (segments[2] >> 8) as u8,
                segments[2] as u8,
            );
            if is_blocked_literal_ip(std::net::IpAddr::V4(embedded)) {
                return true;
            }
        }
        // NAT64 well-known prefix (64:ff9b::/96, RFC 6052) embeds an IPv4 address in the low 32 bits.
        // `crate::web_fetch::is_private_ip` / `is_cloud_metadata_ip` already unwrap this internally for RFC1918 / link-local / metadata ranges, but loopback (127.0.0.0/8) and unspecified (0.0.0.0) are only checked against `canonical` below, which this branch does not populate for NAT64.
        // Recurse explicitly so e.g. `64:ff9b::7f00:1` (embedded 127.0.0.1) is not missed.
        if let Some(embedded) = crate::web_fetch::extract_nat64_well_known(&v6) {
            if is_blocked_literal_ip(std::net::IpAddr::V4(embedded)) {
                return true;
            }
        }
    }

    let canonical = match ip {
        std::net::IpAddr::V6(v6) => v6
            .to_ipv4()
            .map(std::net::IpAddr::V4)
            .unwrap_or(std::net::IpAddr::V6(v6)),
        std::net::IpAddr::V4(_) => ip,
    };

    if canonical.is_loopback()
        || canonical.is_unspecified()
        || crate::web_fetch::is_private_ip(&canonical)
        || crate::web_fetch::is_cloud_metadata_ip(&canonical)
    {
        return true;
    }

    match canonical {
        // Treat the complete "this network" block as non-public.
        // URL parsers and OS resolvers may interpret alternate spellings as these values.
        std::net::IpAddr::V4(v4) => v4.octets()[0] == 0,
        std::net::IpAddr::V6(_) => false,
    }
}

/// Build link context string to inject into agent messages.
///
/// Returns None if no links found or link understanding is disabled.
pub fn build_link_context(text: &str, config: &LinkConfig) -> Option<String> {
    if !config.enabled {
        return None;
    }

    let urls = extract_urls(text, config.max_links);
    if urls.is_empty() {
        return None;
    }

    let mut context = String::from("\n\n[Link Context - URLs detected in message]\n");
    for url in &urls {
        context.push_str(&format!("- {url}\n"));
    }
    context.push_str(
        "Use web_fetch to retrieve content from these URLs if relevant to the user's request.\n",
    );
    Some(context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_urls_basic() {
        let text = "Check out https://example.com and http://test.org/page";
        let urls = extract_urls(text, 10);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("example.com"));
        assert!(urls[1].contains("test.org"));
    }

    #[test]
    fn test_extract_urls_dedup() {
        let text = "Visit https://example.com and also https://example.com again";
        let urls = extract_urls(text, 10);
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn test_extract_urls_max_limit() {
        let text = "https://a.com https://b.com https://c.com https://d.com https://e.com";
        let urls = extract_urls(text, 3);
        assert_eq!(urls.len(), 3);
    }

    #[test]
    fn test_extract_urls_no_urls() {
        let text = "No URLs here, just plain text.";
        let urls = extract_urls(text, 10);
        assert!(urls.is_empty());
    }

    #[test]
    fn test_ssrf_localhost_blocked() {
        assert!(is_private_url("http://localhost/admin"));
        assert!(is_private_url("http://127.0.0.1:8080/secret"));
        assert!(is_private_url("http://0.0.0.0/"));
        assert!(is_private_url("http://[::1]/"));
    }

    #[test]
    fn test_ssrf_userinfo_cannot_hide_private_host() {
        assert!(is_private_url("http://evil.example@127.0.0.1/admin"));
        assert!(is_private_url("http://user:pass@10.0.0.1/secret"));
    }

    #[test]
    fn test_ssrf_blocks_complete_local_and_link_local_ranges() {
        assert!(is_private_url("http://127.0.0.2/admin"));
        assert!(is_private_url("http://127.255.255.254/admin"));
        assert!(is_private_url("http://0.1.2.3/admin"));
        assert!(is_private_url("http://169.254.42.42/latest"));
    }

    #[test]
    fn test_ssrf_blocks_embedded_and_alternate_ipv4_encodings() {
        assert!(is_private_url("http://[::ffff:127.0.0.1]/admin"));
        assert!(is_private_url("http://[::7f00:1]/admin"));
        assert!(is_private_url("http://[2002:7f00:1::]/admin"));
        assert!(is_private_url("http://2130706433/admin"));
        assert!(is_private_url("http://0x7f000001/admin"));
        assert!(is_private_url("http://0177.0.0.1/admin"));
    }

    #[test]
    fn test_ssrf_blocks_ipv6_unique_local_and_link_local() {
        // fc00::/7 (ULA, RFC 4193) and fe80::/10 (link-local) are matched by
        // `crate::web_fetch::is_private_ip`'s IPv6 branch.
        assert!(is_private_url("http://[fc00::1]/admin"));
        assert!(is_private_url("http://[fdff:ffff:ffff:ffff::1]/admin"));
        assert!(is_private_url("http://[fe80::1]/admin"));
        assert!(!is_private_url("http://[2001:4860:4860::8888]/admin")); // public (Google DNS)
    }

    #[test]
    fn test_ssrf_blocks_nat64_embedded_ips() {
        // 64:ff9b::/96 (RFC 6052) embeds an IPv4 address in the low 32 bits.
        // `is_private_ip`/`is_cloud_metadata_ip` unwrap this internally for RFC1918/link-local/metadata ranges, but loopback and unspecified are only caught by the explicit recursion added above — regression coverage for that gap.
        assert!(is_private_url("http://[64:ff9b::7f00:1]/admin")); // embeds 127.0.0.1
        assert!(is_private_url("http://[64:ff9b::]/admin")); // embeds 0.0.0.0
        assert!(is_private_url("http://[64:ff9b::a00:1]/admin")); // embeds 10.0.0.1
        assert!(is_private_url("http://[64:ff9b::a9fe:a9fe]/admin")); // embeds 169.254.169.254
        assert!(!is_private_url("http://[64:ff9b::808:808]/admin")); // embeds 8.8.8.8 (public)
    }

    #[test]
    fn test_ssrf_private_ranges_blocked() {
        assert!(is_private_url("http://10.0.0.1/internal"));
        assert!(is_private_url("http://192.168.1.1/admin"));
        assert!(is_private_url("http://172.16.0.1/secret"));
        assert!(is_private_url("http://172.31.255.255/data"));
    }

    #[test]
    fn test_ssrf_metadata_blocked() {
        assert!(is_private_url("http://169.254.169.254/latest/meta-data/"));
        assert!(is_private_url("http://metadata.google.internal/"));
        assert!(is_private_url("http://metadata.google.internal./"));
        assert!(is_private_url("http://localhost./admin"));
    }

    #[test]
    fn test_ssrf_public_allowed() {
        assert!(!is_private_url("https://example.com/page"));
        assert!(!is_private_url("https://api.github.com/repos"));
        assert!(!is_private_url("https://docs.rust-lang.org/"));
    }

    #[test]
    fn test_ssrf_172_non_private() {
        // 172.32.x.x is NOT private
        assert!(!is_private_url("http://172.32.0.1/ok"));
        assert!(!is_private_url("http://172.15.0.1/ok"));
    }

    #[test]
    fn test_extract_urls_filters_private() {
        let text =
            "Public: https://example.com Private: http://localhost/admin http://192.168.1.1/secret";
        let urls = extract_urls(text, 10);
        assert_eq!(urls.len(), 1);
        assert!(urls[0].contains("example.com"));
    }

    #[test]
    fn test_build_link_context_disabled() {
        let config = LinkConfig {
            enabled: false,
            ..Default::default()
        };
        let result = build_link_context("https://example.com", &config);
        assert!(result.is_none());
    }

    #[test]
    fn test_build_link_context_enabled() {
        let config = LinkConfig {
            enabled: true,
            ..Default::default()
        };
        let result = build_link_context("Check https://example.com", &config);
        assert!(result.is_some());
        let ctx = result.unwrap();
        assert!(ctx.contains("example.com"));
        assert!(ctx.contains("Link Context"));
    }

    #[test]
    fn test_build_link_context_no_urls() {
        let config = LinkConfig {
            enabled: true,
            ..Default::default()
        };
        let result = build_link_context("No URLs here", &config);
        assert!(result.is_none());
    }
}
