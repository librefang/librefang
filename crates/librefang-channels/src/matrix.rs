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
const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
// Tightened from 1500ms / 256 chars after the Synapse rate-limit lift
// (rc_message: 5/s, burst 60). The previous values produced "first +
// final only" cadence on typical 2-3s responses (~150 chars/sec). At
// 700ms / 96 chars a 3s response yields ~4-5 visible edits, which
// reads as actual streaming in Element. Telegram's equivalent is
// 1000ms with no char budget; we run hotter because matrix /sync
// resolution is finer.
const STREAM_EDIT_INTERVAL_MS: u64 = 700;
const STREAM_EDIT_CHAR_BUDGET: usize = 96;
/// Maximum number of per-(room, target_event) lifecycle reaction entries to track.
const PHASE_REACTIONS_CAPACITY: usize = 1024;

/// 429 retry backoff envelope.
///
/// Synapse rate-limit replies usually carry `Retry-After` in seconds; we
/// clamp the resulting sleep so a malformed / missing header (→ default)
/// or an overlong cooldown (some homeservers send minutes) doesn't stall
/// the streaming edit loop. Floor stops 0-second hints from turning the
/// retry into a hot loop against the homeserver.
const MIN_RETRY_BACKOFF_MS: u64 = 100;
const MAX_RETRY_BACKOFF_MS: u64 = 5_000;
const DEFAULT_RETRY_BACKOFF_MS: u64 = 500;

/// Insertion-ordered cache mapping (room_id, target_event_id) -> reaction event_id.
type PhaseReactionCache = Arc<RwLock<std::collections::VecDeque<((String, String), String)>>>;

/// Outcome of a single Matrix HTTP call.
///
/// Carries the 429 case as a structured variant so the retry path in
/// `api_edit_event_with_retry` doesn't have to parse error-message
/// strings to recover `Retry-After`. Everything else funnels into
/// `Other` and surfaces to public callers via the `From` impl below
/// as the same boxed trait object they already expect.
#[derive(Debug)]
enum MatrixApiError {
    RateLimited { retry_after_ms: Option<u64> },
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for MatrixApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RateLimited { retry_after_ms } => match retry_after_ms {
                Some(ms) => write!(f, "Matrix rate-limited (429), Retry-After {ms}ms"),
                None => write!(f, "Matrix rate-limited (429)"),
            },
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MatrixApiError {}

impl MatrixApiError {
    /// Erase the typed wrapper into the `Box<dyn Error + Send + Sync>` shape
    /// public callers already expect.
    ///
    /// `Other` is unwrapped (not re-wrapped) so existing call sites that
    /// match on the underlying error-message string keep working — the
    /// typed enum is internal-only, used by the retry path to recover
    /// `Retry-After` without parsing strings. A blanket `From<E: Error>`
    /// impl is supplied by std for any type implementing
    /// `Error + Send + Sync`, so a custom `From<MatrixApiError>` here
    /// would conflict — this named method is the explicit form.
    fn into_boxed(self) -> Box<dyn std::error::Error + Send + Sync> {
        match self {
            Self::Other(inner) => inner,
            rate => Box::new(rate),
        }
    }
}

/// Parse `Retry-After` as a delta-seconds integer. RFC 7231 also allows
/// HTTP-date here but Synapse / Dendrite / Conduit all emit the integer
/// form for `M_LIMIT_EXCEEDED`, and pulling in a date parser for the
/// other shape isn't worth it. `None` → caller uses `DEFAULT_RETRY_BACKOFF_MS`.
fn parse_retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs: u64 = v.trim().parse().ok()?;
    Some(secs.saturating_mul(1000))
}

/// Build the `m.replace` edit body for a target event. Extracted so the
/// retry wrapper can construct the body once and reuse it across both
/// attempts under a single shared txn_id.
fn build_edit_body(target_event_id: &str, new_text: &str) -> serde_json::Value {
    let safe_text = librefang_types::truncate_str(new_text, MAX_MESSAGE_LEN);
    let html = markdown_to_matrix_html(safe_text);
    serde_json::json!({
        "msgtype": "m.text",
        "body": format!("* {safe_text}"),
        "format": "org.matrix.custom.html",
        "formatted_body": format!("* {html}"),
        "m.new_content": {
            "msgtype": "m.text",
            "body": safe_text,
            "format": "org.matrix.custom.html",
            "formatted_body": html,
        },
        "m.relates_to": {
            "rel_type": "m.replace",
            "event_id": target_event_id,
        }
    })
}

/// Render CommonMark `text` into the HTML subset Element/Matrix clients
/// accept for `formatted_body` (per the Matrix spec's "client-server message
/// formatting" appendix). Used for `m.text` `body` + `formatted_body` pairs;
/// `body` keeps the raw markdown so clients that ignore `format` still get a
/// readable fallback.
fn markdown_to_matrix_html(text: &str) -> String {
    use pulldown_cmark::{html, Event, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_TASKLISTS);
    // Demote raw HTML in the source to plain text so an LLM-authored response
    // can't inject `<script>` / `<iframe>` / `<img onerror=...>` into the
    // formatted_body. pulldown-cmark's default is pass-through, which would
    // be a real injection sink given untrusted model output.
    let parser = Parser::new_ext(text, opts).map(|ev| match ev {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        other => other,
    });
    let mut out = String::with_capacity(text.len() + 32);
    html::push_html(&mut out, parser);
    out
}

/// Build a JSON `m.text` content body with both `body` (raw markdown for
/// fallback) and `formatted_body` (rendered HTML). Optional `extra` is merged
/// into the resulting object — used to attach `m.relates_to` / `m.new_content`.
fn text_body_with_html(raw: &str, extra: Option<serde_json::Value>) -> serde_json::Value {
    let mut v = serde_json::json!({
        "msgtype": "m.text",
        "body": raw,
        "format": "org.matrix.custom.html",
        "formatted_body": markdown_to_matrix_html(raw),
    });
    if let (Some(serde_json::Value::Object(extras)), Some(obj)) = (extra, v.as_object_mut()) {
        for (k, val) in extras {
            obj.insert(k, val);
        }
    }
    v
}

/// Convert mxc://server/mediaId -> an HTTPS download URL.
///
/// Emits the MSC3916 authenticated endpoint
/// (`/_matrix/client/v1/media/download/{server}/{mediaId}`). Modern
/// Synapse (≥1.100, default since ~1.120) freezes the legacy
/// unauthenticated `/_matrix/media/v3/download` path and returns
/// `404 M_NOT_FOUND` on it — see `MatrixAdapter::fetch_headers_for`,
/// which supplies the bot's `Authorization: Bearer <access_token>`
/// when the bridge fetches this URL.
pub(crate) fn mxc_to_http(mxc: &str, homeserver_url: &str) -> Option<String> {
    let stripped = mxc.strip_prefix("mxc://")?;
    let (server, media_id) = stripped.split_once('/')?;
    if server.is_empty() || media_id.is_empty() {
        return None;
    }
    Some(format!(
        "{homeserver_url}/_matrix/client/v1/media/download/{server}/{media_id}"
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

/// Parse an `m.image` event content into ChannelContent::Image.
pub(crate) fn parse_media_image(
    c: &serde_json::Value,
    hs: &str,
) -> Option<crate::types::ChannelContent> {
    let mxc = c.get("url")?.as_str()?;
    let url = mxc_to_http(mxc, hs)?;
    let mime_type = c
        .get("info")
        .and_then(|i| i.get("mimetype"))
        .and_then(|m| m.as_str())
        .map(String::from);
    let caption = c.get("body").and_then(|b| b.as_str()).map(String::from);
    Some(crate::types::ChannelContent::Image {
        url,
        caption,
        mime_type,
    })
}

/// Parse an `m.file` event content into ChannelContent::File.
/// Matrix v1.10+ adds a `filename` field; if present, it wins over `body`.
pub(crate) fn parse_media_file(
    c: &serde_json::Value,
    hs: &str,
) -> Option<crate::types::ChannelContent> {
    let mxc = c.get("url")?.as_str()?;
    let url = mxc_to_http(mxc, hs)?;
    let filename = c
        .get("filename")
        .and_then(|f| f.as_str())
        .or_else(|| c.get("body").and_then(|b| b.as_str()))
        .unwrap_or("file")
        .to_string();
    Some(crate::types::ChannelContent::File { url, filename })
}

/// Parse an `m.audio` event content. Voice messages (msc3245.voice marker)
/// promote to ChannelContent::Voice; everything else is ChannelContent::Audio.
pub(crate) fn parse_media_audio(
    c: &serde_json::Value,
    hs: &str,
) -> Option<crate::types::ChannelContent> {
    let mxc = c.get("url")?.as_str()?;
    let url = mxc_to_http(mxc, hs)?;
    let caption = c.get("body").and_then(|b| b.as_str()).map(String::from);
    let duration_ms = c
        .get("info")
        .and_then(|i| i.get("duration"))
        .and_then(|d| d.as_u64())
        .unwrap_or(0);
    let duration_seconds = (duration_ms / 1000) as u32;
    if c.get("org.matrix.msc3245.voice").is_some() {
        Some(crate::types::ChannelContent::Voice {
            url,
            caption,
            duration_seconds,
        })
    } else {
        Some(crate::types::ChannelContent::Audio {
            url,
            caption,
            duration_seconds,
            title: None,
            performer: None,
        })
    }
}

/// Parse an `m.video` event content into ChannelContent::Video.
pub(crate) fn parse_media_video(
    c: &serde_json::Value,
    hs: &str,
) -> Option<crate::types::ChannelContent> {
    let mxc = c.get("url")?.as_str()?;
    let url = mxc_to_http(mxc, hs)?;
    let caption = c.get("body").and_then(|b| b.as_str()).map(String::from);
    let duration_ms = c
        .get("info")
        .and_then(|i| i.get("duration"))
        .and_then(|d| d.as_u64())
        .unwrap_or(0);
    let duration_seconds = (duration_ms / 1000) as u32;
    let filename = c.get("body").and_then(|b| b.as_str()).map(String::from);
    Some(crate::types::ChannelContent::Video {
        url,
        caption,
        duration_seconds,
        filename,
    })
}

/// Dispatch helper: return ChannelContent for a content blob based on msgtype.
/// Returns None for empty bodies, malformed content, or unhandled msgtypes.
pub(crate) fn parse_inbound_msg_content(
    content: &serde_json::Value,
    hs: &str,
) -> Option<crate::types::ChannelContent> {
    let msgtype = content
        .get("msgtype")
        .and_then(|m| m.as_str())
        .unwrap_or("m.text");
    match msgtype {
        "m.text" | "m.notice" | "m.emote" => {
            let body = content.get("body").and_then(|b| b.as_str())?;
            if body.is_empty() {
                return None;
            }
            if body.starts_with('/') {
                let parts: Vec<&str> = body.splitn(2, ' ').collect();
                let cmd = parts[0].trim_start_matches('/').to_string();
                let args: Vec<String> = parts
                    .get(1)
                    .map(|a| a.split_whitespace().map(String::from).collect())
                    .unwrap_or_default();
                Some(crate::types::ChannelContent::Command { name: cmd, args })
            } else {
                Some(crate::types::ChannelContent::Text(body.to_string()))
            }
        }
        "m.image" => parse_media_image(content, hs),
        "m.file" => parse_media_file(content, hs),
        "m.audio" => parse_media_audio(content, hs),
        "m.video" => parse_media_video(content, hs),
        _ => None,
    }
}

/// Convert a `MediaGroupItem` to the corresponding `ChannelContent` variant
/// so that `MediaGroup` handling can recurse into `send()` for each item.
fn media_group_item_to_channel_content(
    item: crate::types::MediaGroupItem,
) -> crate::types::ChannelContent {
    match item {
        crate::types::MediaGroupItem::Photo { url, caption } => {
            crate::types::ChannelContent::Image {
                url,
                caption,
                mime_type: None,
            }
        }
        crate::types::MediaGroupItem::Video {
            url,
            caption,
            duration_seconds,
        } => crate::types::ChannelContent::Video {
            url,
            caption,
            duration_seconds,
            filename: None,
        },
    }
}

/// Fetch bytes from a URL for outbound media upload.
///
/// In production: goes through [`crate::http_client::fetch_url_bytes`] which
/// applies the SSRF guard + redirect re-validation before any I/O.
///
/// In tests: bypasses the SSRF guard via
/// [`crate::http_client::fetch_url_bytes_unchecked`] so wiremock servers on
/// `127.0.0.1` work without a resolving SSRF proxy.
async fn fetch_outbound_media(
    url: &str,
    max_bytes: usize,
) -> Result<(Vec<u8>, Option<String>), Box<dyn std::error::Error + Send + Sync>> {
    #[cfg(not(test))]
    {
        Ok(crate::http_client::fetch_url_bytes(url, max_bytes, &[]).await?)
    }
    #[cfg(test)]
    {
        Ok(crate::http_client::fetch_url_bytes_unchecked(
            crate::http_client::safe_fetch_client(),
            url,
            max_bytes,
            &[],
        )
        .await?)
    }
}

/// Render `text` followed by `[Label]` hints for each button row.
/// Used for outbound EditInteractive / Interactive on Matrix, which has
/// no native interactive button support — text suffix is the standard fallback.
fn format_with_button_hints(
    text: &str,
    buttons: &[Vec<crate::types::InteractiveButton>],
) -> String {
    if buttons.is_empty() {
        return text.to_string();
    }
    let mut out = String::from(text);
    for (i, row) in buttons.iter().enumerate() {
        // Skip the leading newline only when the body is empty and this is
        // the first row — otherwise every row starts on its own line.
        if i > 0 || !out.is_empty() {
            out.push('\n');
        }
        for btn in row {
            out.push_str(&format!("[{}] ", btn.label));
        }
    }
    out.trim_end().to_string()
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
    /// Rooms we've already warned about being E2EE.
    /// First encrypted event in each room emits a WARN; subsequent ones are silent.
    pub(crate) e2ee_warned_rooms: Arc<RwLock<std::collections::HashSet<String>>>,
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
            e2ee_warned_rooms: Arc::new(RwLock::new(std::collections::HashSet::new())),
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

    /// Send a client event with a caller-supplied `txn_id`.
    ///
    /// Used by both `api_send_event` (fresh txn_id per call) and
    /// `api_edit_event_with_retry` (same txn_id across both attempts so
    /// Matrix's (sender, txn_id) idempotency turns a delayed-success
    /// race into a server-side no-op).
    ///
    /// Returns the structured `MatrixApiError` so retry callers can
    /// distinguish 429 (with optional `Retry-After`) from terminal
    /// errors without parsing error-message strings.
    async fn send_event_inner(
        &self,
        room_id: &str,
        event_type: &str,
        txn_id: &str,
        body: &serde_json::Value,
    ) -> Result<String, MatrixApiError> {
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
            .await
            .map_err(|e| MatrixApiError::Other(Box::new(e)))?;
        let status = resp.status();
        if status.as_u16() == 429 {
            return Err(MatrixApiError::RateLimited {
                retry_after_ms: parse_retry_after_ms(resp.headers()),
            });
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(MatrixApiError::Other(
                format!("Matrix {event_type} failed {status}: {text}").into(),
            ));
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| MatrixApiError::Other(Box::new(e)))?;
        v["event_id"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| MatrixApiError::Other("Matrix response missing event_id".into()))
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
        self.send_event_inner(room_id, event_type, &txn_id, body)
            .await
            .map_err(MatrixApiError::into_boxed)
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
    ///
    /// Test-only — production code reads `phase_reactions` directly inside
    /// `send_reaction` via the remove/insert helpers below.
    #[cfg(test)]
    async fn phase_reaction_lookup(&self, key: &(String, String)) -> Option<String> {
        self.phase_reactions
            .read()
            .await
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }

    /// Remove (room, target). Returns the previous reaction id if present. O(n).
    async fn phase_reaction_remove(&self, key: &(String, String)) -> Option<String> {
        let mut w = self.phase_reactions.write().await;
        if let Some(pos) = w.iter().position(|(k, _)| k == key) {
            w.remove(pos).map(|(_, v)| v)
        } else {
            None
        }
    }

    /// Insert (room, target) -> reaction_id, evicting oldest if at capacity.
    /// If the key already exists, the value is replaced in place (preserving position).
    async fn phase_reaction_insert(&self, key: (String, String), reaction_id: String) {
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

    /// Returns true the first time this room is observed as E2EE.
    /// Caller should emit a `warn!` log when it returns true.
    ///
    /// Test-only — the production /sync loop manipulates the
    /// `e2ee_warned_rooms` set directly so it can `drop(w)` before the
    /// `warn!` macro fires without holding the write lock across the log.
    #[cfg(test)]
    async fn warn_e2ee_once_check(&self, room_id: &str) -> bool {
        let mut w = self.e2ee_warned_rooms.write().await;
        w.insert(room_id.to_string())
    }

    /// Upload bytes to Matrix media repo. Returns mxc:// URI.
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
    ///
    /// Inputs longer than `MAX_MESSAGE_LEN` are UTF-8-safely truncated
    /// inside `build_edit_body`. An edit can only target one event_id, so
    /// unlike `api_send_message` we cannot split into multiple events
    /// here — callers that need to preserve every byte (streaming
    /// overflow) must split BEFORE calling. This cap is defensive: it
    /// stops `send(EditInteractive)` / `send(DeleteMessage)` paths from
    /// producing an oversized `m.room.message` that Synapse would reject
    /// with a hard-to-debug 413 / `M_TOO_LARGE`.
    async fn api_edit_event(
        &self,
        room_id: &str,
        target_event_id: &str,
        new_text: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = build_edit_body(target_event_id, new_text);
        self.api_send_event(room_id, "m.room.message", &body).await
    }

    /// `api_edit_event` with one `Retry-After`-aware retry on 429.
    ///
    /// Mints ONE `txn_id` and threads it through both attempts so that
    /// Matrix's `(sender, txn_id)` idempotency contract (server-side
    /// dedup) protects against the delayed-success-then-retry race —
    /// without this, a 429 that masks a quietly-successful first PUT
    /// would land a duplicate `m.replace` event in the room.
    ///
    /// The retry-after value comes from the structured `RateLimited`
    /// variant on `MatrixApiError` (no string-parsing of the boxed
    /// error). Sleep is clamped to `[MIN_RETRY_BACKOFF_MS,
    /// MAX_RETRY_BACKOFF_MS]` so a missing / zero / overlong header
    /// doesn't either spam the homeserver or stall streaming.
    async fn api_edit_event_with_retry(
        &self,
        room_id: &str,
        target_event_id: &str,
        new_text: &str,
    ) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let body = build_edit_body(target_event_id, new_text);
        let txn_id = uuid::Uuid::new_v4().to_string();
        match self
            .send_event_inner(room_id, "m.room.message", &txn_id, &body)
            .await
        {
            Ok(id) => Ok(id),
            Err(MatrixApiError::RateLimited { retry_after_ms }) => {
                let sleep_ms = retry_after_ms
                    .unwrap_or(DEFAULT_RETRY_BACKOFF_MS)
                    .clamp(MIN_RETRY_BACKOFF_MS, MAX_RETRY_BACKOFF_MS);
                tokio::time::sleep(Duration::from_millis(sleep_ms)).await;
                self.send_event_inner(room_id, "m.room.message", &txn_id, &body)
                    .await
                    .map_err(MatrixApiError::into_boxed)
            }
            Err(other) => Err(other.into_boxed()),
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
            let body = text_body_with_html(chunk, None);
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

    /// Authenticated-media (MSC3916) requires the bot's access token on
    /// every download. Only attach it when the URL points at THIS bot's
    /// own homeserver — never to a model-controlled hostname, since the
    /// token grants full account access (send messages, leave rooms,
    /// read DMs).
    fn fetch_headers_for(&self, url: &str) -> Vec<(String, String)> {
        let url_host = match url::Url::parse(url) {
            Ok(u) => u.host_str().map(|s| s.to_ascii_lowercase()),
            Err(_) => return Vec::new(),
        };
        let hs_host = url::Url::parse(&self.homeserver_url)
            .ok()
            .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()));
        match (url_host, hs_host) {
            (Some(u), Some(hs)) if u == hs => vec![(
                "Authorization".to_string(),
                format!("Bearer {}", &*self.access_token),
            )],
            _ => Vec::new(),
        }
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
        let e2ee_warned_rooms = Arc::clone(&self.e2ee_warned_rooms);
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
                                if event_type == "m.room.encrypted" {
                                    let mut w = e2ee_warned_rooms.write().await;
                                    if w.insert(room_id.clone()) {
                                        drop(w);
                                        warn!("Matrix room {room_id} is E2EE; encrypted events ignored (E2EE not yet supported)");
                                    }
                                    continue;
                                }
                                if event_type != "m.room.message" {
                                    continue;
                                }

                                let sender = event["sender"].as_str().unwrap_or("");
                                if sender == user_id {
                                    continue; // Skip own messages
                                }

                                let msg_content =
                                    match parse_inbound_msg_content(&event["content"], &homeserver)
                                    {
                                        Some(c) => c,
                                        None => continue,
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
            ChannelContent::DeleteMessage { message_id } => {
                self.api_redact(&user.platform_id, &message_id, None)
                    .await?;
            }
            ChannelContent::EditInteractive {
                message_id,
                text,
                buttons,
            } => {
                let combined = format_with_button_hints(&text, &buttons);
                self.api_edit_event(&user.platform_id, &message_id, &combined)
                    .await?;
            }
            ChannelContent::Image {
                url,
                caption,
                mime_type,
            } => {
                let (bytes, ct) = fetch_outbound_media(&url, MAX_UPLOAD_BYTES).await?;
                let mt = mime_type
                    .clone()
                    .or(ct)
                    .unwrap_or_else(|| "application/octet-stream".to_string());
                let fname = caption.clone().unwrap_or_else(|| "image".to_string());
                let size = bytes.len();
                let mxc = self.api_upload_media(bytes, &fname, &mt).await?;
                let body = serde_json::json!({
                    "msgtype": "m.image",
                    "body": caption.unwrap_or(fname.clone()),
                    "filename": fname,
                    "url": mxc,
                    "info": { "mimetype": mt, "size": size },
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::File { url, filename } => {
                let (bytes, ct) = fetch_outbound_media(&url, MAX_UPLOAD_BYTES).await?;
                let mt = ct.unwrap_or_else(|| "application/octet-stream".to_string());
                let size = bytes.len();
                let mxc = self.api_upload_media(bytes, &filename, &mt).await?;
                let body = serde_json::json!({
                    "msgtype": "m.file",
                    "body": filename.clone(),
                    "filename": filename,
                    "url": mxc,
                    "info": { "mimetype": mt, "size": size },
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::FileData {
                data,
                filename,
                mime_type,
            } => {
                let size = data.len();
                let mxc = self.api_upload_media(data, &filename, &mime_type).await?;
                let body = serde_json::json!({
                    "msgtype": "m.file",
                    "body": filename.clone(),
                    "filename": filename,
                    "url": mxc,
                    "info": { "mimetype": mime_type, "size": size },
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::Audio {
                url,
                caption,
                duration_seconds,
                ..
            } => {
                let (bytes, ct) = fetch_outbound_media(&url, MAX_UPLOAD_BYTES).await?;
                let mt = ct.unwrap_or_else(|| "application/octet-stream".to_string());
                let fname = caption.clone().unwrap_or_else(|| "audio".to_string());
                let size = bytes.len();
                let mxc = self.api_upload_media(bytes, &fname, &mt).await?;
                let body = serde_json::json!({
                    "msgtype": "m.audio",
                    "body": caption.clone().unwrap_or(fname.clone()),
                    "filename": fname,
                    "url": mxc,
                    "info": {
                        "mimetype": mt,
                        "size": size,
                        "duration": (duration_seconds as u64) * 1000,
                    },
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::Voice {
                url,
                caption,
                duration_seconds,
            } => {
                let (bytes, ct) = fetch_outbound_media(&url, MAX_UPLOAD_BYTES).await?;
                let mt = ct.unwrap_or_else(|| "application/octet-stream".to_string());
                let fname = caption.clone().unwrap_or_else(|| "voice".to_string());
                let size = bytes.len();
                let mxc = self.api_upload_media(bytes, &fname, &mt).await?;
                let body = serde_json::json!({
                    "msgtype": "m.audio",
                    "body": caption.clone().unwrap_or(fname.clone()),
                    "filename": fname,
                    "url": mxc,
                    "info": {
                        "mimetype": mt,
                        "size": size,
                        "duration": (duration_seconds as u64) * 1000,
                    },
                    "org.matrix.msc3245.voice": {},
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::Video {
                url,
                caption,
                duration_seconds,
                filename,
            } => {
                let (bytes, ct) = fetch_outbound_media(&url, MAX_UPLOAD_BYTES).await?;
                let mt = ct.unwrap_or_else(|| "application/octet-stream".to_string());
                let fname = filename
                    .unwrap_or_else(|| caption.clone().unwrap_or_else(|| "video".to_string()));
                let size = bytes.len();
                let mxc = self.api_upload_media(bytes, &fname, &mt).await?;
                let body = serde_json::json!({
                    "msgtype": "m.video",
                    "body": caption.unwrap_or(fname.clone()),
                    "filename": fname,
                    "url": mxc,
                    "info": {
                        "mimetype": mt,
                        "size": size,
                        "duration": (duration_seconds as u64) * 1000,
                    },
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::Animation {
                url,
                caption,
                duration_seconds: _,
            } => {
                let (bytes, ct) = fetch_outbound_media(&url, MAX_UPLOAD_BYTES).await?;
                let mt = ct.unwrap_or_else(|| "application/octet-stream".to_string());
                let fname = caption.clone().unwrap_or_else(|| "animation".to_string());
                let size = bytes.len();
                let mxc = self.api_upload_media(bytes, &fname, &mt).await?;
                let body = serde_json::json!({
                    "msgtype": "m.image",
                    "body": caption.clone().unwrap_or(fname.clone()),
                    "filename": fname,
                    "url": mxc,
                    "info": { "mimetype": mt, "size": size },
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::Sticker { file_id } => {
                self.api_send_message(&user.platform_id, &format!("(sticker: {file_id})"))
                    .await?;
            }
            ChannelContent::MediaGroup { items } => {
                for item in items {
                    let cc: ChannelContent = media_group_item_to_channel_content(item);
                    Box::pin(self.send(user, cc)).await?;
                }
            }
            ChannelContent::Location { lat, lon } => {
                let body = serde_json::json!({
                    "msgtype": "m.location",
                    "body": format!("Location {lat},{lon}"),
                    "geo_uri": format!("geo:{lat},{lon}"),
                });
                self.api_send_event(&user.platform_id, "m.room.message", &body)
                    .await?;
            }
            ChannelContent::ButtonCallback { action, .. } => {
                debug!(
                    "Matrix: ButtonCallback (action={action}) outbound is unsupported, skipping"
                );
            }
            ChannelContent::Poll { .. } | ChannelContent::PollAnswer { .. } => {
                self.api_send_message(&user.platform_id, "(poll unsupported)")
                    .await?;
            }
            ChannelContent::Interactive { text, buttons } => {
                let combined = format_with_button_hints(&text, &buttons);
                self.api_send_message(&user.platform_id, &combined).await?;
            }
            ChannelContent::Command { name, args: _ } => {
                debug!(
                    "Matrix: outbound Command (name={name}) is a no-op (Command is inbound-only)"
                );
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
                    let extras = serde_json::json!({
                        "m.relates_to": {
                            "rel_type": "m.thread",
                            "event_id": thread_id,
                            "is_falling_back": true,
                            "m.in_reply_to": { "event_id": thread_id },
                        }
                    });
                    let body = text_body_with_html(chunk, Some(extras));
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
            Some(tid) => text_body_with_html(
                "…",
                Some(serde_json::json!({
                    "m.relates_to": {
                        "rel_type": "m.thread",
                        "event_id": tid,
                        "is_falling_back": true,
                        "m.in_reply_to": { "event_id": tid },
                    }
                })),
            ),
            None => text_body_with_html("…", None),
        };
        let mut placeholder_id = self
            .api_send_event(&user.platform_id, "m.room.message", &placeholder_body)
            .await?;

        let mut buffer = String::new();
        let mut last_flushed_len: usize = 0;
        let mut last_edit = std::time::Instant::now();
        let interval = Duration::from_millis(STREAM_EDIT_INTERVAL_MS);
        // First-delta policy: flush as soon as ANY content arrives so the
        // placeholder `…` is replaced with real text quickly, even when
        // the deltas are short. Without this, tool-progress markers like
        // `\n\n🔧 tool_name\n\n` (~35 chars) never reach the 256-char
        // budget and the user sits on `…` for the full 1500ms — and for
        // tool-only sequences (rapid tool calls with no LLM prose
        // between them) the buffer-on-recv check never re-fires the
        // timer, so EVERY tool marker is invisible until the final
        // drain. Telegram already does this implicitly by sending the
        // first delta as the initial message body (telegram.rs:2214).
        let mut flushed_initial = false;

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
            let body = text_body_with_html(&tail, None);
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
            // Flush on the very first non-empty buffer so the placeholder
            // is replaced immediately; afterwards, fall back to the
            // time/size debounce for the rest of the stream.
            let force_first_flush = !flushed_initial && !buffer.is_empty();
            if force_first_flush
                || elapsed >= interval
                || added >= STREAM_EDIT_CHAR_BUDGET
                || buffer.len() > MAX_MESSAGE_LEN
            {
                last_flushed_len =
                    flush(self, &user.platform_id, &mut placeholder_id, &mut buffer).await?;
                last_edit = std::time::Instant::now();
                flushed_initial = true;
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

    #[test]
    fn test_markdown_to_matrix_html_renders_common_subset() {
        let html = markdown_to_matrix_html("**bold** and *italic* and `code`");
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<em>italic</em>"));
        assert!(html.contains("<code>code</code>"));

        let h = markdown_to_matrix_html("# h1\n## h2");
        assert!(h.contains("<h1>h1</h1>"));
        assert!(h.contains("<h2>h2</h2>"));

        let l = markdown_to_matrix_html("- one\n- two");
        assert!(l.contains("<ul>") && l.contains("<li>one</li>") && l.contains("<li>two</li>"));

        let link = markdown_to_matrix_html("[lf](https://example.org)");
        assert!(link.contains("<a href=\"https://example.org\">lf</a>"));

        // Tables (GFM) — enabled via Options::ENABLE_TABLES.
        let t = markdown_to_matrix_html("| a | b |\n|---|---|\n| 1 | 2 |");
        assert!(t.contains("<table>") && t.contains("<td>1</td>"));

        // HTML escapes — bare angle brackets in the source must NOT inject raw tags.
        let e = markdown_to_matrix_html("plain <script>alert(1)</script> text");
        assert!(!e.contains("<script>"), "must escape script tag, got: {e}");
    }

    #[test]
    fn test_text_body_with_html_includes_format_and_merges_extras() {
        let v = text_body_with_html(
            "**bold**",
            Some(serde_json::json!({"m.relates_to": {"rel_type": "m.replace", "event_id": "$x"}})),
        );
        assert_eq!(v["msgtype"], "m.text");
        assert_eq!(v["body"], "**bold**");
        assert_eq!(v["format"], "org.matrix.custom.html");
        assert!(v["formatted_body"]
            .as_str()
            .unwrap()
            .contains("<strong>bold</strong>"));
        assert_eq!(v["m.relates_to"]["rel_type"], "m.replace");
        assert_eq!(v["m.relates_to"]["event_id"], "$x");
    }

    #[test]
    fn test_format_with_button_hints_multi_row_each_on_own_line() {
        let btn = |label: &str| crate::types::InteractiveButton {
            label: label.to_string(),
            action: label.to_string(),
            style: None,
            url: None,
        };
        // Two rows: the second row must appear on its own line, not appended
        // to the first row's trailing space.
        let result = format_with_button_hints(
            "Pick one:",
            &[vec![btn("Yes"), btn("No")], vec![btn("Cancel")]],
        );
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 3, "expected 3 lines, got: {result:?}");
        assert_eq!(lines[0], "Pick one:");
        assert!(lines[1].contains("[Yes]") && lines[1].contains("[No]"));
        assert!(
            lines[2].contains("[Cancel]"),
            "row 2 must be on its own line, got: {result:?}"
        );

        // Empty body: no leading blank line before first row.
        let result_empty = format_with_button_hints("", &[vec![btn("A")], vec![btn("B")]]);
        assert!(
            !result_empty.starts_with('\n'),
            "empty body must not produce a leading newline, got: {result_empty:?}"
        );
        let empty_lines: Vec<&str> = result_empty.lines().collect();
        assert_eq!(
            empty_lines.len(),
            2,
            "expected 2 lines, got: {result_empty:?}"
        );
        assert!(
            empty_lines[1].contains("[B]"),
            "second row must be on its own line"
        );
    }

    #[tokio::test]
    async fn test_e2ee_event_emits_warn_once_per_room() {
        let adapter = make_adapter("http://unused".to_string());
        // First call: room not yet in set, returns true (caller should warn).
        let r1a = adapter.warn_e2ee_once_check("!room1:test").await;
        let r1b = adapter.warn_e2ee_once_check("!room1:test").await;
        let r2 = adapter.warn_e2ee_once_check("!room2:test").await;
        assert!(r1a, "first observation in room1 must signal warn");
        assert!(!r1b, "second observation in room1 must be silent");
        assert!(r2, "first observation in a new room must warn");
    }

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
                "format": "org.matrix.custom.html",
                "formatted_body": "<p>hello matrix</p>\n",
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
            "https://hs.example.com/_matrix/client/v1/media/download/srv/abc"
        );
    }

    #[test]
    fn test_mxc_to_http_rejects_non_mxc() {
        assert!(mxc_to_http("https://other.example/x", "https://hs").is_none());
        assert!(mxc_to_http("mxc://no-slash", "https://hs").is_none());
        assert!(mxc_to_http("", "https://hs").is_none());
    }

    /// Auth header is attached to URLs that point at the bot's own
    /// homeserver, and ONLY to those — leaking the access token to a
    /// model-controlled host would let a forged inbound message
    /// exfiltrate full account access. Host comparison is
    /// case-insensitive so `Matrix.Example.com` matches
    /// `matrix.example.com`.
    #[test]
    fn test_fetch_headers_for_only_attaches_to_homeserver() {
        let adapter = make_adapter("https://matrix.example.com".to_string());

        let h = adapter.fetch_headers_for(
            "https://matrix.example.com/_matrix/client/v1/media/download/srv/abc",
        );
        assert_eq!(h.len(), 1);
        assert_eq!(h[0].0, "Authorization");
        assert_eq!(h[0].1, "Bearer secret-access-token");

        // Different host (e.g. an attacker-supplied URL): no headers.
        let h = adapter
            .fetch_headers_for("https://attacker.example/_matrix/client/v1/media/download/x/y");
        assert!(h.is_empty(), "must not leak token to other hosts: {h:?}");

        // Case-insensitive host match.
        let h = adapter.fetch_headers_for(
            "https://Matrix.Example.COM/_matrix/client/v1/media/download/srv/abc",
        );
        assert_eq!(h.len(), 1);

        // Garbage URL → no headers, no panic.
        assert!(adapter.fetch_headers_for("not-a-url").is_empty());
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
        // Placeholder + first-delta flush + final edit. The first non-empty
        // buffer always flushes immediately so the `…` placeholder is
        // replaced with real content fast (parity with telegram's
        // first-token send, fixes the tool-progress streaming gap where
        // short markers like `\n\n🔧 X\n\n` never reach the char budget).
        // The remaining 9 deltas of "dN" total ~18 chars, well under
        // STREAM_EDIT_CHAR_BUDGET (96) and the 700ms interval, so they
        // fold into the single final edit. → exactly 3 PUTs:
        // placeholder + first-flush + final.
        assert!(calls >= 3, "expected at least 3 PUTs, got {calls}");
        assert!(
            calls <= 3,
            "expected at most 3 PUTs (debounce should suppress all but first + final mid-stream edits), got {calls}"
        );
    }

    /// Regression for the tool-progress streaming gap. The kernel's
    /// streaming bridge interleaves `\n\n🔧 toolname\n\n` markers
    /// (~30-40 chars each) into the delta channel for every tool
    /// invocation. Before the first-delta-flushes-immediately change,
    /// rapid tool-only sequences (no LLM prose between calls) accumulated
    /// markers below the 256-char budget AND fired all checks back to
    /// back without crossing the 1500ms wall-clock threshold, so the
    /// placeholder `…` was the ONLY thing the user saw until the agent
    /// loop terminated and the final drain fired. Pin the new behaviour:
    /// the very first marker must reach the placeholder edit before the
    /// stream ends.
    #[tokio::test]
    async fn test_send_streaming_flushes_first_tool_marker() {
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
        let (tx, rx) = mpsc::channel(8);
        // Send a single short marker, then immediately close. Total
        // accumulated chars ≪ 256, total wall-clock ≪ 1500ms — the only
        // thing that can cause a mid-stream flush is the
        // first-delta-flushes-immediately rule.
        let sender = tokio::spawn(async move {
            let _ = tx
                .send("\n\n🔧 mcp_filesystem_read_file\n\n".to_string())
                .await;
            drop(tx);
        });
        adapter
            .send_streaming(&user, rx, None)
            .await
            .expect("streaming must succeed");
        sender.await.unwrap();
        let calls = send_calls.load(Ordering::SeqCst);
        // Placeholder + first-flush (with marker visible) + final
        // (idempotent, same content) = 3 PUTs minimum. The point is
        // calls > 2 — i.e. SOMETHING beyond the placeholder + end-drain
        // has happened.
        assert!(
            calls >= 3,
            "expected ≥3 PUTs so the tool marker reaches the user mid-stream; got {calls}"
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

    #[tokio::test]
    async fn test_send_delete_message_calls_redact() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        let server = MockServer::start().await;
        let calls = StdArc::new(AtomicUsize::new(0));
        let cc = calls.clone();
        Mock::given(method("PUT"))
            .and(path_regex(r"^/_matrix/client/v3/rooms/.+/redact/.+$"))
            .respond_with(move |_: &wiremock::Request| {
                cc.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event_id": "$rdct:test"
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
        adapter
            .send(
                &user,
                ChannelContent::DeleteMessage {
                    message_id: "$victim:test".to_string(),
                },
            )
            .await
            .expect("delete must succeed");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "expected one redact call");
    }

    #[tokio::test]
    async fn test_send_edit_interactive_uses_replace() {
        use crate::types::{ChannelContent, ChannelUser, InteractiveButton};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .and(body_partial_json(serde_json::json!({
                "m.relates_to": { "rel_type": "m.replace", "event_id": "$orig:test" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$edit:test"
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
            .send(
                &user,
                ChannelContent::EditInteractive {
                    message_id: "$orig:test".to_string(),
                    text: "Choose:".to_string(),
                    buttons: vec![vec![
                        InteractiveButton {
                            label: "Yes".to_string(),
                            action: "yes".to_string(),
                            style: None,
                            url: None,
                        },
                        InteractiveButton {
                            label: "No".to_string(),
                            action: "no".to_string(),
                            style: None,
                            url: None,
                        },
                    ]],
                },
            )
            .await
            .expect("edit interactive must succeed");
    }

    #[test]
    fn test_inbound_image_event() {
        let content = serde_json::json!({
            "msgtype": "m.image",
            "body": "screenshot.png",
            "url": "mxc://srv/img1",
            "info": { "mimetype": "image/png", "size": 1234 }
        });
        let cc = parse_media_image(&content, "https://hs.example").expect("must parse");
        match cc {
            crate::types::ChannelContent::Image {
                url,
                caption,
                mime_type,
            } => {
                assert_eq!(
                    url,
                    "https://hs.example/_matrix/client/v1/media/download/srv/img1"
                );
                assert_eq!(caption, Some("screenshot.png".to_string()));
                assert_eq!(mime_type, Some("image/png".to_string()));
            }
            _ => panic!("expected Image variant"),
        }
    }

    #[test]
    fn test_inbound_file_event() {
        // body field as filename fallback (no separate `filename` field).
        let content = serde_json::json!({
            "msgtype": "m.file",
            "body": "report.pdf",
            "url": "mxc://srv/file1",
            "info": { "mimetype": "application/pdf", "size": 9000 }
        });
        let cc = parse_media_file(&content, "https://hs.example").expect("must parse");
        match cc {
            crate::types::ChannelContent::File { url, filename } => {
                assert_eq!(
                    url,
                    "https://hs.example/_matrix/client/v1/media/download/srv/file1"
                );
                assert_eq!(filename, "report.pdf");
            }
            _ => panic!("expected File variant"),
        }

        // Matrix v1.10+: `filename` field takes precedence over `body`.
        let v110 = serde_json::json!({
            "msgtype": "m.file",
            "body": "Caption text",
            "filename": "actual_name.pdf",
            "url": "mxc://srv/file2",
            "info": { "mimetype": "application/pdf" }
        });
        let cc = parse_media_file(&v110, "https://hs.example").expect("must parse");
        match cc {
            crate::types::ChannelContent::File { filename, .. } => {
                assert_eq!(
                    filename, "actual_name.pdf",
                    "v1.10 filename field should win"
                );
            }
            _ => panic!("expected File variant"),
        }
    }

    #[test]
    fn test_inbound_audio_event() {
        let content = serde_json::json!({
            "msgtype": "m.audio",
            "body": "song.mp3",
            "url": "mxc://srv/audio1",
            "info": { "mimetype": "audio/mpeg", "duration": 65000 }
        });
        let cc = parse_media_audio(&content, "https://hs.example").expect("must parse");
        match cc {
            crate::types::ChannelContent::Audio {
                duration_seconds, ..
            } => {
                assert_eq!(duration_seconds, 65, "ms should convert to seconds");
            }
            _ => panic!("expected Audio variant"),
        }

        // Voice marker promotes to Voice.
        let voice = serde_json::json!({
            "msgtype": "m.audio",
            "body": "voice.ogg",
            "url": "mxc://srv/voice1",
            "info": { "mimetype": "audio/ogg", "duration": 5000 },
            "org.matrix.msc3245.voice": {}
        });
        let cc = parse_media_audio(&voice, "https://hs.example").expect("must parse");
        assert!(matches!(cc, crate::types::ChannelContent::Voice { .. }));
    }

    #[test]
    fn test_inbound_video_event() {
        let content = serde_json::json!({
            "msgtype": "m.video",
            "body": "clip.mp4",
            "url": "mxc://srv/vid1",
            "info": { "mimetype": "video/mp4", "duration": 12000 }
        });
        let cc = parse_media_video(&content, "https://hs.example").expect("must parse");
        match cc {
            crate::types::ChannelContent::Video {
                duration_seconds,
                filename,
                ..
            } => {
                assert_eq!(duration_seconds, 12);
                assert_eq!(filename, Some("clip.mp4".to_string()));
            }
            _ => panic!("expected Video variant"),
        }
    }

    #[test]
    fn test_inbound_unknown_msgtype_skipped() {
        let content = serde_json::json!({"msgtype":"m.foo","body":"unknown"});
        assert!(parse_inbound_msg_content(&content, "https://hs.example").is_none());
    }

    // ── Task 17: Outbound Image ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_outbound_image_event_shape() {
        use crate::types::{ChannelContent, ChannelUser};
        let server = MockServer::start().await;
        let bytes_url = format!("{}/dummy.png", server.uri());
        Mock::given(method("GET"))
            .and(wiremock::matchers::path("/dummy.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"\x89PNG\r\n\x1a\n".to_vec())
                    .insert_header("Content-Type", "image/png"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::path("/_matrix/media/v3/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content_uri": "mxc://srv/img1"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(body_partial_json(serde_json::json!({
                "msgtype": "m.image",
                "url": "mxc://srv/img1",
                "info": { "mimetype": "image/png" }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$img:test"
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
            .send(
                &user,
                ChannelContent::Image {
                    url: bytes_url,
                    caption: Some("hello".to_string()),
                    mime_type: Some("image/png".to_string()),
                },
            )
            .await
            .expect("image must succeed");
    }

    // ── Task 18: Outbound File + FileData ─────────────────────────────────────

    #[tokio::test]
    async fn test_outbound_file_uploads_then_sends() {
        use crate::types::{ChannelContent, ChannelUser};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        let server = MockServer::start().await;
        let bytes_url = format!("{}/x.pdf", server.uri());
        Mock::given(method("GET"))
            .and(wiremock::matchers::path("/x.pdf"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"%PDF-1.4 dummy".to_vec())
                    .insert_header("Content-Type", "application/pdf"),
            )
            .mount(&server)
            .await;
        let upload_calls = StdArc::new(AtomicUsize::new(0));
        let send_calls = StdArc::new(AtomicUsize::new(0));
        let uc = upload_calls.clone();
        Mock::given(method("POST"))
            .and(wiremock::matchers::path("/_matrix/media/v3/upload"))
            .respond_with(move |_: &wiremock::Request| {
                uc.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "content_uri": "mxc://srv/file1"
                }))
            })
            .mount(&server)
            .await;
        let sc = send_calls.clone();
        Mock::given(method("PUT"))
            .and(body_partial_json(serde_json::json!({
                "msgtype": "m.file",
                "url": "mxc://srv/file1"
            })))
            .respond_with(move |_: &wiremock::Request| {
                sc.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event_id": "$f:test"
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
        adapter
            .send(
                &user,
                ChannelContent::File {
                    url: bytes_url,
                    filename: "x.pdf".to_string(),
                },
            )
            .await
            .expect("file must succeed");
        assert_eq!(upload_calls.load(Ordering::SeqCst), 1);
        assert_eq!(send_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_outbound_filedata_skips_fetch() {
        use crate::types::{ChannelContent, ChannelUser};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        let server = MockServer::start().await;
        let upload_calls = StdArc::new(AtomicUsize::new(0));
        let uc = upload_calls.clone();
        Mock::given(method("POST"))
            .and(wiremock::matchers::path("/_matrix/media/v3/upload"))
            .respond_with(move |_: &wiremock::Request| {
                uc.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "content_uri": "mxc://srv/fd1"
                }))
            })
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$fd:test"
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
            .send(
                &user,
                ChannelContent::FileData {
                    data: b"raw bytes".to_vec(),
                    filename: "raw.bin".to_string(),
                    mime_type: "application/octet-stream".to_string(),
                },
            )
            .await
            .expect("filedata must succeed");
        assert_eq!(upload_calls.load(Ordering::SeqCst), 1);
    }

    // ── Task 19: Outbound Audio/Voice/Video/Animation ─────────────────────────

    #[tokio::test]
    async fn test_outbound_voice_includes_msc3245_marker() {
        use crate::types::{ChannelContent, ChannelUser};
        let server = MockServer::start().await;
        let bytes_url = format!("{}/v.ogg", server.uri());
        Mock::given(method("GET"))
            .and(wiremock::matchers::path("/v.ogg"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"OggS dummy".to_vec())
                    .insert_header("Content-Type", "audio/ogg"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::path("/_matrix/media/v3/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content_uri": "mxc://srv/v1"
            })))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(body_partial_json(serde_json::json!({
                "msgtype": "m.audio",
                "org.matrix.msc3245.voice": {}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$voice:test"
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
            .send(
                &user,
                ChannelContent::Voice {
                    url: bytes_url,
                    caption: None,
                    duration_seconds: 4,
                },
            )
            .await
            .expect("voice must succeed");
    }

    // ── Task 20: Sticker/MediaGroup/Location/Poll/Interactive/ButtonCallback ──

    #[tokio::test]
    async fn test_outbound_location_event() {
        use crate::types::{ChannelContent, ChannelUser};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(body_partial_json(serde_json::json!({
                "msgtype": "m.location",
                "geo_uri": "geo:37.422,-122.0841"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$loc:test"
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
            .send(
                &user,
                ChannelContent::Location {
                    lat: 37.422,
                    lon: -122.0841,
                },
            )
            .await
            .expect("location must succeed");
    }

    #[tokio::test]
    async fn test_outbound_sticker_falls_back_to_text() {
        use crate::types::{ChannelContent, ChannelUser};
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(body_partial_json(serde_json::json!({
                "msgtype": "m.text",
                "body": "(sticker: stk-123)"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "$stk:test"
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
            .send(
                &user,
                ChannelContent::Sticker {
                    file_id: "stk-123".to_string(),
                },
            )
            .await
            .expect("sticker fallback must succeed");
    }

    #[tokio::test]
    async fn test_outbound_media_group_sends_each() {
        use crate::types::{ChannelContent, ChannelUser, MediaGroupItem};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        let server = MockServer::start().await;
        let bytes_url = format!("{}/p.png", server.uri());
        Mock::given(method("GET"))
            .and(wiremock::matchers::path("/p.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(b"\x89PNG".to_vec())
                    .insert_header("Content-Type", "image/png"),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(wiremock::matchers::path("/_matrix/media/v3/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "content_uri": "mxc://srv/x"
            })))
            .mount(&server)
            .await;
        let send_calls = StdArc::new(AtomicUsize::new(0));
        let sc = send_calls.clone();
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .respond_with(move |_: &wiremock::Request| {
                sc.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event_id": "$g:test"
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
        let items = vec![
            MediaGroupItem::Photo {
                url: bytes_url.clone(),
                caption: None,
            },
            MediaGroupItem::Photo {
                url: bytes_url.clone(),
                caption: None,
            },
            MediaGroupItem::Photo {
                url: bytes_url.clone(),
                caption: None,
            },
        ];
        adapter
            .send(&user, ChannelContent::MediaGroup { items })
            .await
            .expect("media group must succeed");
        assert_eq!(
            send_calls.load(Ordering::SeqCst),
            3,
            "3 items -> 3 PUT events"
        );
    }

    // ── PR #4831 follow-up: typed-error retry + idempotent txn_id ────────────

    #[test]
    fn test_parse_retry_after_ms_returns_seconds_form() {
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("2"),
        );
        assert_eq!(parse_retry_after_ms(&h), Some(2_000));

        // Whitespace tolerated.
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("  4  "),
        );
        assert_eq!(parse_retry_after_ms(&h), Some(4_000));
    }

    #[test]
    fn test_parse_retry_after_ms_returns_none_for_invalid() {
        // Missing header → None (caller falls back to default).
        let h = reqwest::header::HeaderMap::new();
        assert_eq!(parse_retry_after_ms(&h), None);

        // HTTP-date form (RFC 7231 alternative) is intentionally unsupported —
        // Synapse / Dendrite / Conduit emit the integer form for
        // M_LIMIT_EXCEEDED.
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("Wed, 21 Oct 2026 07:28:00 GMT"),
        );
        assert_eq!(parse_retry_after_ms(&h), None);

        // Garbage.
        let mut h = reqwest::header::HeaderMap::new();
        h.insert(
            reqwest::header::RETRY_AFTER,
            reqwest::header::HeaderValue::from_static("not-a-number"),
        );
        assert_eq!(parse_retry_after_ms(&h), None);
    }

    /// `api_edit_event` must defensively truncate oversized inputs so a
    /// caller that hands it >4096 bytes (e.g. an `EditInteractive` with
    /// a long body and many button-row hints) does NOT produce a
    /// `m.room.message` that Synapse rejects with `M_TOO_LARGE`. The
    /// fallback body has a `* ` prefix added on top so the truncation
    /// must happen before that prefix is appended.
    #[tokio::test]
    async fn test_api_edit_event_truncates_at_max_len() {
        use std::sync::Arc as StdArc;
        use std::sync::Mutex as StdMutex;
        let server = MockServer::start().await;
        let captured: StdArc<StdMutex<Option<serde_json::Value>>> =
            StdArc::new(StdMutex::new(None));
        let cap = captured.clone();
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .respond_with(move |req: &wiremock::Request| {
                if let Ok(b) = serde_json::from_slice::<serde_json::Value>(&req.body) {
                    *cap.lock().unwrap() = Some(b);
                }
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "event_id": "$edit:test"
                }))
            })
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        // 5000 ASCII bytes; well above MAX_MESSAGE_LEN (4096).
        let oversized = "a".repeat(5000);
        adapter
            .api_edit_event("!room:test", "$orig:test", &oversized)
            .await
            .expect("edit must succeed");
        let body = captured
            .lock()
            .unwrap()
            .clone()
            .expect("body must be captured");
        // m.new_content.body is the truncated content (no `* ` prefix).
        let new_body = body["m.new_content"]["body"]
            .as_str()
            .expect("m.new_content.body must be string");
        assert_eq!(
            new_body.len(),
            MAX_MESSAGE_LEN,
            "new_content.body must be exactly MAX_MESSAGE_LEN, was {}",
            new_body.len()
        );
        // Outer body carries the `* ` edit fallback prefix on top of the
        // truncated content, so it is MAX_MESSAGE_LEN + 2 bytes.
        let outer_body = body["body"].as_str().expect("body must be string");
        assert_eq!(
            outer_body.len(),
            MAX_MESSAGE_LEN + 2,
            "outer body = '* ' + truncated content"
        );
    }

    /// Regression for the txn_id idempotency gap flagged in the PR #4831
    /// review. When `api_edit_event_with_retry` sees a 429 it must retry
    /// with the SAME `txn_id` — Matrix dedupes on `(sender, txn_id)`, so
    /// reusing it turns a delayed-success-then-retry race into a server
    /// no-op rather than a duplicate `m.replace` event in the room.
    #[tokio::test]
    async fn test_api_edit_event_with_retry_reuses_txn_id_on_429() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc as StdArc;
        use std::sync::Mutex as StdMutex;
        let server = MockServer::start().await;
        let calls = StdArc::new(AtomicUsize::new(0));
        let cc = calls.clone();
        let captured_txns: StdArc<StdMutex<Vec<String>>> = StdArc::new(StdMutex::new(Vec::new()));
        let tc = captured_txns.clone();
        Mock::given(method("PUT"))
            .and(path_regex(
                r"^/_matrix/client/v3/rooms/.+/send/m\.room\.message/.+$",
            ))
            .respond_with(move |req: &wiremock::Request| {
                // The last path segment is the txn_id.
                let path = req.url.path().to_string();
                if let Some(txn) = path.rsplit('/').next() {
                    tc.lock().unwrap().push(txn.to_string());
                }
                let n = cc.fetch_add(1, Ordering::SeqCst);
                match n {
                    0 => ResponseTemplate::new(429)
                        .insert_header("Retry-After", "0")
                        .set_body_string("{\"errcode\":\"M_LIMIT_EXCEEDED\"}"),
                    _ => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "event_id": "$retry_ok:test"
                    })),
                }
            })
            .mount(&server)
            .await;
        let adapter = make_adapter(server.uri());
        let id = adapter
            .api_edit_event_with_retry("!room:test", "$orig:test", "retry me")
            .await
            .expect("second attempt must succeed");
        assert_eq!(id, "$retry_ok:test");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "expected first 429 + one successful retry"
        );
        let txns = captured_txns.lock().unwrap().clone();
        assert_eq!(txns.len(), 2, "two PUTs captured");
        assert_eq!(
            txns[0], txns[1],
            "retry MUST reuse the first attempt's txn_id (Matrix dedupes on (sender, txn_id))"
        );
        // Sanity: the shared txn_id is a real UUID, not the empty string
        // — a regression where we drop the txn_id entirely would also
        // cause the equality check above to pass.
        assert_eq!(txns[0].len(), 36, "txn_id should be UUID-shaped");
    }

    /// `MatrixApiError::From` unwraps `Other` so existing callers
    /// matching on the underlying error-message text keep working —
    /// only the `RateLimited` variant adds new wrapping. Pin both
    /// behaviours.
    #[test]
    fn test_matrix_api_error_into_boxed_unwraps_other() {
        let inner: Box<dyn std::error::Error + Send + Sync> = "boom".into();
        let boxed = MatrixApiError::Other(inner).into_boxed();
        assert_eq!(boxed.to_string(), "boom", "Other should unwrap, not wrap");

        let boxed_rate = MatrixApiError::RateLimited {
            retry_after_ms: Some(1500),
        }
        .into_boxed();
        let msg = boxed_rate.to_string();
        assert!(
            msg.contains("429") && msg.contains("1500"),
            "RateLimited display must mention 429 and the parsed Retry-After, got: {msg}"
        );
    }
}
