# MCP OAuth `token_endpoint` host pinning (#3713 / #4665)

When LibreFang completes the OAuth authorization-code exchange for an
MCP server, the URL it POSTs the code to comes from discovery metadata
(`/.well-known/oauth-authorization-server`), which is attacker-influenced
data. A tampered or maliciously-served metadata document could otherwise
redirect the exchange to an attacker-controlled host and exfiltrate the
auth code. This doc captures the pinning policy that guards that step.

## Threat

The flow is:

1. Operator pastes an MCP server URL into `config.toml`. This is the
   only value the attacker cannot influence.
2. Daemon fetches discovery metadata over HTTPS to learn the
   `authorization_endpoint` and `token_endpoint`.
3. User authorizes against `authorization_endpoint`; the auth server
   redirects back to LibreFang with an authorization code.
4. Daemon POSTs the code + PKCE verifier to `token_endpoint`.

Step 4 is the dangerous one: if `token_endpoint` points anywhere the
operator did not authorize against, the code (or the eventual access
token) leaves the trust boundary the user thought they were inside.

## Policy

`token_endpoint_host_matches` in
`crates/librefang-api/src/routes/mcp_auth.rs` is the single point that
decides whether the exchange may proceed. It compares the host inside
`token_endpoint` against the host the operator typed in `config.toml`
(stored as `issuer_host` in the vault during `auth_initiate`). The
exchange is refused unless one of these holds:

| Rule | Accept when | Refs |
|---|---|---|
| 1 | Hosts are an **exact case-insensitive match**. | #3713 (original strict pin) |
| 2 | Both hosts share the same **registrable domain (eTLD+1)** under the Public Suffix List. | #4665 (cross-domain proxies) |

Rule 2 is symmetric: the operator-typed host and the metadata-declared
host can sit in any parent/child arrangement under the same eTLD+1
(`sub→root`, `root→sub`, `sibling→sibling`). The trust boundary is the
registrable domain itself, not its hierarchy.

The PSL is consulted via the `psl` crate, which ships the list baked-in
at compile time (no runtime fetch, no network dependency on a security
check).

### Hosts that fall through to Rule 1 only

Some hosts are not DNS names with a known public suffix — IP literals,
`localhost`, single-label internal names. The PSL has no opinion on
those, and a "registrable domain" check would either be meaningless or
unsafe (see "IP literal carve-out" below). Rule 1 is the only
acceptance path for them.

## Why Rule 2 was needed (#4665)

Several MCP services legitimately split discovery from token exchange
across two hostnames in the same registrable domain:

- **Slack** — `mcp.slack.com` advertises a token endpoint at
  `https://slack.com/api/oauth.v2.user.access`.
- **Notion** — `mcp.notion.com` delegates to `api.notion.com`.

The strict #3713 pin refused both flows and left operators without a
workaround. Rule 2 admits these cases by recognising that
`mcp.slack.com` and `slack.com` share the registrable domain `slack.com`
under the PSL.

## Threat trade-off accepted by Rule 2

Loosening from "exact host" to "same eTLD+1" admits a class of attack:
someone who controls *any* sibling subdomain on the issuer's
registrable domain could redirect the token exchange to a host they own
**if they also tamper with HTTPS-validated discovery metadata**. That
residual risk is accepted because:

1. **Metadata fetches are HTTPS-validated.** Tampering requires either a
   compromise of the legitimate auth server's HTTPS endpoint or a
   working MITM against the daemon's TLS — both raise the bar
   substantially over plain DNS poisoning.
2. **Sibling-subdomain takeover within an org's own registrable domain
   typically implies the org itself is compromised.** The most
   plausible class — dangling DNS records pointing to deleted
   third-party services — is bounded by org hygiene, not by this
   check. (PSL private domains, see below, exclude the largest such
   third-party-hosted shapes from Rule 2 entirely.)
3. **The strict pin left no escape hatch for legitimate cross-domain
   OAuth delegation.** Operators who hit it had no per-server toggle
   and were forced to abandon the integration.

## PSL private-domain section is load-bearing

The PSL has a *private* section that lists multi-tenant hosting
boundaries — `*.github.io`, `*.herokuapp.com`, `*.s3.amazonaws.com`,
`*.vercel.app`, etc. For those, `psl::domain_str("user1.github.io")`
returns `Some("user1.github.io")`, **not** `Some("github.io")`. So two
GitHub Pages tenants do not share a registrable domain and Rule 2 will
not accept a cross-tenant redirect. Without this property, an attacker
who can register `attacker.github.io` could false-match an issuer on
`victim.github.io`.

This is not a property of LibreFang's code — it is a property of the
PSL — but the policy depends on it, so the regression test
`token_endpoint_psl_private_domain_does_not_false_match` pins it.

## IP literal carve-out

`psl::domain_str` is **not** documented to return `None` for every IP
shape. For an IPv4 address with an unknown TLD label, the PSL's default
rule emits the rightmost two labels as the "registrable domain", which
means `psl::domain_str("10.0.0.1")` returns `Some("0.1")` — the same
value as for `192.168.0.1` and `127.0.0.1`. Without intervention,
`token_endpoint` `https://10.0.0.1/...` would Rule-2-match an
`issuer_host` of `127.0.0.1`.

`token_endpoint_host_matches` therefore short-circuits IP literals
before the PSL path: if either host parses as an `IpAddr` (after
stripping brackets emitted by `url::Url::host_str` for IPv6), only Rule
1 can accept the pair. Coverage:

- `token_endpoint_ip_host_requires_exact_match` (IPv4)
- `token_endpoint_ipv6_host_requires_exact_match` (bracketed IPv6)
- `token_endpoint_ipv4_with_shared_trailing_labels_must_not_match`
  (pins the carve-out specifically)
- `token_endpoint_ip_does_not_match_domain` (mixed shapes)

## What this policy does not cover

- **SSRF on metadata / token fetch.** That is a separate policy in
  `librefang-runtime::mcp_oauth::is_ssrf_blocked_url` (blocklist of
  loopback, link-local, internal ranges). It uses its own host check
  with a different threat model and is intentionally not migrated to
  the eTLD+1 rule.
- **Per-server opt-in to the strict #3713 behaviour.** Issue #4665
  considered an `oauth.strict_host_pin = true` per-server toggle for
  paranoid operators; it is tracked as a follow-up but is not shipped
  here. Operators who want it today can refuse to add the MCP server.
- **Authorization endpoint host.** The pin is on `token_endpoint`
  because that is where the auth code is sent. The user-facing
  `authorization_endpoint` is a navigation target, not a credential
  recipient, and is not pinned by this helper.

## Files

- `crates/librefang-api/src/routes/mcp_auth.rs` —
  `token_endpoint_host_matches`, `is_ip_literal`, callsite in
  `auth_callback`. Doc comment on the helper carries the inline version
  of this trade-off so a future maintainer touching the function does
  not need to find this file first.
- `Cargo.toml` (workspace) and
  `crates/librefang-api/Cargo.toml` — the `psl = "2"` dependency is
  pinned at the workspace level with a comment explaining why `psl`
  was picked over `publicsuffix` (compile-time-baked data, no runtime
  fetch).
