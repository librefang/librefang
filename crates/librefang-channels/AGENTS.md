# librefang-channels — agent notes

> **This file is a stub.** The previous content drifted out of sync
> with the live module layout, and the verbose webhook-security /
> SSRF / sidecar-onboarding sections live in better-maintained form
> elsewhere (`docs/architecture/sidecar-channels.md`, the SDK's
> sidecar adapter docstrings, the top-level `CLAUDE.md` "channels"
> entry, and `CONTRIBUTING.md`). Keep this file short on purpose; the
> source files in `src/` and `channels-allowlist.txt` are the
> authoritative reference.

## Purpose

Channel infrastructure crate. Every channel adapter runs out-of-process
as a sidecar (`librefang.sidecar.adapters.*` in `sdk/python/`); this
crate owns the trampoline that connects the kernel to those sidecars
(`sidecar.rs`), the shared bridge types every adapter speaks, and the
shared HTTP client.

## Module map (verify against `src/` before relying on this)

Every module compiles unconditionally — there are no `channel-*` /
`all-channels` feature gates left. Modules currently declared in
`src/lib.rs`: `attachment_enrich`, `bridge`, `commands`,
`embedded_sdk`, `formatter`, `group_history`, `http_client`,
`message_journal`, `message_truncator`, `rate_limiter`, `roster`,
`router`, `sanitizer`, **`sidecar`** (the trampoline),
`thread_ownership`, `types`.

## Sidecar-only policy

A new channel is an out-of-process sidecar adapter, not a new module
in this crate. `scripts/hooks/pre-commit` and
`cargo xtask channel-policy` (CI) reject any file under
`crates/librefang-channels/src/{<name>.rs, <name>/*.rs}` that contains
`ChannelAdapter for` whose basename is not in
`src/channels-allowlist.txt`. That allowlist currently contains only
`sidecar` and is documented to only ever shrink — adding a name back
is an explicit maintainer decision in a separate reviewed commit.

See `docs/architecture/sidecar-channels.md` and the existing adapters
under `sdk/python/librefang/sidecar/adapters/` for the canonical
onboarding flow.

## Cross-cutting rules

Webhook HMAC verification, SSRF guards on
`WEBHOOK_CALLBACK_URL`, the channel-derived `SessionId::for_channel`
contract, and the boundary against `librefang-kernel` /
`librefang-runtime` are all documented in the top-level `CLAUDE.md`
and in the live sidecar adapter sources. Do not duplicate them
here — duplication is precisely how this file rotted in the first
place.
