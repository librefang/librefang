# `rumqttc 0.25.1` drags in `rustls-webpki 0.102.8` with 4 unfixed CVEs

**Severity:** High
**Category:** Dependencies and supply chain
**Labels:** `security`, `cve`, `dependencies`, `high`

## Affected files
- `Cargo.lock`: `rumqttc 0.25.1`, `rustls-webpki 0.102.8`
- `crates/librefang-channels/Cargo.toml:118` (`channel-mqtt` feature)
- `deny.toml:67-80` (blanket ignore)

## Description

Four active RUSTSEC advisories:

- **RUSTSEC-2026-0049** — CRL distribution-point handling
- **RUSTSEC-2026-0098** — name-constraint URI handling
- **RUSTSEC-2026-0099** — wildcard name-constraint bypass
- **RUSTSEC-2026-0104** — CRL parser DoS

Anyone running MQTT-over-TLS is on the attack surface.

The current mitigation is a **blanket** ignore in `deny.toml` — cargo-deny does not yet support per-crate-version scoping. The comment at `:67-80` notes the intent to switch to `crate = "rustls-webpki@0.102.8"` later. Consequence: any new crate that pulls in `rustls-webpki 0.102.x` silently bypasses the audit.

## Recommendation

1. In `librefang-channels`, default-off the `channel-mqtt` feature until rumqttc ships a release using `rustls-webpki ≥ 0.103.13`;
2. Once cargo-deny supports per-version scoping, replace the 4 blanket ignores with `crate = "rustls-webpki@0.102.8"`;
3. As an interim measure, consider forking rumqttc to bump directly (the upstream bytebeamio/rumqtt issue tracker has stalled).
