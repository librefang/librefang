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
use librefang_sidecar::{run_stdio, EmitFn, MessageBuilder, Send as SendCmd, SidecarAdapter};
use std::sync::Arc;
use tokio::sync::Mutex;

struct EchoAdapter {
    /// Stash an emit handle so `on_send` can write a synthetic inbound `message` echoing what the daemon just sent us.
    /// `produce` runs once at startup and captures the handle here.
    emit: Arc<Mutex<Option<EmitFn>>>,
}

#[async_trait]
impl SidecarAdapter for EchoAdapter {
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }

    async fn on_send(&self, cmd: SendCmd) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Log to stderr — stdout is protocol-only.
        eprintln!("[echo] received send: {}", cmd.text);

        // Echo back as an inbound message from a synthetic user.
        if let Some(emit) = self.emit.lock().await.as_ref() {
            emit(
                MessageBuilder::new("echo-user", "Echo")
                    .text(format!("you said: {}", cmd.text))
                    .channel_id(cmd.channel_id.clone())
                    .platform("echo")
                    .build(),
            );
        }
        Ok(())
    }

    async fn produce(&self, emit: EmitFn) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Capture the emit handle for `on_send` to use, then block
        // forever (the framework will cancel us on shutdown).
        *self.emit.lock().await = Some(emit);
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
    run_stdio(EchoAdapter {
        emit: Arc::new(Mutex::new(None)),
    })
    .await
}
