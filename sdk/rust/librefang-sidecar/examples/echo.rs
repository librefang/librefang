//! Minimal echo sidecar — does not talk to any platform; on every `send` command from the daemon it emits a synthetic `message` event that contains the same text, attributed to a fake user.
//! Useful as a smoke test against the LibreFang supervisor and as a template for new adapters.
//!
//! Run as a sidecar by adding to `~/.librefang/config.toml`:
//!
//! ```toml
//! [[sidecar_channels]]
//! name = "rust-echo"
//! command = "/abs/path/to/target/debug/examples/echo"
//! args = []
//! restart = true
//! ```

use async_trait::async_trait;
use librefang_sidecar::{
    run_stdio_main, EmitFn, Field, FieldType, MessageBuilder, Schema, SendCommand, SidecarAdapter,
};
use tokio::sync::watch;

struct EchoAdapter {
    /// `produce` writes `Some(emit)` to this channel as its first action; `on_send` does `rx.wait_for(|v| v.is_some())` to acquire the handle.
    /// Uses `tokio::sync::watch` instead of `Notify + Mutex<Option<EmitFn>>` because watch has proper signal-storage semantics: a `send` made before any waiter exists is still observable by the next waiter, eliminating the cold-start lost-wake race where `on_send` arriving before `produce` is scheduled would otherwise park forever.
    emit_tx: watch::Sender<Option<EmitFn>>,
    emit_rx: watch::Receiver<Option<EmitFn>>,
}

impl EchoAdapter {
    fn new() -> Self {
        let (emit_tx, emit_rx) = watch::channel(None);
        Self { emit_tx, emit_rx }
    }

    fn schema() -> Schema {
        Schema::new(
            "rust-echo",
            "Rust Echo",
            "Minimal echo sidecar — emits each inbound send back as a synthetic message. No platform integration.",
            vec![Field::new("greeting", "Optional greeting prefix", FieldType::Text)
                .placeholder("you said:")],
        )
    }
}

#[async_trait]
impl SidecarAdapter for EchoAdapter {
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }

    async fn on_send(
        &self,
        cmd: SendCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Log to stderr — stdout is protocol-only.
        eprintln!("[echo] received send: {}", cmd.text);
        // Wait until `produce` has published the emit handle.
        // `watch::Receiver::wait_for` returns immediately if the predicate is already true (so a `send` arriving after `produce` does not block), and parks otherwise — surviving the cold-start race regardless of scheduling order.
        let mut rx = self.emit_rx.clone();
        let guard = rx.wait_for(|v| v.is_some()).await.map_err(
            |e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("emit channel closed: {e}").into()
            },
        )?;
        let emit = guard
            .as_ref()
            .expect("wait_for predicate guarantees Some")
            .clone();
        // Release the watch read-borrow before invoking `emit` so a copy-pasted adapter whose emit closure mutates `self.emit_tx` or any other watch state cannot deadlock — echo itself only writes through the mpsc inside `emit`, so the borrow could outlive this call without harm, but the example sets the pattern.
        drop(guard);
        emit(
            MessageBuilder::new("echo-user", "Echo")
                .text(format!("you said: {}", cmd.text))
                .channel_id(cmd.channel_id.clone())
                .platform("echo")
                .build(),
        );
        Ok(())
    }

    async fn produce(&self, emit: EmitFn) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Publish the emit handle so any concurrent on_send is unblocked, then park forever so the runtime keeps treating us as live.
        // A clean Ok(()) return would also be fine — the run loop only exits the produce side on Err — but `pending` keeps the cancellation point explicit, and the runtime now aborts the inner produce task on shutdown so this future does not leak past run() return.
        let _ = self.emit_tx.send(Some(emit));
        std::future::pending::<()>().await;
        Ok(())
    }

    async fn on_shutdown(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        eprintln!("[echo] clean shutdown");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // `run_stdio_main` handles the daemon's `--describe` discovery contract (emit schema JSON + return) before touching any platform-side state, and only constructs the adapter via the builder closure when not in discovery mode — important for adapters whose `new()` reads env vars that are not yet configured at boot.
    // The builder closure returns `Result<EchoAdapter, DynError>` so a real adapter that validates env vars in `new()` can fail cleanly with a structured error instead of `expect()`-panicking; echo has nothing to fail on so it just wraps in `Ok`.
    run_stdio_main(EchoAdapter::schema, || Ok(EchoAdapter::new())).await
}
