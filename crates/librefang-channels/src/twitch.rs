//! Twitch IRC channel adapter.
//!
//! Connects to Twitch's IRC gateway (`irc.chat.twitch.tv`) over plain TCP and
//! implements the IRC protocol for sending and receiving chat messages. Handles
//! PING/PONG keepalive, channel joins, and PRIVMSG parsing.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use zeroize::Zeroizing;

const TWITCH_IRC_HOST: &str = "irc.chat.twitch.tv";
const TWITCH_IRC_PORT: u16 = 6667;
const MAX_MESSAGE_LEN: usize = 500;

/// Twitch IRC channel adapter.
///
/// Connects to Twitch chat via the IRC protocol and bridges messages to the
/// LibreFang channel system. Supports multiple channels simultaneously.
pub struct TwitchAdapter {
    /// SECURITY: OAuth token is zeroized on drop.
    oauth_token: Zeroizing<String>,
    /// Twitch channels to join (without the '#' prefix).
    channels: Vec<String>,
    /// Bot's IRC nickname.
    nick: String,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// IRC endpoint host. Defaults to `TWITCH_IRC_HOST`; tests override
    /// via `with_irc_endpoint` to point at a local TCP listener.
    irc_host: String,
    /// IRC endpoint port. Defaults to `TWITCH_IRC_PORT`.
    irc_port: u16,
}

impl TwitchAdapter {
    /// Create a new Twitch adapter.
    ///
    /// # Arguments
    /// * `oauth_token` - Twitch OAuth token (without the "oauth:" prefix; it will be added).
    /// * `channels` - Channel names to join (without '#' prefix).
    /// * `nick` - Bot's IRC nickname (must match the token owner's Twitch username).
    pub fn new(oauth_token: String, channels: Vec<String>, nick: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            oauth_token: Zeroizing::new(oauth_token),
            channels,
            nick,
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            irc_host: TWITCH_IRC_HOST.to_string(),
            irc_port: TWITCH_IRC_PORT,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the Twitch IRC endpoint. `#[cfg(test)]`-only — used by
    /// integration tests to point the adapter at a local TCP listener.
    #[cfg(test)]
    pub fn with_irc_endpoint(mut self, host: String, port: u16) -> Self {
        self.irc_host = host;
        self.irc_port = port;
        self
    }

    /// Format the OAuth token for the IRC PASS command.
    fn pass_string(&self) -> String {
        let token = self.oauth_token.as_str();
        if token.starts_with("oauth:") {
            format!("PASS {token}\r\n")
        } else {
            format!("PASS oauth:{token}\r\n")
        }
    }
}

/// Parse an IRC PRIVMSG line into its components.
///
/// Expected format: `:nick!user@host PRIVMSG #channel :message text`
/// Returns `(nick, channel, message)` on success.
fn parse_privmsg(line: &str) -> Option<(String, String, String)> {
    // Must start with ':'
    if !line.starts_with(':') {
        return None;
    }

    let without_prefix = &line[1..];
    let parts: Vec<&str> = without_prefix.splitn(2, ' ').collect();
    if parts.len() < 2 {
        return None;
    }

    let nick = parts[0].split('!').next()?.to_string();
    let rest = parts[1];

    // Expect "PRIVMSG #channel :message"
    if !rest.starts_with("PRIVMSG ") {
        return None;
    }

    let after_cmd = &rest[8..]; // skip "PRIVMSG "
    let channel_end = after_cmd.find(' ')?;
    let channel = after_cmd[..channel_end].to_string();
    let msg_start = after_cmd[channel_end..].find(':')?;
    let message = after_cmd[channel_end + msg_start + 1..].to_string();

    Some((nick, channel, message))
}

#[async_trait]
impl ChannelAdapter for TwitchAdapter {
    fn name(&self) -> &str {
        "twitch"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("twitch".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        info!("Twitch adapter connecting to {TWITCH_IRC_HOST}:{TWITCH_IRC_PORT}");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let pass = self.pass_string();
        let nick_cmd = format!("NICK {}\r\n", self.nick);
        let join_cmds: Vec<String> = self
            .channels
            .iter()
            .map(|ch| {
                let ch = ch.trim_start_matches('#');
                format!("JOIN #{ch}\r\n")
            })
            .collect();
        let bot_nick = self.nick.to_lowercase();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();

        tokio::spawn(async move {
            let mut backoff = Duration::from_secs(1);

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                // Connect to Twitch IRC
                let stream = match TcpStream::connect((TWITCH_IRC_HOST, TWITCH_IRC_PORT)).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Twitch: connection failed: {e}, retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(60));
                        continue;
                    }
                };

                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);

                // Authenticate
                if write_half.write_all(pass.as_bytes()).await.is_err() {
                    warn!("Twitch: failed to send PASS");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    continue;
                }
                if write_half.write_all(nick_cmd.as_bytes()).await.is_err() {
                    warn!("Twitch: failed to send NICK");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    continue;
                }

                // Join channels
                for join in &join_cmds {
                    if write_half.write_all(join.as_bytes()).await.is_err() {
                        warn!("Twitch: failed to send JOIN");
                        break;
                    }
                }

                info!("Twitch IRC connected and joined channels");
                backoff = Duration::from_secs(1);

                // Read loop
                let should_reconnect = loop {
                    let mut line = String::new();
                    let read_result = tokio::select! {
                        _ = shutdown_rx.changed() => {
                            info!("Twitch adapter shutting down");
                            let _ = write_half.write_all(b"QUIT :Shutting down\r\n").await;
                            return;
                        }
                        result = reader.read_line(&mut line) => result,
                    };

                    match read_result {
                        Ok(0) => {
                            info!("Twitch IRC connection closed");
                            break true;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!("Twitch IRC read error: {e}");
                            break true;
                        }
                    }

                    let line = line.trim_end_matches('\n').trim_end_matches('\r');

                    // Handle PING
                    if line.starts_with("PING") {
                        let pong = line.replacen("PING", "PONG", 1);
                        let _ = write_half.write_all(format!("{pong}\r\n").as_bytes()).await;
                        continue;
                    }

                    // Parse PRIVMSG
                    if let Some((sender_nick, channel, message)) = parse_privmsg(line) {
                        // Skip own messages
                        if sender_nick.to_lowercase() == bot_nick {
                            continue;
                        }

                        if message.is_empty() {
                            continue;
                        }

                        let msg_content = if message.starts_with('/') || message.starts_with('!') {
                            let trimmed = message.trim_start_matches('/').trim_start_matches('!');
                            let parts: Vec<&str> = trimmed.splitn(2, ' ').collect();
                            let cmd = parts[0];
                            let args: Vec<String> = parts
                                .get(1)
                                .map(|a| a.split_whitespace().map(String::from).collect())
                                .unwrap_or_default();
                            ChannelContent::Command {
                                name: cmd.to_string(),
                                args,
                            }
                        } else {
                            ChannelContent::Text(message.clone())
                        };

                        let mut channel_msg = ChannelMessage {
                            channel: ChannelType::Custom("twitch".to_string()),
                            platform_message_id: uuid::Uuid::new_v4().to_string(),
                            sender: ChannelUser {
                                platform_id: channel.clone(),
                                display_name: sender_nick,
                                librefang_user: None,
                            },
                            content: msg_content,
                            target_agent: None,
                            timestamp: Utc::now(),
                            is_group: true, // Twitch channels are always group
                            thread_id: None,
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
                };

                if !should_reconnect || *shutdown_rx.borrow() {
                    break;
                }

                warn!("Twitch: reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(Duration::from_secs(60));
            }

            info!("Twitch IRC loop stopped");
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let channel = &user.platform_id;
        let text = match content {
            ChannelContent::Text(text) => text,
            _ => "(Unsupported content type)".to_string(),
        };

        // Connect briefly to send the message
        // In production, a persistent write connection would be maintained.
        let stream = TcpStream::connect((self.irc_host.as_str(), self.irc_port)).await?;
        let (_reader, mut writer) = stream.into_split();

        writer.write_all(self.pass_string().as_bytes()).await?;
        writer
            .write_all(format!("NICK {}\r\n", self.nick).as_bytes())
            .await?;

        // Wait briefly for auth to complete
        tokio::time::sleep(Duration::from_millis(500)).await;

        let chunks = split_message(&text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let msg = format!("PRIVMSG {channel} :{chunk}\r\n");
            writer.write_all(msg.as_bytes()).await?;
        }

        writer.write_all(b"QUIT\r\n").await?;
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

    #[test]
    fn test_twitch_adapter_creation() {
        let adapter = TwitchAdapter::new(
            "test-oauth-token".to_string(),
            vec!["testchannel".to_string()],
            "librefang_bot".to_string(),
        );
        assert_eq!(adapter.name(), "twitch");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("twitch".to_string())
        );
    }

    #[test]
    fn test_twitch_pass_string_with_prefix() {
        let adapter = TwitchAdapter::new("oauth:abc123".to_string(), vec![], "bot".to_string());
        assert_eq!(adapter.pass_string(), "PASS oauth:abc123\r\n");
    }

    #[test]
    fn test_twitch_pass_string_without_prefix() {
        let adapter = TwitchAdapter::new("abc123".to_string(), vec![], "bot".to_string());
        assert_eq!(adapter.pass_string(), "PASS oauth:abc123\r\n");
    }

    #[test]
    fn test_parse_privmsg_valid() {
        let line = ":nick123!user@host PRIVMSG #channel :Hello world!";
        let (nick, channel, message) = parse_privmsg(line).unwrap();
        assert_eq!(nick, "nick123");
        assert_eq!(channel, "#channel");
        assert_eq!(message, "Hello world!");
    }

    #[test]
    fn test_parse_privmsg_no_message() {
        // Missing colon before message
        let line = ":nick!user@host PRIVMSG #channel";
        assert!(parse_privmsg(line).is_none());
    }

    #[test]
    fn test_parse_privmsg_not_privmsg() {
        let line = ":server 001 bot :Welcome";
        assert!(parse_privmsg(line).is_none());
    }

    #[test]
    fn test_parse_privmsg_command() {
        let line = ":user!u@h PRIVMSG #ch :!help me";
        let (nick, channel, message) = parse_privmsg(line).unwrap();
        assert_eq!(nick, "user");
        assert_eq!(channel, "#ch");
        assert_eq!(message, "!help me");
    }

    #[test]
    fn test_parse_privmsg_empty_prefix() {
        let line = "PING :tmi.twitch.tv";
        assert!(parse_privmsg(line).is_none());
    }

    // ----- send() path tests (issue #3820) -----
    //
    // Twitch's `send()` opens a fresh TCP connection per call and writes
    // raw IRC frames (PASS / NICK / PRIVMSG / QUIT). Wiremock is HTTP-only,
    // so the fixture is a plain `tokio::net::TcpListener`: it accepts one
    // connection, drains the byte stream until the client closes (`QUIT`
    // + drop), and we assert the captured bytes contain the expected
    // wire-format frames in order.
    //
    // The hard-coded `(TWITCH_IRC_HOST, TWITCH_IRC_PORT)` connect target
    // is now overridable via `with_irc_endpoint` (test-only).

    use tokio::io::AsyncReadExt as _;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    fn twitch_user(channel: &str) -> ChannelUser {
        ChannelUser {
            platform_id: channel.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    /// Bind a local TCP listener; spawn a task that reads to EOF and
    /// returns the captured bytes via a oneshot. Returns `(host, port,
    /// rx)`.
    async fn capture_irc_listener() -> (String, u16, oneshot::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = oneshot::channel::<Vec<u8>>();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = Vec::new();
            let _ = stream.read_to_end(&mut buf).await;
            let _ = tx.send(buf);
        });
        (addr.ip().to_string(), addr.port(), rx)
    }

    #[tokio::test]
    async fn twitch_send_writes_pass_nick_privmsg_quit_in_order() {
        let (host, port, captured) = capture_irc_listener().await;
        let adapter = TwitchAdapter::new(
            "oauth:abc123".to_string(),
            vec![],
            "librefang_bot".to_string(),
        )
        .with_irc_endpoint(host, port);

        adapter
            .send(
                &twitch_user("#librefang"),
                ChannelContent::Text("hi twitch".into()),
            )
            .await
            .expect("twitch send must succeed against capturing listener");

        let bytes = tokio::time::timeout(Duration::from_secs(5), captured)
            .await
            .expect("listener must capture frames within 5s")
            .expect("oneshot must not be dropped");
        let text =
            String::from_utf8(bytes).expect("captured bytes must be valid utf-8 (IRC is ASCII)");

        let pass_idx = text
            .find("PASS oauth:abc123\r\n")
            .expect("expected PASS frame with the configured oauth token, got: \n---\n{text}\n---");
        let nick_idx = text
            .find("NICK librefang_bot\r\n")
            .expect("expected NICK frame with the configured nick");
        let privmsg_idx = text
            .find("PRIVMSG #librefang :hi twitch\r\n")
            .expect("expected PRIVMSG frame to the channel with the text body");
        let quit_idx = text
            .find("QUIT\r\n")
            .expect("expected QUIT frame to terminate the session");

        assert!(
            pass_idx < nick_idx,
            "PASS must precede NICK (text was: {text})"
        );
        assert!(
            nick_idx < privmsg_idx,
            "NICK must precede PRIVMSG (text was: {text})"
        );
        assert!(
            privmsg_idx < quit_idx,
            "PRIVMSG must precede QUIT (text was: {text})"
        );
    }
}
