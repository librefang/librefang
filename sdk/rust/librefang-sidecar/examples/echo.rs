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
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

struct EchoAdapter {
    /// Stash the emit handle that `produce` is given so `on_send` can write a synthetic inbound `message` echoing what the daemon just sent us.
    /// Wrapped in `std::sync::Mutex` (not `tokio::sync::Mutex`) because no `.await` happens while the guard is held — the lock is purely for `Option::clone` / `Option::replace`.
    emit: Arc<Mutex<Option<EmitFn>>>,
    /// Fires when `produce` finishes capturing the emit handle, so an `on_send` that arrives before `produce` has been scheduled (the supervisor can write a queued `send` to stdin before the runtime spawns the producer task) waits instead of silently dropping the echo.
    emit_ready: Arc<Notify>,
}

impl EchoAdapter {
    fn new() -> Self {
        Self {
            emit: Arc::new(Mutex::new(None)),
            emit_ready: Arc::new(Notify::new()),
        }
    }

    fn schema() -> Schema {
        Schema::new(
            "rust-echo",
            "Rust Echo",
            "Minimal echo sidecar — emits each inbound send back as a synthetic message. No platform integration.",
            vec![Field::new("greeting", "Optional greeting prefix", FieldType::Text).placeholder("you said:")],
        )
    }

    /// Acquire a fresh clone of the emit handle, waiting until `produce` has installed it.
    /// Loop on `notified()` because `Notify` only signals waiters subscribed before the call to `notify_waiters`; a stricter wait-then-recheck pattern survives the cold-start race regardless of scheduling order.
    async fn wait_for_emit(&self) -> EmitFn {
        loop {
            if let Some(e) = self.emit.lock().unwrap().clone() {
                return e;
            }
            self.emit_ready.notified().await;
        }
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
        let emit = self.wait_for_emit().await;
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
        // Install the emit handle, wake any on_send already waiting, then block forever so the runtime keeps treating us as live (a clean Ok(()) return is also fine — the run loop only exits the produce side on Err — but `pending` keeps the cancellation point explicit).
        *self.emit.lock().unwrap() = Some(emit);
        self.emit_ready.notify_waiters();
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
    // `run_stdio_main` handles the daemon's `--describe` discovery contract (emit schema JSON + exit) before any platform-side state is touched, and otherwise drives `run_stdio`.
    run_stdio_main(EchoAdapter::new(), EchoAdapter::schema()).await
}
