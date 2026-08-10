Resolve and validate every DNS address for Python webhook callback URLs, then connect directly to that validated address set while preserving HTTPS SNI. (@houko)
Callback delivery previously checked only IP literals and reserved hostname strings, so a public-looking hostname could resolve or rebind to loopback, RFC-1918, link-local, cloud metadata, or a private IPv4 endpoint embedded in IPv6.
The callback transport now bypasses environment proxies and never re-resolves the hostname after validation; DNS failure or any unsafe answer fails closed before the signed request is sent.
