# Custom Channel Adapter Example

This guide walks you through implementing a new channel adapter for LibreFang.
Channel adapters bridge external messaging platforms (Slack, Telegram, IRC, etc.)
into the LibreFang kernel by converting platform-specific messages into unified
`ChannelMessage` events.

LibreFang ships with 40+ adapters in `crates/librefang-channels/src/`. Each one
is gated behind a cargo feature flag so users only compile what they need.

## The `ChannelAdapter` Trait

Every adapter implements the `ChannelAdapter` trait defined in
`crates/librefang-channels/src/types.rs`:

```rust
use async_trait::async_trait;
use std::pin::Pin;
use futures::Stream;

#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Human-readable name (e.g. "myplatform").
    fn name(&self) -> &str;

    /// The channel type this adapter handles.
    fn channel_type(&self) -> ChannelType;

    /// Start receiving messages. Returns a stream of incoming ChannelMessage.
    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>;

    /// Send a response back to a user on this channel.
    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>>;

    /// Send a typing indicator (optional -- default no-op).
    async fn send_typing(&self, _user: &ChannelUser) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Send a lifecycle reaction to a message (optional -- default no-op).
    async fn send_reaction(
        &self,
        _user: &ChannelUser,
        _message_id: &str,
        _reaction: &LifecycleReaction,
    ) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    /// Stop the adapter and clean up resources.
    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>>;

    /// Current health status (optional -- default returns disconnected).
    fn status(&self) -> ChannelStatus {
        ChannelStatus::default()
    }

    /// Reply in a thread (optional -- default falls back to send()).
    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        _thread_id: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.send(user, content).await
    }
}
```

Only `name()`, `channel_type()`, `start()`, `send()`, and `stop()` are required.
The rest have default implementations you can override when your platform supports
typing indicators, reactions, or threaded replies.

## Step-by-Step: Implement a New Adapter

We will create a fictional `"myplatform"` adapter as an example.

### 1. Create the adapter source file

Add `crates/librefang-channels/src/myplatform.rs`:

```rust
//! MyPlatform channel adapter.

use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use zeroize::Zeroizing;

pub struct MyPlatformAdapter {
    /// SECURITY: credentials are zeroized on drop.
    api_key: Zeroizing<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    client: reqwest::Client,
}

impl MyPlatformAdapter {
    pub fn new(api_key: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            api_key: Zeroizing::new(api_key),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl ChannelAdapter for MyPlatformAdapter {
    fn name(&self) -> &str {
        "myplatform"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("myplatform".to_string())
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>> {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let mut shutdown_rx = self.shutdown_rx.clone();

        info!("MyPlatform adapter starting");

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        info!("MyPlatform adapter shutting down");
                        return;
                    }
                    // Replace this with your platform's polling/websocket/SSE logic.
                    // When a message arrives, convert it to ChannelMessage and send:
                    //
                    // let msg = ChannelMessage {
                    //     channel: ChannelType::Custom("myplatform".to_string()),
                    //     platform_message_id: "msg-123".to_string(),
                    //     sender: ChannelUser {
                    //         platform_id: "user-456".to_string(),
                    //         display_name: "Alice".to_string(),
                    //         librefang_user: None,
                    //     },
                    //     content: ChannelContent::Text("Hello!".to_string()),
                    //     target_agent: None,
                    //     timestamp: Utc::now(),
                    //     is_group: false,
                    //     thread_id: None,
                    //     metadata: HashMap::new(),
                    // };
                    // let _ = tx.send(msg).await;
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let text = match content {
            ChannelContent::Text(t) => t,
            _ => "(Unsupported content type)".to_string(),
        };

        // POST the response back to your platform's API.
        // Use self.client and self.api_key here.
        info!("MyPlatform: sending to {}: {}", user.platform_id, text);
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}
```

Key patterns to follow:
- Use `Zeroizing<String>` for secrets (API keys, tokens) so they are wiped from memory on drop.
- Use `watch::channel(false)` for graceful shutdown signaling.
- Use `mpsc::channel` to bridge async message reception into the `Stream` that the kernel consumes.
- Parse `/command args` into `ChannelContent::Command` if your platform supports slash commands.
- Use `split_message(text, MAX_LEN)` (from `crate::types`) to chunk long replies.

### 2. Register the module with a feature gate

In `crates/librefang-channels/src/lib.rs`, add (in alphabetical order):

```rust
#[cfg(feature = "channel-myplatform")]
pub mod myplatform;
```

### 3. Add the feature flag to Cargo.toml

In `crates/librefang-channels/Cargo.toml`:

```toml
# Under [features], add the individual feature:
channel-myplatform = []

# If it needs extra dependencies:
channel-myplatform = ["dep:some-sdk"]
```

Then add `"channel-myplatform"` to the `all-channels` feature list. If it should
be compiled by default, also add it to `default`.

### 4. Add unit tests

At the bottom of your adapter file, add a `#[cfg(test)] mod tests` block. At
minimum, test:

- Adapter creation and `name()` / `channel_type()` return values
- Any parsing/signature logic (see `webhook.rs` for signature tests)
- Serialization round-trips for any custom types

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_myplatform_adapter_creation() {
        let adapter = MyPlatformAdapter::new("test-key".to_string());
        assert_eq!(adapter.name(), "myplatform");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("myplatform".to_string())
        );
    }
}
```

### 5. Run the checks

```bash
# Build the workspace with your feature enabled
cargo build --workspace --lib --features channel-myplatform

# Run all tests
cargo test --workspace --features channel-myplatform

# Lint
cargo clippy --workspace --all-targets --features channel-myplatform -- -D warnings
```

## Reference Adapters

For real-world examples, study these existing adapters (simplest to most complex):

| Adapter | File | Pattern |
|---------|------|---------|
| **ntfy** | `ntfy.rs` | SSE subscription + plain POST publishing |
| **webhook** | `webhook.rs` | HTTP server with HMAC-SHA256 signature verification |
| **gotify** | `gotify.rs` | WebSocket subscription + REST API publishing |
| **slack** | `slack.rs` | WebSocket (Socket Mode) + Web API |
| **telegram** | `telegram.rs` | Long-polling + Bot API |

## Further Reading

See [channel integrations docs](https://docs.librefang.ai/integrations/channels) for the full
architecture reference, including the channel bridge, message router, formatter
pipeline, and config TOML schema.
