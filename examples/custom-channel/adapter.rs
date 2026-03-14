//! Minimal custom channel adapter example.
//!
//! This is a standalone example showing the structure of a `ChannelAdapter`
//! implementation. It is NOT a compilable crate — it demonstrates the pattern
//! you would follow inside `crates/librefang-channels/src/`.
//!
//! This example adapter watches a directory for `.txt` files (simulating
//! incoming messages) and writes responses to an output directory.

use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::watch;

// These imports come from `crate::types` when inside librefang-channels.
use librefang_channels::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
    LifecycleReaction,
};

/// A minimal channel adapter that reads messages from a directory.
pub struct FileChannelAdapter {
    /// Directory to watch for incoming `.txt` files.
    inbox_dir: String,
    /// Directory to write outbound responses.
    outbox_dir: String,
    /// Shutdown signal sender.
    shutdown_tx: Arc<watch::Sender<bool>>,
    /// Shutdown signal receiver.
    shutdown_rx: watch::Receiver<bool>,
}

impl FileChannelAdapter {
    pub fn new(inbox_dir: String, outbox_dir: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            inbox_dir,
            outbox_dir,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
}

#[async_trait]
impl ChannelAdapter for FileChannelAdapter {
    fn name(&self) -> &str {
        "file-channel"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("file".to_string())
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        let inbox = self.inbox_dir.clone();
        let mut shutdown = self.shutdown_rx.clone();

        // Create a stream that polls the inbox directory for new .txt files.
        let stream = async_stream::stream! {
            let mut seen = std::collections::HashSet::new();
            loop {
                tokio::select! {
                    _ = shutdown.changed() => break,
                    _ = tokio::time::sleep(std::time::Duration::from_secs(2)) => {}
                }

                if let Ok(entries) = std::fs::read_dir(&inbox) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().map_or(false, |e| e == "txt")
                            && seen.insert(path.clone())
                        {
                            if let Ok(text) = std::fs::read_to_string(&path) {
                                yield ChannelMessage {
                                    channel: ChannelType::Custom("file".to_string()),
                                    platform_message_id: path.display().to_string(),
                                    sender: ChannelUser {
                                        platform_id: "file-user".to_string(),
                                        display_name: "File User".to_string(),
                                        librefang_user: None,
                                    },
                                    content: ChannelContent::Text(text),
                                    target_agent: None,
                                    timestamp: Utc::now(),
                                    is_group: false,
                                    thread_id: None,
                                    metadata: HashMap::new(),
                                };
                            }
                        }
                    }
                }
            }
        };

        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let ChannelContent::Text(text) = &content {
            let filename = format!(
                "{}/response-{}.txt",
                self.outbox_dir,
                Utc::now().timestamp_millis()
            );
            std::fs::write(&filename, text)?;
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        ChannelStatus {
            connected: !*self.shutdown_rx.borrow(),
            ..Default::default()
        }
    }
}
