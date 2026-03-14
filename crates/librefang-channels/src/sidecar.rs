//! Sidecar channel adapter — runs an external process that communicates via JSON-RPC over stdin/stdout.
//!
//! This allows external processes written in any language (Python, Go, JS, etc.)
//! to act as channel adapters without touching Rust code. Communication uses
//! newline-delimited JSON (one JSON object per line) over stdin/stdout.

use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, watch, Mutex};
use tracing::{debug, error, info, warn};

// ── JSON-RPC Protocol Types ────────────────────────────────────────

/// Messages from the sidecar process TO LibreFang (one JSON per line on stdout).
#[derive(Debug, Deserialize)]
#[serde(tag = "method")]
pub enum SidecarEvent {
    /// A new message received from the platform.
    #[serde(rename = "message")]
    Message { params: SidecarMessageParams },
    /// Adapter is ready to receive commands.
    #[serde(rename = "ready")]
    Ready,
    /// Adapter encountered an error.
    #[serde(rename = "error")]
    Error { params: SidecarErrorParams },
}

#[derive(Debug, Deserialize)]
pub struct SidecarMessageParams {
    pub user_id: String,
    pub user_name: String,
    pub text: Option<String>,
    pub channel_id: Option<String>,
    pub platform: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SidecarErrorParams {
    pub message: String,
}

/// Commands from LibreFang TO the sidecar process (one JSON per line on stdin).
#[derive(Debug, Serialize)]
#[serde(tag = "method")]
pub enum SidecarCommand {
    /// Send a message to the platform.
    #[serde(rename = "send")]
    Send { params: SidecarSendParams },
    /// Graceful shutdown request.
    #[serde(rename = "shutdown")]
    Shutdown,
}

#[derive(Debug, Serialize)]
pub struct SidecarSendParams {
    pub channel_id: String,
    pub text: String,
}

// ── Sidecar Adapter Implementation ─────────────────────────────────

/// A channel adapter that delegates to an external subprocess via JSON-RPC
/// over stdin/stdout.
pub struct SidecarAdapter {
    name: String,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    channel_type: ChannelType,
    /// Shared handle to the child's stdin for sending commands.
    stdin_tx: Arc<Mutex<Option<tokio::process::ChildStdin>>>,
    /// Handle to the child process (kept alive to prevent kill_on_drop).
    child: Arc<Mutex<Option<tokio::process::Child>>>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Current status.
    status: Arc<std::sync::Mutex<ChannelStatus>>,
}

impl SidecarAdapter {
    /// Create a new sidecar adapter from a config entry.
    pub fn new(config: &librefang_types::config::SidecarChannelConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let channel_type = config
            .channel_type
            .as_ref()
            .map(|s| ChannelType::Custom(s.clone()))
            .unwrap_or_else(|| ChannelType::Custom(config.name.clone()));

        Self {
            name: config.name.clone(),
            command: config.command.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            channel_type,
            stdin_tx: Arc::new(Mutex::new(None)),
            child: Arc::new(Mutex::new(None)),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            status: Arc::new(std::sync::Mutex::new(ChannelStatus::default())),
        }
    }

    /// Write a command to the sidecar process stdin.
    async fn send_command(&self, cmd: &SidecarCommand) -> Result<(), Box<dyn std::error::Error>> {
        let mut guard = self.stdin_tx.lock().await;
        let stdin = guard
            .as_mut()
            .ok_or("Sidecar process stdin not available")?;
        let mut line = serde_json::to_string(cmd)?;
        line.push('\n');
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for SidecarAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        info!(
            name = %self.name,
            command = %self.command,
            "Starting sidecar channel adapter"
        );

        let mut child = Command::new(&self.command)
            .args(&self.args)
            .envs(&self.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                format!(
                    "Failed to spawn sidecar '{}' ({}): {e}",
                    self.name, self.command
                )
            })?;

        // Take ownership of stdin
        let child_stdin = child
            .stdin
            .take()
            .ok_or("Failed to capture sidecar stdin")?;
        {
            let mut guard = self.stdin_tx.lock().await;
            *guard = Some(child_stdin);
        }

        // Take stdout for reading events
        let child_stdout = child
            .stdout
            .take()
            .ok_or("Failed to capture sidecar stdout")?;

        // Take stderr for logging
        let child_stderr = child
            .stderr
            .take()
            .ok_or("Failed to capture sidecar stderr")?;

        // Store child handle to keep the process alive
        {
            let mut guard = self.child.lock().await;
            *guard = Some(child);
        }

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let channel_type = self.channel_type.clone();
        let adapter_name = self.name.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let status = self.status.clone();

        // Mark as connected
        {
            let mut s = status.lock().unwrap_or_else(|e| e.into_inner());
            s.connected = true;
            s.started_at = Some(Utc::now());
        }

        // Spawn stderr forwarder
        let stderr_name = adapter_name.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(child_stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                warn!(adapter = %stderr_name, "[sidecar stderr] {line}");
            }
        });

        // Spawn stdout reader
        let status_clone = status.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(child_stdout);
            let mut lines = reader.lines();

            loop {
                tokio::select! {
                    result = lines.next_line() => {
                        match result {
                            Ok(Some(line)) => {
                                let line = line.trim().to_string();
                                if line.is_empty() {
                                    continue;
                                }
                                match serde_json::from_str::<SidecarEvent>(&line) {
                                    Ok(SidecarEvent::Ready) => {
                                        info!(adapter = %adapter_name, "Sidecar adapter ready");
                                    }
                                    Ok(SidecarEvent::Message { params }) => {
                                        debug!(
                                            adapter = %adapter_name,
                                            user = %params.user_name,
                                            "Received message from sidecar"
                                        );
                                        let msg = ChannelMessage {
                                            channel: channel_type.clone(),
                                            platform_message_id: uuid::Uuid::new_v4().to_string(),
                                            sender: ChannelUser {
                                                platform_id: params.user_id,
                                                display_name: params.user_name,
                                                librefang_user: None,
                                            },
                                            content: ChannelContent::Text(
                                                params.text.unwrap_or_default(),
                                            ),
                                            target_agent: None,
                                            timestamp: Utc::now(),
                                            is_group: false,
                                            thread_id: None,
                                            metadata: {
                                                let mut m = HashMap::new();
                                                if let Some(ch) = params.channel_id {
                                                    m.insert(
                                                        "channel_id".to_string(),
                                                        serde_json::Value::String(ch),
                                                    );
                                                }
                                                if let Some(p) = params.platform {
                                                    m.insert(
                                                        "platform".to_string(),
                                                        serde_json::Value::String(p),
                                                    );
                                                }
                                                m
                                            },
                                        };
                                        // Update status
                                        {
                                            let mut s = status_clone.lock().unwrap_or_else(|e| e.into_inner());
                                            s.messages_received += 1;
                                            s.last_message_at = Some(Utc::now());
                                        }
                                        if tx.send(msg).await.is_err() {
                                            debug!(adapter = %adapter_name, "Message receiver dropped, stopping sidecar reader");
                                            break;
                                        }
                                    }
                                    Ok(SidecarEvent::Error { params }) => {
                                        warn!(
                                            adapter = %adapter_name,
                                            error = %params.message,
                                            "Sidecar adapter reported error"
                                        );
                                        let mut s = status_clone.lock().unwrap_or_else(|e| e.into_inner());
                                        s.last_error = Some(params.message);
                                    }
                                    Err(e) => {
                                        warn!(
                                            adapter = %adapter_name,
                                            line = %line,
                                            "Failed to parse sidecar event: {e}"
                                        );
                                    }
                                }
                            }
                            Ok(None) => {
                                info!(adapter = %adapter_name, "Sidecar process stdout closed");
                                let mut s = status_clone.lock().unwrap_or_else(|e| e.into_inner());
                                s.connected = false;
                                break;
                            }
                            Err(e) => {
                                error!(adapter = %adapter_name, "Error reading sidecar stdout: {e}");
                                let mut s = status_clone.lock().unwrap_or_else(|e| e.into_inner());
                                s.connected = false;
                                s.last_error = Some(format!("stdout read error: {e}"));
                                break;
                            }
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        info!(adapter = %adapter_name, "Sidecar reader received shutdown signal");
                        break;
                    }
                }
            }
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let text = match content {
            ChannelContent::Text(t) => t,
            other => serde_json::to_string(&other)?,
        };

        let channel_id = user.platform_id.clone();
        let cmd = SidecarCommand::Send {
            params: SidecarSendParams { channel_id, text },
        };
        self.send_command(&cmd).await?;

        // Update status
        {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            s.messages_sent += 1;
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        info!(name = %self.name, "Stopping sidecar channel adapter");

        // Send shutdown command (best-effort)
        let _ = self.send_command(&SidecarCommand::Shutdown).await;

        // Signal shutdown to the reader task
        let _ = self.shutdown_tx.send(true);

        // Close stdin to signal EOF
        {
            let mut guard = self.stdin_tx.lock().await;
            *guard = None;
        }

        // Wait briefly, then kill the child process
        {
            let mut guard = self.child.lock().await;
            if let Some(ref mut child) = *guard {
                // Give the process a moment to exit gracefully
                match tokio::time::timeout(std::time::Duration::from_secs(2), child.wait()).await {
                    Ok(Ok(status)) => {
                        debug!(name = %self.name, ?status, "Sidecar process exited");
                    }
                    _ => {
                        // Force kill if it didn't exit
                        let _ = child.kill().await;
                        debug!(name = %self.name, "Sidecar process killed");
                    }
                }
            }
            *guard = None;
        }

        // Update status
        {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            s.connected = false;
        }

        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        self.status
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sidecar_event_message_deserialization() {
        let json = r#"{"method":"message","params":{"user_id":"u1","user_name":"Alice","text":"Hello","channel_id":"ch1","platform":"test"}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Message { params } => {
                assert_eq!(params.user_id, "u1");
                assert_eq!(params.user_name, "Alice");
                assert_eq!(params.text, Some("Hello".to_string()));
                assert_eq!(params.channel_id, Some("ch1".to_string()));
                assert_eq!(params.platform, Some("test".to_string()));
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_sidecar_event_ready_deserialization() {
        let json = r#"{"method":"ready"}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        assert!(matches!(event, SidecarEvent::Ready));
    }

    #[test]
    fn test_sidecar_event_error_deserialization() {
        let json = r#"{"method":"error","params":{"message":"Connection failed"}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Error { params } => {
                assert_eq!(params.message, "Connection failed");
            }
            _ => panic!("Expected Error variant"),
        }
    }

    #[test]
    fn test_sidecar_event_message_minimal() {
        let json = r#"{"method":"message","params":{"user_id":"u1","user_name":"Bot"}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Message { params } => {
                assert_eq!(params.user_id, "u1");
                assert!(params.text.is_none());
                assert!(params.channel_id.is_none());
                assert!(params.platform.is_none());
            }
            _ => panic!("Expected Message variant"),
        }
    }

    #[test]
    fn test_sidecar_command_send_serialization() {
        let cmd = SidecarCommand::Send {
            params: SidecarSendParams {
                channel_id: "ch1".to_string(),
                text: "Hello world".to_string(),
            },
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""method":"send"#));
        assert!(json.contains(r#""channel_id":"ch1"#));
        assert!(json.contains(r#""text":"Hello world"#));
    }

    #[test]
    fn test_sidecar_command_shutdown_serialization() {
        let cmd = SidecarCommand::Shutdown;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains(r#""method":"shutdown"#));
    }

    #[test]
    fn test_sidecar_command_send_roundtrip() {
        let cmd = SidecarCommand::Send {
            params: SidecarSendParams {
                channel_id: "test-channel".to_string(),
                text: "Test message with \"quotes\" and \nnewlines".to_string(),
            },
        };
        let json = serde_json::to_string(&cmd).unwrap();
        // Verify it's valid JSON that can be parsed back
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["method"], "send");
        assert_eq!(value["params"]["channel_id"], "test-channel");
    }

    #[tokio::test]
    async fn test_sidecar_adapter_spawn_echo() {
        // Integration test: spawn the Python echo adapter if python3 is available
        let python = which_python();
        if python.is_none() {
            // Skip test if python3 is not available
            return;
        }
        let python = python.unwrap();

        // Find the example adapter
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let adapter_path = std::path::Path::new(manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("examples/sidecar-channel-python/adapter.py");

        if !adapter_path.exists() {
            // Skip if the example doesn't exist yet
            return;
        }

        let config = librefang_types::config::SidecarChannelConfig {
            name: "test-echo".to_string(),
            command: python,
            args: vec![adapter_path.to_string_lossy().to_string()],
            env: HashMap::new(),
            channel_type: None,
        };

        let adapter = SidecarAdapter::new(&config);
        let mut stream = adapter.start().await.unwrap();

        use futures::StreamExt;

        // Wait for the process to start and emit the "ready" event.
        // The ready event is consumed by the reader task (not forwarded as a ChannelMessage),
        // so we just need a short delay for the process to boot.
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;

        // Send a message to trigger an echo
        adapter
            .send(
                &ChannelUser {
                    platform_id: "test-ch".to_string(),
                    display_name: "Tester".to_string(),
                    librefang_user: None,
                },
                ChannelContent::Text("Hello sidecar!".to_string()),
            )
            .await
            .expect("Failed to send message to sidecar — process may have exited early");

        // Read the echo reply
        let msg = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
            .await
            .expect("Timed out waiting for echo reply")
            .expect("Stream ended unexpectedly");

        match &msg.content {
            ChannelContent::Text(t) => {
                assert!(t.contains("Echo:"), "Expected echo response, got: {t}");
                assert!(
                    t.contains("Hello sidecar!"),
                    "Expected echoed text, got: {t}"
                );
            }
            other => panic!("Expected Text content, got: {other:?}"),
        }

        // Stop the adapter
        adapter.stop().await.unwrap();
        let status = adapter.status();
        assert!(!status.connected);
    }

    /// Find python3 or python on PATH.
    fn which_python() -> Option<String> {
        for name in &["python3", "python"] {
            if std::process::Command::new(name)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok()
            {
                return Some(name.to_string());
            }
        }
        None
    }
}
