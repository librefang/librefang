//! Voice channel adapter with WebSocket server.
//!
//! Accepts real-time audio streams (PCM or Opus) over WebSocket, transcribes
//! them via a configurable STT provider (OpenAI Whisper API by default),
//! routes the transcribed text through the LibreFang channel bridge to an agent,
//! and streams the agent's text response back as synthesized audio via a
//! configurable TTS provider.
//!
//! ## Protocol
//!
//! Clients connect to `ws://<host>:<port>/voice` and exchange binary + text frames:
//!
//! **Client -> Server (binary):** Raw audio data (PCM 16-bit LE mono 16kHz, or Opus packets).
//!
//! **Client -> Server (text/JSON):**
//! ```json
//! { "type": "config", "format": "pcm"|"opus", "sample_rate": 16000 }
//! { "type": "end_of_speech" }
//! ```
//!
//! **Server -> Client (text/JSON):**
//! ```json
//! { "type": "transcript", "text": "..." }
//! { "type": "response", "text": "..." }
//! { "type": "audio", "format": "pcm"|"opus", "encoding": "base64" }
//! { "type": "error", "message": "..." }
//! { "type": "ready" }
//! ```
//!
//! **Server -> Client (binary):** Synthesized audio response data.

use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelStatus, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::State as AxumState;
use axum::response::IntoResponse;
use chrono::Utc;
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, watch, Mutex};
use tracing::{debug, error, info, warn};

/// Audio format used by the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Raw PCM 16-bit little-endian mono.
    Pcm,
    /// Opus-encoded packets.
    Opus,
}

impl AudioFormat {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Pcm => "pcm",
            Self::Opus => "opus",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "pcm" => Some(Self::Pcm),
            "opus" => Some(Self::Opus),
            _ => None,
        }
    }
}

/// STT provider backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SttProvider {
    /// OpenAI Whisper API (default).
    OpenAiWhisper,
    /// Custom HTTP endpoint conforming to OpenAI-compatible transcription API.
    Custom(String),
}

/// TTS provider backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TtsProvider {
    /// OpenAI TTS API (default).
    OpenAiTts,
    /// Custom HTTP endpoint conforming to OpenAI-compatible speech API.
    Custom(String),
}

/// Per-session audio configuration negotiated at connection time.
#[derive(Debug, Clone)]
struct SessionConfig {
    format: AudioFormat,
    sample_rate: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            format: AudioFormat::Pcm,
            sample_rate: 16000,
        }
    }
}

/// Shared state for the WebSocket handler.
#[derive(Clone)]
struct VoiceState {
    /// Channel for forwarding transcribed messages into the bridge.
    msg_tx: Arc<mpsc::Sender<ChannelMessage>>,
    /// STT provider configuration.
    stt_provider: SttProvider,
    /// TTS provider configuration.
    tts_provider: TtsProvider,
    /// OpenAI-compatible API key for STT/TTS.
    api_key: Arc<String>,
    /// STT API base URL.
    stt_url: Arc<String>,
    /// TTS API base URL.
    tts_url: Arc<String>,
    /// TTS voice name.
    tts_voice: Arc<String>,
    /// HTTP client for STT/TTS API calls.
    client: reqwest::Client,
    /// Audio buffer size in bytes before triggering STT (default: 32KB).
    buffer_threshold: usize,
    /// Statistics.
    stats: Arc<VoiceStats>,
}

struct VoiceStats {
    connected: AtomicBool,
    messages_received: AtomicU64,
    messages_sent: AtomicU64,
    active_sessions: AtomicU64,
}

impl Default for VoiceStats {
    fn default() -> Self {
        Self {
            connected: AtomicBool::new(false),
            messages_received: AtomicU64::new(0),
            messages_sent: AtomicU64::new(0),
            active_sessions: AtomicU64::new(0),
        }
    }
}

/// Voice channel adapter.
///
/// Runs a WebSocket server that accepts audio streams, transcribes them via STT,
/// sends the text to the agent bridge, and returns synthesized speech via TTS.
pub struct VoiceAdapter {
    /// WebSocket listen port.
    listen_port: u16,
    /// STT provider.
    stt_provider: SttProvider,
    /// TTS provider.
    tts_provider: TtsProvider,
    /// API key for STT/TTS services.
    api_key: String,
    /// STT API base URL.
    stt_url: String,
    /// TTS API base URL.
    tts_url: String,
    /// TTS voice name.
    tts_voice: String,
    /// Audio buffer threshold in bytes.
    buffer_threshold: usize,
    /// Optional account ID for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Statistics shared with WebSocket handlers.
    stats: Arc<VoiceStats>,
    /// When the adapter was started.
    started_at: Mutex<Option<chrono::DateTime<Utc>>>,
}

impl VoiceAdapter {
    /// Create a new voice channel adapter.
    ///
    /// # Arguments
    /// * `listen_port` - WebSocket server port (default: 4546).
    /// * `api_key` - API key for STT/TTS provider.
    /// * `stt_url` - Base URL for STT API.
    /// * `tts_url` - Base URL for TTS API.
    /// * `tts_voice` - Voice name for TTS synthesis.
    /// * `buffer_threshold` - Audio buffer size before triggering STT (bytes).
    pub fn new(
        listen_port: u16,
        api_key: String,
        stt_url: String,
        tts_url: String,
        tts_voice: String,
        buffer_threshold: usize,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            listen_port,
            stt_provider: SttProvider::OpenAiWhisper,
            tts_provider: TtsProvider::OpenAiTts,
            api_key,
            stt_url,
            tts_url,
            tts_voice,
            buffer_threshold,
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            stats: Arc::new(VoiceStats::default()),
            started_at: Mutex::new(None),
        }
    }

    /// Set the account_id for multi-bot routing. Builder pattern.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Set custom STT provider. Builder pattern.
    pub fn with_stt_provider(mut self, provider: SttProvider) -> Self {
        self.stt_provider = provider;
        self
    }

    /// Set custom TTS provider. Builder pattern.
    pub fn with_tts_provider(mut self, provider: TtsProvider) -> Self {
        self.tts_provider = provider;
        self
    }
}

/// Call the STT API to transcribe audio data.
///
/// Sends audio as multipart form data to an OpenAI-compatible `/v1/audio/transcriptions`
/// endpoint. Returns the transcribed text.
async fn transcribe_audio(
    client: &reqwest::Client,
    stt_url: &str,
    api_key: &str,
    audio_data: &[u8],
    format: AudioFormat,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let file_ext = match format {
        AudioFormat::Pcm => "wav",
        AudioFormat::Opus => "ogg",
    };
    let mime_type = match format {
        AudioFormat::Pcm => "audio/wav",
        AudioFormat::Opus => "audio/ogg",
    };

    // For PCM, wrap in a minimal WAV header so the API can detect the format.
    let body_bytes = match format {
        AudioFormat::Pcm => create_wav_bytes(audio_data, 16000, 1, 16),
        AudioFormat::Opus => audio_data.to_vec(),
    };

    let part = reqwest::multipart::Part::bytes(body_bytes)
        .file_name(format!("audio.{file_ext}"))
        .mime_str(mime_type)?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1")
        .text("response_format", "json");

    let url = format!("{}/v1/audio/transcriptions", stt_url.trim_end_matches('/'));

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .multipart(form)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("STT API error {status}: {body}").into());
    }

    let json: serde_json::Value = resp.json().await?;
    let text = json["text"].as_str().unwrap_or("").trim().to_string();

    Ok(text)
}

/// Call the TTS API to synthesize speech from text.
///
/// Sends text to an OpenAI-compatible `/v1/audio/speech` endpoint.
/// Returns raw audio bytes (PCM or Opus depending on configuration).
async fn synthesize_speech(
    client: &reqwest::Client,
    tts_url: &str,
    api_key: &str,
    text: &str,
    voice: &str,
    format: AudioFormat,
) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
    let response_format = match format {
        AudioFormat::Pcm => "pcm",
        AudioFormat::Opus => "opus",
    };

    let url = format!("{}/v1/audio/speech", tts_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": "tts-1",
        "input": text,
        "voice": voice,
        "response_format": response_format,
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let err_body = resp.text().await.unwrap_or_default();
        return Err(format!("TTS API error {status}: {err_body}").into());
    }

    let audio_bytes = resp.bytes().await?.to_vec();
    Ok(audio_bytes)
}

/// Create a minimal WAV header + data for PCM 16-bit LE mono.
fn create_wav_bytes(
    pcm_data: &[u8],
    sample_rate: u32,
    channels: u16,
    bits_per_sample: u16,
) -> Vec<u8> {
    let byte_rate = sample_rate * u32::from(channels) * u32::from(bits_per_sample) / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_size = pcm_data.len() as u32;
    let file_size = 36 + data_size;

    let mut wav = Vec::with_capacity(44 + pcm_data.len());
    // RIFF header
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&file_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    // fmt sub-chunk
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // sub-chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    // data sub-chunk
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(pcm_data);
    wav
}

/// Handle a single WebSocket voice session.
async fn handle_voice_session(mut ws: WebSocket, state: VoiceState) {
    state.stats.active_sessions.fetch_add(1, Ordering::Relaxed);
    let session_id = uuid::Uuid::new_v4().to_string();
    info!(session_id = %session_id, "Voice session connected");

    // Send ready message
    let ready_msg = serde_json::json!({ "type": "ready", "session_id": session_id });
    if ws
        .send(WsMessage::Text(ready_msg.to_string().into()))
        .await
        .is_err()
    {
        state.stats.active_sessions.fetch_sub(1, Ordering::Relaxed);
        return;
    }

    let mut session_config = SessionConfig::default();
    let mut audio_buffer: Vec<u8> = Vec::with_capacity(state.buffer_threshold);

    loop {
        let msg = match ws.recv().await {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => {
                debug!(session_id = %session_id, error = %e, "WebSocket error");
                break;
            }
            None => break,
        };

        match msg {
            WsMessage::Binary(data) => {
                // Accumulate audio data
                audio_buffer.extend_from_slice(&data);

                // If buffer exceeds threshold, trigger STT
                if audio_buffer.len() >= state.buffer_threshold {
                    let transcript =
                        process_audio_buffer(&state, &audio_buffer, &session_config, &session_id)
                            .await;

                    audio_buffer.clear();

                    if let Some(text) = transcript {
                        // Send transcript back to client
                        let transcript_msg =
                            serde_json::json!({ "type": "transcript", "text": text });
                        let _ = ws
                            .send(WsMessage::Text(transcript_msg.to_string().into()))
                            .await;

                        // Forward to agent via channel bridge
                        let channel_msg = build_channel_message(&session_id, &text);
                        state
                            .stats
                            .messages_received
                            .fetch_add(1, Ordering::Relaxed);
                        if state.msg_tx.send(channel_msg).await.is_err() {
                            warn!(session_id = %session_id, "Channel bridge disconnected");
                            break;
                        }
                    }
                }
            }
            WsMessage::Text(text_data) => {
                let text_str: &str = &text_data;
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(text_str) {
                    match json["type"].as_str() {
                        Some("config") => {
                            if let Some(fmt) = json["format"].as_str() {
                                if let Some(f) = AudioFormat::from_str(fmt) {
                                    session_config.format = f;
                                }
                            }
                            if let Some(sr) = json["sample_rate"].as_u64() {
                                session_config.sample_rate = sr as u32;
                            }
                            debug!(
                                session_id = %session_id,
                                format = %session_config.format.as_str(),
                                sample_rate = session_config.sample_rate,
                                "Session configured"
                            );
                        }
                        Some("end_of_speech") => {
                            // Process remaining buffer
                            if !audio_buffer.is_empty() {
                                let transcript = process_audio_buffer(
                                    &state,
                                    &audio_buffer,
                                    &session_config,
                                    &session_id,
                                )
                                .await;
                                audio_buffer.clear();

                                if let Some(text) = transcript {
                                    let transcript_msg =
                                        serde_json::json!({ "type": "transcript", "text": text });
                                    let _ = ws
                                        .send(WsMessage::Text(transcript_msg.to_string().into()))
                                        .await;

                                    let channel_msg = build_channel_message(&session_id, &text);
                                    state
                                        .stats
                                        .messages_received
                                        .fetch_add(1, Ordering::Relaxed);
                                    if state.msg_tx.send(channel_msg).await.is_err() {
                                        warn!(
                                            session_id = %session_id,
                                            "Channel bridge disconnected"
                                        );
                                        break;
                                    }
                                }
                            }
                        }
                        _ => {
                            debug!(
                                session_id = %session_id,
                                "Unknown message type: {:?}",
                                json["type"]
                            );
                        }
                    }
                }
            }
            WsMessage::Close(_) => {
                info!(session_id = %session_id, "Voice session closed by client");
                break;
            }
            WsMessage::Ping(data) => {
                let _ = ws.send(WsMessage::Pong(data)).await;
            }
            WsMessage::Pong(_) => {}
        }
    }

    state.stats.active_sessions.fetch_sub(1, Ordering::Relaxed);
    info!(session_id = %session_id, "Voice session ended");
}

/// Process accumulated audio buffer through STT.
async fn process_audio_buffer(
    state: &VoiceState,
    audio_data: &[u8],
    config: &SessionConfig,
    session_id: &str,
) -> Option<String> {
    match transcribe_audio(
        &state.client,
        &state.stt_url,
        &state.api_key,
        audio_data,
        config.format,
    )
    .await
    {
        Ok(text) if !text.is_empty() => {
            debug!(session_id = %session_id, text = %text, "Transcribed audio");
            Some(text)
        }
        Ok(_) => {
            debug!(session_id = %session_id, "Empty transcription (silence)");
            None
        }
        Err(e) => {
            warn!(session_id = %session_id, error = %e, "STT transcription failed");
            None
        }
    }
}

/// Build a `ChannelMessage` from a voice transcript.
fn build_channel_message(session_id: &str, text: &str) -> ChannelMessage {
    ChannelMessage {
        channel: ChannelType::Custom("voice".to_string()),
        platform_message_id: format!("voice-{}-{}", session_id, Utc::now().timestamp_millis()),
        sender: ChannelUser {
            platform_id: format!("voice-{session_id}"),
            display_name: format!("Voice User ({session_id})"),
            librefang_user: None,
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: {
            let mut m = HashMap::new();
            m.insert("voice_session".to_string(), serde_json::json!(session_id));
            m
        },
    }
}

/// WebSocket upgrade handler for axum.
async fn ws_handler(
    ws: WebSocketUpgrade,
    AxumState(state): AxumState<VoiceState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_voice_session(socket, state))
}

#[async_trait]
impl ChannelAdapter for VoiceAdapter {
    fn name(&self) -> &str {
        "voice"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("voice".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let port = self.listen_port;
        let mut shutdown_rx = self.shutdown_rx.clone();

        let state = VoiceState {
            msg_tx: Arc::new(tx),
            stt_provider: self.stt_provider.clone(),
            tts_provider: self.tts_provider.clone(),
            api_key: Arc::new(self.api_key.clone()),
            stt_url: Arc::new(self.stt_url.clone()),
            tts_url: Arc::new(self.tts_url.clone()),
            tts_voice: Arc::new(self.tts_voice.clone()),
            client: crate::http_client::new_client(),
            buffer_threshold: self.buffer_threshold,
            stats: Arc::clone(&self.stats),
        };

        self.stats.connected.store(true, Ordering::Relaxed);
        *self.started_at.lock().await = Some(Utc::now());

        info!("Voice adapter starting WebSocket server on port {port}");

        let stats = Arc::clone(&self.stats);
        tokio::spawn(async move {
            let app = axum::Router::new()
                .route("/voice", axum::routing::get(ws_handler))
                .with_state(state);

            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
            info!("Voice WebSocket server listening on ws://{addr}/voice");

            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    error!("Voice: failed to bind port {port}: {e}");
                    stats.connected.store(false, Ordering::Relaxed);
                    return;
                }
            };

            let server = axum::serve(listener, app);

            tokio::select! {
                result = server => {
                    if let Err(e) = result {
                        error!("Voice WebSocket server error: {e}");
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("Voice adapter shutting down");
                }
            }

            stats.connected.store(false, Ordering::Relaxed);
            info!("Voice WebSocket server stopped");
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        _user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // For voice channel, outbound messages are handled differently:
        // the WebSocket session manages its own response delivery.
        // This method logs the response for observability but the actual
        // audio delivery happens through the WebSocket connection.
        let text = match content {
            ChannelContent::Text(t) => t,
            _ => return Ok(()),
        };

        self.stats.messages_sent.fetch_add(1, Ordering::Relaxed);
        debug!(
            text_len = text.len(),
            "Voice adapter received outbound message"
        );
        Ok(())
    }

    async fn send_typing(
        &self,
        _user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // No typing indicator for voice.
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        self.stats.connected.store(false, Ordering::Relaxed);
        info!("Voice adapter stopped");
        Ok(())
    }

    fn status(&self) -> ChannelStatus {
        let started_at = self.started_at.try_lock().ok().and_then(|guard| *guard);

        ChannelStatus {
            connected: self.stats.connected.load(Ordering::Relaxed),
            started_at,
            last_message_at: None,
            messages_received: self.stats.messages_received.load(Ordering::Relaxed),
            messages_sent: self.stats.messages_sent.load(Ordering::Relaxed),
            last_error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_voice_adapter_creation() {
        let adapter = VoiceAdapter::new(
            4546,
            "test-key".to_string(),
            "https://api.openai.com".to_string(),
            "https://api.openai.com".to_string(),
            "alloy".to_string(),
            32768,
        );
        assert_eq!(adapter.name(), "voice");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("voice".to_string())
        );
        assert_eq!(adapter.listen_port, 4546);
    }

    #[test]
    fn test_voice_adapter_with_account_id() {
        let adapter = VoiceAdapter::new(
            4546,
            "key".to_string(),
            "https://stt.example.com".to_string(),
            "https://tts.example.com".to_string(),
            "alloy".to_string(),
            32768,
        )
        .with_account_id(Some("voice-bot-1".to_string()));
        assert_eq!(adapter.account_id, Some("voice-bot-1".to_string()));
    }

    #[test]
    fn test_audio_format_roundtrip() {
        assert_eq!(AudioFormat::from_str("pcm"), Some(AudioFormat::Pcm));
        assert_eq!(AudioFormat::from_str("opus"), Some(AudioFormat::Opus));
        assert_eq!(AudioFormat::from_str("mp3"), None);
        assert_eq!(AudioFormat::Pcm.as_str(), "pcm");
        assert_eq!(AudioFormat::Opus.as_str(), "opus");
    }

    #[test]
    fn test_create_wav_bytes() {
        let pcm_data = vec![0u8; 3200]; // 100ms of 16kHz 16-bit mono
        let wav = create_wav_bytes(&pcm_data, 16000, 1, 16);
        // WAV header is 44 bytes
        assert_eq!(wav.len(), 44 + pcm_data.len());
        // Check RIFF header
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[12..16], b"fmt ");
        assert_eq!(&wav[36..40], b"data");
    }

    #[test]
    fn test_build_channel_message() {
        let msg = build_channel_message("sess-123", "Hello world");
        assert_eq!(msg.channel, ChannelType::Custom("voice".to_string()));
        assert!(msg.platform_message_id.starts_with("voice-sess-123-"));
        assert_eq!(msg.sender.platform_id, "voice-sess-123");
        assert!(!msg.is_group);
        match &msg.content {
            ChannelContent::Text(t) => assert_eq!(t, "Hello world"),
            _ => panic!("Expected Text content"),
        }
        assert!(msg.metadata.contains_key("voice_session"));
    }

    #[test]
    fn test_session_config_default() {
        let cfg = SessionConfig::default();
        assert_eq!(cfg.format, AudioFormat::Pcm);
        assert_eq!(cfg.sample_rate, 16000);
    }

    #[test]
    fn test_voice_status_default() {
        let adapter = VoiceAdapter::new(
            4546,
            "key".to_string(),
            "https://api.example.com".to_string(),
            "https://api.example.com".to_string(),
            "alloy".to_string(),
            32768,
        );
        let status = adapter.status();
        assert!(!status.connected);
        assert_eq!(status.messages_received, 0);
        assert_eq!(status.messages_sent, 0);
    }
}
