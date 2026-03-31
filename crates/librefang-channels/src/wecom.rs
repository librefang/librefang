//! WeCom intelligent bot adapter.
//!
//! Supports two connection modes:
//! - **WebSocket** (default): connects to `wss://openws.work.weixin.qq.com`
//!   using Bot ID and Secret. Receives messages via `aibot_msg_callback`
//!   frames and replies via `aibot_send_msg` frames.
//! - **Callback**: starts an HTTP server that receives WeCom webhook
//!   callbacks (JSON + AES encrypted). Replies via `response_url` from the
//!   callback payload, or the bot's webhook send endpoint.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use axum::response::IntoResponse;
use chrono::Utc;
use futures::{SinkExt, Stream, StreamExt};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

// ── Constants ──────────────────────────────────────────────────────

/// WeCom intelligent bot WebSocket endpoint.
const WECOM_WS_URL: &str = "wss://openws.work.weixin.qq.com";

/// Maximum text length per reply.
const MAX_MESSAGE_LEN: usize = 4096;

/// Heartbeat interval for WebSocket mode.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// Initial reconnect backoff for WebSocket mode.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Maximum reconnect backoff for WebSocket mode.
const MAX_BACKOFF: Duration = Duration::from_secs(30);

// ── Shared helpers (WebSocket mode) ────────────────────────────────

/// Parse an incoming WebSocket text frame as JSON.
fn parse_ws_frame(text: &str) -> Option<serde_json::Value> {
    serde_json::from_str(text).ok()
}

/// Get the command/action from a frame (supports both `cmd` and `action` keys).
fn frame_cmd(frame: &serde_json::Value) -> Option<&str> {
    frame
        .get("cmd")
        .or_else(|| frame.get("action"))
        .and_then(|v| v.as_str())
}

/// Get the data/body payload from a frame (supports both `body` and `data` keys).
fn frame_body(frame: &serde_json::Value) -> Option<&serde_json::Value> {
    frame.get("body").or_else(|| frame.get("data"))
}

/// Get `req_id` from a frame — checks `headers.req_id` first, then `data.req_id`.
fn frame_req_id(frame: &serde_json::Value) -> Option<&str> {
    frame
        .get("headers")
        .and_then(|h| h.get("req_id"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            frame_body(frame)
                .and_then(|b| b.get("req_id"))
                .and_then(|v| v.as_str())
        })
}

/// Extract a message callback from a parsed JSON frame.
/// Returns (req_id, from_user_id, content, is_group).
fn extract_msg_callback(frame: &serde_json::Value) -> Option<(String, String, String, bool)> {
    let cmd = frame_cmd(frame)?;
    if cmd != "aibot_msg_callback" {
        return None;
    }
    let body = frame_body(frame)?;
    let req_id = frame_req_id(frame)
        .or_else(|| body.get("req_id").and_then(|v| v.as_str()))?
        .to_string();
    let from_user = body
        .get("from")
        .and_then(|f| f.get("userid").or_else(|| f.get("user_id")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let msgtype = body.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
    let content = match msgtype {
        "text" => body
            .get("text")
            .and_then(|t| t.get("content"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => {
            debug!(msgtype, "Unsupported WeCom bot message type, skipping");
            return None;
        }
    };

    if from_user.is_empty() || content.is_empty() {
        return None;
    }

    let is_group = body
        .get("chattype")
        .or_else(|| body.get("chat_type"))
        .and_then(|v| v.as_str())
        .map(|t| t == "group")
        .unwrap_or(false);

    Some((req_id, from_user, content, is_group))
}

/// Check if a frame is a subscribe success acknowledgement.
///
/// The server response may not include a `cmd` field — it just returns
/// `{"errcode": 0, "errmsg": "ok", "headers": {"req_id": "aibot_subscribe_..."}}`.
/// We detect success by checking if `headers.req_id` starts with `"aibot_subscribe"` and errcode is 0.
fn is_subscribe_success(frame: &serde_json::Value) -> bool {
    let errcode = frame.get("errcode").and_then(|v| v.as_i64());

    // Method 1: explicit cmd field
    if let Some(cmd) = frame_cmd(frame) {
        return cmd == "aibot_subscribe" && (errcode == Some(0) || errcode.is_none());
    }

    // Method 2: infer from headers.req_id prefix (server omits cmd in ack frames)
    if let Some(req_id) = frame_req_id(frame) {
        return req_id.starts_with("aibot_subscribe") && errcode == Some(0);
    }

    false
}

// ── Callback-mode crypto helpers ───────────────────────────────────

/// AES-CBC decrypt with PKCS#7 padding (32-byte block alignment).
fn decrypt_aes_cbc(key: &[u8], encrypted_base64: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    use cbc::cipher::{BlockDecryptMut, KeyIvInit};

    let mut encrypted = base64::engine::general_purpose::STANDARD
        .decode(encrypted_base64)
        .map_err(|e| format!("base64 decode error: {e}"))?;

    if key.len() != 32 {
        return Err(format!(
            "invalid WeCom AES key length: expected 32 bytes, got {}",
            key.len()
        ));
    }

    type Aes256CbcDecrypt = cbc::Decryptor<aes::Aes256>;
    let iv = &key[..16];
    let cipher = Aes256CbcDecrypt::new(key.into(), iv.into());

    let decrypted = cipher
        .decrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut encrypted)
        .map_err(|e| format!("decrypt error: {e}"))?;

    let decrypted = decrypted.to_vec();
    let pad = decrypted
        .last()
        .copied()
        .ok_or_else(|| "decrypted payload is empty".to_string())? as usize;

    if pad == 0 || pad > 32 || decrypted.len() < pad {
        return Err(format!("invalid WeCom PKCS7 padding length: {pad}"));
    }
    if !decrypted[decrypted.len() - pad..]
        .iter()
        .all(|byte| *byte as usize == pad)
    {
        return Err("invalid WeCom PKCS7 padding bytes".to_string());
    }

    Ok(decrypted[..decrypted.len() - pad].to_vec())
}

/// Verify a WeCom callback signature: SHA1(sort(token, timestamp, nonce, encrypt)).
fn is_valid_wecom_signature(
    token: &str,
    timestamp: &str,
    nonce: &str,
    encrypt: &str,
    msg_signature: &str,
) -> bool {
    let mut parts = [token, timestamp, nonce, encrypt];
    parts.sort_unstable();

    let mut hasher = Sha1::new();
    hasher.update(parts.concat().as_bytes());
    hex::encode(hasher.finalize()) == msg_signature
}

/// Decode the AES-encrypted EncodingAESKey and decrypt a payload.
/// Returns the message body string (without the 16-byte random prefix and receiveid suffix).
fn decode_wecom_payload(encoding_aes_key: &str, encrypted_payload: &str) -> Result<String, String> {
    use base64::{
        alphabet,
        engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
        Engine,
    };

    let aes_key_engine = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::RequireNone)
            .with_decode_allow_trailing_bits(true),
    );

    let aes_key = aes_key_engine
        .decode(encoding_aes_key)
        .map_err(|e| format!("aes key decode error: {e}"))?;
    let decrypted = decrypt_aes_cbc(&aes_key, encrypted_payload)?;

    // Structure: 16-byte random + 4-byte msg_len (big-endian) + msg + receiveid
    if decrypted.len() < 20 {
        return Err("decrypted payload too short".to_string());
    }

    let msg_len =
        u32::from_be_bytes([decrypted[16], decrypted[17], decrypted[18], decrypted[19]]) as usize;
    if decrypted.len() < 20 + msg_len {
        return Err("decrypted payload shorter than declared length".to_string());
    }

    String::from_utf8(decrypted[20..20 + msg_len].to_vec())
        .map_err(|e| format!("payload is not valid utf-8: {e}"))
}

/// AES-CBC encrypt with PKCS#7 padding for building callback responses.
#[allow(dead_code)] // used by build_encrypted_response (passive reply, not yet wired)
fn encrypt_aes_cbc(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};

    if key.len() != 32 {
        return Err(format!(
            "invalid WeCom AES key length: expected 32 bytes, got {}",
            key.len()
        ));
    }

    // PKCS#7 padding to 32-byte boundary
    let pad_len = 32 - (plaintext.len() % 32);
    let mut padded = plaintext.to_vec();
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));

    type Aes256CbcEncrypt = cbc::Encryptor<aes::Aes256>;
    let iv = &key[..16];
    let cipher = Aes256CbcEncrypt::new(key.into(), iv.into());
    let len = padded.len();
    let encrypted = cipher
        .encrypt_padded_mut::<aes::cipher::block_padding::NoPadding>(&mut padded, len)
        .map_err(|e| format!("AES-CBC encryption failed: {e}"))?;

    Ok(encrypted.to_vec())
}

/// Build an encrypted response JSON for passive reply in callback mode.
/// Format: `{"encrypt": "...", "msgsignature": "...", "timestamp": "...", "nonce": "..."}`
#[allow(dead_code)] // passive reply not yet wired in callback mode
fn build_encrypted_response(
    encoding_aes_key: &str,
    token: &str,
    reply_json: &str,
) -> Result<String, String> {
    use base64::{
        alphabet,
        engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
        Engine,
    };

    let aes_key_engine = GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new()
            .with_decode_padding_mode(DecodePaddingMode::RequireNone)
            .with_decode_allow_trailing_bits(true),
    );

    let aes_key = aes_key_engine
        .decode(encoding_aes_key)
        .map_err(|e| format!("aes key decode error: {e}"))?;

    // Build plaintext: 16-byte random + 4-byte len + msg + receiveid("")
    let mut plaintext = Vec::new();
    let random_bytes: [u8; 16] = rand_bytes();
    plaintext.extend_from_slice(&random_bytes);
    let msg_bytes = reply_json.as_bytes();
    plaintext.extend_from_slice(&(msg_bytes.len() as u32).to_be_bytes());
    plaintext.extend_from_slice(msg_bytes);
    // receiveid is empty string for intelligent bot

    let encrypted = encrypt_aes_cbc(&aes_key, &plaintext)?;
    let encrypt_b64 = base64::engine::general_purpose::STANDARD.encode(&encrypted);

    let timestamp = Utc::now().timestamp().to_string();
    let nonce = format!("{}", rand_u64());

    // Compute signature
    let mut parts = [token, &timestamp, &nonce, &encrypt_b64];
    parts.sort_unstable();
    let mut hasher = Sha1::new();
    hasher.update(parts.concat().as_bytes());
    let signature = hex::encode(hasher.finalize());

    Ok(serde_json::json!({
        "encrypt": encrypt_b64,
        "msgsignature": signature,
        "timestamp": timestamp,
        "nonce": nonce,
    })
    .to_string())
}

/// Generate 16 random bytes (non-cryptographic, sufficient for nonce).
#[allow(dead_code)]
fn rand_bytes() -> [u8; 16] {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u64(Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64);
    let a = h.finish();
    let s2 = RandomState::new();
    let mut h2 = s2.build_hasher();
    h2.write_u64(a.wrapping_add(42));
    let b = h2.finish();
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_le_bytes());
    out[8..].copy_from_slice(&b.to_le_bytes());
    out
}

/// Generate a random u64 for nonce strings.
#[allow(dead_code)]
fn rand_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut h = s.build_hasher();
    h.write_u64(Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64);
    h.finish()
}

// ── WeComAdapter ───────────────────────────────────────────────────

/// Maps user_id → latest req_id so we can use `aibot_respond_msg` for replies.
type ReqIdMap = Arc<RwLock<HashMap<String, String>>>;

/// Operational mode determined at construction time.
enum Mode {
    /// WebSocket long-connection.
    Websocket {
        ws_tx: Arc<RwLock<Option<mpsc::UnboundedSender<String>>>>,
        /// Tracks the most recent req_id per user for passive replies.
        pending_req_ids: ReqIdMap,
    },
    /// HTTP callback — intelligent bot JSON protocol.
    Callback {
        /// HTTP client for response_url replies.
        client: reqwest::Client,
        /// Token for callback signature verification.
        token: Option<String>,
        /// Encoding AES key for message encryption/decryption.
        encoding_aes_key: Option<String>,
        /// Bot webhook key for proactive messages (extracted from first response_url).
        webhook_key: Arc<RwLock<Option<String>>>,
    },
}

/// WeCom intelligent bot adapter.
pub struct WeComAdapter {
    bot_id: String,
    secret: Zeroizing<String>,
    account_id: Option<String>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    mode: Mode,
}

impl WeComAdapter {
    /// Create a new adapter in WebSocket mode.
    pub fn new(bot_id: String, secret: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            bot_id,
            secret: Zeroizing::new(secret),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            mode: Mode::Websocket {
                ws_tx: Arc::new(RwLock::new(None)),
                pending_req_ids: Arc::new(RwLock::new(HashMap::new())),
            },
        }
    }

    /// Create a new adapter in callback (HTTP webhook) mode.
    pub fn new_callback(
        bot_id: String,
        secret: String,
        _webhook_port: u16,
        token: Option<String>,
        encoding_aes_key: Option<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            bot_id,
            secret: Zeroizing::new(secret),
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            mode: Mode::Callback {
                client: crate::http_client::new_client(),
                token,
                encoding_aes_key,
                webhook_key: Arc::new(RwLock::new(None)),
            },
        }
    }

    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Build a `aibot_respond_msg` reply frame (WebSocket mode).
    ///
    /// WeCom intelligent bot's `aibot_respond_msg` does NOT support `msgtype: "text"`.
    /// Valid types are: stream, markdown, template_card, stream_with_template_card,
    /// file, image, voice, video. For plain text replies we use `markdown`.
    fn build_reply_frame(req_id: &str, text: &str) -> String {
        serde_json::json!({
            "cmd": "aibot_respond_msg",
            "headers": {
                "req_id": req_id,
            },
            "body": {
                "msgtype": "markdown",
                "markdown": {
                    "content": text,
                }
            }
        })
        .to_string()
    }

    /// Build a `aibot_send_msg` proactive message frame (WebSocket mode).
    fn build_send_frame(user_id: &str, text: &str) -> String {
        serde_json::json!({
            "cmd": "aibot_send_msg",
            "headers": {
                "req_id": format!("aibot_send_msg_{}", Utc::now().timestamp_millis()),
            },
            "body": {
                "receiver": {
                    "userid": user_id,
                },
                "msgtype": "markdown",
                "markdown": {
                    "content": text,
                }
            }
        })
        .to_string()
    }

    // ── WebSocket mode start ───────────────────────────────────────

    fn start_websocket(
        bot_id: String,
        secret: Zeroizing<String>,
        account_id: Arc<Option<String>>,
        mut shutdown_rx: watch::Receiver<bool>,
        ws_tx_shared: Arc<RwLock<Option<mpsc::UnboundedSender<String>>>>,
        pending_req_ids: ReqIdMap,
    ) -> Pin<Box<dyn Stream<Item = ChannelMessage> + Send>> {
        let (msg_tx, msg_rx) = mpsc::channel::<ChannelMessage>(256);

        info!(bot_id = %bot_id, "Starting WeCom bot adapter (WebSocket)");

        tokio::spawn(async move {
            let mut backoff = INITIAL_BACKOFF;

            'outer: loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                let ws_result = tokio_tungstenite::connect_async(WECOM_WS_URL).await;

                let ws_stream = match ws_result {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        warn!(
                            "WeCom bot WebSocket connection failed: {e}, retrying in {backoff:?}"
                        );
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                        continue;
                    }
                };

                backoff = INITIAL_BACKOFF;
                info!("WeCom bot WebSocket connected");

                let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();

                let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<String>();
                {
                    let mut guard = ws_tx_shared.write().await;
                    *guard = Some(frame_tx);
                }

                let subscribe_frame = serde_json::json!({
                    "cmd": "aibot_subscribe",
                    "headers": {
                        "req_id": format!("aibot_subscribe_{}", Utc::now().timestamp_millis()),
                    },
                    "body": {
                        "bot_id": bot_id,
                        "secret": secret.as_str(),
                    }
                })
                .to_string();

                if let Err(e) = ws_sink.send(WsMessage::Text(subscribe_frame.into())).await {
                    warn!("Failed to send subscribe frame: {e}");
                    continue 'outer;
                }

                let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
                heartbeat.tick().await;

                let should_reconnect = 'inner: loop {
                    tokio::select! {
                        _ = shutdown_rx.changed() => {
                            info!("WeCom bot adapter shutting down");
                            break 'inner false;
                        }
                        _ = heartbeat.tick() => {
                            let ping = serde_json::json!({
                                "cmd": "ping",
                                "headers": {
                                    "req_id": format!("ping_{}", Utc::now().timestamp_millis()),
                                }
                            }).to_string();
                            if let Err(e) = ws_sink.send(WsMessage::Text(ping.into())).await {
                                warn!("WeCom bot heartbeat failed: {e}");
                                break 'inner true;
                            }
                        }
                        Some(frame_text) = frame_rx.recv() => {
                            info!(frame_len = frame_text.len(), "WeCom bot: sending frame over WebSocket");
                            debug!(frame = %frame_text, "WeCom bot: outgoing WS frame content");
                            if let Err(e) = ws_sink.send(WsMessage::Text(frame_text.into())).await {
                                error!("WeCom bot WS sink send failed: {e}");
                                break 'inner true;
                            }
                            info!("WeCom bot: frame sent over WebSocket successfully");
                        }
                        ws_msg = ws_stream_rx.next() => {
                            match ws_msg {
                                Some(Ok(WsMessage::Text(text))) => {
                                    let text_str: &str = &text;
                                    debug!(raw_frame_len = text_str.len(), "WeCom bot received WS frame");
                                    let Some(frame) = parse_ws_frame(text_str) else {
                                        warn!(raw = %text_str, "WeCom bot: unparseable frame");
                                        continue 'inner;
                                    };

                                    let cmd = frame_cmd(&frame).unwrap_or("unknown");
                                    debug!(cmd = cmd, "WeCom bot parsed frame cmd");

                                    if is_subscribe_success(&frame) {
                                        info!("WeCom bot subscribed successfully");
                                        continue 'inner;
                                    }

                                    // Subscribe failure
                                    if cmd == "aibot_subscribe" {
                                        let errcode = frame.get("errcode").and_then(|v| v.as_i64());
                                        let errmsg = frame.get("errmsg").and_then(|v| v.as_str()).unwrap_or("");
                                        error!(
                                            errcode = ?errcode,
                                            errmsg = errmsg,
                                            "WeCom bot subscribe FAILED"
                                        );
                                        continue 'inner;
                                    }

                                    if cmd == "aibot_event_callback" {
                                        debug!(event = ?frame_body(&frame), "WeCom bot event");
                                        continue 'inner;
                                    }

                                    if cmd == "pong" {
                                        continue 'inner;
                                    }

                                    // Server ack frames (no cmd, just errcode + headers.req_id)
                                    // e.g. ping ack, send_msg ack, respond_msg ack
                                    if cmd == "unknown" {
                                        if let Some(req_id) = frame_req_id(&frame) {
                                            let errcode = frame.get("errcode").and_then(|v| v.as_i64());
                                            if errcode == Some(0) {
                                                debug!(req_id = req_id, "WeCom bot: server ack OK");
                                            } else {
                                                let errmsg = frame.get("errmsg").and_then(|v| v.as_str()).unwrap_or("");
                                                error!(req_id = req_id, errcode = ?errcode, errmsg = errmsg, "WeCom bot: server ack error");
                                            }
                                            continue 'inner;
                                        }
                                    }

                                    // Log response frames from server (e.g. aibot_respond_msg / aibot_send_msg ack)
                                    if cmd == "aibot_respond_msg" || cmd == "aibot_send_msg" {
                                        let errcode = frame.get("errcode").and_then(|v| v.as_i64());
                                        let errmsg = frame.get("errmsg").and_then(|v| v.as_str()).unwrap_or("");
                                        if errcode.unwrap_or(0) != 0 {
                                            error!(
                                                cmd = cmd,
                                                errcode = ?errcode,
                                                errmsg = errmsg,
                                                "WeCom bot send/reply got error response from server"
                                            );
                                        } else {
                                            info!(
                                                cmd = cmd,
                                                errcode = ?errcode,
                                                "WeCom bot send/reply acknowledged by server"
                                            );
                                        }
                                        continue 'inner;
                                    }

                                    if let Some((req_id, from_user, content, is_group)) =
                                        extract_msg_callback(&frame)
                                    {
                                        info!(
                                            req_id = %req_id,
                                            from_user = %from_user,
                                            content_len = content.len(),
                                            is_group = is_group,
                                            "WeCom bot received message via WebSocket"
                                        );
                                        let mut msg = ChannelMessage {
                                            channel: ChannelType::Custom("wecom".to_string()),
                                            platform_message_id: req_id.clone(),
                                            sender: ChannelUser {
                                                platform_id: from_user.clone(),
                                                display_name: from_user.clone(),
                                                librefang_user: None,
                                            },
                                            content: ChannelContent::Text(content),
                                            target_agent: None,
                                            timestamp: Utc::now(),
                                            is_group,
                                            thread_id: None,
                                            metadata: HashMap::new(),
                                        };

                                        msg.metadata.insert(
                                            "wecom_req_id".to_string(),
                                            serde_json::json!(req_id),
                                        );

                                        // Store response_url if present in the message body
                                        if let Some(body) = frame_body(&frame) {
                                            if let Some(url) = body.get("response_url").and_then(|v| v.as_str()) {
                                                if !url.is_empty() {
                                                    msg.metadata.insert(
                                                        "wecom_response_url".to_string(),
                                                        serde_json::json!(url),
                                                    );
                                                }
                                            }
                                        }

                                        // Cache req_id so send() can use aibot_respond_msg
                                        {
                                            let mut map = pending_req_ids.write().await;
                                            map.insert(from_user.clone(), req_id.clone());
                                            info!(
                                                user_id = %from_user,
                                                req_id = %req_id,
                                                pending_count = map.len(),
                                                "WeCom bot cached req_id for user"
                                            );
                                        }

                                        if let Some(ref aid) = *account_id {
                                            msg.metadata.insert(
                                                "account_id".to_string(),
                                                serde_json::json!(aid),
                                            );
                                        }

                                        if msg_tx.send(msg).await.is_err() {
                                            error!("WeCom bot: msg_tx.send failed (receiver dropped)");
                                        } else {
                                            debug!("WeCom bot: message dispatched to bridge");
                                        }
                                    } else {
                                        debug!(frame = %frame, "WeCom bot: frame did not match any handler");
                                    }
                                }
                                Some(Ok(WsMessage::Ping(data))) => {
                                    let _ = ws_sink.send(WsMessage::Pong(data)).await;
                                }
                                Some(Ok(WsMessage::Close(_))) => {
                                    info!("WeCom bot WebSocket closed by server");
                                    break 'inner true;
                                }
                                Some(Err(e)) => {
                                    warn!("WeCom bot WebSocket error: {e}");
                                    break 'inner true;
                                }
                                None => {
                                    info!("WeCom bot WebSocket stream ended");
                                    break 'inner true;
                                }
                                _ => {}
                            }
                        }
                    }
                };

                {
                    let mut guard = ws_tx_shared.write().await;
                    *guard = None;
                }

                if !should_reconnect {
                    break 'outer;
                }

                warn!("WeCom bot reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        });

        Box::pin(tokio_stream::wrappers::ReceiverStream::new(msg_rx))
    }

    // ── Callback mode start ────────────────────────────────────────

    #[allow(clippy::type_complexity, dead_code)]
    fn start_callback(
        account_id: Arc<Option<String>>,
        mut shutdown_rx: watch::Receiver<bool>,
        port: u16,
        token: Option<String>,
        encoding_aes_key: Option<String>,
        webhook_key: Arc<RwLock<Option<String>>>,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        info!("WeCom bot adapter starting callback server on port {port}");

        tokio::spawn(async move {
            let token = Arc::new(token);
            let encoding_aes_key = Arc::new(encoding_aes_key);
            let tx = Arc::new(tx);

            let app = axum::Router::new().route(
                "/wecom/webhook",
                // GET: URL verification
                axum::routing::get({
                    let encoding_aes_key = Arc::clone(&encoding_aes_key);
                    let token = Arc::clone(&token);
                    move |axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>| {
                        let encoding_aes_key = Arc::clone(&encoding_aes_key);
                        let token = Arc::clone(&token);
                        async move {
                            if let (Some(echostr), Some(msg_sig), Some(timestamp), Some(nonce)) = (
                                params.get("echostr"),
                                params.get("msg_signature"),
                                params.get("timestamp"),
                                params.get("nonce"),
                            ) {
                                let Some(token_str) = token.as_deref() else {
                                    return (
                                        axum::http::StatusCode::BAD_REQUEST,
                                        "missing WeCom callback token",
                                    )
                                        .into_response();
                                };

                                if !is_valid_wecom_signature(
                                    token_str, timestamp, nonce, echostr, msg_sig,
                                ) {
                                    return (
                                        axum::http::StatusCode::FORBIDDEN,
                                        "invalid WeCom callback signature",
                                    )
                                        .into_response();
                                }

                                let body = match encoding_aes_key.as_deref() {
                                    Some(aes_key) if !aes_key.is_empty() => {
                                        match decode_wecom_payload(aes_key, echostr) {
                                            Ok(plain) => plain,
                                            Err(err) => {
                                                warn!(error = %err, "Failed to decrypt WeCom echostr");
                                                return (
                                                    axum::http::StatusCode::BAD_REQUEST,
                                                    "invalid WeCom echostr",
                                                )
                                                    .into_response();
                                            }
                                        }
                                    }
                                    _ => echostr.clone(),
                                };

                                return (
                                    axum::http::StatusCode::OK,
                                    [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                                    body,
                                )
                                    .into_response();
                            }
                            (
                                axum::http::StatusCode::BAD_REQUEST,
                                "missing WeCom verification parameters",
                            )
                                .into_response()
                        }
                    }
                })
                // POST: message callback (JSON protocol)
                .post({
                    let token = Arc::clone(&token);
                    let encoding_aes_key = Arc::clone(&encoding_aes_key);
                    let tx = Arc::clone(&tx);
                    let webhook_key = Arc::clone(&webhook_key);
                    move |axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
                          body: String| {
                        let token = Arc::clone(&token);
                        let encoding_aes_key = Arc::clone(&encoding_aes_key);
                        let tx = Arc::clone(&tx);
                        let account_id = Arc::clone(&account_id);
                        let webhook_key = Arc::clone(&webhook_key);
                        async move {
                            // Parse JSON body: {"encrypt": "BASE64_ENCRYPTED"}
                            let body_json: serde_json::Value = match serde_json::from_str(&body) {
                                Ok(v) => v,
                                Err(e) => {
                                    warn!("WeCom callback: invalid JSON body: {e}");
                                    return (axum::http::StatusCode::BAD_REQUEST, "invalid JSON")
                                        .into_response();
                                }
                            };

                            let encrypt = match body_json.get("encrypt").and_then(|v| v.as_str()) {
                                Some(e) => e.to_string(),
                                None => {
                                    warn!("WeCom callback: missing 'encrypt' field");
                                    return (axum::http::StatusCode::BAD_REQUEST, "missing encrypt field")
                                        .into_response();
                                }
                            };

                            // Verify signature
                            if let Some(token_str) = token.as_deref() {
                                let timestamp = params.get("timestamp").map(|s| s.as_str()).unwrap_or("");
                                let nonce = params.get("nonce").map(|s| s.as_str()).unwrap_or("");
                                let msg_sig = params.get("msg_signature").map(|s| s.as_str()).unwrap_or("");

                                if !is_valid_wecom_signature(token_str, timestamp, nonce, &encrypt, msg_sig) {
                                    warn!("WeCom callback: invalid signature");
                                    return (axum::http::StatusCode::FORBIDDEN, "invalid signature")
                                        .into_response();
                                }
                            }

                            // Decrypt the payload
                            let decrypted_json = match encoding_aes_key.as_deref() {
                                Some(aes_key) if !aes_key.is_empty() => {
                                    match decode_wecom_payload(aes_key, &encrypt) {
                                        Ok(plain) => plain,
                                        Err(err) => {
                                            warn!(error = %err, "WeCom callback: decrypt failed");
                                            return (axum::http::StatusCode::BAD_REQUEST, "decrypt failed")
                                                .into_response();
                                        }
                                    }
                                }
                                _ => encrypt,
                            };

                            // Parse decrypted JSON message
                            debug!(decrypted_json = %decrypted_json, "WeCom callback: decrypted payload");
                            let msg: serde_json::Value = match serde_json::from_str(&decrypted_json) {
                                Ok(v) => v,
                                Err(e) => {
                                    warn!(raw = %decrypted_json, "WeCom callback: invalid decrypted JSON: {e}");
                                    return (axum::http::StatusCode::OK, "").into_response();
                                }
                            };

                            let msgtype = msg.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
                            let user_id = msg
                                .get("from")
                                .and_then(|f| f.get("userid"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let chat_type = msg.get("chattype").and_then(|v| v.as_str()).unwrap_or("single");
                            let is_group = chat_type == "group";
                            let msg_id = msg.get("msgid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let response_url = msg.get("response_url").and_then(|v| v.as_str()).unwrap_or("").to_string();

                            info!(
                                msgtype = msgtype,
                                from_user = %user_id,
                                chat_type = chat_type,
                                "Received WeCom bot callback"
                            );

                            // Extract and cache webhook key from response_url
                            if !response_url.is_empty() {
                                if let Some(key) = response_url
                                    .split("key=")
                                    .nth(1)
                                    .map(|k| k.split('&').next().unwrap_or(k).to_string())
                                {
                                    let mut guard = webhook_key.write().await;
                                    *guard = Some(key);
                                }
                            }

                            // Handle event messages
                            if msgtype == "event" {
                                return (axum::http::StatusCode::OK, "").into_response();
                            }

                            // Handle text messages
                            if msgtype == "text" {
                                let content = msg
                                    .get("text")
                                    .and_then(|t| t.get("content"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                if !user_id.is_empty() && !content.is_empty() {
                                    let mut channel_msg = ChannelMessage {
                                        channel: ChannelType::Custom("wecom".to_string()),
                                        platform_message_id: msg_id,
                                        sender: ChannelUser {
                                            platform_id: user_id.clone(),
                                            display_name: user_id.clone(),
                                            librefang_user: None,
                                        },
                                        content: ChannelContent::Text(content),
                                        target_agent: None,
                                        timestamp: Utc::now(),
                                        is_group,
                                        thread_id: None,
                                        metadata: HashMap::new(),
                                    };

                                    // Store response_url for async reply
                                    if !response_url.is_empty() {
                                        channel_msg.metadata.insert(
                                            "wecom_response_url".to_string(),
                                            serde_json::json!(response_url),
                                        );
                                    }

                                    if let Some(ref aid) = *account_id {
                                        channel_msg.metadata.insert(
                                            "account_id".to_string(),
                                            serde_json::json!(aid),
                                        );
                                    }

                                    let _ = tx.send(channel_msg).await;
                                }
                            }

                            // Return empty response (passive reply not used — we reply async)
                            (axum::http::StatusCode::OK, "").into_response()
                        }
                    }
                }),
            );

            let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
            info!("WeCom callback server listening on http://0.0.0.0:{port}");

            let listener = match tokio::net::TcpListener::bind(addr).await {
                Ok(l) => l,
                Err(e) => {
                    warn!("WeCom: failed to bind port {port}: {e}");
                    return;
                }
            };

            let server = axum::serve(listener, app);

            tokio::select! {
                result = server => {
                    if let Err(e) = result {
                        warn!("WeCom callback server error: {e}");
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("WeCom callback adapter shutting down");
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}

#[async_trait]
impl ChannelAdapter for WeComAdapter {
    fn name(&self) -> &str {
        "wecom"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("wecom".to_string())
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        // Only callback mode uses HTTP webhook routes.
        // WebSocket mode connects outbound and does not need a local HTTP server.
        let Mode::Callback {
            token,
            encoding_aes_key,
            webhook_key,
            ..
        } = &self.mode
        else {
            return None;
        };

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let account_id = Arc::new(self.account_id.clone());

        let token = Arc::new(token.clone());
        let encoding_aes_key = Arc::new(encoding_aes_key.clone());
        let tx = Arc::new(tx);
        let webhook_key = Arc::clone(webhook_key);

        let app = axum::Router::new().route(
            "/webhook",
            // GET: URL verification
            axum::routing::get({
                let encoding_aes_key = Arc::clone(&encoding_aes_key);
                let token = Arc::clone(&token);
                move |axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>| {
                    let encoding_aes_key = Arc::clone(&encoding_aes_key);
                    let token = Arc::clone(&token);
                    async move {
                        if let (Some(echostr), Some(msg_sig), Some(timestamp), Some(nonce)) = (
                            params.get("echostr"),
                            params.get("msg_signature"),
                            params.get("timestamp"),
                            params.get("nonce"),
                        ) {
                            let Some(token_str) = token.as_deref() else {
                                return (
                                    axum::http::StatusCode::BAD_REQUEST,
                                    "missing WeCom callback token",
                                )
                                    .into_response();
                            };

                            if !is_valid_wecom_signature(
                                token_str, timestamp, nonce, echostr, msg_sig,
                            ) {
                                return (
                                    axum::http::StatusCode::FORBIDDEN,
                                    "invalid WeCom callback signature",
                                )
                                    .into_response();
                            }

                            let body = match encoding_aes_key.as_deref() {
                                Some(aes_key) if !aes_key.is_empty() => {
                                    match decode_wecom_payload(aes_key, echostr) {
                                        Ok(plain) => plain,
                                        Err(err) => {
                                            warn!(error = %err, "Failed to decrypt WeCom echostr");
                                            return (
                                                axum::http::StatusCode::BAD_REQUEST,
                                                "invalid WeCom echostr",
                                            )
                                                .into_response();
                                        }
                                    }
                                }
                                _ => echostr.clone(),
                            };

                            return (
                                axum::http::StatusCode::OK,
                                [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                                body,
                            )
                                .into_response();
                        }
                        (
                            axum::http::StatusCode::BAD_REQUEST,
                            "missing WeCom verification parameters",
                        )
                            .into_response()
                    }
                }
            })
            // POST: message callback (JSON protocol)
            .post({
                let token = Arc::clone(&token);
                let encoding_aes_key = Arc::clone(&encoding_aes_key);
                let tx = Arc::clone(&tx);
                let webhook_key = Arc::clone(&webhook_key);
                move |axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
                      body: String| {
                    let token = Arc::clone(&token);
                    let encoding_aes_key = Arc::clone(&encoding_aes_key);
                    let tx = Arc::clone(&tx);
                    let account_id = Arc::clone(&account_id);
                    let webhook_key = Arc::clone(&webhook_key);
                    async move {
                        // Parse JSON body: {"encrypt": "BASE64_ENCRYPTED"}
                        let body_json: serde_json::Value = match serde_json::from_str(&body) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!("WeCom callback: invalid JSON body: {e}");
                                return (axum::http::StatusCode::BAD_REQUEST, "invalid JSON")
                                    .into_response();
                            }
                        };

                        let encrypt = match body_json.get("encrypt").and_then(|v| v.as_str()) {
                            Some(e) => e.to_string(),
                            None => {
                                warn!("WeCom callback: missing 'encrypt' field");
                                return (axum::http::StatusCode::BAD_REQUEST, "missing encrypt field")
                                    .into_response();
                            }
                        };

                        // Verify signature
                        if let Some(token_str) = token.as_deref() {
                            let timestamp = params.get("timestamp").map(|s| s.as_str()).unwrap_or("");
                            let nonce = params.get("nonce").map(|s| s.as_str()).unwrap_or("");
                            let msg_sig = params.get("msg_signature").map(|s| s.as_str()).unwrap_or("");

                            if !is_valid_wecom_signature(token_str, timestamp, nonce, &encrypt, msg_sig) {
                                warn!("WeCom callback: invalid signature");
                                return (axum::http::StatusCode::FORBIDDEN, "invalid signature")
                                    .into_response();
                            }
                        }

                        // Decrypt the payload
                        let decrypted_json = match encoding_aes_key.as_deref() {
                            Some(aes_key) if !aes_key.is_empty() => {
                                match decode_wecom_payload(aes_key, &encrypt) {
                                    Ok(plain) => plain,
                                    Err(err) => {
                                        warn!(error = %err, "WeCom callback: decrypt failed");
                                        return (axum::http::StatusCode::BAD_REQUEST, "decrypt failed")
                                            .into_response();
                                    }
                                }
                            }
                            _ => encrypt,
                        };

                        // Parse decrypted JSON message
                        debug!(decrypted_json = %decrypted_json, "WeCom callback: decrypted payload");
                        let msg: serde_json::Value = match serde_json::from_str(&decrypted_json) {
                            Ok(v) => v,
                            Err(e) => {
                                warn!(raw = %decrypted_json, "WeCom callback: invalid decrypted JSON: {e}");
                                return (axum::http::StatusCode::OK, "").into_response();
                            }
                        };

                        let msgtype = msg.get("msgtype").and_then(|v| v.as_str()).unwrap_or("");
                        let user_id = msg
                            .get("from")
                            .and_then(|f| f.get("userid"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let chat_type = msg.get("chattype").and_then(|v| v.as_str()).unwrap_or("single");
                        let is_group = chat_type == "group";
                        let msg_id = msg.get("msgid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let response_url = msg.get("response_url").and_then(|v| v.as_str()).unwrap_or("").to_string();

                        info!(
                            msgtype = msgtype,
                            from_user = %user_id,
                            chat_type = chat_type,
                            "Received WeCom bot callback"
                        );

                        // Extract and cache webhook key from response_url
                        if !response_url.is_empty() {
                            if let Some(key) = response_url
                                .split("key=")
                                .nth(1)
                                .map(|k| k.split('&').next().unwrap_or(k).to_string())
                            {
                                let mut guard = webhook_key.write().await;
                                *guard = Some(key);
                            }
                        }

                        // Handle event messages
                        if msgtype == "event" {
                            return (axum::http::StatusCode::OK, "").into_response();
                        }

                        // Handle text messages
                        if msgtype == "text" {
                            let content = msg
                                .get("text")
                                .and_then(|t| t.get("content"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();

                            if !user_id.is_empty() && !content.is_empty() {
                                let mut channel_msg = ChannelMessage {
                                    channel: ChannelType::Custom("wecom".to_string()),
                                    platform_message_id: msg_id,
                                    sender: ChannelUser {
                                        platform_id: user_id.clone(),
                                        display_name: user_id.clone(),
                                        librefang_user: None,
                                    },
                                    content: ChannelContent::Text(content),
                                    target_agent: None,
                                    timestamp: Utc::now(),
                                    is_group,
                                    thread_id: None,
                                    metadata: HashMap::new(),
                                };

                                // Store response_url for async reply
                                if !response_url.is_empty() {
                                    channel_msg.metadata.insert(
                                        "wecom_response_url".to_string(),
                                        serde_json::json!(response_url),
                                    );
                                }

                                if let Some(ref aid) = *account_id {
                                    channel_msg.metadata.insert(
                                        "account_id".to_string(),
                                        serde_json::json!(aid),
                                    );
                                }

                                let _ = tx.send(channel_msg).await;
                            }
                        }

                        // Return empty response (passive reply not used — we reply async)
                        (axum::http::StatusCode::OK, "").into_response()
                    }
                }
            }),
        );

        info!("WeCom: registered webhook routes on shared server at /channels/wecom");

        Some((
            app,
            Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)),
        ))
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let account_id = Arc::new(self.account_id.clone());
        let shutdown_rx = self.shutdown_rx.clone();

        match &self.mode {
            Mode::Websocket {
                ws_tx,
                pending_req_ids,
            } => Ok(Self::start_websocket(
                self.bot_id.clone(),
                self.secret.clone(),
                account_id,
                shutdown_rx,
                Arc::clone(ws_tx),
                Arc::clone(pending_req_ids),
            )),
            Mode::Callback { .. } => {
                // Callback mode is handled by create_webhook_routes().
                // If we reach here, return an empty stream as fallback.
                let (_tx, rx) = mpsc::channel::<ChannelMessage>(1);
                Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
            }
        }
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let text = match content {
            ChannelContent::Text(t) => t,
            ChannelContent::Command { .. } => {
                warn!("WeCom bot: commands not supported");
                return Ok(());
            }
            _ => {
                warn!("WeCom bot: unsupported content type");
                return Ok(());
            }
        };

        info!(
            user_id = %user.platform_id,
            text_len = text.len(),
            mode = match &self.mode {
                Mode::Websocket { .. } => "websocket",
                Mode::Callback { .. } => "callback",
            },
            "WeCom bot send() called"
        );

        match &self.mode {
            Mode::Websocket {
                ws_tx,
                pending_req_ids,
            } => {
                let guard = ws_tx.read().await;
                let frame_tx = match guard.as_ref() {
                    Some(tx) => tx,
                    None => {
                        error!(user_id = %user.platform_id, "WeCom bot WebSocket not connected (ws_tx is None)");
                        return Err("WeCom bot WebSocket not connected".into());
                    }
                };
                let user_id = &user.platform_id;

                // Try to use aibot_respond_msg with the cached req_id for this user
                let req_id = {
                    let mut map = pending_req_ids.write().await;
                    let rid = map.remove(user_id);
                    info!(
                        user_id = %user_id,
                        req_id = ?rid,
                        pending_count = map.len(),
                        "WeCom bot req_id lookup"
                    );
                    rid
                };

                let chunks: Vec<&str> = split_message(&text, MAX_MESSAGE_LEN);
                info!(
                    user_id = %user_id,
                    chunk_count = chunks.len(),
                    reply_mode = if req_id.is_some() { "aibot_respond_msg" } else { "aibot_send_msg" },
                    "WeCom bot sending chunks"
                );

                for (i, chunk) in chunks.into_iter().enumerate() {
                    let frame = if let Some(ref rid) = req_id {
                        Self::build_reply_frame(rid, chunk)
                    } else {
                        Self::build_send_frame(user_id, chunk)
                    };
                    debug!(
                        chunk_index = i,
                        frame_len = frame.len(),
                        "WeCom bot queuing frame"
                    );
                    frame_tx.send(frame).map_err(|e| {
                        error!("WeCom bot frame_tx.send failed: {e}");
                        format!("WeCom bot send failed: {e}")
                    })?;
                }
                info!(user_id = %user_id, "WeCom bot send() completed (WebSocket)");
            }
            Mode::Callback {
                client,
                webhook_key,
                ..
            } => {
                // Try response_url from user metadata first, fall back to webhook key
                let response_url: Option<String> = None; // TODO: response_url is per-message, not available on ChannelUser

                if let Some(url) = response_url {
                    info!(url = %url, "WeCom bot replying via response_url");
                    // Use response_url (one-time, per-message)
                    for chunk in split_message(&text, MAX_MESSAGE_LEN) {
                        let payload = serde_json::json!({
                            "msgtype": "text",
                            "text": { "content": chunk }
                        });
                        let resp = client.post(&url).json(&payload).send().await?;
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        if !status.is_success() {
                            error!(status = %status, body = %body, "WeCom response_url error");
                        } else {
                            debug!(status = %status, body = %body, "WeCom response_url reply sent");
                        }
                    }
                } else {
                    // Fall back to webhook key
                    let key_guard = webhook_key.read().await;
                    let key = match key_guard.as_ref() {
                        Some(k) => k.clone(),
                        None => {
                            error!(user_id = %user.platform_id, "WeCom callback: no webhook key available (no messages received yet)");
                            return Err("WeCom callback: no webhook key available (no messages received yet)".into());
                        }
                    };
                    drop(key_guard);

                    let url = format!(
                        "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key={}",
                        key
                    );
                    info!(url = %url, "WeCom bot replying via webhook key");
                    for chunk in split_message(&text, MAX_MESSAGE_LEN) {
                        let payload = serde_json::json!({
                            "msgtype": "text",
                            "text": { "content": chunk }
                        });
                        let resp = client.post(&url).json(&payload).send().await?;
                        let status = resp.status();
                        let body = resp.text().await.unwrap_or_default();
                        if !status.is_success() {
                            error!(status = %status, body = %body, "WeCom webhook send error");
                            return Err(format!("WeCom webhook error {status}: {body}").into());
                        } else {
                            info!(status = %status, body = %body, "WeCom webhook reply sent");
                        }
                    }
                }
                info!(user_id = %user.platform_id, "WeCom bot send() completed (Callback)");
            }
        }

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    // ── WebSocket mode tests ────────────────────────────────────

    #[test]
    fn test_adapter_name() {
        let adapter = WeComAdapter::new("bot_id".to_string(), "secret".to_string());
        assert_eq!(adapter.name(), "wecom");
    }

    #[test]
    fn test_adapter_channel_type() {
        let adapter = WeComAdapter::new("bot_id".to_string(), "secret".to_string());
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("wecom".to_string())
        );
    }

    #[test]
    fn test_callback_adapter_name() {
        let adapter = WeComAdapter::new_callback(
            "bot_id".to_string(),
            "secret".to_string(),
            8454,
            Some("token".to_string()),
            Some("aes_key".to_string()),
        );
        assert_eq!(adapter.name(), "wecom");
    }

    #[test]
    fn test_build_reply_frame() {
        let frame = WeComAdapter::build_reply_frame("req123", "hello");
        let parsed: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(parsed["cmd"], "aibot_respond_msg");
        assert_eq!(parsed["headers"]["req_id"], "req123");
        assert_eq!(parsed["body"]["msgtype"], "markdown");
        assert_eq!(parsed["body"]["markdown"]["content"], "hello");
    }

    #[test]
    fn test_build_send_frame() {
        let frame = WeComAdapter::build_send_frame("user1", "hi");
        let parsed: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(parsed["cmd"], "aibot_send_msg");
        assert_eq!(parsed["body"]["receiver"]["userid"], "user1");
        assert_eq!(parsed["body"]["msgtype"], "markdown");
        assert_eq!(parsed["body"]["markdown"]["content"], "hi");
    }

    #[test]
    fn test_extract_msg_callback_cmd_format() {
        // Official protocol: cmd/headers/body
        let frame = serde_json::json!({
            "cmd": "aibot_msg_callback",
            "headers": { "req_id": "req123" },
            "body": {
                "from": { "user_id": "user1" },
                "msgtype": "text",
                "text": { "content": "hello bot" },
                "chat_type": "single",
            }
        });
        let (req_id, user, content, is_group) = extract_msg_callback(&frame).unwrap();
        assert_eq!(req_id, "req123");
        assert_eq!(user, "user1");
        assert_eq!(content, "hello bot");
        assert!(!is_group);
    }

    #[test]
    fn test_extract_msg_callback_legacy_format() {
        // Legacy format: action/data (backwards compat)
        let frame = serde_json::json!({
            "action": "aibot_msg_callback",
            "data": {
                "req_id": "req123",
                "from": { "user_id": "user1" },
                "msgtype": "text",
                "text": { "content": "hello bot" },
                "chat_type": "single",
            }
        });
        let (req_id, user, content, is_group) = extract_msg_callback(&frame).unwrap();
        assert_eq!(req_id, "req123");
        assert_eq!(user, "user1");
        assert_eq!(content, "hello bot");
        assert!(!is_group);
    }

    #[test]
    fn test_extract_msg_callback_group() {
        let frame = serde_json::json!({
            "cmd": "aibot_msg_callback",
            "headers": { "req_id": "req456" },
            "body": {
                "from": { "user_id": "user2" },
                "msgtype": "text",
                "text": { "content": "group msg" },
                "chat_type": "group",
            }
        });
        let (_, _, _, is_group) = extract_msg_callback(&frame).unwrap();
        assert!(is_group);
    }

    #[test]
    fn test_extract_msg_callback_ignores_non_text() {
        let frame = serde_json::json!({
            "cmd": "aibot_msg_callback",
            "headers": { "req_id": "req789" },
            "body": {
                "from": { "user_id": "user3" },
                "msgtype": "image",
            }
        });
        assert!(extract_msg_callback(&frame).is_none());
    }

    #[test]
    fn test_extract_msg_callback_ignores_other_actions() {
        let frame = serde_json::json!({
            "cmd": "aibot_event_callback",
            "body": { "event": "enter_chat" }
        });
        assert!(extract_msg_callback(&frame).is_none());
    }

    #[test]
    fn test_is_subscribe_success_cmd() {
        let frame = serde_json::json!({
            "cmd": "aibot_subscribe",
            "errcode": 0,
            "errmsg": "ok"
        });
        assert!(is_subscribe_success(&frame));
    }

    #[test]
    fn test_is_subscribe_success_no_cmd() {
        // Real server response: no cmd field, just errcode + headers.req_id
        let frame = serde_json::json!({
            "errcode": 0,
            "errmsg": "ok",
            "headers": { "req_id": "aibot_subscribe_1774529981774" }
        });
        assert!(is_subscribe_success(&frame));
    }

    #[test]
    fn test_is_subscribe_success_legacy() {
        let frame = serde_json::json!({
            "action": "aibot_subscribe",
            "errcode": 0,
            "errmsg": "ok"
        });
        assert!(is_subscribe_success(&frame));
    }

    #[test]
    fn test_is_subscribe_failure() {
        let frame = serde_json::json!({
            "cmd": "aibot_subscribe",
            "errcode": 40001,
            "errmsg": "invalid secret"
        });
        assert!(!is_subscribe_success(&frame));
    }

    #[test]
    fn test_extract_msg_callback_real_format() {
        // Actual format from WeCom server (from logs)
        let frame = serde_json::json!({
            "cmd": "aibot_msg_callback",
            "headers": { "req_id": "eiS8BA_YSomRowVhAtPz5QAA" },
            "body": {
                "aibotid": "aibcf7gdd",
                "chattype": "single",
                "from": { "userid": "0000002" },
                "msgid": "08f2b98a",
                "msgtype": "text",
                "response_url": "https://qyapi.weixin.qq.com/cgi-bin/aibot/response?response_code=xxx",
                "text": { "content": "你好" }
            }
        });
        let (req_id, user, content, is_group) = extract_msg_callback(&frame).unwrap();
        assert_eq!(req_id, "eiS8BA_YSomRowVhAtPz5QAA");
        assert_eq!(user, "0000002");
        assert_eq!(content, "你好");
        assert!(!is_group);
    }

    #[test]
    fn test_max_message_length() {
        assert_eq!(MAX_MESSAGE_LEN, 4096);
    }

    // ── Callback-mode crypto tests ──────────────────────────────

    #[test]
    fn test_wecom_signature_validation() {
        assert!(is_valid_wecom_signature(
            "token",
            "1710000000",
            "nonce",
            "echostr",
            "bf56bf867459f80e3ceb854596f39f02a5ac5e13",
        ));
        assert!(!is_valid_wecom_signature(
            "token",
            "1710000000",
            "nonce",
            "echostr",
            "bad-signature",
        ));
    }

    #[test]
    fn test_decode_wecom_payload() {
        let plain = decode_wecom_payload(
            "ShlNaJ0PrdXQAuCDVqMki7c2JLNnY6mebvQodTv9qoV",
            "/gKbXNFpvlyYNTCneTag1rGm1P4Q5fExE3OPzdYlEyUVDgi55PHVIbo+mHMXWatdW8H8RTQJCly0HBNrWry2Uw==",
        )
        .expect("echostr should decrypt");
        assert_eq!(plain, "openfang-wecom-check");
    }

    #[test]
    fn test_decode_wecom_payload_rejects_invalid_aes_key_length() {
        let err = decode_wecom_payload("abcd", "Zm9v").expect_err("invalid key should error");
        assert!(err.contains("invalid WeCom AES key length"));
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = [0x42u8; 32];
        let plaintext = b"hello world test message padding!";
        let encrypted = encrypt_aes_cbc(&key, plaintext).unwrap();
        let encrypted_b64 = base64::engine::general_purpose::STANDARD.encode(&encrypted);
        let decrypted = decrypt_aes_cbc(&key, &encrypted_b64).unwrap();
        assert_eq!(&decrypted[..plaintext.len()], plaintext);
    }
}
