# Sidecar protocol conformance corpus

`corpus/` holds the canonical wire frames of the sidecar channel
protocol — one JSON file per frame. It is the **single shared oracle**
for the protocol's two independent implementations:

- the Rust supervisor — `crates/librefang-channels/src/sidecar.rs`
  (`SidecarEvent` / `SidecarCommand`)
- the Python SDK — `sdk/python/librefang/sidecar/protocol.py`
  (`parse_command` + the event builders)

Before this corpus existed each side tested the same protocol with its
own hand-written fixtures, so a field rename on one side passed both
test suites and only failed in production. The corpus removes that
blind spot: both suites now assert against these same bytes.

## Directionality

Every frame is produced by one side and consumed by the other. The
conformance tests pin each side to the corpus from its own direction:

| Frame kind | `corpus/` dir | Producer (asserts `serialize == corpus`) | Consumer (asserts `parse(corpus) == expected`) |
|------------|---------------|------------------------------------------|------------------------------------------------|
| Event   (adapter → daemon) | `events/`   | Python SDK builders        | Rust `SidecarEvent` deserialize |
| Command (daemon → adapter) | `commands/` | Rust `SidecarCommand` serialize | Python `parse_command` |

`events/ready_minimal.json` is the bare legacy `{"method":"ready"}`
form. The Python SDK never *emits* it (its `ready()` builder always
writes full params); it exists so the Rust consumer's
backward-compatible acceptance of the pre-capability form stays pinned.
The Python suite documents-and-skips it on the producer side.

## Conformance contract: JSON value equality

Equality is **structural JSON value equality** (same keys, same
values, recursively), *not* raw byte equality. Two conformant JSON
encoders legitimately differ in key order, whitespace, and non-ASCII
escaping; pinning bytes would test the encoder, not the protocol. The
corpus files are pretty-printed for human review; the tests parse both
sides and compare values.

## Adding or changing a frame

The corpus is the contract. Changing it is changing the protocol:

1. Add/modify the `.json` frame here.
2. Extend **every** conformance suite — `crates/librefang-channels/tests/sidecar_protocol_conformance.rs`, `sdk/python/tests/test_sidecar_conformance.py`, and `sdk/rust/librefang-sidecar/tests/conformance.rs`.
   A corpus entry with no assertion on both sides of the wire is not conformance.
3. If the change is not additive-optional, bump `librefang_channels::sidecar::SIDECAR_PROTOCOL_VERSION` — the source of truth — and then every file `crates/librefang-channels/tests/sidecar_version_contract.rs` names, which is what fails if you miss one.

Build a frame the way an adapter builds it, not by passing the corpus's own values in by hand.
`events/ready_full.json` used to be reproduced by calling the `ready()` builder with `protocol_version=1` supplied by the test, which is why nobody noticed for months that the SDK default was `None` and that every real `ready` frame carried `null` (#7140).
Going through `SidecarAdapter.ready_event()` puts the declarative defaults every adapter inherits under the assertion.

See `docs/architecture/sidecar-protocol.md` for the versioned spec,
the frozen-vs-provisional policy, and `protocol_version` semantics.
