//! Sidecar channel adapter — runs an external process that communicates via JSON-RPC over stdin/stdout.
//!
//! This allows external processes written in any language (Python, Go, JS, etc.)
//! to act as channel adapters without touching Rust code. Communication uses
//! newline-delimited JSON (one JSON object per line) over stdin/stdout.

use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
    GroupMember, ParticipantRef,
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
    ///
    /// Boxed: `SidecarMessageParams` carries full `ChannelContent` +
    /// group rosters, so it dwarfs the other variants
    /// (clippy::large_enum_variant). Box keeps `SidecarEvent` small;
    /// serde and field access (incl. partial moves) are transparent.
    #[serde(rename = "message")]
    Message { params: Box<SidecarMessageParams> },
    /// Adapter is ready to receive commands.
    #[serde(rename = "ready")]
    Ready,
    /// Adapter encountered an error.
    #[serde(rename = "error")]
    Error { params: SidecarErrorParams },
    /// A typing indicator from the platform.
    ///
    /// P0 skeleton: not yet wired through to `ChannelAdapter::typing_events`
    /// — that happens in P2. Present now so external adapters can be
    /// developed against the final wire shape.
    #[serde(rename = "typing")]
    Typing { params: SidecarTypingParams },
}

#[derive(Debug, Deserialize)]
pub struct SidecarMessageParams {
    pub user_id: String,
    pub user_name: String,
    pub text: Option<String>,
    pub channel_id: Option<String>,
    pub platform: Option<String>,
    /// Full structured content. When present, supersedes `text`.
    /// Legacy text-only adapters omit this and keep working.
    #[serde(default)]
    pub content: Option<ChannelContent>,
    /// Sender `@handle` if the platform exposes one. Folded into
    /// message metadata — `ChannelUser` has no handle slot, and
    /// routing/identity is the bridge's concern, not the adapter's.
    #[serde(default)]
    pub username: Option<String>,
    /// Optional mapping to a LibreFang user identity.
    #[serde(default)]
    pub librefang_user: Option<String>,
    /// Whether this message came from a group chat (vs DM).
    #[serde(default)]
    pub is_group: bool,
    /// Thread / reply-to identifier, if any.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// Group roster, folded into metadata. The bridge owns
    /// `SenderContext`; the adapter only transports the data.
    #[serde(default)]
    pub group_members: Vec<GroupMember>,
    /// Group participant refs, folded into metadata.
    #[serde(default)]
    pub group_participants: Vec<ParticipantRef>,
    /// Free-form metadata merged into the `ChannelMessage` metadata map.
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SidecarErrorParams {
    pub message: String,
}

/// Inbound typing indicator params (P0 skeleton — consumed in P2).
#[derive(Debug, Deserialize)]
pub struct SidecarTypingParams {
    pub user_id: String,
    pub user_name: String,
    pub is_typing: bool,
}

/// Commands from LibreFang TO the sidecar process (one JSON per line on stdin).
#[derive(Debug, Serialize)]
#[serde(tag = "method")]
pub enum SidecarCommand {
    /// Send a message to the platform.
    #[serde(rename = "send")]
    Send { params: SidecarSendParams },
    /// Acknowledge a `ready` event so the adapter stops re-announcing.
    /// P0 skeleton — the ready/ack handshake is wired in P2.
    #[serde(rename = "ready_ack")]
    ReadyAck,
    /// Send a typing indicator to the platform.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "typing")]
    Typing { params: SidecarTypingCmdParams },
    /// Add a reaction to a platform message.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "reaction")]
    Reaction { params: SidecarReactionParams },
    /// Send an interactive (buttons) message.
    /// P0 skeleton — full button shape lands in P2.
    #[serde(rename = "interactive")]
    Interactive { params: SidecarInteractiveParams },
    /// Begin a streamed response.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "stream_start")]
    StreamStart { params: SidecarStreamStartParams },
    /// A chunk of a streamed response.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "stream_delta")]
    StreamDelta { params: SidecarStreamDeltaParams },
    /// End a streamed response.
    /// P0 skeleton — wired in P2.
    #[serde(rename = "stream_end")]
    StreamEnd { params: SidecarStreamEndParams },
    /// Liveness ping.
    /// P0 skeleton — optional keepalive wired in P2.
    #[serde(rename = "heartbeat")]
    Heartbeat,
    /// Graceful shutdown request.
    #[serde(rename = "shutdown")]
    Shutdown,
}

#[derive(Debug, Serialize)]
pub struct SidecarSendParams {
    pub channel_id: String,
    /// Best-effort flattened text. Legacy adapters read only this;
    /// new adapters read the full `content`.
    pub text: String,
    /// Full structured content (every `ChannelContent` variant).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<ChannelContent>,
    /// Thread to reply into, if any. Populated by `send_in_thread`
    /// (wired in P2); plain `send` leaves it `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Full sender identity (`channel_id` is `user.platform_id`).
    pub user: ChannelUser,
}

/// `typing` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarTypingCmdParams {
    pub channel_id: String,
}

/// `reaction` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarReactionParams {
    pub channel_id: String,
    pub message_id: String,
    pub reaction: String,
}

/// `interactive` command params (P0 skeleton — full button shape lands in P2).
#[derive(Debug, Serialize)]
pub struct SidecarInteractiveParams {
    pub channel_id: String,
    pub text: String,
}

/// `stream_start` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarStreamStartParams {
    pub channel_id: String,
    pub stream_id: String,
}

/// `stream_delta` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarStreamDeltaParams {
    pub stream_id: String,
    pub text: String,
}

/// `stream_end` command params (P0 skeleton — wired in P2).
#[derive(Debug, Serialize)]
pub struct SidecarStreamEndParams {
    pub stream_id: String,
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
    async fn send_command(
        &self,
        cmd: &SidecarCommand,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        info!(
            name = %self.name,
            command = %self.command,
            "Starting sidecar channel adapter"
        );

        let mut cmd = Command::new(&self.command);
        cmd.args(&self.args)
            .envs(&self.env)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|e| {
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
                                        // Unbox once: the field moves below
                                        // (metadata/content/text/sender) need
                                        // an owned local, not a Box (partial
                                        // moves out of Box aren't allowed).
                                        let params = *params;
                                        debug!(
                                            adapter = %adapter_name,
                                            user = %params.user_name,
                                            "Received message from sidecar"
                                        );
                                        let mut metadata = params.metadata;
                                        if let Some(ch) = params.channel_id {
                                            metadata.insert(
                                                "channel_id".to_string(),
                                                serde_json::Value::String(ch),
                                            );
                                        }
                                        if let Some(p) = params.platform {
                                            metadata.insert(
                                                "platform".to_string(),
                                                serde_json::Value::String(p),
                                            );
                                        }
                                        if let Some(h) = params.username {
                                            metadata.insert(
                                                "username".to_string(),
                                                serde_json::Value::String(h),
                                            );
                                        }
                                        if !params.group_members.is_empty() {
                                            if let Ok(v) = serde_json::to_value(
                                                &params.group_members,
                                            ) {
                                                metadata.insert(
                                                    "group_members".to_string(),
                                                    v,
                                                );
                                            }
                                        }
                                        if !params.group_participants.is_empty() {
                                            if let Ok(v) = serde_json::to_value(
                                                &params.group_participants,
                                            ) {
                                                metadata.insert(
                                                    "group_participants"
                                                        .to_string(),
                                                    v,
                                                );
                                            }
                                        }
                                        // `content` supersedes `text`; legacy
                                        // text-only adapters omit it and fall
                                        // back to Text(text).
                                        let content = params
                                            .content
                                            .unwrap_or_else(|| {
                                                ChannelContent::Text(
                                                    params
                                                        .text
                                                        .unwrap_or_default(),
                                                )
                                            });
                                        let msg = ChannelMessage {
                                            channel: channel_type.clone(),
                                            platform_message_id: uuid::Uuid::new_v4().to_string(),
                                            sender: ChannelUser {
                                                platform_id: params.user_id,
                                                display_name: params.user_name,
                                                librefang_user: params.librefang_user,
                                            },
                                            content,
                                            target_agent: None,
                                            timestamp: Utc::now(),
                                            is_group: params.is_group,
                                            thread_id: params.thread_id,
                                            metadata,
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
                                    Ok(other) => {
                                        // P0 skeleton: protocol variants such as
                                        // `Typing` are placeholders, wired in P2.
                                        // Inert here — existing variant behaviour
                                        // is unchanged.
                                        debug!(
                                            adapter = %adapter_name,
                                            "Ignoring not-yet-wired sidecar event: {other:?}"
                                        );
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
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Legacy adapters read only `text`; flatten best-effort.
        // New adapters read the full structured `content`.
        let text = match &content {
            ChannelContent::Text(t) => t.clone(),
            other => serde_json::to_string(other)?,
        };

        let cmd = SidecarCommand::Send {
            params: SidecarSendParams {
                channel_id: user.platform_id.clone(),
                text,
                content: Some(content),
                thread_id: None,
                user: user.clone(),
            },
        };
        self.send_command(&cmd).await?;

        // Update status
        {
            let mut s = self.status.lock().unwrap_or_else(|e| e.into_inner());
            s.messages_sent += 1;
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
    use crate::types::{InteractiveButton, MediaGroupItem};

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
                content: None,
                thread_id: None,
                user: ChannelUser {
                    platform_id: "ch1".to_string(),
                    display_name: "Tester".to_string(),
                    librefang_user: None,
                },
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
                content: None,
                thread_id: None,
                user: ChannelUser {
                    platform_id: "test-channel".to_string(),
                    display_name: "Tester".to_string(),
                    librefang_user: None,
                },
            },
        };
        let json = serde_json::to_string(&cmd).unwrap();
        // Verify it's valid JSON that can be parsed back
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["method"], "send");
        assert_eq!(value["params"]["channel_id"], "test-channel");
    }

    // ── P0 skeleton: new protocol variant roundtrips ──────────────

    #[test]
    fn test_sidecar_event_typing_deserialization() {
        let json =
            r#"{"method":"typing","params":{"user_id":"u1","user_name":"Alice","is_typing":true}}"#;
        let event: SidecarEvent = serde_json::from_str(json).unwrap();
        match event {
            SidecarEvent::Typing { params } => {
                assert_eq!(params.user_id, "u1");
                assert_eq!(params.user_name, "Alice");
                assert!(params.is_typing);
            }
            _ => panic!("Expected Typing variant"),
        }
    }

    #[test]
    fn test_legacy_events_still_parse_after_typing_added() {
        // Regression guard: adding SidecarEvent::Typing must not change
        // parsing of the pre-existing variants.
        assert!(matches!(
            serde_json::from_str::<SidecarEvent>(r#"{"method":"ready"}"#).unwrap(),
            SidecarEvent::Ready
        ));
        assert!(matches!(
            serde_json::from_str::<SidecarEvent>(
                r#"{"method":"message","params":{"user_id":"u","user_name":"n"}}"#
            )
            .unwrap(),
            SidecarEvent::Message { .. }
        ));
    }

    #[test]
    fn test_new_command_variants_serialize_with_distinct_tags() {
        let cmds = vec![
            SidecarCommand::ReadyAck,
            SidecarCommand::Typing {
                params: SidecarTypingCmdParams {
                    channel_id: "c".to_string(),
                },
            },
            SidecarCommand::Reaction {
                params: SidecarReactionParams {
                    channel_id: "c".to_string(),
                    message_id: "m".to_string(),
                    reaction: "👍".to_string(),
                },
            },
            SidecarCommand::Interactive {
                params: SidecarInteractiveParams {
                    channel_id: "c".to_string(),
                    text: "pick".to_string(),
                },
            },
            SidecarCommand::StreamStart {
                params: SidecarStreamStartParams {
                    channel_id: "c".to_string(),
                    stream_id: "s".to_string(),
                },
            },
            SidecarCommand::StreamDelta {
                params: SidecarStreamDeltaParams {
                    stream_id: "s".to_string(),
                    text: "chunk".to_string(),
                },
            },
            SidecarCommand::StreamEnd {
                params: SidecarStreamEndParams {
                    stream_id: "s".to_string(),
                },
            },
            SidecarCommand::Heartbeat,
        ];

        let mut tags = std::collections::BTreeSet::new();
        for cmd in &cmds {
            let v: serde_json::Value =
                serde_json::from_str(&serde_json::to_string(cmd).unwrap()).unwrap();
            let tag = v["method"].as_str().unwrap().to_string();
            assert!(tags.insert(tag.clone()), "duplicate method tag: {tag}");
        }
        let expected: std::collections::BTreeSet<String> = [
            "ready_ack",
            "typing",
            "reaction",
            "interactive",
            "stream_start",
            "stream_delta",
            "stream_end",
            "heartbeat",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(tags, expected);
        // Legacy tags unchanged.
        assert_eq!(
            serde_json::to_string(&SidecarCommand::Shutdown).unwrap(),
            r#"{"method":"shutdown"}"#
        );
    }

    // ── P1: structured content I/O roundtrips ─────────────────────

    fn all_channel_content_variants() -> Vec<ChannelContent> {
        let btn = InteractiveButton {
            label: "Yes".to_string(),
            action: "yes".to_string(),
            style: Some("primary".to_string()),
            url: None,
        };
        vec![
            ChannelContent::Text("hello".to_string()),
            ChannelContent::Image {
                url: "https://x/i.png".to_string(),
                caption: Some("cap".to_string()),
                mime_type: Some("image/png".to_string()),
            },
            ChannelContent::File {
                url: "https://x/f.pdf".to_string(),
                filename: "f.pdf".to_string(),
            },
            ChannelContent::FileData {
                data: vec![1, 2, 3, 4],
                filename: "b.bin".to_string(),
                mime_type: "application/octet-stream".to_string(),
            },
            ChannelContent::Voice {
                url: "https://x/v.ogg".to_string(),
                caption: None,
                duration_seconds: 5,
            },
            ChannelContent::Video {
                url: "https://x/v.mp4".to_string(),
                caption: Some("c".to_string()),
                duration_seconds: 12,
                filename: Some("v.mp4".to_string()),
            },
            ChannelContent::Location {
                lat: 51.5,
                lon: -0.12,
            },
            ChannelContent::Command {
                name: "start".to_string(),
                args: vec!["a".to_string(), "b".to_string()],
            },
            ChannelContent::Interactive {
                text: "pick".to_string(),
                buttons: vec![vec![btn.clone()]],
            },
            ChannelContent::ButtonCallback {
                action: "yes".to_string(),
                message_text: Some("orig".to_string()),
            },
            ChannelContent::DeleteMessage {
                message_id: "m1".to_string(),
            },
            ChannelContent::EditInteractive {
                message_id: "m1".to_string(),
                text: "new".to_string(),
                buttons: vec![vec![btn.clone()]],
            },
            ChannelContent::Audio {
                url: "https://x/a.mp3".to_string(),
                caption: None,
                duration_seconds: 200,
                title: Some("Song".to_string()),
                performer: Some("Artist".to_string()),
            },
            ChannelContent::Animation {
                url: "https://x/a.gif".to_string(),
                caption: None,
                duration_seconds: 3,
            },
            ChannelContent::Sticker {
                file_id: "stk_1".to_string(),
            },
            ChannelContent::MediaGroup {
                items: vec![
                    MediaGroupItem::Photo {
                        url: "https://x/1.jpg".to_string(),
                        caption: Some("one".to_string()),
                    },
                    MediaGroupItem::Video {
                        url: "https://x/2.mp4".to_string(),
                        caption: None,
                        duration_seconds: 7,
                    },
                ],
            },
            ChannelContent::Poll {
                question: "Q?".to_string(),
                options: vec!["A".to_string(), "B".to_string()],
                is_quiz: true,
                correct_option_id: Some(1),
                explanation: Some("because".to_string()),
            },
            ChannelContent::PollAnswer {
                poll_id: "p1".to_string(),
                option_ids: vec![0, 1],
            },
        ]
    }

    #[test]
    fn test_inbound_content_roundtrip_all_variants() {
        for content in all_channel_content_variants() {
            let cv = serde_json::to_value(&content).unwrap();
            let msg = serde_json::json!({
                "method": "message",
                "params": { "user_id": "u", "user_name": "n", "content": cv }
            });
            let ev: SidecarEvent = serde_json::from_value(msg).unwrap();
            match ev {
                SidecarEvent::Message { params } => {
                    let got = params
                        .content
                        .expect("content must survive the wire roundtrip");
                    assert_eq!(
                        serde_json::to_value(&got).unwrap(),
                        cv,
                        "content variant mutated across roundtrip: {cv:?}"
                    );
                }
                other => panic!("expected Message, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_inbound_structured_fields_parse() {
        let msg = serde_json::json!({
            "method": "message",
            "params": {
                "user_id": "u", "user_name": "n", "text": "hi",
                "is_group": true, "thread_id": "t1", "librefang_user": "lf",
                "username": "@handle",
                "group_members": [
                    {"user_id": "g1", "display_name": "G One", "username": "@g1"}
                ],
                "group_participants": [{"jid": "j@x", "display_name": "J"}],
                "metadata": {"k": "v"}
            }
        });
        let ev: SidecarEvent = serde_json::from_value(msg).unwrap();
        let SidecarEvent::Message { params } = ev else {
            panic!("expected Message");
        };
        assert!(params.is_group);
        assert_eq!(params.thread_id.as_deref(), Some("t1"));
        assert_eq!(params.librefang_user.as_deref(), Some("lf"));
        assert_eq!(params.username.as_deref(), Some("@handle"));
        assert_eq!(params.group_members.len(), 1);
        assert_eq!(params.group_members[0].user_id, "g1");
        assert_eq!(params.group_members[0].username.as_deref(), Some("@g1"));
        assert_eq!(params.group_participants.len(), 1);
        assert_eq!(params.group_participants[0].jid, "j@x");
        assert_eq!(
            params.metadata.get("k"),
            Some(&serde_json::Value::String("v".to_string()))
        );
        assert!(params.content.is_none());
    }

    #[test]
    fn test_legacy_text_message_falls_back_to_text() {
        // A pre-existing text-only adapter sends no `content`; the
        // reader must fall back to ChannelContent::Text(text).
        let json =
            r#"{"method":"message","params":{"user_id":"u","user_name":"n","text":"hello"}}"#;
        let ev: SidecarEvent = serde_json::from_str(json).unwrap();
        let SidecarEvent::Message { params } = ev else {
            panic!("expected Message");
        };
        let params = *params;
        assert!(params.content.is_none());
        assert!(params.group_members.is_empty());
        assert!(!params.is_group);
        let resolved = params
            .content
            .unwrap_or_else(|| ChannelContent::Text(params.text.unwrap_or_default()));
        match resolved {
            ChannelContent::Text(t) => assert_eq!(t, "hello"),
            other => panic!("expected Text fallback, got {other:?}"),
        }
    }

    #[test]
    fn test_outbound_send_params_serialization() {
        let user = ChannelUser {
            platform_id: "chan-1".to_string(),
            display_name: "Dee".to_string(),
            librefang_user: None,
        };
        let p = SidecarSendParams {
            channel_id: user.platform_id.clone(),
            text: "flat".to_string(),
            content: Some(ChannelContent::Image {
                url: "https://x/i.png".to_string(),
                caption: None,
                mime_type: None,
            }),
            thread_id: None,
            user: user.clone(),
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["channel_id"], "chan-1");
        assert_eq!(v["text"], "flat");
        assert_eq!(v["content"]["Image"]["url"], "https://x/i.png");
        assert_eq!(v["user"]["platform_id"], "chan-1");
        // thread_id is skipped when None.
        assert!(v.get("thread_id").is_none());

        let p2 = SidecarSendParams {
            thread_id: Some("th-9".to_string()),
            ..p
        };
        let v2 = serde_json::to_value(&p2).unwrap();
        assert_eq!(v2["thread_id"], "th-9");
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
            args: vec!["-u".to_string(), adapter_path.to_string_lossy().to_string()],
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

        // Read the echo reply. Windows-2025 GitHub runners under load have been
        // observed to spend > 10s in Python cold-start (panicked at 11.346s in
        // CI for c176b2a — see #4676). 30s gives ample headroom while still
        // catching real hangs via nextest's overall test timeout.
        let msg = tokio::time::timeout(std::time::Duration::from_secs(30), stream.next())
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
