//! LibreFang sidecar channel adapter SDK (Rust).
//!
//! Write a channel adapter in Rust that runs as a supervised subprocess of LibreFang, speaking newline-delimited JSON-RPC over stdio:
//!
//! ```ignore
//! use async_trait::async_trait;
//! use librefang_sidecar::{run_stdio, EmitFn, SendCommand, SidecarAdapter, events};
//!
//! struct MyAdapter;
//!
//! #[async_trait]
//! impl SidecarAdapter for MyAdapter {
//!     async fn on_send(&self, _cmd: SendCommand) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!         // deliver cmd.text / cmd.content to your platform
//!         Ok(())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     run_stdio(MyAdapter).await
//! }
//! ```
//!
//! The SDK is wire-equivalent with the Python SDK (`librefang.sidecar`) and with the Rust supervisor that lives in `crates/librefang-channels/src/sidecar.rs` inside the LibreFang daemon.
//! The three implementations are kept honest against each other by the shared corpus at `conformance/sidecar/corpus/`.
//!
//! See `docs/architecture/sidecar-channels.md` for the architecture and `docs/architecture/sidecar-protocol.md` for the normative wire spec.

pub mod protocol;
pub mod runtime;

// Re-export the most commonly used items at the top level to match the Python SDK's `librefang.sidecar` namespace shape.
// The inbound-send-command struct is exported as `SendCommand` (not `Send`) so a glob import like
// `use librefang_sidecar::*;` does not shadow `std::marker::Send` and break every `T: Send` trait bound downstream.
pub use protocol::{
    events, parse_command, ChannelUser, Command, Content, Field, FieldType, Interactive,
    InteractiveButton, InteractiveMessage, MessageBuilder, Reaction, Schema, SendCommand,
    StreamDelta, StreamEnd, StreamStart, TypingCmd, UnknownCommand,
};
pub use runtime::{
    run, run_stdio, run_stdio_main, run_stdio_with, with_backoff, DynError, EmitFn,
    ProducerCrashed, SidecarAdapter,
};
