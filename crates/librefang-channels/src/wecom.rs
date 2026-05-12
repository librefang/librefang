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
    // `cipher` 0.5: BlockDecryptMut → BlockModeDecrypt, and padded methods
    // lost the `_mut` suffix. `new_from_slices` avoids the &[u8]→&Array
    // `Into` bound that no longer exists in cipher 0.5's Array type.
    use cbc::cipher::{BlockModeDecrypt, KeyIvInit};

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
    let cipher = Aes256CbcDecrypt::new_from_slices(key, iv)
        .map_err(|e| format!("cipher init failed: {e}"))?;

    let decrypted = cipher
        .decrypt_padded::<aes::cipher::block_padding::NoPadding>(&mut encrypted)
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
    // See `decrypt_aes_cbc` for the cipher 0.5 migration notes.
    use cbc::cipher::{BlockModeEncrypt, KeyIvInit};

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
    let cipher = Aes256CbcEncrypt::new_from_slices(key, iv)
        .map_err(|e| format!("cipher init failed: {e}"))?;
    let len = padded.len();
    let encrypted = cipher
        .encrypt_padded::<aes::cipher::block_padding::NoPadding>(&mut padded, len)
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

/// Replace credential-bearing query parameter values with `***` so URLs are
/// safe to log at INFO. WeCom `response_url` carries a one-shot
/// `response_code` bearer; webhook URLs carry a long-lived `key` secret.
/// Both are sufficient on their own to send messages on the bot's behalf.
fn redact_credential_query_params(url: &str) -> String {
    const REDACTED: &[&str] = &["response_code", "key"];
    let Some((base, query)) = url.split_once('?') else {
        return url.to_string();
    };
    let redacted_query = query
        .split('&')
        .map(|kv| match kv.split_once('=') {
            Some((k, _)) if REDACTED.contains(&k) => format!("{k}=***"),
            _ => kv.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{redacted_query}")
}

// ── WeComAdapter ───────────────────────────────────────────────────

/// Maps user_id → latest req_id so we can use `aibot_respond_msg` for replies.
type ReqIdMap = Arc<RwLock<HashMap<String, String>>>;

/// Operational mode determined at construction time.
enum Mode {
    /// WebSocket long-connection.
    Websocket {
        ws_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
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
        /// per-user `response_url` cache so reply paths can quote
        /// the exact one-time URL the platform delivered with the inbound
        /// message instead of falling back to the webhook key for every
        /// outbound. WeCom invalidates the URL after ~5 min, so we evict on
        /// read using `RESPONSE_URL_TTL`. Keyed by `ChannelUser.platform_id`
        /// (= the WeCom `from.userid`) so the lookup at send time has the
        /// info `ChannelUser` actually carries.
        response_urls:
            Arc<RwLock<std::collections::BTreeMap<String, (String, std::time::Instant)>>>,
    },
}

/// TTL for cached `response_url` entries. WeCom documents the
/// URL as one-shot per inbound; in practice the platform tolerates the
/// reply for a few minutes. Five minutes is conservative — outside that
/// window we drop back to the webhook key (which always works for proactive
/// messages and is what the code did before the cache existed).
const RESPONSE_URL_TTL: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// WeCom intelligent bot adapter.
pub struct WeComAdapter {
    bot_id: String,
    secret: Zeroizing<String>,
    account_id: Option<String>,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    mode: Mode,
    /// Override the WeCom webhook base URL. `None` = production default
    /// (`https://qyapi.weixin.qq.com/cgi-bin/webhook/send`); tests set
    /// this via `with_webhook_base` to point at a local wiremock.
    webhook_base: Option<String>,
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
            webhook_base: None,
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
                response_urls: Arc::new(RwLock::new(std::collections::BTreeMap::new())),
            },
            webhook_base: None,
        }
    }

    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the WeCom webhook base URL. `#[cfg(test)]`-only — used by
    /// wiremock-driven tests to point the adapter at a local mock server.
    #[cfg(test)]
    pub fn with_webhook_base(mut self, url: String) -> Self {
        self.webhook_base = Some(url);
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
        ws_tx_shared: Arc<RwLock<Option<mpsc::Sender<String>>>>,
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

                // Bounded to apply backpressure when the WebSocket sink stalls
                // (rate limit, slow network) instead of letting RSS grow
                // unbounded (#3580). Cap is generous — frames are small and
                // 1024 covers normal burst depth; producers .await on full.
                let (frame_tx, mut frame_rx) = mpsc::channel::<String>(1024);
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
                            let chat_id = msg.get("chatid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let msg_id = msg.get("msgid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let response_url = msg.get("response_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            // Composite identity scopes per-chat for groups so the
                            // same user across multiple rooms stays distinguishable
                            // downstream. See the parallel block in the Callback
                            // handler for the full rationale.
                            let composite_id = if is_group && !chat_id.is_empty() {
                                format!("{user_id}|{chat_id}")
                            } else {
                                user_id.clone()
                            };

                            info!(
                                msgtype = msgtype,
                                from_user = %user_id,
                                chat_type = chat_type,
                                chat_id = %chat_id,
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
                                            platform_id: composite_id.clone(),
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
            response_urls,
            ..
        } = &self.mode
        else {
            return None;
        };

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let account_id = Arc::new(self.account_id.clone());

        let token = Arc::new(token.clone());
        let encoding_aes_key = Arc::new(encoding_aes_key.clone());
        let response_urls_for_route = Arc::clone(response_urls);
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
                let response_urls = Arc::clone(&response_urls_for_route);
                move |axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
                      body: String| {
                    let token = Arc::clone(&token);
                    let encoding_aes_key = Arc::clone(&encoding_aes_key);
                    let tx = Arc::clone(&tx);
                    let account_id = Arc::clone(&account_id);
                    let webhook_key = Arc::clone(&webhook_key);
                    let response_urls = Arc::clone(&response_urls);
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
                        let chat_id = msg.get("chatid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let msg_id = msg.get("msgid").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        let response_url = msg.get("response_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        // Cache + identity key. WeCom routes by `(user_id, chat_id)` —
                        // the same user messaging the bot from multiple groups gets a
                        // distinct response_url per group. A user-only key would let
                        // a later group's URL overwrite an earlier group's, so the
                        // bot's reply to the earlier message would land in the wrong
                        // chat. Composite for groups; raw user_id for single chats
                        // (DMs are 1:1, no per-chat ambiguity, and keeping the bare
                        // user_id preserves existing identity downstream).
                        let composite_id = if is_group && !chat_id.is_empty() {
                            format!("{user_id}|{chat_id}")
                        } else {
                            user_id.clone()
                        };

                        info!(
                            msgtype = msgtype,
                            from_user = %user_id,
                            chat_type = chat_type,
                            chat_id = %chat_id,
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
                                        // Composite identity scopes per-chat for groups
                                        // so downstream routing (cache, sessions) can
                                        // distinguish the same user across rooms.
                                        platform_id: composite_id.clone(),
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
                                    // Cache the per-(user, chat) response_url so the
                                    // `send()` path (which only sees `&ChannelUser`,
                                    // not the originating `ChannelMessage`) can quote
                                    // the exact one-time URL the platform delivered.
                                    // Key matches `sender.platform_id` so the lookup
                                    // is symmetrical.
                                    let mut guard = response_urls.write().await;
                                    guard.insert(
                                        composite_id.clone(),
                                        (response_url.clone(), std::time::Instant::now()),
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
                    frame_tx.send(frame).await.map_err(|e| {
                        error!("WeCom bot frame_tx.send failed: {e}");
                        format!("WeCom bot send failed: {e}")
                    })?;
                }
                info!(user_id = %user_id, "WeCom bot send() completed (WebSocket)");
            }
            Mode::Callback {
                client,
                webhook_key,
                response_urls,
                ..
            } => {
                // look up the cached `response_url` for this
                // user. The cache is populated by the inbound POST handler
                // when the platform delivers a `response_url` alongside the
                // message. Entries older than `RESPONSE_URL_TTL` are evicted
                // here (read-side eviction keeps the map size bounded
                // without a separate sweep).
                // Single `remove` covers both branches: live-and-fresh →
                // return URL (one-shot semantics, the URL is burned by
                // the platform after reply), expired or missing → None
                // and the stale entry is gone either way.
                let response_url: Option<String> = response_urls
                    .write()
                    .await
                    .remove(&user.platform_id)
                    .and_then(|(url, captured_at)| {
                        (captured_at.elapsed() < RESPONSE_URL_TTL).then_some(url)
                    });

                if let Some(url) = response_url {
                    info!(url = %redact_credential_query_params(&url), "WeCom bot replying via response_url");
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

                    let base = self
                        .webhook_base
                        .as_deref()
                        .unwrap_or("https://qyapi.weixin.qq.com/cgi-bin/webhook/send");
                    let url = format!("{}?key={}", base, key);
                    info!(url = %redact_credential_query_params(&url), "WeCom bot replying via webhook key");
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

    // ----- send() path tests (issue #3820) -----
    //
    // WeCom is the most operationally complex adapter in the crate (two
    // modes — Websocket and Callback — plus AES-encrypted callbacks).
    // For send-path coverage we only exercise Callback mode's webhook
    // fallback: it is the only outbound path that goes over plain HTTP
    // (Websocket mode just queues a frame onto an internal mpsc; that's
    // already covered by `test_build_reply_frame` / `test_build_send_frame`).
    //
    // The hard-coded `https://qyapi.weixin.qq.com/cgi-bin/webhook/send`
    // URL is now overridable via `with_webhook_base` (test-only).
    // `webhook_key` lives inside `Mode::Callback` and is normally
    // populated by the callback receiver after the bot's first inbound
    // message; tests seed it directly because `mod tests` is a child
    // module and can pattern-match the variant.

    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn wecom_user(user_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: user_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    async fn seed_webhook_key(adapter: &WeComAdapter, key: &str) {
        if let Mode::Callback { webhook_key, .. } = &adapter.mode {
            *webhook_key.write().await = Some(key.to_string());
        } else {
            panic!("seed_webhook_key requires Callback mode");
        }
    }

    async fn seed_response_url(
        adapter: &WeComAdapter,
        user_id: &str,
        url: &str,
        captured_at: std::time::Instant,
    ) {
        if let Mode::Callback { response_urls, .. } = &adapter.mode {
            response_urls
                .write()
                .await
                .insert(user_id.to_string(), (url.to_string(), captured_at));
        } else {
            panic!("seed_response_url requires Callback mode");
        }
    }

    async fn response_url_cache_size(adapter: &WeComAdapter) -> usize {
        if let Mode::Callback { response_urls, .. } = &adapter.mode {
            response_urls.read().await.len()
        } else {
            panic!("response_url_cache_size requires Callback mode");
        }
    }

    #[tokio::test]
    async fn wecom_callback_send_posts_webhook_with_key_query_and_text_payload() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(query_param("key", "WEBHOOK-KEY-1"))
            .and(body_json(serde_json::json!({
                "msgtype": "text",
                "text": { "content": "hi wecom" },
            })))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"errcode": 0})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            8454,
            Some("token".to_string()),
            None,
        )
        .with_webhook_base(server.uri());
        seed_webhook_key(&adapter, "WEBHOOK-KEY-1").await;

        adapter
            .send(
                &wecom_user("user-42"),
                ChannelContent::Text("hi wecom".into()),
            )
            .await
            .expect("wecom callback send must succeed against mock");
    }

    #[tokio::test]
    async fn wecom_callback_send_returns_err_when_no_webhook_key() {
        // No `seed_webhook_key` call ⇒ webhook_key stays None ⇒ send()
        // must error before any HTTP attempt.
        let server = MockServer::start().await;
        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            8454,
            None,
            None,
        )
        .with_webhook_base(server.uri());

        let err = adapter
            .send(&wecom_user("user-42"), ChannelContent::Text("x".into()))
            .await
            .expect_err("wecom callback send must error when webhook_key is unset");
        assert!(
            err.to_string().to_lowercase().contains("no webhook key"),
            "error should mention missing webhook_key, got: {err}"
        );
        let received = server
            .received_requests()
            .await
            .expect("MockServer should expose received_requests");
        assert!(
            received.is_empty(),
            "wecom must not hit the network when webhook_key is unset; got {} request(s)",
            received.len()
        );
    }

    #[tokio::test]
    async fn wecom_callback_send_returns_err_on_non_2xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            8454,
            None,
            None,
        )
        .with_webhook_base(server.uri());
        seed_webhook_key(&adapter, "WEBHOOK-KEY-1").await;

        let err = adapter
            .send(&wecom_user("user-42"), ChannelContent::Text("x".into()))
            .await
            .expect_err("wecom callback send must propagate non-2xx as Err");
        assert!(
            err.to_string().contains("500"),
            "error should mention status code, got: {err}"
        );
    }

    // ----- response_url cache (#27 / TTL semantics) -----

    #[tokio::test]
    async fn cached_response_url_is_used_then_evicted_on_send() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/wecom-respond"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            8455,
            None,
            None,
        );
        // Seed the cache with a fresh URL pointing at the mock server.
        let url = format!("{}/wecom-respond", server.uri());
        seed_response_url(&adapter, "user-A", &url, std::time::Instant::now()).await;

        adapter
            .send(&wecom_user("user-A"), ChannelContent::Text("hi".into()))
            .await
            .expect("send should hit cached response_url and return Ok");

        // One-shot: cache must be empty after the send consumed the entry.
        assert_eq!(
            response_url_cache_size(&adapter).await,
            0,
            "cache must be drained after one-shot consume"
        );
    }

    #[tokio::test]
    async fn second_send_falls_back_to_webhook_when_cache_empty() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            8456,
            None,
            None,
        )
        .with_webhook_base(server.uri());
        seed_webhook_key(&adapter, "WEBHOOK-KEY-2").await;
        // No cache seed → second-send-style scenario.
        adapter
            .send(&wecom_user("user-B"), ChannelContent::Text("hi".into()))
            .await
            .expect("empty cache must fall back to webhook key path");
    }

    #[tokio::test]
    async fn expired_cached_response_url_is_evicted_and_falls_back() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            8457,
            None,
            None,
        )
        .with_webhook_base(server.uri());
        seed_webhook_key(&adapter, "WEBHOOK-KEY-3").await;

        // Seed with a captured_at older than the TTL: the entry must be
        // evicted on read and the send must fall back to the webhook key.
        let stale = std::time::Instant::now()
            .checked_sub(RESPONSE_URL_TTL + std::time::Duration::from_secs(1))
            .expect("clock skew");
        seed_response_url(&adapter, "user-C", "http://stale-url.invalid/", stale).await;

        adapter
            .send(&wecom_user("user-C"), ChannelContent::Text("hi".into()))
            .await
            .expect("expired entry must not block the fallback path");

        // Cache must be drained even on expiry — otherwise it would
        // accumulate stale entries forever for users who never get fresh
        // inbounds.
        assert_eq!(
            response_url_cache_size(&adapter).await,
            0,
            "expired entry must be removed on read"
        );
    }

    #[tokio::test]
    async fn cached_response_url_is_chat_isolated_in_groups() {
        // Regression: pre-fix the cache keyed on `user_id` only, so the
        // same user pinging the bot from two different groups would have
        // the second URL silently overwrite the first. The bot's reply to
        // the first message would then land in the wrong group.
        // Composite `user_id|chat_id` keys keep entries independent.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            8458,
            None,
            None,
        )
        .with_webhook_base(server.uri());
        seed_webhook_key(&adapter, "WEBHOOK-KEY-4").await;

        // Two composite keys for the same user across two groups. Only
        // the chat-A entry should be consumed when we send to that
        // composite identity; the chat-B entry stays put.
        let url_a = format!("{}/group-a/", server.uri());
        let url_b = format!("{}/group-b/", server.uri());
        let now = std::time::Instant::now();
        seed_response_url(&adapter, "user-X|chat-A", &url_a, now).await;
        seed_response_url(&adapter, "user-X|chat-B", &url_b, now).await;
        assert_eq!(
            response_url_cache_size(&adapter).await,
            2,
            "both composite keys must coexist before send",
        );

        adapter
            .send(
                &wecom_user("user-X|chat-A"),
                ChannelContent::Text("hi".into()),
            )
            .await
            .expect("send must hit url_a");

        assert_eq!(
            response_url_cache_size(&adapter).await,
            1,
            "send must consume only the addressed chat's entry",
        );
    }

    // ----- end-to-end inbound POST handler tests (#4826) -----
    //
    // These tests exercise the full path that the three `cached_response_url_*`
    // tests above deliberately skip: request decryption → signature validation
    // → JSON parsing → cache insertion, all wired through the real axum router
    // returned by `create_webhook_routes()`.
    //
    // Strategy: call `create_webhook_routes()` to obtain the `axum::Router`,
    // drive it with `tower::ServiceExt::oneshot`, then inspect the shared
    // `response_urls` BTreeMap that the adapter and the route handler both
    // reference. Because `Mode::Callback { response_urls, .. }` is an `Arc`,
    // the adapter sees exactly the same map the handler wrote into.

    mod test_fixtures {
        use super::*;
        use base64::Engine;

        /// Build an AES-CBC encrypted, PKCS#7-padded WeCom inbound payload.
        ///
        /// Returns `(encrypt_b64, msg_signature, timestamp, nonce)` — everything
        /// the POST handler needs to verify and decrypt the message.
        pub(super) fn build_signed_encrypted_payload(
            encoding_aes_key: &str,
            token: &str,
            plaintext_json: &str,
        ) -> (String, String, String, String) {
            use base64::{
                alphabet,
                engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
            };

            let aes_key_engine = GeneralPurpose::new(
                &alphabet::STANDARD,
                GeneralPurposeConfig::new()
                    .with_decode_padding_mode(DecodePaddingMode::RequireNone)
                    .with_decode_allow_trailing_bits(true),
            );
            let aes_key = aes_key_engine
                .decode(encoding_aes_key)
                .expect("test AES key must decode");

            // Build plaintext: 16-byte random prefix + 4-byte big-endian msg len + msg bytes
            let msg_bytes = plaintext_json.as_bytes();
            let random_prefix = [0x42u8; 16]; // deterministic for tests
            let mut raw = Vec::new();
            raw.extend_from_slice(&random_prefix);
            raw.extend_from_slice(&(msg_bytes.len() as u32).to_be_bytes());
            raw.extend_from_slice(msg_bytes);
            // receiveid suffix is omitted (empty for intelligent bot callbacks)

            let encrypted = encrypt_aes_cbc(&aes_key, &raw).expect("encrypt must succeed");
            let encrypt_b64 = base64::engine::general_purpose::STANDARD.encode(&encrypted);

            // Compute signature: SHA1(sort(token, timestamp, nonce, encrypt_b64))
            let timestamp = "1710000000";
            let nonce = "testnonce";
            let mut parts = [token, timestamp, nonce, &encrypt_b64];
            parts.sort_unstable();
            let mut hasher = sha1::Sha1::new();
            sha1::Digest::update(&mut hasher, parts.concat().as_bytes());
            let sig = hex::encode(sha1::Digest::finalize(hasher));

            (encrypt_b64, sig, timestamp.to_string(), nonce.to_string())
        }
    }

    /// A fixed 43-character (no-padding) base64 AES-256 key used by the
    /// handler round-trip tests. Decodes to 32 bytes.
    const TEST_ENCODING_AES_KEY: &str = "ShlNaJ0PrdXQAuCDVqMki7c2JLNnY6mebvQodTv9qoV";
    const TEST_TOKEN: &str = "test-token-4826";

    #[tokio::test]
    async fn inbound_post_handler_writes_response_url_to_cache_and_send_consumes_it() {
        use tower::ServiceExt;

        // Build a mock server to capture the outbound `send()` POST.
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/wecom-respond"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&mock_server)
            .await;

        let response_url = format!("{}/wecom-respond", mock_server.uri());

        // Build the encrypted POST body that the inbound handler will decrypt.
        let user_id = "user-4826";
        let chat_id = "chat-abc";
        let plaintext_json = serde_json::json!({
            "msgtype": "text",
            "from": { "userid": user_id },
            "chattype": "group",
            "chatid": chat_id,
            "msgid": "msg-001",
            "text": { "content": "hello from e2e test" },
            "response_url": response_url,
        })
        .to_string();

        let (encrypt_b64, sig, timestamp, nonce) = test_fixtures::build_signed_encrypted_payload(
            TEST_ENCODING_AES_KEY,
            TEST_TOKEN,
            &plaintext_json,
        );

        // Create the adapter and obtain its router.
        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            0, // port unused — we call create_webhook_routes(), not start_callback()
            Some(TEST_TOKEN.to_string()),
            Some(TEST_ENCODING_AES_KEY.to_string()),
        );

        let (router, mut stream) = adapter
            .create_webhook_routes()
            .await
            .expect("Callback mode must return webhook routes");

        // POST the encrypted payload to the handler.
        let url = format!(
            "/webhook?msg_signature={}&timestamp={}&nonce={}",
            sig, timestamp, nonce
        );
        let req = axum::http::Request::builder()
            .method("POST")
            .uri(&url)
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({ "encrypt": encrypt_b64 }).to_string(),
            ))
            .expect("request must build");

        let status = router.clone().oneshot(req).await.unwrap().status();
        assert_eq!(
            status,
            axum::http::StatusCode::OK,
            "inbound POST must return 200"
        );

        // The handler must have inserted `user_id|chat_id` into the cache.
        let expected_key = format!("{user_id}|{chat_id}");
        assert_eq!(
            response_url_cache_size(&adapter).await,
            1,
            "cache must hold exactly one entry after inbound POST"
        );

        // The stream must yield exactly the ChannelMessage the handler sent.
        let channel_msg = stream
            .next()
            .await
            .expect("stream must yield one ChannelMessage");
        assert_eq!(
            channel_msg.sender.platform_id, expected_key,
            "sender.platform_id must be composite user|chat key"
        );
        match &channel_msg.content {
            ChannelContent::Text(t) => assert_eq!(
                t.as_str(),
                "hello from e2e test",
                "content must match plaintext payload"
            ),
            other => panic!("expected ChannelContent::Text, got {other:?}"),
        }
        assert!(
            channel_msg.is_group,
            "chattype=group must set is_group=true"
        );

        // send() must consume the cached response_url — NOT fall back to webhook.
        let user = wecom_user(&expected_key);
        adapter
            .send(&user, ChannelContent::Text("reply".into()))
            .await
            .expect("send must succeed via cached response_url");

        // One-shot: cache is drained after send consumes the entry.
        assert_eq!(
            response_url_cache_size(&adapter).await,
            0,
            "cache must be empty after send consumed the response_url"
        );
    }

    #[tokio::test]
    async fn inbound_post_from_different_chat_does_not_overwrite_first_cache_entry() {
        use tower::ServiceExt;

        let user_id = "user-4826";
        let chat_id_a = "chat-aaa";
        let chat_id_b = "chat-bbb";
        let response_url_a = "https://example.invalid/respond-a";
        let response_url_b = "https://example.invalid/respond-b";

        let adapter = WeComAdapter::new_callback(
            "bot-id".to_string(),
            "secret".to_string(),
            0,
            Some(TEST_TOKEN.to_string()),
            Some(TEST_ENCODING_AES_KEY.to_string()),
        );

        let (router, _stream) = adapter
            .create_webhook_routes()
            .await
            .expect("Callback mode must return webhook routes");

        // Helper: POST one encrypted callback for a given (user, chat, response_url).
        let post_callback = |router: axum::Router, uid: &str, cid: &str, rurl: &str| {
            let plaintext_json = serde_json::json!({
                "msgtype": "text",
                "from": { "userid": uid },
                "chattype": "group",
                "chatid": cid,
                "msgid": format!("msg-{cid}"),
                "text": { "content": "hello" },
                "response_url": rurl,
            })
            .to_string();

            let (encrypt_b64, sig, timestamp, nonce) =
                test_fixtures::build_signed_encrypted_payload(
                    TEST_ENCODING_AES_KEY,
                    TEST_TOKEN,
                    &plaintext_json,
                );

            let url = format!(
                "/webhook?msg_signature={}&timestamp={}&nonce={}",
                sig, timestamp, nonce
            );
            let body = serde_json::json!({ "encrypt": encrypt_b64 }).to_string();
            let req = axum::http::Request::builder()
                .method("POST")
                .uri(&url)
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body))
                .expect("request must build");

            async move { router.oneshot(req).await.unwrap().status() }
        };

        // POST from chat-A.
        let status_a = post_callback(router.clone(), user_id, chat_id_a, response_url_a).await;
        assert_eq!(status_a, axum::http::StatusCode::OK);

        // POST from chat-B (same user_id, different chat_id).
        let status_b = post_callback(router.clone(), user_id, chat_id_b, response_url_b).await;
        assert_eq!(status_b, axum::http::StatusCode::OK);

        // Both composite keys must coexist — chat-B must NOT have overwritten chat-A.
        assert_eq!(
            response_url_cache_size(&adapter).await,
            2,
            "cache must hold two independent entries for different chats of the same user"
        );

        // Verify that the correct URL is keyed under each composite id.
        let key_a = format!("{user_id}|{chat_id_a}");
        let key_b = format!("{user_id}|{chat_id_b}");
        if let Mode::Callback { response_urls, .. } = &adapter.mode {
            let guard = response_urls.read().await;
            assert_eq!(
                guard.get(&key_a).map(|(u, _)| u.as_str()),
                Some(response_url_a),
                "chat-A entry must be intact"
            );
            assert_eq!(
                guard.get(&key_b).map(|(u, _)| u.as_str()),
                Some(response_url_b),
                "chat-B entry must be present independently"
            );
        }
    }

    #[test]
    fn redact_credential_query_params_strips_response_code_and_key() {
        let url = "https://qyapi.weixin.qq.com/cgi-bin/aibot/response?response_code=SECRET-XYZ&other=keep";
        let r = redact_credential_query_params(url);
        assert!(
            !r.contains("SECRET-XYZ"),
            "response_code value must be redacted"
        );
        assert!(
            r.contains("response_code=***"),
            "redacted placeholder expected"
        );
        assert!(
            r.contains("other=keep"),
            "non-secret params must be preserved"
        );

        let webhook = "https://qyapi.weixin.qq.com/cgi-bin/webhook/send?key=AAAA-BBBB-CCCC";
        let rw = redact_credential_query_params(webhook);
        assert!(!rw.contains("AAAA"), "webhook key value must be redacted");
        assert!(rw.contains("key=***"), "redacted webhook key expected");

        // No query string: pass-through.
        assert_eq!(
            redact_credential_query_params("https://example.com/path"),
            "https://example.com/path"
        );
    }
}
