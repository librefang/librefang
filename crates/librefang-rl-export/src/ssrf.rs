//! Server-Side Request Forgery (SSRF) egress allowlist for the RL
//! exporter's outbound base URLs.
//!
//! The exporter is the one component in the workspace explicitly
//! designed to exfiltrate bytes to operator-supplied URLs (the W&B /
//! Tinker / Atropos `base_url` overrides). That makes it the highest-
//! risk surface for SSRF: a user-controlled `base_url` value would
//! otherwise let a tenant route the rollout upload at the cloud
//! metadata IP (`169.254.169.254`), an internal monitoring host, or
//! loopback on the operator's machine.
//!
//! ## Why not reuse `librefang-runtime-mcp::is_ssrf_blocked_url`
//!
//! That helper exists and we keep its policy in step (loopback /
//! private / link-local / IMDS / non-http schemes / userinfo /
//! IPv4-mapped-and-NAT64 IPv6 are all rejected). But it lives behind a
//! Cargo dep on `librefang-runtime-mcp`, which transitively pulls
//! `librefang-kernel-handle`, the MCP discovery surface, and a
//! Tower-shaped middleware stack — adding ~30 crates to a leaf egress
//! exporter is the wrong shape (refs PR feedback: "Don't widen scope
//! into the kernel"). The block-set duplicated here mirrors that
//! helper's policy exactly; the two must change together.
//!
//! ## Two egress modes
//!
//! - [`EgressMode::Public`]: W&B and Tinker are cloud services.
//!   Loopback / private / link-local destinations are rejected
//!   outright — those addresses are never the legitimate upstream for
//!   a public training service.
//! - [`EgressMode::LoopbackOrPrivate`]: Atropos is a local FastAPI
//!   microservice. The exporter accepts loopback (`127.0.0.0/8`,
//!   `::1`) and RFC-1918 private (`10/8`, `172.16/12`, `192.168/16`)
//!   destinations but **rejects everything else**, including public
//!   destinations and link-local / IMDS. Atropos has no auth — exposing
//!   the producer to the public internet is the wrong shape entirely.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use url::Url;

use crate::error::ExportError;

/// Egress allowlist mode for an exporter's outbound base URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EgressMode {
    /// Cloud-service upstream (W&B, Tinker). Loopback / private /
    /// link-local destinations are rejected.
    Public,
    /// Local-microservice upstream (Atropos). Only loopback / RFC-1918
    /// destinations are accepted; public destinations are rejected.
    LoopbackOrPrivate,
}

/// Validate a caller-supplied outbound base URL against the egress
/// allowlist.
///
/// Returns `Ok(())` when the URL is safe to contact in the requested
/// mode, or [`ExportError::InvalidConfig`] with a human-readable
/// reason otherwise.
///
/// The check is purely on the URL string — no DNS resolution is
/// performed. A hostname that *resolves* to a blocked address (e.g.
/// `metadata.google.internal`) is rejected via the literal-hostname
/// list rather than via DNS so the test is deterministic and offline.
pub(crate) fn validate_egress_url(url_str: &str, mode: EgressMode) -> Result<(), ExportError> {
    let parsed = Url::parse(url_str)
        .map_err(|e| ExportError::InvalidConfig(format!("invalid base URL: {e}")))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(ExportError::InvalidConfig(format!(
                "base URL scheme '{scheme}' is not allowed (only http / https are accepted)"
            )));
        }
    }

    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(ExportError::InvalidConfig(
            "base URL must not contain userinfo (user[:pass]@host)".to_string(),
        ));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ExportError::InvalidConfig("base URL has no host component".to_string()))?;

    let is_blocked = host_is_blocked(host);
    let is_private_class = host_is_private_class(host);

    match mode {
        EgressMode::Public => {
            if is_blocked || is_private_class {
                return Err(ExportError::InvalidConfig(format!(
                    "base URL host '{host}' is on the SSRF block list (loopback / private / \
                     link-local destinations are rejected for cloud upstreams)"
                )));
            }
        }
        EgressMode::LoopbackOrPrivate => {
            if is_blocked && !is_private_class {
                // is_blocked includes link-local (169.254/16) and the
                // literal hostnames (`localhost`,
                // `metadata.google.internal`). is_private_class
                // covers loopback and RFC-1918. So the only thing
                // `is_blocked && !is_private_class` rejects here is
                // link-local + the hostname allowlist — exactly the
                // shapes we want to keep out even on the loopback
                // path (e.g. IMDS at 169.254.169.254).
                return Err(ExportError::InvalidConfig(format!(
                    "base URL host '{host}' is a link-local / metadata destination — \
                     rejected even on the loopback-allowed Atropos path"
                )));
            }
            if !is_private_class {
                return Err(ExportError::InvalidConfig(format!(
                    "base URL host '{host}' is a public destination — Atropos is a \
                     local-only microservice and only loopback / RFC-1918 hosts are accepted"
                )));
            }
        }
    }

    Ok(())
}

/// Is `host` on the broad SSRF block list (loopback / private / link-
/// local / known internal hostnames)? Mirrors the policy of
/// `librefang_runtime_mcp::mcp_oauth::is_ssrf_blocked_host` so both
/// helpers reject the same shapes.
fn host_is_blocked(host: &str) -> bool {
    let lower = host.trim_end_matches('.').to_lowercase();
    // IMDS hostnames are never a legitimate exporter backend, and
    // `localhost` / `ip6-localhost` / `ip6-loopback` are loopback
    // aliases. Kept in step with
    // `librefang_runtime_mcp::mcp_oauth::is_ssrf_blocked_host` and
    // `librefang_runtime::web_fetch::check_ssrf`, which block all of these.
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
                v6.is_loopback() || (segs[0] & 0xfe00) == 0xfc00 || (segs[0] & 0xffc0) == 0xfe80
            }
        };
    }
    false
}

/// Is `host` a loopback or RFC-1918 private address? (Subset of
/// `host_is_blocked` — link-local is excluded.)
fn host_is_private_class(host: &str) -> bool {
    let lower = host.trim_end_matches('.').to_lowercase();
    if matches!(
        lower.as_str(),
        "localhost" | "ip6-localhost" | "ip6-loopback"
    ) {
        return true;
    }
    let ip_str = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = ip_str.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => loopback_or_private_v4(v4),
            IpAddr::V6(v6) => {
                if let Some(v4) = ipv6_embedded_ipv4(v6) {
                    if loopback_or_private_v4(v4) {
                        return true;
                    }
                }
                v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00
            }
        };
    }
    false
}

fn blocked_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    // Cloud-metadata / unspecified pivots — never a legitimate operator backend.
    // Present here (the Public-mode block set) but deliberately absent from
    // `loopback_or_private_v4`, so they are rejected even on the Atropos
    // loopback path. Kept in step with
    // `librefang_runtime_mcp::mcp_oauth::is_ssrf_blocked_host`.
    (o[0] == 0 && o[1] == 0 && o[2] == 0 && o[3] == 0)          // 0.0.0.0 unspecified → loopback
        || (o[0] == 100 && (o[1] & 0xc0) == 64)                // 100.64.0.0/10 CGNAT (Alibaba IMDS)
        || (o[0] == 192 && o[1] == 0 && o[2] == 0 && o[3] == 192) // 192.0.0.192 Azure IMDS
        || o[0] == 127
        || o[0] == 10
        || (o[0] == 172 && (o[1] & 0xf0) == 16)
        || (o[0] == 192 && o[1] == 168)
        || (o[0] == 169 && o[1] == 254)
}

fn loopback_or_private_v4(v4: Ipv4Addr) -> bool {
    let o = v4.octets();
    o[0] == 127
        || o[0] == 10
        || (o[0] == 172 && (o[1] & 0xf0) == 16)
        || (o[0] == 192 && o[1] == 168)
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_mode_accepts_public_https() {
        assert!(validate_egress_url("https://api.wandb.ai", EgressMode::Public).is_ok());
        assert!(validate_egress_url(
            "https://tinker.thinkingmachines.dev/services/tinker-prod",
            EgressMode::Public
        )
        .is_ok());
    }

    #[test]
    fn public_mode_rejects_loopback() {
        // Cloud upstream pointed at loopback is always a misconfiguration
        // (or an SSRF attempt). Pin both the literal localhost and the
        // 127.0.0.1 / ::1 forms.
        assert!(validate_egress_url("http://localhost:8000", EgressMode::Public).is_err());
        assert!(validate_egress_url("http://127.0.0.1:8000", EgressMode::Public).is_err());
        assert!(validate_egress_url("http://[::1]:8000", EgressMode::Public).is_err());
        // ip6-localhost / ip6-loopback are loopback aliases (shipped in
        // /etc/hosts on common distros) that resolve to ::1 at request
        // time. The mirror (`librefang_runtime_mcp::mcp_oauth::
        // is_ssrf_blocked_host`) blocks both; this module must too.
        assert!(validate_egress_url("http://ip6-localhost:8000", EgressMode::Public).is_err());
        assert!(validate_egress_url("http://ip6-loopback:8000", EgressMode::Public).is_err());
    }

    #[test]
    fn public_mode_rejects_imds() {
        // 169.254.169.254 is the canonical AWS/GCP/Azure IMDS endpoint.
        // It must never be reachable as an export target.
        assert!(validate_egress_url("http://169.254.169.254/", EgressMode::Public).is_err());
        // GCP's metadata hostname form must also be blocked.
        assert!(
            validate_egress_url("http://metadata.google.internal/", EgressMode::Public).is_err()
        );
    }

    #[test]
    fn rejects_metadata_pivots_that_diverged_from_the_mirror() {
        // These were missing from the block set, diverging from the
        // `librefang_runtime_mcp` helper this module promises to mirror. Each is
        // a metadata/loopback pivot that must be rejected in BOTH modes —
        // including the Atropos loopback path, since none are RFC-1918 private.
        for url in [
            "http://0.0.0.0:8000/",          // unspecified → routes to loopback
            "http://100.100.100.200/",       // Alibaba Cloud IMDS (100.64/10 CGNAT)
            "http://192.0.0.192/",           // Azure IMDS alternative endpoint
            "http://metadata.aws.internal/", // AWS IMDS hostname
            "http://instance-data/",         // AWS IMDS hostname alias
        ] {
            assert!(
                validate_egress_url(url, EgressMode::Public).is_err(),
                "Public mode must reject {url}"
            );
            assert!(
                validate_egress_url(url, EgressMode::LoopbackOrPrivate).is_err(),
                "Atropos/loopback mode must also reject {url}"
            );
        }
    }

    #[test]
    fn public_mode_rejects_rfc1918() {
        assert!(validate_egress_url("http://10.0.0.1/", EgressMode::Public).is_err());
        assert!(validate_egress_url("http://192.168.1.1/", EgressMode::Public).is_err());
        assert!(validate_egress_url("http://172.16.0.1/", EgressMode::Public).is_err());
    }

    #[test]
    fn public_mode_rejects_ipv4_mapped_ipv6_loopback() {
        // ::ffff:127.0.0.1 routes to v4 loopback on the wire even
        // though the URL is IPv6. Must be blocked.
        assert!(validate_egress_url("http://[::ffff:7f00:1]/", EgressMode::Public).is_err());
    }

    #[test]
    fn loopback_mode_accepts_loopback_and_rfc1918() {
        assert!(
            validate_egress_url("http://localhost:8000/", EgressMode::LoopbackOrPrivate).is_ok()
        );
        assert!(
            validate_egress_url("http://127.0.0.1:8000/", EgressMode::LoopbackOrPrivate).is_ok()
        );
        assert!(
            validate_egress_url("http://10.0.0.5:8000/", EgressMode::LoopbackOrPrivate).is_ok()
        );
        assert!(
            validate_egress_url("http://192.168.1.42:8000/", EgressMode::LoopbackOrPrivate).is_ok()
        );
        // The ::1 aliases are loopback-class like `localhost`, so the
        // Atropos loopback path accepts them (matching the mirror's
        // non-metadata classification).
        assert!(
            validate_egress_url("http://ip6-localhost:8000/", EgressMode::LoopbackOrPrivate)
                .is_ok()
        );
        assert!(
            validate_egress_url("http://ip6-loopback:8000/", EgressMode::LoopbackOrPrivate).is_ok()
        );
    }

    #[test]
    fn loopback_mode_rejects_public() {
        // Atropos has no auth — even on the loopback-allowed path,
        // public destinations are rejected.
        assert!(
            validate_egress_url("https://api.wandb.ai/", EgressMode::LoopbackOrPrivate).is_err()
        );
        assert!(validate_egress_url("http://1.1.1.1/", EgressMode::LoopbackOrPrivate).is_err());
    }

    #[test]
    fn loopback_mode_rejects_imds() {
        // 169.254/16 link-local stays blocked even when loopback is allowed.
        assert!(
            validate_egress_url("http://169.254.169.254/", EgressMode::LoopbackOrPrivate).is_err()
        );
    }

    #[test]
    fn rejects_non_http_schemes() {
        assert!(validate_egress_url("file:///etc/passwd", EgressMode::Public).is_err());
        assert!(validate_egress_url("ftp://example.com/x", EgressMode::Public).is_err());
        assert!(validate_egress_url("gopher://example.com/", EgressMode::Public).is_err());
    }

    #[test]
    fn rejects_userinfo_smuggling() {
        // http://allowed.com@169.254.169.254/ would otherwise let a
        // tenant smuggle a destination past a host-only check. The
        // userinfo guard makes that shape impossible.
        assert!(
            validate_egress_url("http://allowed.com@169.254.169.254/", EgressMode::Public).is_err()
        );
        assert!(validate_egress_url(
            "http://user:pass@127.0.0.1/x",
            EgressMode::LoopbackOrPrivate
        )
        .is_err());
    }
}
