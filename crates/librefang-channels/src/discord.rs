//! Discord Gateway adapter for the LibreFang channel bridge.
//!
//! Uses Discord Gateway WebSocket (v10) for receiving messages and the REST API
//! for sending responses. No external Discord crate — just `tokio-tungstenite` + `reqwest`.

use crate::message_truncator::{split_to_utf16_chunks, DISCORD_MESSAGE_LIMIT};
use crate::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelRoleQuery, ChannelType, ChannelUser,
    PlatformRole,
};
use async_trait::async_trait;
use dashmap::DashMap;
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Discord Gateway opcodes.
mod opcode {
    pub const DISPATCH: u64 = 0;
    pub const HEARTBEAT: u64 = 1;
    pub const IDENTIFY: u64 = 2;
    pub const RESUME: u64 = 6;
    pub const RECONNECT: u64 = 7;
    pub const INVALID_SESSION: u64 = 9;
    pub const HELLO: u64 = 10;
    pub const HEARTBEAT_ACK: u64 = 11;
}

/// Discord Gateway adapter using WebSocket.
pub struct DiscordAdapter {
    /// SECURITY: Bot token is zeroized on drop to prevent memory disclosure.
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_guilds: Vec<String>,
    allowed_users: Vec<String>,
    ignore_bots: bool,
    /// Custom text patterns that trigger the bot (case-insensitive).
    mention_patterns: Vec<String>,
    intents: u64,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Initial backoff on WebSocket failures.
    initial_backoff: Duration,
    /// Maximum backoff on WebSocket failures.
    max_backoff: Duration,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Bot's own user ID (populated after READY event).
    bot_user_id: Arc<RwLock<Option<String>>>,
    /// Session ID for resume (populated after READY event).
    session_id: Arc<RwLock<Option<String>>>,
    /// Resume gateway URL.
    resume_gateway_url: Arc<RwLock<Option<String>>>,
    /// Channel-id → guild-id resolution cache. Populated on first
    /// `lookup_role` call for a given channel; channels almost never
    /// move guilds, so this stays valid for the adapter's lifetime.
    /// Keeps `lookup_role` to one Discord API call after the first hit
    /// (the `/guilds/.../members/...` request) instead of three.
    ///
    /// `Some(guild_id)` for guild text/voice channels; `None` for DM
    /// and group-DM channels (no `guild_id` in the channel object).
    /// Caching the `None` case stops the resolver from re-hitting
    /// Discord every time a user DMs the bot.
    channel_to_guild: Arc<DashMap<String, Option<String>>>,
}

impl DiscordAdapter {
    pub fn new(
        token: String,
        allowed_guilds: Vec<String>,
        allowed_users: Vec<String>,
        ignore_bots: bool,
        mention_patterns: Vec<String>,
        intents: u64,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            token: Zeroizing::new(token),
            client: crate::http_client::new_client(),
            allowed_guilds,
            allowed_users,
            ignore_bots,
            mention_patterns,
            intents,
            account_id: None,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            bot_user_id: Arc::new(RwLock::new(None)),
            session_id: Arc::new(RwLock::new(None)),
            resume_gateway_url: Arc::new(RwLock::new(None)),
            channel_to_guild: Arc::new(DashMap::new()),
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

    /// Get the WebSocket gateway URL from the Discord API.
    async fn get_gateway_url(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{DISCORD_API_BASE}/gateway/bot");
        let resp: serde_json::Value = self
            .client
            .get(&url)
            .header("Authorization", format!("Bot {}", self.token.as_str()))
            .send()
            .await?
            .json()
            .await?;

        let ws_url = resp["url"]
            .as_str()
            .ok_or("Missing 'url' in gateway response")?;

        Ok(format!("{ws_url}/?v=10&encoding=json"))
    }

    /// Send a message to a Discord channel via REST API.
    async fn api_send_message(
        &self,
        channel_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages");
        // Discord's 2000-character limit is measured in UTF-16 code units.
        let chunks = split_to_utf16_chunks(text, DISCORD_MESSAGE_LIMIT);

        for chunk in chunks {
            let body = serde_json::json!({ "content": chunk });
            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bot {}", self.token.as_str()))
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                warn!("Discord sendMessage failed: {body_text}");
            }
        }
        Ok(())
    }

    /// Send typing indicator to a Discord channel.
    async fn api_send_typing(
        &self,
        channel_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/typing");
        let _ = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token.as_str()))
            .send()
            .await?;
        Ok(())
    }

    /// Resolve a Discord channel ID to its guild ID.
    ///
    /// `lookup_role` receives a channel ID (because that is what every
    /// `ChannelMessage` carries as `sender.platform_id`, see
    /// `parse_message_create` ~line 715), but Discord's
    /// `/guilds/{id}/members/{uid}` endpoint expects the guild ID. We
    /// resolve via `GET /channels/{channel_id}` once per channel and
    /// cache the result for the adapter's lifetime — channels do not
    /// move guilds in normal operation, so cache invalidation is a
    /// non-issue.
    ///
    /// Returns `Ok(None)` for DM channels (no `guild_id` in the
    /// response) so the caller falls through to default-deny rather
    /// than 404-ing on a phantom guild lookup.
    pub async fn api_resolve_channel_guild(
        &self,
        channel_id: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
        if let Some(cached) = self.channel_to_guild.get(channel_id) {
            return Ok(cached.clone());
        }
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}");
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bot {}", self.token.as_str()))
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!("Discord get channel failed ({status}): {body_text}").into());
        }
        let channel: serde_json::Value = resp.json().await?;
        let resolved = channel["guild_id"].as_str().map(str::to_string);
        // Cache both `Some(guild_id)` and `None` (DM / group-DM) so
        // subsequent calls for the same channel skip the API round-trip
        // entirely. Discord channels do not change guild affiliation in
        // normal operation, and snowflake ids are globally unique, so a
        // permanent cache for the adapter's lifetime is safe.
        self.channel_to_guild
            .insert(channel_id.to_string(), resolved.clone());
        Ok(resolved)
    }

    /// Fetch a guild member and the set of guild-role names they hold.
    ///
    /// Discord's `GET /guilds/{guild_id}/members/{user_id}` returns role IDs;
    /// we resolve them to names via `GET /guilds/{guild_id}/roles` so the
    /// kernel mapping logic can match by human-readable role name (which is
    /// what operators put in `config.toml: [channel_role_mapping.discord]`).
    ///
    /// Returns `Ok(None)` when the user is not in the guild (HTTP 404).
    pub async fn api_get_guild_member_roles(
        &self,
        guild_id: &str,
        user_id: &str,
    ) -> Result<Option<Vec<String>>, Box<dyn std::error::Error + Send + Sync>> {
        // 1. Fetch the member's role IDs.
        let member_url = format!("{DISCORD_API_BASE}/guilds/{guild_id}/members/{user_id}");
        let resp = self
            .client
            .get(&member_url)
            .header("Authorization", format!("Bot {}", self.token.as_str()))
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!("Discord get guild member failed ({status}): {body_text}").into());
        }
        let member: serde_json::Value = resp.json().await?;
        let role_ids = parse_guild_member_role_ids(&member);
        if role_ids.is_empty() {
            return Ok(Some(Vec::new()));
        }

        // 2. Resolve role IDs → role names. The roles list is small (≤ a few
        // hundred per guild) so a single fetch + linear lookup is fine.
        let roles_url = format!("{DISCORD_API_BASE}/guilds/{guild_id}/roles");
        let roles_resp = self
            .client
            .get(&roles_url)
            .header("Authorization", format!("Bot {}", self.token.as_str()))
            .send()
            .await?;
        if !roles_resp.status().is_success() {
            let status = roles_resp.status();
            let body_text = roles_resp.text().await.unwrap_or_default();
            return Err(format!("Discord get guild roles failed ({status}): {body_text}").into());
        }
        let roles_json: serde_json::Value = roles_resp.json().await?;
        let names = resolve_role_ids_to_names(&role_ids, &roles_json);
        Ok(Some(names))
    }
}

/// Extract the `roles` array (list of role IDs) from a guild-member payload.
pub(crate) fn parse_guild_member_role_ids(member: &serde_json::Value) -> Vec<String> {
    member["roles"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve role IDs to role names using a `GET /guilds/{id}/roles` payload.
pub(crate) fn resolve_role_ids_to_names(
    role_ids: &[String],
    roles_json: &serde_json::Value,
) -> Vec<String> {
    let id_to_name: HashMap<String, String> = roles_json
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|r| {
                    let id = r["id"].as_str()?.to_string();
                    let name = r["name"].as_str()?.to_string();
                    Some((id, name))
                })
                .collect()
        })
        .unwrap_or_default();
    role_ids
        .iter()
        .filter_map(|id| id_to_name.get(id).cloned())
        .collect()
}

#[async_trait]
impl ChannelRoleQuery for DiscordAdapter {
    /// `chat_id` here is the Discord **channel** ID (matches what every
    /// `ChannelMessage` carries as `sender.platform_id`); we resolve it
    /// to the owning guild internally before hitting the members API.
    /// DM channels (no guild) yield `Ok(None)` → default-deny `Viewer`.
    async fn lookup_role(
        &self,
        chat_id: &str,
        user_id: &str,
    ) -> Result<Option<PlatformRole>, Box<dyn std::error::Error + Send + Sync>> {
        let Some(guild_id) = self.api_resolve_channel_guild(chat_id).await? else {
            return Ok(None);
        };
        let Some(roles) = self.api_get_guild_member_roles(&guild_id, user_id).await? else {
            return Ok(None);
        };
        if roles.is_empty() {
            return Ok(None);
        }
        // Discord users hold an unordered set of guild roles; pass them
        // all through and let the kernel translator pick the highest-
        // privilege match from `role_map` (Discord-side ordering must
        // not decide LibreFang permissions).
        Ok(Some(PlatformRole::many(roles)))
    }
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn name(&self) -> &str {
        "discord"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Discord
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let gateway_url = self.get_gateway_url().await?;
        info!("Discord gateway URL obtained");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        let token = self.token.clone();
        let intents = self.intents;
        let allowed_guilds = self.allowed_guilds.clone();
        let allowed_users = self.allowed_users.clone();
        let ignore_bots = self.ignore_bots;
        let mention_patterns = self.mention_patterns.clone();
        let bot_user_id = self.bot_user_id.clone();
        let session_id_store = self.session_id.clone();
        let resume_url_store = self.resume_gateway_url.clone();
        let mut shutdown = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();
        let initial_backoff = self.initial_backoff;
        let max_backoff = self.max_backoff;

        tokio::spawn(async move {
            let mut backoff = initial_backoff;
            let mut connect_url = gateway_url;
            // Sequence persists across reconnections for RESUME
            let sequence: Arc<RwLock<Option<u64>>> = Arc::new(RwLock::new(None));

            loop {
                if *shutdown.borrow() {
                    break;
                }

                info!("Connecting to Discord gateway...");

                let ws_result = tokio_tungstenite::connect_async(&connect_url).await;
                let ws_stream = match ws_result {
                    Ok((stream, _)) => stream,
                    Err(e) => {
                        warn!("Discord gateway connection failed: {e}, retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(max_backoff);
                        continue;
                    }
                };

                backoff = initial_backoff;
                info!("Discord gateway connected");

                let (mut ws_tx, mut ws_rx) = ws_stream.split();
                let mut _heartbeat_interval: Option<u64> = None;

                // Inner message loop — returns true if we should reconnect
                let should_reconnect = 'inner: loop {
                    let msg = tokio::select! {
                        msg = ws_rx.next() => msg,
                        _ = shutdown.changed() => {
                            if *shutdown.borrow() {
                                info!("Discord shutdown requested");
                                let _ = ws_tx.close().await;
                                return;
                            }
                            continue;
                        }
                    };

                    let msg = match msg {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => {
                            warn!("Discord WebSocket error: {e}");
                            break 'inner true;
                        }
                        None => {
                            info!("Discord WebSocket closed");
                            break 'inner true;
                        }
                    };

                    let text = match msg {
                        tokio_tungstenite::tungstenite::Message::Text(t) => t,
                        tokio_tungstenite::tungstenite::Message::Close(_) => {
                            info!("Discord gateway closed by server");
                            break 'inner true;
                        }
                        _ => continue,
                    };

                    let payload: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => {
                            warn!("Discord: failed to parse gateway message: {e}");
                            continue;
                        }
                    };

                    let op = payload["op"].as_u64().unwrap_or(999);

                    // Update sequence number
                    if let Some(s) = payload["s"].as_u64() {
                        *sequence.write().await = Some(s);
                    }

                    match op {
                        opcode::HELLO => {
                            let interval =
                                payload["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
                            _heartbeat_interval = Some(interval);
                            debug!("Discord HELLO: heartbeat_interval={interval}ms");

                            // Try RESUME if we have a session, otherwise IDENTIFY
                            let has_session = session_id_store.read().await.is_some();
                            let has_seq = sequence.read().await.is_some();

                            let gateway_msg = if has_session && has_seq {
                                let sid = session_id_store.read().await.clone().unwrap();
                                let seq = *sequence.read().await;
                                info!("Discord: sending RESUME (session={sid})");
                                serde_json::json!({
                                    "op": opcode::RESUME,
                                    "d": {
                                        "token": token.as_str(),
                                        "session_id": sid,
                                        "seq": seq
                                    }
                                })
                            } else {
                                info!("Discord: sending IDENTIFY");
                                serde_json::json!({
                                    "op": opcode::IDENTIFY,
                                    "d": {
                                        "token": token.as_str(),
                                        "intents": intents,
                                        "properties": {
                                            "os": "linux",
                                            "browser": "librefang",
                                            "device": "librefang"
                                        }
                                    }
                                })
                            };

                            if let Err(e) = ws_tx
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::to_string(&gateway_msg).unwrap().into(),
                                ))
                                .await
                            {
                                error!("Discord: failed to send IDENTIFY/RESUME: {e}");
                                break 'inner true;
                            }
                        }

                        opcode::DISPATCH => {
                            let event_name = payload["t"].as_str().unwrap_or("");
                            let d = &payload["d"];

                            match event_name {
                                "READY" => {
                                    let user_id =
                                        d["user"]["id"].as_str().unwrap_or("").to_string();
                                    let username =
                                        d["user"]["username"].as_str().unwrap_or("unknown");
                                    let sid = d["session_id"].as_str().unwrap_or("").to_string();
                                    let resume_url =
                                        d["resume_gateway_url"].as_str().unwrap_or("").to_string();

                                    *bot_user_id.write().await = Some(user_id.clone());
                                    *session_id_store.write().await = Some(sid);
                                    if !resume_url.is_empty() {
                                        *resume_url_store.write().await = Some(resume_url);
                                    }

                                    info!("Discord bot ready: {username} ({user_id})");
                                }

                                "MESSAGE_CREATE" | "MESSAGE_UPDATE" => {
                                    if let Some(mut msg) = parse_discord_message(
                                        d,
                                        &bot_user_id,
                                        &allowed_guilds,
                                        &allowed_users,
                                        ignore_bots,
                                        &mention_patterns,
                                    )
                                    .await
                                    {
                                        debug!(
                                            "Discord {event_name} from {}: {:?}",
                                            msg.sender.display_name, msg.content
                                        );
                                        // Inject account_id for multi-bot routing
                                        if let Some(ref aid) = account_id {
                                            msg.metadata.insert(
                                                "account_id".to_string(),
                                                serde_json::json!(aid),
                                            );
                                        }
                                        if tx.send(msg).await.is_err() {
                                            return;
                                        }
                                    }
                                }

                                "RESUMED" => {
                                    info!("Discord session resumed successfully");
                                }

                                _ => {
                                    debug!("Discord event: {event_name}");
                                }
                            }
                        }

                        opcode::HEARTBEAT => {
                            // Server requests immediate heartbeat
                            let seq = *sequence.read().await;
                            let hb = serde_json::json!({ "op": opcode::HEARTBEAT, "d": seq });
                            let _ = ws_tx
                                .send(tokio_tungstenite::tungstenite::Message::Text(
                                    serde_json::to_string(&hb).unwrap().into(),
                                ))
                                .await;
                        }

                        opcode::HEARTBEAT_ACK => {
                            debug!("Discord heartbeat ACK received");
                        }

                        opcode::RECONNECT => {
                            info!("Discord: server requested reconnect");
                            break 'inner true;
                        }

                        opcode::INVALID_SESSION => {
                            let resumable = payload["d"].as_bool().unwrap_or(false);
                            if resumable {
                                info!("Discord: invalid session (resumable)");
                            } else {
                                info!("Discord: invalid session (not resumable), clearing session");
                                *session_id_store.write().await = None;
                                *sequence.write().await = None;
                            }
                            break 'inner true;
                        }

                        _ => {
                            debug!("Discord: unknown opcode {op}");
                        }
                    }
                };

                if !should_reconnect || *shutdown.borrow() {
                    break;
                }

                // Try resume URL if available
                if let Some(ref url) = *resume_url_store.read().await {
                    connect_url = format!("{url}/?v=10&encoding=json");
                }

                warn!("Discord: reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }

            info!("Discord gateway loop stopped");
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // platform_id is the channel_id for Discord
        let channel_id = &user.platform_id;
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(channel_id, &text).await?;
            }
            _ => {
                self.api_send_message(channel_id, "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(
        &self,
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.api_send_typing(&user.platform_id).await
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

/// Parse a Discord MESSAGE_CREATE or MESSAGE_UPDATE payload into a `ChannelMessage`.
async fn parse_discord_message(
    d: &serde_json::Value,
    bot_user_id: &Arc<RwLock<Option<String>>>,
    allowed_guilds: &[String],
    allowed_users: &[String],
    ignore_bots: bool,
    mention_patterns: &[String],
) -> Option<ChannelMessage> {
    let author = d.get("author")?;
    let author_id = author["id"].as_str()?;

    // Filter out bot's own messages
    if let Some(ref bid) = *bot_user_id.read().await {
        if author_id == bid {
            return None;
        }
    }

    // Filter out other bots (configurable via ignore_bots)
    if ignore_bots && author["bot"].as_bool() == Some(true) {
        return None;
    }

    // Filter by allowed users
    if !allowed_users.is_empty() && !allowed_users.iter().any(|u| u == author_id) {
        debug!("Discord: ignoring message from unlisted user {author_id}");
        return None;
    }

    // Filter by allowed guilds
    if !allowed_guilds.is_empty() {
        if let Some(guild_id) = d["guild_id"].as_str() {
            if !allowed_guilds.iter().any(|g| g == guild_id) {
                return None;
            }
        }
    }

    let content_text = d["content"].as_str().unwrap_or("");
    let attachments = d["attachments"].as_array();
    let has_attachments = attachments.map(|a| !a.is_empty()).unwrap_or(false);

    // Skip messages with no text and no attachments
    if content_text.is_empty() && !has_attachments {
        return None;
    }

    let channel_id = d["channel_id"].as_str()?;
    let message_id = d["id"].as_str().unwrap_or("0");
    let username = author["username"].as_str().unwrap_or("Unknown");
    let discriminator = author["discriminator"].as_str().unwrap_or("0000");
    let display_name = if discriminator == "0" {
        username.to_string()
    } else {
        format!("{username}#{discriminator}")
    };

    let timestamp = d["timestamp"]
        .as_str()
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now);

    // Determine content: first check for attachments, then fall back to text/commands.
    // If both text and attachments exist, attachments take priority (text goes into caption/metadata).
    // NOTE: Attachments take priority over slash commands. If a user sends
    // `/command args` with an attachment, the command text becomes the
    // attachment caption rather than being parsed as a command.
    let content = if has_attachments {
        parse_discord_attachment(attachments.unwrap(), content_text)
    } else if content_text.starts_with('/') {
        let parts: Vec<&str> = content_text.splitn(2, ' ').collect();
        let cmd_name = &parts[0][1..];
        let args = if parts.len() > 1 {
            parts[1].split_whitespace().map(String::from).collect()
        } else {
            vec![]
        };
        ChannelContent::Command {
            name: cmd_name.to_string(),
            args,
        }
    } else {
        ChannelContent::Text(content_text.to_string())
    };

    // Determine if this is a group message (guild_id present = server channel)
    let is_group = d["guild_id"].as_str().is_some();

    // Check if bot was @mentioned (for MentionOnly policy enforcement)
    let was_mentioned = if let Some(ref bid) = *bot_user_id.read().await {
        // Check Discord mentions array
        let mentioned_in_array = d["mentions"]
            .as_array()
            .map(|arr| arr.iter().any(|m| m["id"].as_str() == Some(bid.as_str())))
            .unwrap_or(false);
        // Also check content for <@bot_id> or <@!bot_id> patterns
        let mentioned_in_content = content_text.contains(&format!("<@{bid}>"))
            || content_text.contains(&format!("<@!{bid}>"));
        mentioned_in_array || mentioned_in_content
    } else {
        false
    };
    // Also check custom mention patterns (case-insensitive contains match)
    let was_mentioned = was_mentioned
        || (!mention_patterns.is_empty() && {
            let lower = content_text.to_lowercase();
            mention_patterns
                .iter()
                .any(|pat| lower.contains(&pat.to_lowercase()))
        });

    let mut metadata = HashMap::new();
    if was_mentioned {
        metadata.insert("was_mentioned".to_string(), serde_json::json!(true));
    }

    Some(ChannelMessage {
        channel: ChannelType::Discord,
        platform_message_id: message_id.to_string(),
        sender: ChannelUser {
            platform_id: channel_id.to_string(),
            display_name,
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp,
        is_group,
        thread_id: None,
        metadata,
    })
}

/// Parse the first Discord attachment into a `ChannelContent` variant.
///
/// Discord attachments include a `content_type` field (MIME) and a direct `url`.
/// We map common MIME prefixes to the appropriate content variant:
///   - `image/*`  → `Image`
///   - `video/*`  → `Video`
///   - `audio/*`  → `Voice`
///   - everything else → `File`
fn parse_discord_attachment(
    attachments: &[serde_json::Value],
    companion_text: &str,
) -> ChannelContent {
    // Take the first attachment (most common case; multi-attachment is rare for bots).
    if attachments.len() > 1 {
        warn!(
            "Discord: {} additional attachment(s) ignored, only first processed",
            attachments.len() - 1
        );
    }
    let att = match attachments.first() {
        Some(a) => a,
        None => {
            // Defensive: caller checks has_attachments but guard against empty slice.
            return ChannelContent::Text(companion_text.to_string());
        }
    };

    let url = att["url"].as_str().unwrap_or("").to_string();
    if url.is_empty() {
        warn!("Discord attachment has empty URL, falling back to text");
        return ChannelContent::Text(companion_text.to_string());
    }

    let filename = att["filename"].as_str().unwrap_or("attachment").to_string();
    let content_type = att["content_type"].as_str().unwrap_or("");

    let caption = if companion_text.is_empty() {
        None
    } else {
        Some(companion_text.to_string())
    };

    if content_type.starts_with("image/") {
        ChannelContent::Image {
            url,
            caption,
            mime_type: Some(content_type.to_string()),
        }
    } else if content_type.starts_with("video/") {
        // Discord does not provide duration in attachment metadata.
        ChannelContent::Video {
            url,
            caption,
            duration_seconds: 0,
            filename: Some(filename),
        }
    } else if content_type.starts_with("audio/") {
        if !companion_text.is_empty() {
            warn!(
                "Discord audio attachment has companion text that cannot be sent as caption: {:?}",
                companion_text
            );
        }
        ChannelContent::Voice {
            url,
            caption: None,
            duration_seconds: 0,
        }
    } else {
        if !companion_text.is_empty() {
            warn!(
                "Discord file attachment has companion text that cannot be sent as caption: {:?}",
                companion_text
            );
        }
        ChannelContent::File { url, filename }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_resolution_parses_member_role_ids() {
        let member = serde_json::json!({
            "roles": ["role-mod", "role-vip"],
            "user": { "id": "u1" }
        });
        assert_eq!(
            parse_guild_member_role_ids(&member),
            vec!["role-mod".to_string(), "role-vip".to_string()]
        );
    }

    #[test]
    fn role_resolution_resolves_ids_to_names_in_order() {
        let role_ids = vec!["role-mod".to_string(), "role-vip".to_string()];
        let roles_json = serde_json::json!([
            { "id": "role-mod", "name": "Moderator" },
            { "id": "role-vip", "name": "VIP" },
            { "id": "role-other", "name": "Lurker" },
        ]);
        let names = resolve_role_ids_to_names(&role_ids, &roles_json);
        assert_eq!(names, vec!["Moderator".to_string(), "VIP".to_string()]);
    }

    #[test]
    fn role_resolution_drops_unknown_role_ids() {
        // A role the user holds but that no longer exists in the guild
        // (e.g. just deleted) is silently filtered out.
        let role_ids = vec!["role-stale".to_string(), "role-mod".to_string()];
        let roles_json = serde_json::json!([
            { "id": "role-mod", "name": "Moderator" },
        ]);
        let names = resolve_role_ids_to_names(&role_ids, &roles_json);
        assert_eq!(names, vec!["Moderator".to_string()]);
    }

    #[test]
    fn role_resolution_handles_empty_roles() {
        let member = serde_json::json!({ "roles": [] });
        assert!(parse_guild_member_role_ids(&member).is_empty());
    }

    #[tokio::test]
    async fn test_parse_discord_message_basic() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Hello agent!",
            "author": {
                "id": "user456",
                "username": "alice",
                "discriminator": "0",
                "bot": false
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        assert_eq!(msg.channel, ChannelType::Discord);
        assert_eq!(msg.sender.display_name, "alice");
        assert_eq!(msg.sender.platform_id, "ch1");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello agent!"));
    }

    #[tokio::test]
    async fn test_parse_discord_message_filters_bot() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "My own message",
            "author": {
                "id": "bot123",
                "username": "librefang",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_message_filters_other_bots() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Bot message",
            "author": {
                "id": "other_bot",
                "username": "somebot",
                "discriminator": "0",
                "bot": true
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_ignore_bots_false_allows_other_bots() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Bot message",
            "author": {
                "id": "other_bot",
                "username": "somebot",
                "discriminator": "0",
                "bot": true
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // With ignore_bots=false, other bots' messages should be allowed
        let msg = parse_discord_message(&d, &bot_id, &[], &[], false, &[]).await;
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.sender.display_name, "somebot");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Bot message"));
    }

    #[tokio::test]
    async fn test_parse_discord_ignore_bots_false_still_filters_self() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "My own message",
            "author": {
                "id": "bot123",
                "username": "librefang",
                "discriminator": "0",
                "bot": true
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // Even with ignore_bots=false, the bot's own messages must still be filtered
        let msg = parse_discord_message(&d, &bot_id, &[], &[], false, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_message_guild_filter() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "guild_id": "999",
            "content": "Hello",
            "author": {
                "id": "user1",
                "username": "bob",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // Not in allowed guilds
        let msg =
            parse_discord_message(&d, &bot_id, &["111".into(), "222".into()], &[], true, &[]).await;
        assert!(msg.is_none());

        // In allowed guilds
        let msg = parse_discord_message(&d, &bot_id, &["999".into()], &[], true, &[]).await;
        assert!(msg.is_some());
    }

    #[tokio::test]
    async fn test_parse_discord_command() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "/agent hello-world",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_discord_empty_content() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[]).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_discriminator() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Hi",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "1234"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        assert_eq!(msg.sender.display_name, "alice#1234");
    }

    #[tokio::test]
    async fn test_parse_discord_message_update() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Edited message content",
            "author": {
                "id": "user456",
                "username": "alice",
                "discriminator": "0",
                "bot": false
            },
            "timestamp": "2024-01-01T00:00:00+00:00",
            "edited_timestamp": "2024-01-01T00:01:00+00:00"
        });

        // MESSAGE_UPDATE uses the same parse function as MESSAGE_CREATE
        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        assert_eq!(msg.channel, ChannelType::Discord);
        assert!(
            matches!(msg.content, ChannelContent::Text(ref t) if t == "Edited message content")
        );
    }

    #[tokio::test]
    async fn test_parse_discord_allowed_users_filter() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Hello",
            "author": {
                "id": "user999",
                "username": "bob",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // Not in allowed users
        let msg = parse_discord_message(
            &d,
            &bot_id,
            &[],
            &["user111".into(), "user222".into()],
            true,
            &[],
        )
        .await;
        assert!(msg.is_none());

        // In allowed users
        let msg = parse_discord_message(&d, &bot_id, &[], &["user999".into()], true, &[]).await;
        assert!(msg.is_some());

        // Empty allowed_users = allow all
        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[]).await;
        assert!(msg.is_some());
    }

    #[tokio::test]
    async fn test_parse_discord_mention_detection() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));

        // Message with bot mentioned in mentions array
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "guild_id": "guild1",
            "content": "Hey <@bot123> help me",
            "mentions": [{"id": "bot123", "username": "librefang"}],
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        assert!(msg.is_group);
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );

        // Message without mention in group
        let d2 = serde_json::json!({
            "id": "msg2",
            "channel_id": "ch1",
            "guild_id": "guild1",
            "content": "Just chatting",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg2 = parse_discord_message(&d2, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        assert!(msg2.is_group);
        assert!(!msg2.metadata.contains_key("was_mentioned"));
    }

    #[tokio::test]
    async fn test_parse_discord_dm_not_group() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "dm-ch1",
            "content": "Hello",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        assert!(!msg.is_group);
    }

    #[test]
    fn test_discord_adapter_creation() {
        let adapter = DiscordAdapter::new(
            "test-token".to_string(),
            vec!["123".to_string(), "456".to_string()],
            vec![],
            true,
            vec![],
            37376,
        );
        assert_eq!(adapter.name(), "discord");
        assert_eq!(adapter.channel_type(), ChannelType::Discord);
    }

    #[test]
    fn test_parse_discord_attachment_image() {
        let attachments = vec![serde_json::json!({
            "id": "att1",
            "filename": "photo.png",
            "content_type": "image/png",
            "url": "https://cdn.discord.com/photo.png",
            "size": 12345
        })];
        let content = parse_discord_attachment(&attachments, "look at this");
        match content {
            ChannelContent::Image { url, caption, .. } => {
                assert_eq!(url, "https://cdn.discord.com/photo.png");
                assert_eq!(caption.as_deref(), Some("look at this"));
            }
            other => panic!("Expected Image, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_discord_attachment_video() {
        let attachments = vec![serde_json::json!({
            "id": "att1",
            "filename": "clip.mp4",
            "content_type": "video/mp4",
            "url": "https://cdn.discord.com/clip.mp4",
            "size": 999999
        })];
        let content = parse_discord_attachment(&attachments, "");
        match content {
            ChannelContent::Video {
                url,
                caption,
                filename,
                ..
            } => {
                assert_eq!(url, "https://cdn.discord.com/clip.mp4");
                assert!(caption.is_none());
                assert_eq!(filename.as_deref(), Some("clip.mp4"));
            }
            other => panic!("Expected Video, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_discord_attachment_audio() {
        let attachments = vec![serde_json::json!({
            "id": "att1",
            "filename": "voice.ogg",
            "content_type": "audio/ogg",
            "url": "https://cdn.discord.com/voice.ogg",
            "size": 5000
        })];
        let content = parse_discord_attachment(&attachments, "");
        assert!(matches!(content, ChannelContent::Voice { .. }));
    }

    #[test]
    fn test_parse_discord_attachment_file() {
        let attachments = vec![serde_json::json!({
            "id": "att1",
            "filename": "report.pdf",
            "content_type": "application/pdf",
            "url": "https://cdn.discord.com/report.pdf",
            "size": 50000
        })];
        let content = parse_discord_attachment(&attachments, "");
        match content {
            ChannelContent::File { url, filename } => {
                assert_eq!(url, "https://cdn.discord.com/report.pdf");
                assert_eq!(filename, "report.pdf");
            }
            other => panic!("Expected File, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_discord_message_with_image_attachment() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "check this out",
            "author": {
                "id": "user456",
                "username": "alice",
                "discriminator": "0",
                "bot": false
            },
            "timestamp": "2024-01-01T00:00:00+00:00",
            "attachments": [{
                "id": "att1",
                "filename": "screenshot.png",
                "content_type": "image/png",
                "url": "https://cdn.discord.com/screenshot.png",
                "size": 12345
            }]
        });

        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Image { url, caption, .. } => {
                assert_eq!(url, "https://cdn.discord.com/screenshot.png");
                assert_eq!(caption.as_deref(), Some("check this out"));
            }
            other => panic!("Expected Image, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_discord_message_attachment_no_text() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "",
            "author": {
                "id": "user456",
                "username": "alice",
                "discriminator": "0",
                "bot": false
            },
            "timestamp": "2024-01-01T00:00:00+00:00",
            "attachments": [{
                "id": "att1",
                "filename": "doc.pdf",
                "content_type": "application/pdf",
                "url": "https://cdn.discord.com/doc.pdf",
                "size": 50000
            }]
        });

        // Should NOT be None — attachment-only messages must be accepted
        let msg = parse_discord_message(&d, &bot_id, &[], &[], true, &[])
            .await
            .unwrap();
        assert!(matches!(msg.content, ChannelContent::File { .. }));
    }
}
