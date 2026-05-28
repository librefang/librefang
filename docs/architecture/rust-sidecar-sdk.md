# Rust sidecar SDK

`sdk/rust/librefang-sidecar/` is the first-party Rust SDK for writing LibreFang channel adapters as out-of-process binaries.
It pairs with the Python SDK at `sdk/python/librefang/sidecar/`; both implementations are pinned to the same shared conformance corpus at `conformance/sidecar/corpus/` and are interchangeable on the wire — the supervisor treats `command = "python3 -m my_adapter"` and `command = "/usr/local/bin/my-rust-adapter"` identically.

Shipped with #5821.
See [`sidecar-channels.md`](./sidecar-channels.md) for the architectural case (crash isolation, supply-chain confinement, iteration loop) and [`sidecar-protocol.md`](./sidecar-protocol.md) for the wire spec.

## When to reach for this SDK

The Python SDK is the lowest-friction substrate for most adapters.
Reach for the Rust SDK when one of the following actually matters for your deployment:

- **Binary footprint / startup latency.** A statically-linked Rust binary boots in milliseconds and consumes a few MB of resident memory; a Python sidecar pays the interpreter cost on every supervised respawn.
- **Type safety on the inbound command set.** `Command::Send(s)`, `Command::Typing(t)`, `Command::StreamStart(s)` etc.
  are exhaustively matched at compile time, so a future protocol addition forces the compiler to flag every site that needs to opt in or explicitly ignore the new variant.
- **Existing Rust transport ecosystem.** If your platform's HTTP / WebSocket / SSE client is a Rust crate the broader ecosystem has already hardened (`reqwest` with `rustls`, `tokio-tungstenite`, `eventsource-client`, …), staying in Rust avoids reimplementing the transport against Python wrappers.

If none of those apply, prefer the Python SDK.

## Crate surface

```
src/
├── lib.rs        — re-exports
├── protocol.rs   — wire types: Command, Content, MessageBuilder, Schema, …
└── runtime.rs    — run_stdio_main, EmitFn, panic isolation, with_backoff
```

### Types

- `SidecarAdapter` trait — what every adapter implements.
  Three required methods: `capabilities()`, `on_send(cmd: SendCommand)`, `produce(emit: EmitFn)`.
  `on_command(cmd: Command)` and `header_rules()` are optional with sensible defaults.
- `Command` — externally-tagged enum of every inbound command the supervisor can send.
  `Send`, `Typing`, `Reaction`, `Interactive`, `StreamStart`, `StreamDelta`, `StreamEnd`, plus a non-exhaustive variant so future protocol additions don't break older adapters at compile time.
- `Content` — value-typed builder for the wire-shape `ChannelContent` enum (`Content::text(...)`, `Content::image(...)`, `Content::voice(...)`, …).
  Returns `serde_json::Value`, matching what `MessageBuilder::content` expects.
- `MessageBuilder` — fluent builder for outbound `message` events.
  `.channel_id(...).platform(...).is_group(...).message_id(...).metadata(...).username(...).thread_id(...).content(...).build()`.
- `Schema`, `Field`, `FieldType` — describe the adapter's configuration form, served via `--describe` so the LibreFang dashboard can render the configure UI before the binary is even spawned with real env vars.

### Runtime

- `run_stdio_main(schema_fn, build_fn)` is the canonical entry point.
  `schema_fn: FnOnce() -> Schema` is called eagerly on `--describe`; `build_fn: FnOnce() -> Result<A, DynError>` is called lazily after the supervisor has injected the real environment.
  The two-step layout means the configure form can be served even when required env vars (a bot token, an API key) aren't set yet — operators see the form, fill it in, the supervisor respawns with the env, and *then* `build_fn` runs.
- `EmitFn` is `Arc<dyn Fn(Value) + Send + Sync>` — your `produce` function calls it once per outbound event.
  The runtime serialises each `emit` call to a single stdout line; concurrent emitters across spawned tasks are safe.
- Panics inside `on_command` and `produce` are caught by the runtime and converted to protocol-level error events so a buggy adapter crashes its supervisor cycle (and gets restarted under backoff) without taking the whole daemon out.
  See `format_join_panic` for the join-error → human-string conversion.

### Helpers

- `with_backoff(f, policy)` — exponential backoff for transient errors in `produce`.
  Asserts `initial > 0`, `initial <= maximum`, `factor >= 1.0` so degenerate policies fail fast at construction rather than silently looping.

## Minimal adapter

```rust,no_run
use async_trait::async_trait;
use librefang_sidecar::{
    run_stdio_main, EmitFn, MessageBuilder, Schema, SendCommand, SidecarAdapter,
};

struct EchoAdapter;

#[async_trait]
impl SidecarAdapter for EchoAdapter {
    fn capabilities(&self) -> Vec<String> {
        vec!["typing".into()]
    }

    async fn on_send(
        &self,
        cmd: SendCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // deliver cmd.text / cmd.content to your real platform
        let _ = cmd;
        Ok(())
    }

    async fn produce(
        &self,
        emit: EmitFn,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // emit one synthetic message and exit cleanly
        emit(
            MessageBuilder::new("42", "Alice")
                .text("hello from echo")
                .build(),
        );
        Ok(())
    }
}

fn schema() -> Schema {
    Schema::new("echo", "Echo adapter")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_stdio_main(schema, || Ok(EchoAdapter)).await
}
```

`examples/echo.rs` ships a slightly more interesting version that wires a `tokio::sync::watch::channel(Option<EmitFn>)` so the produce loop can be re-armed cleanly across restarts.

## Configure as a sidecar

```toml
[[sidecar_channels]]
name = "echo"
command = "/abs/path/to/target/release/echo-binary"
args = []
restart = true
```

The dashboard's configure form is populated from the `Schema` your binary serves under `--describe`, so operators set secrets / advanced fields without hand-editing `config.toml`.

## Responsibility split

- **Process restart is LibreFang's job.**
  The supervisor in `crates/librefang-channels/src/sidecar.rs` respawns a crashed child with exponential backoff and a circuit-breaker.
  Your adapter must be *crash-safe*: hold no irreplaceable in-process state across crashes.
- **Platform reconnect is your adapter's job.**
  Reconnecting a dropped WebSocket / long-poll / SSE is your transport's concern.
  Use `with_backoff` for the standard exponential-retry shape.
- **stdout is reserved for protocol frames.** Send all logs to stderr; the daemon collects them into its main log under the channel's name.

## Conformance

`tests/conformance.rs` runs the 13 cross-implementation test vectors from `conformance/sidecar/corpus/` in both directions:

- **Producer-side**: build an event with `MessageBuilder`, assert it serialises byte-identically to the corpus fixture.
- **Consumer-side**: deserialise each command fixture, assert the resulting `Command` variant and field values match expectations.

When you add a new protocol variant or change a wire shape, update the corpus first; the Python and Rust SDKs both pin against the same files and fail in parallel if the contract drifts.

## Common pitfalls

- **Don't `tokio::spawn` a future that captures the `EmitFn` and outlives `produce`.** The supervisor expects `produce` to be the single owner of the emit channel.
- **Don't hold a `tokio::sync::Mutex` across an `.await` of a network call that could block for many seconds.** Other tasks waiting on the same mutex stall too.
  Prefer per-resource locks (or `DashMap` for keyed state).
- **Don't print to stdout.** `println!` directly corrupts the wire. The compiler can't catch this; treat it as a discipline.
- **Don't return early from `produce` on a recoverable error.** The supervisor will restart your whole subprocess on a `produce` return; reserve that for unrecoverable conditions (missing required env, bad config).
  Use `with_backoff` for transient failures.

## See also

- [`sidecar-channels.md`](./sidecar-channels.md) — the architectural case for sidecar-first channels, the supervisor's process model, and the policy gate keeping in-process adapters out.
- [`sidecar-protocol.md`](./sidecar-protocol.md) — the wire spec, conformance corpus layout, and per-method/event field semantics.
- [`rust-telegram-sidecar.md`](./rust-telegram-sidecar.md) — the first first-party adapter built against this SDK; concrete reference for the Markdown→HTML pipeline, UTF-16 chunking, streaming-edit debounce, and Bot-API retry shape.
