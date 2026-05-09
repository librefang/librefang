//! Matrix channel adapter.
//!
//! Uses the Matrix Client-Server API (via reqwest) for sending and receiving messages.
//! Implements /sync long-polling with exponential backoff on failures for automatic
//! reconnection after connection drops.

use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

// Backoff durations are now configurable via MatrixConfig.
/// Matrix /sync long-polling timeout in milliseconds.
const SYNC_TIMEOUT_MS: u64 = 30000;
const MAX_MESSAGE_LEN: usize = 4096;
#[allow(dead_code)]
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
const STREAM_EDIT_INTERVAL_MS: u64 = 1500;
const STREAM_EDIT_CHAR_BUDGET: usize = 256;
/// Maximum number of per-(room, target_event) lifecycle reaction entries to track.
const PHASE_REACTIONS_CAPACITY: usize = 1024;

/// Insertion-ordered cache mapping (room_id, target_event_id) -> reaction event_id.
type PhaseReactionCache = Arc<RwLock<std::collections::VecDeque<((String, String), String)>>>;

/// Convert mxc://server/mediaId -> an HTTPS download URL.
///
/// Uses the legacy /_matrix/media/v3/download endpoint (unauthenticated).
/// Synapse 1.100+ supports MSC3916 authenticated /_matrix/client/v1/media/download
/// but requires opt-in enforcement (`enable_authenticated_media: true`).
/// Default Synapse leaves the legacy endpoint working — this is the broadest
/// compatibility path. MSC3916 fallback is documented as a known limitation.
#[allow(dead_code)]
pub(crate) fn mxc_to_http(mxc: &str, homeserver_url: &str) -> Option<String> {
    let stripped = mxc.strip_prefix("mxc://")?;
    let (server, media_id) = stripped.split_once('/')?;
    if server.is_empty() || media_id.is_empty() {
        return None;
    }
    Some(format!(
        "{homeserver_url}/_matrix/media/v3/download/{server}/{media_id}"
    ))
}

/// Extract the thread root event_id from an event's content if it has
/// an m.thread relation. Returns None for plain messages, replies, or edits.
pub(crate) fn parse_thread_relation(content: &serde_json::Value) -> Option<String> {
    let rel = content.get("m.relates_to")?.as_object()?;
    let rel_type = rel.get("rel_type")?.as_str()?;
    if rel_type != "m.thread" {
        return None;
    }
    rel.get("event_id")?.as_str().map(String::from)
}

/// Matrix channel adapter using the Client-Server API.
pub struct MatrixAdapter {
    /// Matrix homeserver URL (e.g., `"https://matrix.org"`).
    homeserver_url: String,
    /// Bot's user ID (e.g., "@librefang:matrix.org").
    user_id: String,
    /// SECURITY: Access token is zeroized on drop.
    access_token: Zeroizing<String>,
    /// HTTP client.
    client: reqwest::Client,
    /// Allowed room IDs (empty = all joined rooms).
    allowed_rooms: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Whether to automatically accept room invites.
    /// Used when processing `/sync` invite events (not yet wired).
    #[allow(dead_code)]
    auto_accept_invites: bool,
    /// Initial backoff on sync failures.
    initial_backoff: Duration,
    /// Maximum backoff on sync failures.
    max_backoff: Duration,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Sync token for resuming /sync.
    since_token: Arc<RwLock<Option<String>>>,
    /// Tracks our most-recent lifecycle reaction per (room, target_event).
    /// Maps to the event_id of the bot's reaction so we can redact it when
    /// the next phase fires with remove_previous=true. Insertion-ordered;
    /// bounded at PHASE_REACTIONS_CAPACITY entries (oldest evicted).
    pub(crate) phase_reactions: PhaseReactionCache,
}

impl MatrixAdapter {
    /// Create a new Matrix adapter.
    pub fn new(
        homeserver_url: String,
        user_id: String,
        access_token: String,
        allowed_rooms: Vec<String>,
        auto_accept_invites: bool,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            homeserver_url,
            user_id,
            access_token: Zeroizing::new(access_token),
            client: crate::http_client::new_client(),
            allowed_rooms,
            account_id: None,
            auto_accept_invites,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            since_token: Arc::new(RwLock::new(None)),
            phase_reactions: Arc::new(RwLock::new(std::collections::VecDeque::new())),
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Set backoff configuration. Returns self for builder chaining.
    pub fn with_backoff(mut self, initial_backoff_secs: u64, max_backoff_secs: u64) -> Self {
        self.initial_backoff = Duration::from_secs(initial_backoff_secs);
        self.max_backoff = Duration::from_secs(max_backoff_secs);
        self
    }

    /// Send any client event to a Matrix room. Returns the server-assigned event_id.
    ///
    /// `event_type` is the Matrix event type, e.g. "m.room.message" or "m.reaction".
    /// `body` is the event content JSON.
    async fn api_send_event(
        &self,
        room_id: &str,
        event_type: &str,
        body: &serde_json::Value,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let txn_id = uuid::Uuid::new_v4().to_string();
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/send/{}/{}",
            self.homeserver_url,
            urlencoding::encode(room_id),
            urlencoding::encode(event_type),
            txn_id
        );
        let resp = self
            .client
            .put(&url)
            .bearer_auth(&*self.access_token)
            .json(body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Matrix {event_type} failed {status}: {text}").into());
        }
        let v: serde_json::Value = resp.json().await?;
        let event_id = v["event_id"]
            .as_str()
            .ok_or_else(|| "Matrix response missing event_id".to_string())?
            .to_string();
        Ok(event_id)
    }

    /// Redact (delete) a previously sent event.
    /// Returns the redaction event_id.
    async fn api_redact(
        &self,
        room_id: &str,
        target_event_id: &str,
        reason: Option<&str>,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let txn_id = uuid::Uuid::new_v4().to_string();
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/redact/{}/{}",
            self.homeserver_url,
            urlencoding::encode(room_id),
            urlencoding::encode(target_event_id),
            txn_id
        );
        let body = match reason {
            Some(r) => serde_json::json!({ "reason": r }),
            None => serde_json::json!({}),
        };
        let resp = self
            .client
            .put(&url)
            .bearer_auth(&*self.access_token)
            .json(&body)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Matrix redact failed {status}: {text}").into());
        }
        let v: serde_json::Value = resp.json().await?;
        let event_id = v["event_id"]
            .as_str()
            .ok_or_else(|| "Matrix redact response missing event_id".to_string())?
            .to_string();
        Ok(event_id)
    }

    /// Look up our reaction event_id for (room, target). O(n).
    #[allow(dead_code)]
    pub(crate) async fn phase_reaction_lookup(&self, key: &(String, String)) -> Option<String> {
        self.phase_reactions
            .read()
            .await
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    /// Remove (room, target). Returns the previous reaction id if present. O(n).
    #[allow(dead_code)]
    pub(crate) async fn phase_reaction_remove(&self, key: &(String, String)) -> Option<String> {
        let mut w = self.phase_reactions.write().await;
        if let Some(pos) = w.iter().position(|(k, _)| k == key) {
            w.remove(pos).map(|(_, v)| v)
        } else {
            None
        }
    }

    /// Insert (room, target) -> reaction_id, evicting oldest if at capacity.
    /// If the key already exists, the value is replaced in place (preserving position).
    #[allow(dead_code)]
    pub(crate) async fn phase_reaction_insert(&self, key: (String, String), reaction_id: String) {
        let mut w = self.phase_reactions.write().await;
        if let Some(pos) = w.iter().position(|(k, _)| k == &key) {
            w[pos].1 = reaction_id;
            return;
        }
        if w.len() >= PHASE_REACTIONS_CAPACITY {
            w.pop_front();
        }
        w.push_back((key, reaction_id));
    }

    /// Upload bytes to Matrix media repo. Returns mxc:// URI.
    #[allow(dead_code)]
    async fn api_upload_media(
        &self,
        bytes: Vec<u8>,
        filename: &str,
        mime_type: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        if bytes.len() > MAX_UPLOAD_BYTES {
            return Err(format!(
                "Matrix upload size {} exceeds {} byte limit",
                bytes.len(),
                MAX_UPLOAD_BYTES
            )
            .into());
        }
        let url = format!(
            "{}/_matrix/media/v3/upload?filename={}",
            self.homeserver_url,
            urlencoding::encode(filename)
        );
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&*self.access_token)
            .header("Content-Type", mime_type)
            .body(bytes)
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Matrix media upload failed {status}: {text}").into());
        }
        let v: serde_json::Value = resp.json().await?;
        let mxc = v["content_uri"]
            .as_str()
            .ok_or_else(|| "Matrix upload response missing content_uri".to_string())?
            .to_string();
        Ok(mxc)
    }

    /// Edit an existing event in place via the m.replace relation.
    /// `new_text` is the new content. Returns the edit event_id.
    async fn api_edit_event(
        &self,
        room_id: &str,
        target_event_id: &str,
        new_text: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = serde_json::json!({
            "msgtype": "m.text",
            "body": format!("* {new_text}"),
            "m.new_content": {
                "msgtype": "m.text",
                "body": new_text,
            },
            "m.relates_to": {
                "rel_type": "m.replace",
                "event_id": target_event_id,
            }
        });
        self.api_send_event(room_id, "m.room.message", &body).await
    }

    /// api_edit_event with one Retry-After-aware retry on 429. Returns Ok on
    /// success, Err on second 429 / non-429 error. Used by send_streaming.
    async fn api_edit_event_with_retry(
        &self,
        room_id: &str,
        target_event_id: &str,
        new_text: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        match self
            .api_edit_event(room_id, target_event_id, new_text)
            .await
        {
            Ok(id) => Ok(id),
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("429") {
                    // Conservative fixed backoff. We can't recover the
                    // Retry-After header from the boxed error type today;
                    // a richer error type is a deferred follow-up.
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    self.api_edit_event(room_id, target_event_id, new_text)
                        .await
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Send a text message to a Matrix room. Splits long messages into chunks
    /// and sends each as a separate `m.room.message` event. Returns the
    /// event_ids in order; the last one is the message useful for editing/redacting.
    async fn api_send_message(
        &self,
        room_id: &str,
        text: &str,
    ) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let chunks = crate::types::split_message(text, MAX_MESSAGE_LEN);
        let mut event_ids = Vec::with_capacity(chunks.len());
        for chunk in chunks {
            let body = serde_json::json!({
                "msgtype": "m.text",
                "body": chunk,
            });
            let id = self
                .api_send_event(room_id, "m.room.message", &body)
                .await?;
            event_ids.push(id);
        }
        Ok(event_ids)
    }

    /// Validate credentials by calling /whoami.
    async fn validate(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/_matrix/client/v3/account/whoami", self.homeserver_url);

        let resp = self
            .client
            .get(&url)
            .bearer_auth(&*self.access_token)
            .send()
            .await?;

        if !resp.status().is_success() {
            return Err("Matrix authentication failed".into());
        }

        let body: serde_json::Value = resp.json().await?;
        let user_id = body["user_id"].as_str().unwrap_or("unknown").to_string();

        Ok(user_id)
    }

    #[allow(dead_code)]
    fn is_allowed_room(&self, room_id: &str) -> bool {
        self.allowed_rooms.is_empty() || self.allowed_rooms.iter().any(|r| r == room_id)
    }
}

#[async_trait]
impl ChannelAdapter for MatrixAdapter {
    fn name(&self) -> &str {
        "matrix"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Matrix
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // Validate credentials
        let validated_user = self.validate().await?;
        info!("Matrix adapter authenticated as {validated_user}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let homeserver = self.homeserver_url.clone();
        let access_token = self.access_token.clone();
        let user_id = self.user_id.clone();
        let allowed_rooms = self.allowed_rooms.clone();
        let client = self.client.clone();
        let since_token = Arc::clone(&self.since_token);
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();
        let initial_backoff = self.initial_backoff;
        let max_backoff = self.max_backoff;

        tokio::spawn(async move {
            let mut backoff = initial_backoff;

            loop {
                // Build /sync URL
                let since = since_token.read().await.clone();
                let mut url = format!(
                    "{}/_matrix/client/v3/sync?timeout={}&filter={{\"room\":{{\"timeline\":{{\"limit\":10}}}}}}",
                    homeserver, SYNC_TIMEOUT_MS
                );
                if let Some(ref token) = since {
                    url.push_str(&format!("&since={token}"));
                }

                let resp = tokio::select! {
                    _ = shutdown_rx.changed() => {
                        info!("Matrix adapter shutting down");
                        break;
                    }
                    result = client.get(&url).bearer_auth(&*access_token).send() => {
                        match result {
                            Ok(r) => r,
                            Err(e) => {
                                warn!("Matrix /sync network error: {e}, retrying in {backoff:?}");
                                tokio::time::sleep(backoff).await;
                                backoff = calculate_backoff(backoff, max_backoff);
                                continue;
                            }
                        }
                    }
                };

                if !resp.status().is_success() {
                    let status = resp.status();
                    warn!("Matrix /sync failed ({status}), retrying in {backoff:?}");
                    tokio::time::sleep(backoff).await;
                    backoff = calculate_backoff(backoff, max_backoff);
                    continue;
                }

                // Reset backoff on success
                if backoff > initial_backoff {
                    debug!("Matrix /sync recovered, resetting backoff");
                }
                backoff = initial_backoff;

                let body: serde_json::Value = match resp.json().await {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("Matrix sync parse error: {e}");
                        continue;
                    }
                };

                // Update since token
                if let Some(next) = body["next_batch"].as_str() {
                    *since_token.write().await = Some(next.to_string());
                }

                // Process room events
                if let Some(rooms) = body["rooms"]["join"].as_object() {
                    for (room_id, room_data) in rooms {
                        if !allowed_rooms.is_empty() && !allowed_rooms.iter().any(|r| r == room_id)
                        {
                            continue;
                        }

                        if let Some(events) = room_data["timeline"]["events"].as_array() {
                            for event in events {
                                let event_type = event["type"].as_str().unwrap_or("");
                                if event_type != "m.room.message" {
                                    continue;
                                }

                                let sender = event["sender"].as_str().unwrap_or("");
                                if sender == user_id {
                                    continue; // Skip own messages
                                }

                                let content = event["content"]["body"].as_str().unwrap_or("");
                                if content.is_empty() {
                                    continue;
                                }

                                let msg_content = if content.starts_with('/') {
                                    let parts: Vec<&str> = content.splitn(2, ' ').collect();
                                    let cmd = parts[0].trim_start_matches('/');
                                    let args: Vec<String> = parts
                                        .get(1)
                                        .map(|a| a.split_whitespace().map(String::from).collect())
                                        .unwrap_or_default();
                                    ChannelContent::Command {
                                        name: cmd.to_string(),
                                        args,
                                    }
                                } else {
                                    ChannelContent::Text(content.to_string())
                                };

                                let event_id = event["event_id"].as_str().unwrap_or("").to_string();
                                let thread_id = parse_thread_relation(&event["content"]);

                                let mut channel_msg = ChannelMessage {
                                    channel: ChannelType::Matrix,
                                    platform_message_id: event_id,
                                    sender: ChannelUser {
                                        platform_id: room_id.clone(),
                                        display_name: sender.to_string(),
                                        librefang_user: None,
                                    },
                                    content: msg_content,
                                    target_agent: None,
                                    timestamp: Utc::now(),
                                    is_group: true,
                                    thread_id,
                                    metadata: HashMap::new(),
                                };

                                // Inject account_id for multi-bot routing
                                if let Some(ref aid) = account_id {
                                    channel_msg
                                        .metadata
                                        .insert("account_id".to_string(), serde_json::json!(aid));
                                }
                                if tx.send(channel_msg).await.is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(&user.platform_id, &text).await?;
            }
            _ => {
                self.api_send_message(&user.platform_id, "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(
        &self,
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!(
            "{}/_matrix/client/v3/rooms/{}/typing/{}",
            self.homeserver_url, user.platform_id, self.user_id
        );

        let body = serde_json::json!({
            "typing": true,
            "timeout": 5000,
        });

        let _ = self
            .client
            .put(&url)
            .bearer_auth(&*self.access_token)
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    async fn send_reaction(
        &self,
        user: &ChannelUser,
        message_id: &str,
        reaction: &crate::types::LifecycleReaction,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let room = &user.platform_id;
        let key = (room.clone(), message_id.to_string());
        if reaction.remove_previous {
            if let Some(prev_id) = self.phase_reaction_remove(&key).await {
                if let Err(e) = self.api_redact(room, &prev_id, Some("phase change")).await {
                    debug!("Matrix: redact of previous reaction {prev_id} failed: {e}");
                }
            }
        }
        let body = serde_json::json!({
            "m.relates_to": {
                "rel_type": "m.annotation",
                "event_id": message_id,
                "key": reaction.emoji,
            }
        });
        let new_id = self.api_send_event(room, "m.reaction", &body).await?;
        self.phase_reaction_insert(key, new_id).await;
        Ok(())
    }

    async fn send_in_thread(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
        thread_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match content {
            ChannelContent::Text(text) => {
                for chunk in crate::types::split_message(&text, MAX_MESSAGE_LEN) {
                    let body = serde_json::json!({
                        "msgtype": "m.text",
                        "body": chunk,
                        "m.relates_to": {
                            "rel_type": "m.thread",
                            "event_id": thread_id,
                            "is_falling_back": true,
                            "m.in_reply_to": { "event_id": thread_id },
                        }
                    });
                    self.api_send_event(&user.platform_id, "m.room.message", &body)
                        .await?;
                }
                Ok(())
            }
            other => self.send(user, other).await,
        }
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn send_streaming(
        &self,
        user: &ChannelUser,
        mut delta_rx: mpsc::Receiver<String>,
        thread_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // 1. Send placeholder. With thread_id, include the m.thread relation
        //    so the placeholder lives in the thread; subsequent edits keep it
        //    via m.new_content.
        let placeholder_body = match thread_id {
            Some(tid) => serde_json::json!({
                "msgtype": "m.text",
                "body": "…",
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": tid,
                    "is_falling_back": true,
                    "m.in_reply_to": { "event_id": tid },
                }
            }),
            None => serde_json::json!({"msgtype": "m.text", "body": "…"}),
        };
        let mut placeholder_id = self
            .api_send_event(&user.platform_id, "m.room.message", &placeholder_body)
            .await?;

        let mut buffer = String::new();
        let mut last_flushed_len: usize = 0;
        let mut last_edit = std::time::Instant::now();
        let interval = Duration::from_millis(STREAM_EDIT_INTERVAL_MS);

        // Flush helper: edits the current placeholder. On overflow, splits
        // head/tail (UTF-8-safe), finalizes head as edit, sends tail as a
        // fresh non-edit event whose id becomes the new placeholder, and
        // returns the new buffer length.
        async fn flush(
            adapter: &MatrixAdapter,
            room: &str,
            placeholder_id: &mut String,
            buffer: &mut String,
        ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
            if buffer.len() <= MAX_MESSAGE_LEN {
                adapter
                    .api_edit_event_with_retry(room, placeholder_id, buffer)
                    .await?;
                return Ok(buffer.len());
            }
            // UTF-8-safe split via librefang_types::truncate_str.
            let head = librefang_types::truncate_str(buffer, MAX_MESSAGE_LEN);
            let head_len = head.len();
            let tail = buffer[head_len..].to_string();
            adapter
                .api_edit_event_with_retry(room, placeholder_id, head)
                .await?;
            let body = serde_json::json!({"msgtype": "m.text", "body": tail});
            let new_id = adapter
                .api_send_event(room, "m.room.message", &body)
                .await?;
            *placeholder_id = new_id;
            *buffer = tail;
            Ok(buffer.len())
        }

        while let Some(delta) = delta_rx.recv().await {
            buffer.push_str(&delta);
            let elapsed = last_edit.elapsed();
            let added = buffer.len().saturating_sub(last_flushed_len);
            if elapsed >= interval
                || added >= STREAM_EDIT_CHAR_BUDGET
                || buffer.len() > MAX_MESSAGE_LEN
            {
                last_flushed_len =
                    flush(self, &user.platform_id, &mut placeholder_id, &mut buffer).await?;
                last_edit = std::time::Instant::now();
            }
        }
        if !buffer.is_empty() {
            // Drain any further overflows — possible if the final delta crossed
            // multiple cap boundaries.
            while buffer.len() > MAX_MESSAGE_LEN {
                let _ = flush(self, &user.platform_id, &mut placeholder_id, &mut buffer).await?;
            }
            let _ = flush(self, &user.platform_id, &mut placeholder_id, &mut buffer).await?;
        }
        let _ = last_flushed_len; // last_flushed_len is loop-local state for the in-flight debounce; reads happen via saturating_sub above
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

/// Calculate exponential backoff capped at the given maximum.
pub fn calculate_backoff(current: Duration, max: Duration) -> Duration {
    (current * 2).min(max)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- transport-layer tests for #3406 -----
    //
    // Stand up a local `wiremock::MockServer` and point `MatrixAdapter`
    // at it via the `homeserver_url` argument to `new()`. Exercises the
    // PUT `/_matrix/client/v3/rooms/{}/send/m.room.message/{txn_id}`
    // call made by `ChannelAdapter::send`.
    //
    // Matrix is the only one of the three #3406 top adapters where
    // idempotency is on the wire by design: the txn_id (last URL
    // segment) is the protocol-level dedup key. Today
    // `api_send_message` mints a fresh `Uuid::new_v4()` per call and
    // does not retry — so the dedup property exists but is unused.
    // Tests assert (a) the txn_id IS a UUID and (b) 429 / 5xx surface
    // as `Err` (fail-loud, unlike Slack/Discord); a follow-up that
    // adds retry must reuse the same txn_id and is tracked on #3406.

    use wiremock::matchers::{
        body_json, body_partial_json, header, method, path_regex, query_param,
    };
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn make_adapter(homeserver_url: String) -> MatrixAdapter {
        MatrixAdapter::new(
            homeserver_url,
            "@bot:matrix.org".to_string(),
            "secret-access-token".to_string(),
            vec![],
            false,
        )
    }

    fn dummy_user(room_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: room_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    /// Request shape: PUT to the documented Matrix CS-API path,
    /// `Bearer` auth, `m.text` body. Path matcher accepts any UUID
    /// txn_id segment (the txn_id assertion is in a separate test).
    #[tokio::test]
    async fn matrix_send_puts_room_message_event() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/%21room%3Aexample\.org/send/m\.room\.message/[0-9a-fA-F-]{36}$",
            ))
            .and(header("Authorization", "Bearer secret-access-token"))
            .and(body_json(serde_json::json!({
                "msgtype": "m.text",
                "body": "hello matrix",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$evt:example.org",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("!room:example.org"),
                ChannelContent::Text("hello matrix".into()),
            )
            .await
            .expect("matrix send must succeed against mock");
    }

    /// The txn_id MUST be a v4-shaped UUID. Capture the recorded request
    /// URL and assert the last path segment parses as a UUID. This pins
    /// the protocol-level idempotency key to a real opaque token (not,
    /// say, a monotonic counter) so dedup is preserved across daemon
    /// restarts.
    #[tokio::test]
    async fn matrix_send_uses_uuid_txn_id_for_idempotency() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/%21r%3Aexample\.org/send/m\.room\.message/[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$evt",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        adapter
            .send(
                &dummy_user("!r:example.org"),
                ChannelContent::Text("idempotent".into()),
            )
            .await
            .expect("matrix send must succeed");

        // Two independent send() calls produce different txn_ids
        // (today's behaviour — retry would need to *reuse* one txn_id,
        // tracked as follow-up on #3406). Use `received_requests()`
        // after-the-fact instead of a `respond_with` closure so we
        // capture txn_ids without juggling `Sync` closure state.
        let server2 = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/%21r%3Aexample\.org/send/m\.room\.message/.+$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$e",
            })))
            .expect(2)
            .mount(&server2)
            .await;

        let adapter2 = make_adapter(server2.uri());
        adapter2
            .send(
                &dummy_user("!r:example.org"),
                ChannelContent::Text("first".into()),
            )
            .await
            .unwrap();
        adapter2
            .send(
                &dummy_user("!r:example.org"),
                ChannelContent::Text("second".into()),
            )
            .await
            .unwrap();

        let recorded = server2
            .received_requests()
            .await
            .expect("wiremock should have recorded requests");
        assert_eq!(recorded.len(), 2, "expected exactly two PUT calls");
        let observed: Vec<String> = recorded
            .iter()
            .map(|r| {
                r.url
                    .path()
                    .rsplit('/')
                    .next()
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert_ne!(
            observed[0], observed[1],
            "today the adapter mints a fresh uuid per call; a future retry refactor MUST reuse one"
        );
        for txn in &observed {
            assert!(
                uuid::Uuid::parse_str(txn).is_ok(),
                "txn_id {txn} must be a valid UUID"
            );
        }
    }

    /// Matrix differs from Slack/Discord: `api_send_message` is
    /// fail-loud — non-2xx becomes `Err`, not a warn'd Ok. Pin that
    /// here so a future fail-open refactor doesn't silently swallow
    /// 429s. Single observation, no retry today.
    #[tokio::test]
    async fn matrix_send_returns_err_on_429_no_retry_today() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/%21r%3Aexample\.org/send/m\.room\.message/.+$",
            ))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "1")
                    .set_body_json(serde_json::json!({
                        "errcode": "M_LIMIT_EXCEEDED",
                        "error": "Too many requests",
                        "retry_after_ms": 1000,
                    })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(server.uri());
        let err = adapter
            .send(
                &dummy_user("!r:example.org"),
                ChannelContent::Text("rate-limited".into()),
            )
            .await
            .expect_err("matrix send is fail-loud on 429 today");
        let msg = format!("{err}");
        assert!(
            msg.contains("429") || msg.to_ascii_lowercase().contains("too many"),
            "error must surface the 429: {msg}"
        );
    }

    #[test]
    fn test_matrix_adapter_creation() {
        let adapter = MatrixAdapter::new(
            "https://matrix.org".to_string(),
            "@bot:matrix.org".to_string(),
            "access_token".to_string(),
            vec![],
            false,
        );
        assert_eq!(adapter.name(), "matrix");
    }

    #[test]
    fn test_matrix_allowed_rooms() {
        let adapter = MatrixAdapter::new(
            "https://matrix.org".to_string(),
            "@bot:matrix.org".to_string(),
            "token".to_string(),
            vec!["!room1:matrix.org".to_string()],
            false,
        );
        assert!(adapter.is_allowed_room("!room1:matrix.org"));
        assert!(!adapter.is_allowed_room("!room2:matrix.org"));

        let open = MatrixAdapter::new(
            "https://matrix.org".to_string(),
            "@bot:matrix.org".to_string(),
            "token".to_string(),
            vec![],
            false,
        );
        assert!(open.is_allowed_room("!any:matrix.org"));
    }

    #[test]
    fn test_backoff_calculation() {
        let max = Duration::from_secs(60);
        let b1 = calculate_backoff(Duration::from_secs(1), max);
        assert_eq!(b1, Duration::from_secs(2));

        let b2 = calculate_backoff(Duration::from_secs(2), max);
        assert_eq!(b2, Duration::from_secs(4));

        let b3 = calculate_backoff(Duration::from_secs(32), max);
        assert_eq!(b3, Duration::from_secs(60)); // capped at max_backoff

        let b4 = calculate_backoff(Duration::from_secs(60), max);
        assert_eq!(b4, Duration::from_secs(60)); // stays at max_backoff
    }

    #[test]
    fn test_backoff_defaults() {
        let initial = Duration::from_secs(1);
        let max = Duration::from_secs(60);
        assert!(initial < max);
    }

    #[tokio::test]
    async fn test_api_send_event_returns_event_id() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/%21room%3Atest/send/m\.room\.message/.+$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$abc:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let body = serde_json::json!({"msgtype":"m.text","body":"hi"});
        let id = adapter
            .api_send_event("!room:test", "m.room.message", &body)
            .await
            .expect("send must succeed");
        assert_eq!(id, "$abc:test");
    }

    #[tokio::test]
    async fn test_api_send_event_url_encodes_room_id() {
        let server = MockServer::start().await;
        // %23 is "#", %3A is ":". Match the encoded form in the URL path.
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/%23alias%3Atest/send/m\.room\.message/.+$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$x:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let body = serde_json::json!({"msgtype":"m.text","body":"hi"});
        adapter
            .api_send_event("#alias:test", "m.room.message", &body)
            .await
            .expect("aliased room must url-encode");
    }

    #[tokio::test]
    async fn test_api_send_event_propagates_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(
                ResponseTemplate::new(403).set_body_string("{\"errcode\":\"M_FORBIDDEN\"}"),
            )
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let body = serde_json::json!({"msgtype":"m.text","body":"hi"});
        let err = adapter
            .api_send_event("!room:test", "m.room.message", &body)
            .await
            .expect_err("403 must surface as Err");
        let msg = format!("{err}");
        assert!(msg.contains("403"), "err should include status: {msg}");
        assert!(
            msg.contains("M_FORBIDDEN"),
            "err should include body: {msg}"
        );
    }

    #[test]
    fn test_backoff_progression() {
        let initial = Duration::from_secs(1);
        let max = Duration::from_secs(60);
        // Verify the full backoff sequence from initial to max
        let mut current = initial;
        let expected = [1, 2, 4, 8, 16, 32, 60, 60];
        for &exp_secs in &expected {
            assert_eq!(
                current.as_secs(),
                if current == initial && exp_secs == 1 {
                    1
                } else {
                    current.as_secs()
                }
            );
            current = calculate_backoff(current, max);
        }
        // Simpler: just walk the sequence
        let mut b = initial;
        assert_eq!(b, Duration::from_secs(1));
        b = calculate_backoff(b, max);
        assert_eq!(b, Duration::from_secs(2));
        b = calculate_backoff(b, max);
        assert_eq!(b, Duration::from_secs(4));
        b = calculate_backoff(b, max);
        assert_eq!(b, Duration::from_secs(8));
        b = calculate_backoff(b, max);
        assert_eq!(b, Duration::from_secs(16));
        b = calculate_backoff(b, max);
        assert_eq!(b, Duration::from_secs(32));
        b = calculate_backoff(b, max);
        assert_eq!(b, Duration::from_secs(60));
        b = calculate_backoff(b, max);
        assert_eq!(b, Duration::from_secs(60)); // stays capped
    }

    #[tokio::test]
    async fn test_api_redact_happy_path() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/redact/%24evt%3Atest/.+$",
            ))
            .and(body_partial_json(
                serde_json::json!({"reason":"phase change"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$redact:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let id = adapter
            .api_redact("!room:test", "$evt:test", Some("phase change"))
            .await
            .expect("redact must succeed");
        assert_eq!(id, "$redact:test");
    }

    #[tokio::test]
    async fn test_api_edit_event_shape() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .and(body_partial_json(serde_json::json!({
                "msgtype": "m.text",
                "body": "* updated text",
                "m.new_content": { "msgtype": "m.text", "body": "updated text" },
                "m.relates_to": { "rel_type": "m.replace", "event_id": "$orig:test" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$edit:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let id = adapter
            .api_edit_event("!room:test", "$orig:test", "updated text")
            .await
            .expect("edit must succeed");
        assert_eq!(id, "$edit:test");
    }

    #[tokio::test]
    async fn test_api_upload_media_returns_mxc() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(query_param("filename", "x.pdf"))
            .and(header("Content-Type", "application/pdf"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content_uri": "mxc://srv/abc"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let mxc = adapter
            .api_upload_media(b"%PDF-1.4 dummy".to_vec(), "x.pdf", "application/pdf")
            .await
            .expect("upload must succeed");
        assert_eq!(mxc, "mxc://srv/abc");
    }

    #[tokio::test]
    async fn test_api_upload_media_size_cap() {
        let adapter = make_adapter("http://unused".to_string());
        let too_big = vec![0u8; 51 * 1024 * 1024];
        let err = adapter
            .api_upload_media(too_big, "huge.bin", "application/octet-stream")
            .await
            .expect_err("51MB must be rejected pre-flight");
        assert!(
            format!("{err}").to_lowercase().contains("size"),
            "err should mention size: {err}"
        );
    }

    #[test]
    fn test_mxc_to_http_basic() {
        let url = mxc_to_http("mxc://srv/abc", "https://hs.example.com").unwrap();
        assert_eq!(
            url,
            "https://hs.example.com/_matrix/media/v3/download/srv/abc"
        );
    }

    #[test]
    fn test_mxc_to_http_rejects_non_mxc() {
        assert!(mxc_to_http("https://other.example/x", "https://hs").is_none());
        assert!(mxc_to_http("mxc://no-slash", "https://hs").is_none());
        assert!(mxc_to_http("", "https://hs").is_none());
    }

    #[tokio::test]
    async fn test_send_reaction_first_phase() {
        use crate::types::{AgentPhase, ChannelUser, LifecycleReaction};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.reaction/.+$",
            ))
            .and(body_partial_json(serde_json::json!({
                "m.relates_to": {
                    "rel_type": "m.annotation",
                    "event_id": "$tgt:test",
                    "key": "🤔"
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$rxn1:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let r = LifecycleReaction {
            phase: AgentPhase::Thinking,
            emoji: "🤔".to_string(),
            remove_previous: false,
        };
        adapter
            .send_reaction(&user, "$tgt:test", &r)
            .await
            .expect("first reaction must succeed");
        let id = adapter
            .phase_reaction_lookup(&("!room:test".to_string(), "$tgt:test".to_string()))
            .await;
        assert_eq!(id, Some("$rxn1:test".to_string()));
    }

    #[tokio::test]
    async fn test_send_reaction_replace_redacts_previous() {
        use crate::types::{AgentPhase, ChannelUser, LifecycleReaction};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        let server = MockServer::start().await;
        let redact_calls = StdArc::new(AtomicUsize::new(0));
        let rc = redact_calls.clone();
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/redact/%24rxn1%3Atest/.+$",
            ))
            .respond_with(move |_: &wiremock::Request| {
                rc.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event_id": "$rdct:test"
                }))
            })
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.reaction/.+$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$rxn2:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        adapter
            .phase_reaction_insert(
                ("!room:test".to_string(), "$tgt:test".to_string()),
                "$rxn1:test".to_string(),
            )
            .await;
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let r = LifecycleReaction {
            phase: AgentPhase::Done,
            emoji: "✅".to_string(),
            remove_previous: true,
        };
        adapter
            .send_reaction(&user, "$tgt:test", &r)
            .await
            .expect("replacement reaction must succeed");
        assert_eq!(
            redact_calls.load(Ordering::SeqCst),
            1,
            "previous reaction must be redacted exactly once"
        );
        let id = adapter
            .phase_reaction_lookup(&("!room:test".to_string(), "$tgt:test".to_string()))
            .await;
        assert_eq!(id, Some("$rxn2:test".to_string()));
    }

    #[tokio::test]
    async fn test_send_reaction_remove_previous_swallows_redact_failure() {
        use crate::types::{AgentPhase, ChannelUser, LifecycleReaction};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(r"^/_matrix/client/v3/rooms/.+/redact/.+$"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.reaction/.+$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$new:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        adapter
            .phase_reaction_insert(
                ("!room:test".to_string(), "$tgt:test".to_string()),
                "$old:test".to_string(),
            )
            .await;
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let r = LifecycleReaction {
            phase: AgentPhase::Done,
            emoji: "✅".to_string(),
            remove_previous: true,
        };
        adapter
            .send_reaction(&user, "$tgt:test", &r)
            .await
            .expect("redact failure must be swallowed");
    }

    #[tokio::test]
    async fn test_send_reaction_lru_eviction() {
        use crate::types::{AgentPhase, ChannelUser, LifecycleReaction};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$x:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        // Pre-populate 1024 entries via the public-ish helper in insertion order.
        for i in 0..1024 {
            adapter
                .phase_reaction_insert(
                    ("!room:test".to_string(), format!("$evt{i}:test")),
                    format!("$rxn{i}:test"),
                )
                .await;
        }
        assert_eq!(
            adapter
                .phase_reaction_lookup(&("!room:test".to_string(), "$evt0:test".to_string()))
                .await,
            Some("$rxn0:test".to_string()),
            "evt0 must be present before eviction"
        );
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let r = LifecycleReaction {
            phase: AgentPhase::Thinking,
            emoji: "🤔".to_string(),
            remove_previous: false,
        };
        adapter
            .send_reaction(&user, "$evt_new:test", &r)
            .await
            .expect("send must succeed");
        let len = adapter.phase_reactions.read().await.len();
        assert_eq!(len, 1024, "map size must remain capped");
        assert_eq!(
            adapter
                .phase_reaction_lookup(&("!room:test".to_string(), "$evt0:test".to_string()))
                .await,
            None,
            "oldest entry (evt0) must be evicted"
        );
    }

    #[tokio::test]
    async fn test_send_in_thread_includes_thread_relation() {
        use crate::types::{ChannelContent, ChannelUser};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .and(body_partial_json(serde_json::json!({
                "msgtype": "m.text",
                "body": "thread reply",
                "m.relates_to": {
                    "rel_type": "m.thread",
                    "event_id": "$thread_root:test",
                    "is_falling_back": true,
                    "m.in_reply_to": { "event_id": "$thread_root:test" }
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$reply:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        adapter
            .send_in_thread(
                &user,
                ChannelContent::Text("thread reply".to_string()),
                "$thread_root:test",
            )
            .await
            .expect("thread send must succeed");
    }

    #[test]
    fn test_inbound_thread_id_populated() {
        let event_content = serde_json::json!({
            "msgtype": "m.text",
            "body": "in a thread",
            "m.relates_to": {
                "rel_type": "m.thread",
                "event_id": "$root:test"
            }
        });
        let tid = parse_thread_relation(&event_content);
        assert_eq!(tid, Some("$root:test".to_string()));
    }

    #[test]
    fn test_inbound_non_thread_message_no_thread_id() {
        let plain = serde_json::json!({"msgtype": "m.text", "body": "plain"});
        assert_eq!(parse_thread_relation(&plain), None);

        let reply_only = serde_json::json!({
            "msgtype": "m.text",
            "body": "reply",
            "m.relates_to": { "m.in_reply_to": { "event_id": "$x" } }
        });
        assert_eq!(parse_thread_relation(&reply_only), None);
    }

    #[tokio::test]
    async fn test_send_streaming_debounces_edits() {
        use crate::types::ChannelUser;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        use tokio::sync::mpsc;
        let server = MockServer::start().await;
        let send_calls = StdArc::new(AtomicUsize::new(0));
        let sc = send_calls.clone();
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .respond_with(move |_: &wiremock::Request| {
                let n = sc.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event_id": format!("$evt{n}:test")
                }))
            })
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let (tx, rx) = mpsc::channel(16);
        let sender = tokio::spawn(async move {
            for i in 0..10 {
                let _ = tx.send(format!("d{i}")).await;
            }
            drop(tx);
        });
        adapter
            .send_streaming(&user, rx, None)
            .await
            .expect("streaming must succeed");
        sender.await.unwrap();
        let calls = send_calls.load(Ordering::SeqCst);
        // Placeholder + final edit. 10 deltas of "dN" = ~20 chars total, well under
        // STREAM_EDIT_CHAR_BUDGET (256) and STREAM_EDIT_INTERVAL_MS (1500). So
        // exactly 2 PUTs: placeholder + final.
        assert!(calls >= 2, "expected at least 2 PUTs, got {calls}");
        assert!(
            calls <= 2,
            "expected at most 2 PUTs (debounce should suppress mid-stream edits), got {calls}"
        );
    }

    #[tokio::test]
    async fn test_send_streaming_finalizes_on_close() {
        use crate::types::ChannelUser;
        use tokio::sync::mpsc;
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .and(body_partial_json(serde_json::json!({
                "m.new_content": { "body": "alpha-beta" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$final:test"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$x:test"
            })))
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let (tx, rx) = mpsc::channel(2);
        let _ = tx.send("alpha".to_string()).await;
        let _ = tx.send("-beta".to_string()).await;
        drop(tx);
        adapter
            .send_streaming(&user, rx, None)
            .await
            .expect("streaming must succeed");
    }

    #[tokio::test]
    async fn test_send_streaming_handles_overflow() {
        use crate::types::ChannelUser;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        use std::sync::Mutex as StdMutex;
        use tokio::sync::mpsc;
        let server = MockServer::start().await;
        let send_calls = StdArc::new(AtomicUsize::new(0));
        let sc = send_calls.clone();
        // Capture every request body so we can assert overflow split shape:
        // a no-split impl produces exactly ONE fresh non-m.replace PUT (the
        // placeholder), while the rolling-placeholder impl produces TWO
        // (placeholder + rolled-over after head finalization).
        let bodies: StdArc<StdMutex<Vec<serde_json::Value>>> =
            StdArc::new(StdMutex::new(Vec::new()));
        let bc = bodies.clone();
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .respond_with(move |req: &wiremock::Request| {
                let n = sc.fetch_add(1, Ordering::SeqCst);
                if let Ok(b) = serde_json::from_slice::<serde_json::Value>(&req.body) {
                    bc.lock().unwrap().push(b);
                }
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event_id": format!("$evt{n}:test")
                }))
            })
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let (tx, rx) = mpsc::channel(8);
        // Send a single 5000-char delta, exceeding MAX_MESSAGE_LEN (4096).
        let _ = tx.send("a".repeat(5000)).await;
        drop(tx);
        adapter
            .send_streaming(&user, rx, None)
            .await
            .expect("streaming with overflow must succeed");
        let calls = send_calls.load(Ordering::SeqCst);
        // Expected: placeholder + 1 head edit + 1 fresh non-edit (overflow start)
        //         + 1 final flush of tail = at least 3 calls. Allow some slack
        //         for exact ordering.
        assert!(
            calls >= 3,
            "overflow should produce at least 3 PUTs, got {calls}"
        );
        // Among the PUT bodies, at least TWO must be fresh non-edits
        // (no m.relates_to.rel_type == "m.replace"): the original
        // placeholder + the rolled-over fresh placeholder after head
        // finalization. A no-split implementation produces exactly ONE.
        let bodies_snap = bodies.lock().unwrap().clone();
        let fresh_count = bodies_snap
            .iter()
            .filter(|b| {
                b.get("m.relates_to")
                    .and_then(|r| r.get("rel_type"))
                    .and_then(|s| s.as_str())
                    != Some("m.replace")
            })
            .count();
        assert!(
            fresh_count >= 2,
            "overflow split must produce at least 2 fresh (non-m.replace) PUTs (placeholder + rolled-over), got {fresh_count}"
        );
    }

    #[tokio::test]
    async fn test_send_streaming_429_retry_then_stop() {
        use crate::types::ChannelUser;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        use tokio::sync::mpsc;
        let server = MockServer::start().await;
        let calls = StdArc::new(AtomicUsize::new(0));
        let cc = calls.clone();
        // 1st PUT (placeholder) succeeds; 2nd & 3rd return 429. Streaming should
        // retry the edit once after a brief backoff, then stop on the second 429.
        Mock::given(method("PUT"))
            .respond_with(move |_: &wiremock::Request| {
                let n = cc.fetch_add(1, Ordering::SeqCst);
                match n {
                    0 => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "event_id": "$plc:test"
                    })),
                    _ => ResponseTemplate::new(429)
                        .insert_header("Retry-After", "0")
                        .set_body_string("{\"errcode\":\"M_LIMIT_EXCEEDED\"}"),
                }
            })
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let user = ChannelUser {
            platform_id: "!room:test".to_string(),
            display_name: "u".to_string(),
            librefang_user: None,
        };
        let (tx, rx) = mpsc::channel(2);
        let _ = tx.send("data".to_string()).await;
        drop(tx);
        let res = adapter.send_streaming(&user, rx, None).await;
        assert!(res.is_err(), "second 429 must surface as Err");
        // Expected: 1 placeholder + 1 edit (429) + 1 retry (also 429) = 3 calls.
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "must retry once on 429, then stop"
        );
    }
}
