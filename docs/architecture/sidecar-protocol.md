# Sidecar channel protocol — wire specification (v1)

Status: **v1, frozen core**. This document is the normative wire
contract between the Rust supervisor and any sidecar channel adapter.
The architecture (supervision, backoff, capability gating, config) is
described in [`sidecar-channels.md`](./sidecar-channels.md); this
document specifies only the bytes on the wire.

There are three independent implementations of this protocol today:

- **Rust supervisor** — `crates/librefang-channels/src/sidecar.rs` (`SidecarEvent`, `SidecarCommand`, ~24 unit tests).
  Consumes events, produces commands.
- **Python SDK** — `sdk/python/librefang/sidecar/protocol.py` (`parse_command`, the event builders, its own pytest).
  Produces events, consumes commands.
- **Rust SDK** — `sdk/rust/librefang-sidecar/src/protocol.rs` (`parse_command`, the event builders + `MessageBuilder`, its own `cargo test`).
  Produces events, consumes commands.
  Usage / API reference: [`rust-sidecar-sdk.md`](./rust-sidecar-sdk.md).

All three are kept honest against each other by a single shared corpus, `conformance/sidecar/corpus/`.
Each implementation carries its own conformance test that asserts against the corpus in the direction it participates in:

- `crates/librefang-channels/tests/sidecar_protocol_conformance.rs`
- `sdk/python/tests/test_sidecar_conformance.py`
- `sdk/rust/librefang-sidecar/tests/conformance.rs`

The corpus — not this prose — is the executable contract; this document explains it.

## Transport

Newline-delimited JSON, one object per line:

- **Events**: adapter → supervisor, written to the adapter's
  **stdout**.
- **Commands**: supervisor → adapter, written to the adapter's
  **stdin**.

stderr is free-form adapter logging and is not part of the protocol.

## Envelope

Every frame is a JSON object with a string `method` and an optional
`params` object:

```json
{ "method": "<name>", "params": { ... } }
```

`method`-only frames (no `params`) are valid for the parameterless
variants (`ready_ack`, `shutdown`, `heartbeat`, and the bare legacy
`ready`). A present-but-`null` `params` is accepted equivalently to an
omitted one. Unknown `method` values **must not crash the receiver**:
the Rust side and the Python `parse_command` both degrade unknown
methods to a tolerated "unknown" case so either end can add a frame
without breaking an older peer.

The Rust enums are `#[serde(tag = "method")]`, so the variant payload
lives under `params`. This is the contract — do not reshape the
envelope.

## Conformance: JSON value equality

Equality is **structural JSON value equality**, not byte equality (see
`conformance/sidecar/README.md` for the rationale). Each frame is
produced by one side and consumed by the other; the conformance tests
pin the producer's serialization and the consumer's parse against the
same corpus file.

## `protocol_version`

`ready.params.protocol_version` is an optional integer. **Current value: `1`.**

`librefang_channels::sidecar::SIDECAR_PROTOCOL_VERSION` is the source of truth for that number.
Every other copy of it — this sentence, the corpus fixture `conformance/sidecar/corpus/events/ready_full.json`, the Python SDK's `librefang.sidecar.protocol.PROTOCOL_VERSION`, and the Rust SDK's `librefang_sidecar::protocol::PROTOCOL_VERSION` — is pinned to the constant by `crates/librefang-channels/tests/sidecar_version_contract.rs`, which fails the build when any of them drifts.

**Checked, not enforced.**
The supervisor classifies the declared value against its own constant (`classify_protocol_version`) and logs a `WARN` naming the adapter when the two differ in either direction, or when the adapter declared nothing at all.
It still does not reject the adapter or close the connection — a skewed sidecar keeps running, it just stops being invisible.

Absent ⇒ treated as "unspecified", not as a specific version, and warned about.
Every SDK-built adapter has declared a version since #7140, so an absent value now means either a hand-rolled adapter or an SDK install old enough to predate the default — both worth an operator's attention, which is why the bare-`ready` case warns rather than passing silently.
Adapters inherit the value from `SidecarAdapter.protocol_version` in either SDK and should not set it per adapter; override it only when an adapter deliberately speaks an older protocol.

The version increments when a **frozen-core** frame changes in a non-additive-optional way (a removed/renamed field, a changed type, a new required field).
Adding a new optional field, a new capability string, or a new frame method is additive and does **not** bump the version (older peers tolerate it via the unknown-method / `serde default` paths).

## Frozen core (v1)

These frames are implemented by **both** implementations today and are
covered by the corpus. Their shape is frozen under the versioning rule
above.

### Events (adapter → supervisor, `corpus/events/`)

| Frame | `params` | Notes |
|-------|----------|-------|
| `ready` (full) | `capabilities[]`, `account_id`, `suppress_error_responses`, `notification_recipients[]`, `header_rules[]`, `protocol_version` | Capability strings gate the optional adapter features. |
| `ready` (minimal) | *(omitted)* | Bare legacy form. Rust must accept; the SDK never emits it. |
| `message` (content) | `user_id`, `user_name`, `content`, `text` (mirror), `channel_id`, `platform`, … | `content` supersedes `text`; plain text is mirrored into `text` for pre-capability supervisors. |
| `message` (command) | `user_id`, `user_name`, `content` = `{"Command": {"name", "args"}}`, `message_id`, `channel_id`, `platform` | A slash command. No `text` mirror exists — only plain-text `content` has a lossless flattening — so a supervisor that reads `text` and ignores `content` sees an empty message. |
| `message` (minimal) | `user_id`, `user_name`, `text` | Legacy text-only adapter. |
| `error` | `message` | |
| `typing` | `user_id`, `user_name`, `is_typing` | |

`message.params` additionally carries optional `message_id`,
`username`, `librefang_user`, `is_group`, `thread_id`,
`group_members[]`, `group_participants[]`, `metadata` — all
`#[serde(default)]` on the Rust side and omitted-when-unset by the SDK
builder. Their presence/absence is additive and not version-bumping.

### Commands (supervisor → adapter, `corpus/commands/`)

| Frame | `params` |
|-------|----------|
| `send` (full) | `channel_id`, `text`, `content`, `thread_id`, `user` |
| `send` (minimal) | `channel_id`, `text`, `user` (`content`/`thread_id` omitted when `None`) |
| `ready_ack` | *(none)* |
| `shutdown` | *(none)* |
| `heartbeat` | *(none)* |
| `typing` | `channel_id` |
| `reaction` | `channel_id`, `message_id`, `reaction` |
| `interactive` | `channel_id`, `message` (`InteractiveMessage`) |
| `stream_start` | `channel_id`, `stream_id`, `thread_id?` |
| `stream_delta` | `stream_id`, `text` |
| `stream_end` | `stream_id` |

The Rust SDK deliberately rejects a typed command when a field shown
without `?` is missing. Its reader emits a protocol `error` event and
continues. The Python SDK's legacy parser is more permissive and may
replace missing values with empty strings; adapters must not rely on
that leniency. Optional fields continue to use their documented
defaults in both SDKs.

`send.params.user` is `ChannelUser` =
`{ platform_id, display_name, librefang_user }` (`librefang_user`
serialized as `null` when absent — it is not skipped).
`send.params.content` / `thread_id` use
`skip_serializing_if = "Option::is_none"`, so the minimal `send` omits
them entirely rather than emitting `null`.

## Provisional surface (not yet frozen)

`content` (in `message` and `send`) is the externally-tagged
`ChannelContent` enum: `{"Text": "..."}`, `{"Image": {...}}`,
`{"File": {...}}`, `FileData`, `Voice`, `Video`, `Location`,
`Command`, `Interactive`, `ButtonCallback`, `DeleteMessage`,
`EditInteractive`, `Audio`, `Animation`, `Sticker`, `MediaGroup`,
`Poll`, `PollAnswer`. The **`Text`** variant is frozen-core (it is on
the wire for every text adapter and is corpus-pinned). The richer
variants are **provisional**: their shape mirrors
`crate::types::ChannelContent` and the SDK `Content` builders, but
they are not yet exercised end-to-end by a migrated adapter, so they
are documented and SDK-tested but not promised stable.

A provisional variant is **promoted to frozen-core** (added to the
corpus, asserted on both sides) when a real migrated adapter exercises
it end-to-end. The telegram sidecar migration (PR #5232 and its
follow-on increments) is the first such driver: each increment that
lands a `ChannelContent` variant on a live path promotes that variant
here. The capability-gated commands (`typing`, `reaction`,
`interactive`, `stream_*`) are frozen-core *as envelopes*; the
semantics of the rich content they may carry follow the same
promotion rule.

This is the converged scope: freeze what both implementations already
exercise, grow the frozen set with each migration, version-bump only
on a breaking change to the frozen set.
