`rustls` moves to 0.23.45, which rejects TLS 1.3 handshake messages that arrive across an encryption level boundary (RUSTSEC-2026-0285).
Every outbound connection this daemon makes goes through it — the Bot API client, the channel transports and the registry — so the advisory is not confined to one feature.
The bump pulls `aws-lc-rs` 1.18.1 and `rustls-webpki` 0.103.15 with it, which is the smallest set the new version accepts; nothing else in the lockfile moves. (#8347) (@nevgenov)
