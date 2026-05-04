//! Real-client-IP resolution for proxied deployments.
//!
//! When the daemon sits behind a reverse proxy (Cloudflare Tunnel, nginx,
//! Traefik, …), the TCP peer address is the proxy, not the original
//! browser. Forwarding headers (`X-Forwarded-For`, `X-Real-IP`,
//! `CF-Connecting-IP`, `Forwarded`) carry the real client IP, but
//! trusting them blindly is exploitable: any internet client can forge
//! the header and rotate its apparent source per request, defeating
//! per-IP rate limiting and connection caps.
//!
//! This module gates header trust on a verified upstream proxy. The
//! caller passes the TCP peer address plus the operator-configured
//! `trusted_proxies` allowlist (CIDRs/IPs) and `trust_forwarded_for`
//! master switch. Header parsing only happens when both flags are
//! set AND the peer matches the allowlist; otherwise the peer IP is
//! returned unchanged. This is fail-closed by default — an empty
//! allowlist means no header trust, regardless of `trust_forwarded_for`.
//!
//! Header preference, when trust applies:
//!
//! 1. `CF-Connecting-IP` — Cloudflare strips client-supplied versions
//!    and re-sets it from the connecting client. Single value.
//! 2. `X-Real-IP` — single-value header set by most reverse proxies.
//! 3. `Forwarded` (RFC 7239) — `for=` parameter, leftmost.
//! 4. `X-Forwarded-For` — comma-separated chain. Walked
//!    **right-to-left**, dropping hops that match `trusted_proxies`;
//!    the first non-matching value is the real client. Walking right-
//!    to-left prevents a malicious leftmost hop from being trusted.
//!
//! For the single-value headers (`CF-Connecting-IP`, `X-Real-IP`), the
//! trusted proxy is fully authoritative — we don't second-guess what
//! it set there. We only sanity-check that the value is a parseable
//! IP and reject unspecified addresses (`0.0.0.0`, `::`), which can't
//! be a real client. Loopback and RFC1918/ULA addresses are accepted
//! as-is; the proxy may legitimately pass an internal-network client
//! to us.

use axum::body::Body;
use axum::http::{HeaderMap, Request};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

/// Compiled form of `trusted_proxies` strings. Build once at boot
/// (`compile_trusted_proxies`) and reuse for every request.
#[derive(Clone, Debug, Default)]
pub struct TrustedProxies {
    entries: Vec<CidrEntry>,
}

#[derive(Clone, Debug)]
enum CidrEntry {
    V4 { network: u32, prefix: u8 },
    V6 { network: u128, prefix: u8 },
}

impl TrustedProxies {
    /// Compile a list of CIDR / IP strings. Invalid entries are skipped
    /// with a warning so a single typo can't take down boot.
    pub fn compile(raw: &[String]) -> Self {
        let mut entries = Vec::with_capacity(raw.len());
        for s in raw {
            match parse_cidr(s.trim()) {
                Some(e) => entries.push(e),
                None => tracing::warn!(
                    entry = %s,
                    "trusted_proxies: ignoring unparseable CIDR/IP entry"
                ),
            }
        }
        Self { entries }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True if `ip` falls inside any compiled CIDR / IP entry.
    pub fn contains(&self, ip: IpAddr) -> bool {
        self.entries.iter().any(|e| match (e, ip) {
            (CidrEntry::V4 { network, prefix }, IpAddr::V4(v4)) => {
                cidr_match_v4(*network, *prefix, v4)
            }
            (CidrEntry::V6 { network, prefix }, IpAddr::V6(v6)) => {
                cidr_match_v6(*network, *prefix, v6)
            }
            // Mixed family never matches. (Don't auto-promote v4↔v6.)
            _ => false,
        })
    }
}

fn parse_cidr(s: &str) -> Option<CidrEntry> {
    if s.is_empty() {
        return None;
    }
    let (addr_part, prefix_part) = match s.split_once('/') {
        Some((a, p)) => (a, Some(p)),
        None => (s, None),
    };
    let ip = IpAddr::from_str(addr_part).ok()?;
    match ip {
        IpAddr::V4(v4) => {
            let prefix = match prefix_part {
                Some(p) => p.parse::<u8>().ok().filter(|n| *n <= 32)?,
                None => 32,
            };
            // Mask the address down to its network so contains() works
            // regardless of whether the operator wrote `10.0.0.5/8` vs
            // `10.0.0.0/8`.
            let masked = mask_v4(u32::from(v4), prefix);
            Some(CidrEntry::V4 {
                network: masked,
                prefix,
            })
        }
        IpAddr::V6(v6) => {
            let prefix = match prefix_part {
                Some(p) => p.parse::<u8>().ok().filter(|n| *n <= 128)?,
                None => 128,
            };
            let masked = mask_v6(u128::from(v6), prefix);
            Some(CidrEntry::V6 {
                network: masked,
                prefix,
            })
        }
    }
}

fn mask_v4(ip: u32, prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        ip & (u32::MAX << (32 - prefix))
    }
}

fn mask_v6(ip: u128, prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        ip & (u128::MAX << (128 - prefix))
    }
}

fn cidr_match_v4(network: u32, prefix: u8, ip: Ipv4Addr) -> bool {
    mask_v4(u32::from(ip), prefix) == network
}

fn cidr_match_v6(network: u128, prefix: u8, ip: Ipv6Addr) -> bool {
    mask_v6(u128::from(ip), prefix) == network
}

/// Resolve the real client IP, gated on `trust_forwarded_for` AND the
/// TCP peer matching `trusted_proxies`. Returns `peer` when trust does
/// not apply or no header parse succeeds.
///
/// **Fail-closed**: any unexpected condition (disabled flag, untrusted
/// peer, malformed headers) collapses back onto the peer. This is the
/// behavior the auth rate limiter and per-IP WS slot key were
/// originally designed for and the limiter's safety properties depend
/// on it.
pub fn resolve_real_client_ip(
    peer: IpAddr,
    headers: &HeaderMap,
    trusted_proxies: &TrustedProxies,
    trust_forwarded_for: bool,
) -> IpAddr {
    if !trust_forwarded_for || trusted_proxies.is_empty() || !trusted_proxies.contains(peer) {
        return peer;
    }

    // Preference 1: CF-Connecting-IP (Cloudflare strips client-supplied versions).
    if let Some(ip) = single_ip_header(headers, "cf-connecting-ip") {
        return ip;
    }
    // Preference 2: X-Real-IP (single-value, proxy-controlled).
    if let Some(ip) = single_ip_header(headers, "x-real-ip") {
        return ip;
    }
    // Preference 3: Forwarded (RFC 7239) — leftmost `for=` parameter.
    if let Some(ip) = parse_forwarded_for_param(headers) {
        return ip;
    }
    // Preference 4: X-Forwarded-For — walk right-to-left, dropping
    // trusted hops; first non-matching value is the real client.
    if let Some(ip) = parse_xff_rightmost_untrusted(headers, trusted_proxies) {
        return ip;
    }

    peer
}

/// Convenience wrapper for axum middleware: pulls the TCP peer from
/// `ConnectInfo<SocketAddr>`, falls back to `0.0.0.0` when the
/// extension is missing (mis-wired router — same fallback as the
/// existing GCRA limiter), then delegates to [`resolve_real_client_ip`].
pub fn resolve_from_request(
    request: &Request<Body>,
    trusted_proxies: &TrustedProxies,
    trust_forwarded_for: bool,
) -> IpAddr {
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
    resolve_real_client_ip(
        peer,
        request.headers(),
        trusted_proxies,
        trust_forwarded_for,
    )
}

fn single_ip_header(headers: &HeaderMap, name: &str) -> Option<IpAddr> {
    // Asymmetry vs the XFF path: `HeaderMap::get` returns only the
    // first instance; if a misconfigured proxy emitted multiple
    // `cf-connecting-ip` / `x-real-ip` headers the later ones are
    // ignored. By design — these headers are single-value per RFC and
    // per Cloudflare's docs, and the trusted proxy is authoritative
    // for what it set there.
    let parsed: IpAddr = headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())?;
    // Reject the unspecified address — `0.0.0.0` / `::` can't be a
    // real client and almost certainly indicates a misconfigured
    // proxy or a bogus header. Loopback and private/ULA addresses
    // are accepted: the trusted proxy may legitimately pass an
    // internal-network client to us.
    if parsed.is_unspecified() {
        return None;
    }
    Some(parsed)
}

fn parse_xff_rightmost_untrusted(
    headers: &HeaderMap,
    trusted_proxies: &TrustedProxies,
) -> Option<IpAddr> {
    // Multiple XFF headers concatenate by HTTP spec; collect them all
    // into one chain. Order across header instances is preserved.
    let mut chain: Vec<IpAddr> = Vec::new();
    for v in headers.get_all("x-forwarded-for").iter() {
        let Ok(s) = v.to_str() else { continue };
        for part in s.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            // XFF values are usually bare IPs but some proxies append
            // `:port` (v4) or use the bracketed `[v6]:port` form. Try
            // bare-parse first, then on failure strip an optional
            // surrounding `[...]` (which `IpAddr::from_str` rejects)
            // and an optional `:port` suffix. We only attempt the port
            // strip when a `:` is present; for unbracketed v6 like
            // `2001:db8::1` the bare parse already succeeds.
            let candidate = match trimmed.parse::<IpAddr>() {
                Ok(ip) => ip,
                Err(_) => {
                    // Strip surrounding brackets if any: `[v6]` or `[v6]:port`.
                    let inner = if let Some(rest) = trimmed.strip_prefix('[') {
                        match rest.split_once(']') {
                            Some((host, _suffix)) => host,
                            None => continue,
                        }
                    } else {
                        // Bare-parse already failed; only thing left to try
                        // is `host:port`. Use rsplit so we strip just the
                        // trailing port — but bail if there are multiple
                        // colons (unbracketed v6) since `host:port` parsing
                        // is undefined for that case and we'd silently
                        // truncate.
                        match trimmed.rsplit_once(':') {
                            Some((host, _port)) if !host.contains(':') => host,
                            _ => continue,
                        }
                    };
                    match inner.parse::<IpAddr>() {
                        Ok(ip) => ip,
                        Err(_) => continue,
                    }
                }
            };
            chain.push(candidate);
        }
    }
    // Walk right-to-left: skip trusted hops; first untrusted is the
    // real client. If every hop is trusted (unusual but possible — all
    // hops are our own infra), fall through to None.
    chain
        .into_iter()
        .rev()
        .find(|ip| !trusted_proxies.contains(*ip))
}

fn parse_forwarded_for_param(headers: &HeaderMap) -> Option<IpAddr> {
    // RFC 7239 example: `Forwarded: for=192.0.2.60;proto=http;by=203.0.113.43`
    // Multiple values comma-separated, parameters semicolon-separated.
    let header = headers.get("forwarded")?.to_str().ok()?;
    let first = header.split(',').next()?;
    for param in first.split(';') {
        let param = param.trim();
        // RFC 7230 §3.2.4 — parameter names are case-insensitive
        // (so `for=`, `For=`, `FOR=`, `fOr=` all match).
        let Some((key, value)) = param.split_once('=') else {
            continue;
        };
        if !key.eq_ignore_ascii_case("for") {
            continue;
        }
        // Value may be quoted, may include a port, may be `[v6]:port`,
        // and may be the obfuscated `_token` form (which we ignore).
        let unquoted = value.trim_matches('"');
        // NOTE: an obfuscated identifier (`for=_hidden`) or `unknown`
        // here short-circuits the entire `Forwarded` walk — we don't
        // try the next list element. By design: the RFC allows the
        // proxy to redact the client identity and we honor that
        // intent rather than peeking past it. Callers that want a
        // real IP fall through to the `X-Forwarded-For` header next.
        if unquoted.starts_with('_') || unquoted.eq_ignore_ascii_case("unknown") {
            return None;
        }
        // Strip optional surrounding brackets (`[v6]`) and any `:port`.
        // Note: bare v4:port and `[v6]:port` are both legal here.
        let parsed = if let Some(rest) = unquoted.strip_prefix('[') {
            let end = rest.find(']')?;
            rest[..end].parse::<IpAddr>().ok()
        } else if unquoted.matches(':').count() == 1 {
            // Looks like `v4:port`.
            unquoted
                .rsplit_once(':')
                .and_then(|(h, _)| h.parse::<IpAddr>().ok())
        } else {
            unquoted.parse::<IpAddr>().ok()
        };
        if let Some(ip) = parsed {
            return Some(ip);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};

    fn tp(entries: &[&str]) -> TrustedProxies {
        TrustedProxies::compile(&entries.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn cidr_match_v4_basic() {
        let t = tp(&["10.0.0.0/8"]);
        assert!(t.contains(ip("10.1.2.3")));
        assert!(t.contains(ip("10.255.255.255")));
        assert!(!t.contains(ip("11.0.0.0")));
        assert!(!t.contains(ip("127.0.0.1")));
    }

    #[test]
    fn cidr_match_v4_unmasked_input() {
        // Operator wrote `172.19.0.5/16` instead of `172.19.0.0/16`.
        let t = tp(&["172.19.0.5/16"]);
        assert!(t.contains(ip("172.19.99.42")));
        assert!(!t.contains(ip("172.20.0.1")));
    }

    #[test]
    fn cidr_match_bare_ip() {
        let t = tp(&["127.0.0.1", "::1"]);
        assert!(t.contains(ip("127.0.0.1")));
        assert!(t.contains(ip("::1")));
        assert!(!t.contains(ip("127.0.0.2")));
    }

    #[test]
    fn cidr_match_v6() {
        let t = tp(&["2001:db8::/32"]);
        assert!(t.contains(ip("2001:db8:1234::1")));
        assert!(!t.contains(ip("2001:db9::1")));
    }

    #[test]
    fn cidr_invalid_entries_skipped() {
        let t = tp(&["not-an-ip", "10.0.0.0/99", "10.0.0.0/8"]);
        // The good entry survives.
        assert!(t.contains(ip("10.1.1.1")));
        assert!(!t.contains(ip("11.1.1.1")));
    }

    fn hdr(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            let name = HeaderName::from_bytes(k.as_bytes()).unwrap();
            h.append(name, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn returns_peer_when_disabled() {
        let t = tp(&["172.19.0.0/16"]);
        let h = hdr(&[("x-forwarded-for", "1.2.3.4")]);
        assert_eq!(
            resolve_real_client_ip(ip("172.19.0.1"), &h, &t, false),
            ip("172.19.0.1"),
            "trust_forwarded_for=false must always return peer"
        );
    }

    #[test]
    fn returns_peer_when_allowlist_empty() {
        let t = tp(&[]);
        let h = hdr(&[("x-forwarded-for", "1.2.3.4")]);
        assert_eq!(
            resolve_real_client_ip(ip("172.19.0.1"), &h, &t, true),
            ip("172.19.0.1"),
            "empty allowlist disables header trust regardless of flag"
        );
    }

    #[test]
    fn returns_peer_when_peer_not_trusted() {
        // Spoof attempt: random internet client claims to be behind a proxy.
        let t = tp(&["172.19.0.0/16"]);
        let h = hdr(&[
            ("x-forwarded-for", "10.0.0.1"),
            ("cf-connecting-ip", "10.0.0.2"),
        ]);
        assert_eq!(
            resolve_real_client_ip(ip("203.0.113.7"), &h, &t, true),
            ip("203.0.113.7"),
            "untrusted peer must not be allowed to forge headers"
        );
    }

    #[test]
    fn cf_connecting_ip_wins() {
        let t = tp(&["172.19.0.0/16"]);
        let h = hdr(&[
            ("x-forwarded-for", "9.9.9.9"),
            ("x-real-ip", "8.8.8.8"),
            ("cf-connecting-ip", "1.2.3.4"),
        ]);
        assert_eq!(
            resolve_real_client_ip(ip("172.19.0.1"), &h, &t, true),
            ip("1.2.3.4"),
            "CF-Connecting-IP has top preference"
        );
    }

    #[test]
    fn x_real_ip_when_no_cf() {
        let t = tp(&["172.19.0.0/16"]);
        let h = hdr(&[("x-forwarded-for", "9.9.9.9"), ("x-real-ip", "8.8.8.8")]);
        assert_eq!(
            resolve_real_client_ip(ip("172.19.0.1"), &h, &t, true),
            ip("8.8.8.8")
        );
    }

    #[test]
    fn xff_rightmost_untrusted_wins() {
        // Browser → CF (untrusted from our POV) → cloudflared
        // → librefang. cloudflared's XFF: "browser, cf-edge".
        // Both browser and cf-edge are untrusted; rightmost untrusted
        // is cf-edge. (In real Cloudflare deployments you'd lean on
        // CF-Connecting-IP, which Cloudflare sets to the actual browser.)
        let t = tp(&["172.19.0.0/16"]);
        let h = hdr(&[("x-forwarded-for", "203.0.113.7, 162.158.1.1")]);
        assert_eq!(
            resolve_real_client_ip(ip("172.19.0.1"), &h, &t, true),
            ip("162.158.1.1"),
            "rightmost-untrusted is the closest non-our-infra hop"
        );
    }

    #[test]
    fn xff_skips_trusted_hops() {
        // Chain: real-client, our-proxy-1, our-proxy-2 (peer).
        // Both proxies are in trusted_proxies; rightmost untrusted = real client.
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("x-forwarded-for", "203.0.113.7, 10.0.0.5, 10.0.0.6")]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn xff_multi_header_concatenates() {
        let t = tp(&["10.0.0.0/8"]);
        let mut h = HeaderMap::new();
        h.append("x-forwarded-for", HeaderValue::from_static("1.2.3.4"));
        h.append("x-forwarded-for", HeaderValue::from_static("10.0.0.5"));
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("1.2.3.4")
        );
    }

    #[test]
    fn xff_with_port_suffix_parses() {
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("x-forwarded-for", "203.0.113.7:54321")]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn forwarded_rfc7239_basic() {
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("forwarded", "for=192.0.2.60;proto=http;by=203.0.113.43")]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("192.0.2.60")
        );
    }

    #[test]
    fn forwarded_rfc7239_v6_bracketed() {
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("forwarded", r#"for="[2001:db8::1]:1234""#)]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("2001:db8::1")
        );
    }

    #[test]
    fn forwarded_rfc7239_obfuscated_falls_through() {
        // `for=_hidden` is the RFC-blessed obfuscated form. We refuse
        // to invent an IP from it, but we should still try other
        // headers / fall back to peer rather than panic.
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("forwarded", "for=_hidden"), ("x-forwarded-for", "1.2.3.4")]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("1.2.3.4")
        );
    }

    #[test]
    fn malformed_headers_fall_back_to_peer() {
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[
            ("cf-connecting-ip", "garbage"),
            ("x-real-ip", "also-garbage"),
            ("x-forwarded-for", "not-an-ip, neither-is-this"),
        ]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("10.0.0.1"),
            "all-garbage headers must fall back to peer, not panic"
        );
    }

    #[test]
    fn xff_bracketed_v6_with_port_parses() {
        // Some proxies write IPv6 in XFF as `[v6]:port`. Bare-parse fails
        // on the bracket form; strip the brackets and try again.
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("x-forwarded-for", "[2001:db8::1]:1234")]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("2001:db8::1")
        );
    }

    #[test]
    fn xff_bracketed_v6_no_port_parses() {
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("x-forwarded-for", "[2001:db8::1]")]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("2001:db8::1")
        );
    }

    #[test]
    fn forwarded_rfc7239_for_param_case_insensitive() {
        // RFC 7230 §3.2.4: parameter names are case-insensitive.
        let t = tp(&["10.0.0.0/8"]);
        for header in [
            "FOR=192.0.2.60",
            "fOr=192.0.2.60",
            "FoR=192.0.2.60;proto=http",
            "by=203.0.113.43;FOR=192.0.2.60",
        ] {
            let h = hdr(&[("forwarded", header)]);
            assert_eq!(
                resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
                ip("192.0.2.60"),
                "case-insensitive `for=` failed on header {header:?}"
            );
        }
    }

    #[test]
    fn forwarded_rfc7239_unknown_token_case_insensitive() {
        // `unknown` and any case variant of it short-circuits the
        // Forwarded walk just like the obfuscated `_token` form.
        let t = tp(&["10.0.0.0/8"]);
        for variant in ["unknown", "Unknown", "UNKNOWN"] {
            let header = format!("for={variant}");
            let h = hdr(&[
                ("forwarded", header.as_str()),
                ("x-forwarded-for", "1.2.3.4"),
            ]);
            assert_eq!(
                resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
                ip("1.2.3.4"),
                "`for={variant}` should fall through to XFF"
            );
        }
    }

    #[test]
    fn single_value_header_rejects_unspecified() {
        // A misconfigured proxy that injects `0.0.0.0` / `::` into
        // these single-value headers must not be allowed to claim
        // them as the real client. Fall through to the next header
        // / peer rather than key on the unspecified address.
        let t = tp(&["10.0.0.0/8"]);
        for (hdr_name, value) in [
            ("cf-connecting-ip", "0.0.0.0"),
            ("x-real-ip", "0.0.0.0"),
            ("cf-connecting-ip", "::"),
            ("x-real-ip", "::"),
        ] {
            let h = hdr(&[(hdr_name, value), ("x-forwarded-for", "1.2.3.4")]);
            assert_eq!(
                resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
                ip("1.2.3.4"),
                "{hdr_name}={value} should be rejected and fall through to XFF"
            );
        }
    }

    #[test]
    fn single_value_header_accepts_loopback_and_private() {
        // Loopback / RFC1918 / ULA addresses are NOT bogus — the
        // proxy may legitimately pass an internal client to us.
        let t = tp(&["10.0.0.0/8"]);
        for value in ["127.0.0.1", "10.0.0.5", "192.168.1.1", "::1", "fc00::1"] {
            let h = hdr(&[("cf-connecting-ip", value)]);
            assert_eq!(
                resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
                ip(value),
                "private/loopback {value} must NOT be filtered out"
            );
        }
    }

    /// Regression for the WS per-IP slot key (both `agent_ws` and
    /// `terminal_ws` go through this path). An attacker on the open
    /// internet that rotates `X-Forwarded-For` per upgrade must NOT
    /// be able to inflate the per-IP WS slot count under fake
    /// identities — the slot key must collapse back onto the real
    /// TCP peer because the peer is not in `trusted_proxies`.
    #[test]
    fn ws_slot_key_resists_spoof_from_untrusted_peer() {
        let trusted = tp(&["172.19.0.0/16"]);
        let attacker_peer = ip("198.51.100.7");
        let mut keys = std::collections::HashSet::new();
        for fake in [
            "10.0.0.1",
            "10.0.0.2",
            "10.0.0.3",
            "203.0.113.7, 162.158.1.1",
        ] {
            let h = hdr(&[("x-forwarded-for", fake)]);
            let resolved = resolve_real_client_ip(attacker_peer, &h, &trusted, true);
            keys.insert(resolved);
        }
        assert_eq!(
            keys.len(),
            1,
            "rotating X-Forwarded-For from an untrusted peer must collapse \
             to a single per-IP WS slot key (the real TCP peer)"
        );
        assert!(
            keys.contains(&attacker_peer),
            "the single resolved key must be the attacker's real TCP peer"
        );
    }

    /// Companion to the spoof test: when the peer IS trusted, two
    /// different forwarded clients produce two different WS slot
    /// keys (so two real browsers behind the same cloudflared edge
    /// don't share one slot — which is the original bug both
    /// `agent_ws` and `terminal_ws` are fixing).
    #[test]
    fn ws_slot_key_separates_real_clients_behind_trusted_proxy() {
        let trusted = tp(&["172.19.0.0/16"]);
        let proxy_peer = ip("172.19.0.1");

        let h_a = hdr(&[("x-forwarded-for", "203.0.113.10")]);
        let h_b = hdr(&[("x-forwarded-for", "203.0.113.20")]);
        assert_ne!(
            resolve_real_client_ip(proxy_peer, &h_a, &trusted, true),
            resolve_real_client_ip(proxy_peer, &h_b, &trusted, true),
            "two browsers behind the same trusted proxy must get distinct WS slot keys"
        );
    }

    #[test]
    fn xff_all_hops_trusted_falls_through() {
        // Every hop matches trusted_proxies — there is no real client
        // in the chain. We refuse to guess and fall back to peer.
        let t = tp(&["10.0.0.0/8"]);
        let h = hdr(&[("x-forwarded-for", "10.0.0.5, 10.0.0.6")]);
        assert_eq!(
            resolve_real_client_ip(ip("10.0.0.1"), &h, &t, true),
            ip("10.0.0.1")
        );
    }
}
