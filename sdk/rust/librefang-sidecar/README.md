# librefang-sidecar

Rust SDK for writing [LibreFang](https://librefang.ai) sidecar channel adapters.
An adapter is an out-of-process subprocess that LibreFang supervises and talks to over newline-delimited JSON-RPC on stdio; this crate gives you the protocol types, the `SidecarAdapter` trait, and a `run_stdio` driver so you do not have to implement either yourself.

The wire protocol is shared with the Python SDK (`librefang.sidecar` in `sdk/python/`) and pinned by the conformance corpus at `conformance/sidecar/corpus/` — see `tests/conformance.rs` for the cross-implementation tests.

## When to use this crate

- You already write Rust and want type-safe access to the inbound command set without going through `serde_json::Value` by hand.
- You need a small, stdlib-shaped binary as your channel adapter (Python startup / footprint matters for your deployment).
- You want to reuse Rust transport crates (`tokio-tungstenite`, `reqwest`, …) that an external Rust ecosystem has already hardened.

The Python SDK at `sdk/python/librefang/sidecar/` remains the lowest-friction substrate for most adapters; this crate exists so the sidecar boundary is **language-agnostic by construction**, not Python-shaped by accident.

## Minimal adapter

```rust,no_run
use async_trait::async_trait;
use librefang_sidecar::{
    run_stdio, EmitFn, MessageBuilder, SendCommand, SidecarAdapter,
};

struct EchoAdapter;

#[async_trait]
impl SidecarAdapter for EchoAdapter {
    fn capabilities(&self) -> Vec<String> {
        vec!["typing".into()]
    }

    async fn on_send(
        &self,
        _cmd: SendCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // deliver cmd.text / cmd.content to your real platform
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    run_stdio(EchoAdapter).await
}
```

## Configure as a sidecar

Add a `[[sidecar_channels]]` block to `~/.librefang/config.toml`:

```toml
[[sidecar_channels]]
name = "echo"
command = "/path/to/your/built/echo-binary"
args = []
restart = true
```

Then `librefang start` and the daemon will spawn it under its supervisor.
See [`docs/architecture/sidecar-channels.md`](../../../docs/architecture/sidecar-channels.md) for supervision tunables (backoff, circuit-break, etc.).

## Responsibility split

- **Process restart is LibreFang's job.**
  The supervisor in `librefang-channels::sidecar` respawns a crashed child with exponential backoff and a circuit-breaker.
  Your adapter must be *crash-safe*: hold no irreplaceable in-process state.
- **Platform reconnect is your adapter's job.**
  Reconnecting a dropped WebSocket / long-poll / SSE stream is your transport's concern.
  [`with_backoff`] helps.

## stdout is reserved

stdout is for protocol frames only.
Send all logs to stderr (the daemon collects them into its main log).

## License

MIT.
