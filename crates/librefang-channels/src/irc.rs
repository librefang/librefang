//! IRC channel adapter for the LibreFang channel bridge.
//!
//! Uses raw TCP via `tokio::net::TcpStream` with `tokio::io` buffered I/O for
//! plaintext IRC connections. Implements the core IRC protocol: NICK, USER, JOIN,
//! PRIVMSG, PING/PONG. A `use_tls: bool` field is reserved for future TLS support
//! (would require a `tokio-native-tls` dependency).

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
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Maximum IRC message length per RFC 2812 (including CRLF).
/// We use 510 for the payload (512 minus CRLF).
const MAX_MESSAGE_LEN: usize = 510;

/// Maximum length for a single PRIVMSG payload, accounting for the
/// `:nick!user@host PRIVMSG #channel :` prefix overhead (~80 chars conservative).
const MAX_PRIVMSG_PAYLOAD: usize = 400;

// Backoff durations are now configurable via IrcConfig.

/// IRC channel adapter using raw TCP and the IRC text protocol.
///
/// Connects to an IRC server, authenticates with NICK/USER (and optional PASS),
/// joins configured channels, and listens for PRIVMSG events.
pub struct IrcAdapter {
    /// IRC server hostname (e.g., "irc.libera.chat").
    server: String,
    /// IRC server port (typically 6667 for plaintext, 6697 for TLS).
    port: u16,
    /// Bot's IRC nickname.
    nick: String,
    /// SECURITY: Optional server password, zeroized on drop.
    password: Option<Zeroizing<String>>,
    /// IRC channels to join (e.g., ["#librefang", "#bots"]).
    channels: Vec<String>,
    /// Reserved for future TLS support. Currently only plaintext is implemented.
    #[allow(dead_code)]
    use_tls: bool,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Initial backoff on connection failures.
    initial_backoff: Duration,
    /// Maximum backoff on connection failures.
    max_backoff: Duration,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Shared write handle for sending messages from the `send()` method.
    /// Populated after `start()` connects to the server.
    write_tx: Arc<RwLock<Option<mpsc::Sender<String>>>>,
}

impl IrcAdapter {
    /// Create a new IRC adapter.
    ///
    /// * `server` — IRC server hostname.
    /// * `port` — IRC server port (6667 for plaintext).
    /// * `nick` — Bot's IRC nickname.
    /// * `password` — Optional server password (PASS command).
    /// * `channels` — IRC channels to join (must start with `#`).
    /// * `use_tls` — Reserved for future TLS support (currently ignored).
    pub fn new(
        server: String,
        port: u16,
        nick: String,
        password: Option<String>,
        channels: Vec<String>,
        use_tls: bool,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            server,
            port,
            nick,
            password: password.map(Zeroizing::new),
            channels,
            use_tls,
            account_id: None,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(60),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            write_tx: Arc::new(RwLock::new(None)),
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

    /// Format the server address as `host:port`.
    fn addr(&self) -> String {
        format!("{}:{}", self.server, self.port)
    }
}

/// An IRC protocol line parsed into its components.
#[derive(Debug)]
struct IrcLine {
    /// Optional prefix (e.g., ":nick!user@host").
    prefix: Option<String>,
    /// The IRC command (e.g., "PRIVMSG", "PING", "001").
    command: String,
    /// Parameters following the command.
    params: Vec<String>,
    /// Trailing parameter (after `:` in the params).
    trailing: Option<String>,
}

/// Parse a raw IRC line into structured components.
///
/// IRC line format: `[:prefix] COMMAND [params...] [:trailing]`
fn parse_irc_line(line: &str) -> Option<IrcLine> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let mut remaining = line;
    let prefix = if remaining.starts_with(':') {
        let space = remaining.find(' ')?;
        let pfx = remaining[1..space].to_string();
        remaining = &remaining[space + 1..];
        Some(pfx)
    } else {
        None
    };

    // Split off the trailing parameter (after " :")
    let (main_part, trailing) = if let Some(idx) = remaining.find(" :") {
        let trail = remaining[idx + 2..].to_string();
        (&remaining[..idx], Some(trail))
    } else {
        (remaining, None)
    };

    let mut parts = main_part.split_whitespace();
    let command = parts.next()?.to_string();
    let params: Vec<String> = parts.map(String::from).collect();

    Some(IrcLine {
        prefix,
        command,
        params,
        trailing,
    })
}

/// Extract the nickname from an IRC prefix like "nick!user@host".
fn nick_from_prefix(prefix: &str) -> &str {
    prefix.split('!').next().unwrap_or(prefix)
}

/// Parse a PRIVMSG IRC line into a `ChannelMessage`.
fn parse_privmsg(line: &IrcLine, bot_nick: &str) -> Option<ChannelMessage> {
    if line.command != "PRIVMSG" {
        return None;
    }

    let prefix = line.prefix.as_deref()?;
    let sender_nick = nick_from_prefix(prefix);

    // Skip messages from the bot itself
    if sender_nick.eq_ignore_ascii_case(bot_nick) {
        return None;
    }

    let target = line.params.first()?;
    let text = line.trailing.as_deref().unwrap_or("");
    if text.is_empty() {
        return None;
    }

    // Determine if this is a channel message (group) or a DM
    let is_group = target.starts_with('#') || target.starts_with('&');

    // The "platform_id" is the channel name for group messages, or the
    // sender's nick for DMs (so replies go back to the right place).
    let platform_id = if is_group {
        target.to_string()
    } else {
        sender_nick.to_string()
    };

    // Parse commands (messages starting with /)
    let content = if text.starts_with('/') {
        let parts: Vec<&str> = text.splitn(2, ' ').collect();
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
        ChannelContent::Text(text.to_string())
    };

    Some(ChannelMessage {
        channel: ChannelType::Custom("irc".to_string()),
        platform_message_id: String::new(), // IRC has no message IDs
        sender: ChannelUser {
            platform_id,
            display_name: sender_nick.to_string(),
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group,
        thread_id: None,
        metadata: HashMap::new(),
    })
}

#[async_trait]
impl ChannelAdapter for IrcAdapter {
    fn name(&self) -> &str {
        "irc"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("irc".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let (write_cmd_tx, mut write_cmd_rx) = mpsc::channel::<String>(64);

        // Store the write channel so `send()` can use it
        *self.write_tx.write().await = Some(write_cmd_tx.clone());

        let addr = self.addr();
        let nick = self.nick.clone();
        let password = self.password.clone();
        let channels = self.channels.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let account_id = self.account_id.clone();
        let initial_backoff = self.initial_backoff;
        let max_backoff = self.max_backoff;

        tokio::spawn(async move {
            let mut backoff = initial_backoff;

            loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                info!("Connecting to IRC server at {addr}...");

                let stream = match TcpStream::connect(&addr).await {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("IRC connection failed: {e}, retrying in {backoff:?}");
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(max_backoff);
                        continue;
                    }
                };

                backoff = initial_backoff;
                info!("IRC connected to {addr}");

                let (reader, mut writer) = stream.into_split();
                let mut lines = BufReader::new(reader).lines();

                // Send PASS (if configured), NICK, and USER
                let mut registration = String::new();
                if let Some(ref pass) = password {
                    registration.push_str(&format!("PASS {}\r\n", pass.as_str()));
                }
                registration.push_str(&format!("NICK {nick}\r\n"));
                registration.push_str(&format!("USER {nick} 0 * :LibreFang Bot\r\n"));

                if let Err(e) = writer.write_all(registration.as_bytes()).await {
                    warn!("IRC registration send failed: {e}");
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                    continue;
                }

                let nick_clone = nick.clone();
                let channels_clone = channels.clone();
                let mut joined = false;

                // Inner message loop — returns true if we should reconnect
                let should_reconnect = 'inner: loop {
                    tokio::select! {
                        line_result = lines.next_line() => {
                            let line = match line_result {
                                Ok(Some(l)) => l,
                                Ok(None) => {
                                    info!("IRC connection closed");
                                    break 'inner true;
                                }
                                Err(e) => {
                                    warn!("IRC read error: {e}");
                                    break 'inner true;
                                }
                            };

                            debug!("IRC < {line}");

                            let parsed = match parse_irc_line(&line) {
                                Some(p) => p,
                                None => continue,
                            };

                            match parsed.command.as_str() {
                                // PING/PONG keepalive
                                "PING" => {
                                    let pong_param = parsed.trailing
                                        .as_deref()
                                        .or(parsed.params.first().map(|s| s.as_str()))
                                        .unwrap_or("");
                                    let pong = format!("PONG :{pong_param}\r\n");
                                    if let Err(e) = writer.write_all(pong.as_bytes()).await {
                                        warn!("IRC PONG send failed: {e}");
                                        break 'inner true;
                                    }
                                }

                                // RPL_WELCOME (001) — registration complete, join channels
                                "001" if !joined => {
                                    info!("IRC registered as {nick_clone}");
                                    for ch in &channels_clone {
                                        let join_cmd = format!("JOIN {ch}\r\n");
                                        if let Err(e) = writer.write_all(join_cmd.as_bytes()).await {
                                            warn!("IRC JOIN send failed: {e}");
                                            break 'inner true;
                                        }
                                        info!("IRC joining {ch}");
                                    }
                                    joined = true;
                                }

                                // PRIVMSG — incoming message
                                "PRIVMSG" => {
                                    if let Some(mut msg) = parse_privmsg(&parsed, &nick_clone) {
                                        debug!(
                                            "IRC message from {}: {:?}",
                                            msg.sender.display_name, msg.content
                                        );
                                        // Inject account_id for multi-bot routing
                                if let Some(ref aid) = account_id {
                                    msg.metadata.insert("account_id".to_string(), serde_json::json!(aid));
                                }
                                if tx.send(msg).await.is_err() {
                                            return;
                                        }
                                    }
                                }

                                // ERR_NICKNAMEINUSE (433) — nickname taken
                                "433" => {
                                    warn!("IRC: nickname '{nick_clone}' is already in use");
                                    let alt_nick = format!("{nick_clone}_");
                                    let cmd = format!("NICK {alt_nick}\r\n");
                                    let _ = writer.write_all(cmd.as_bytes()).await;
                                }

                                // JOIN confirmation
                                "JOIN" => {
                                    if let Some(ref prefix) = parsed.prefix {
                                        let joiner = nick_from_prefix(prefix);
                                        let channel = parsed.trailing
                                            .as_deref()
                                            .or(parsed.params.first().map(|s| s.as_str()))
                                            .unwrap_or("?");
                                        if joiner.eq_ignore_ascii_case(&nick_clone) {
                                            info!("IRC joined {channel}");
                                        }
                                    }
                                }

                                _ => {
                                    // Ignore other commands
                                }
                            }
                        }

                        // Outbound message requests from `send()`
                        Some(raw_cmd) = write_cmd_rx.recv() => {
                            if let Err(e) = writer.write_all(raw_cmd.as_bytes()).await {
                                warn!("IRC write failed: {e}");
                                break 'inner true;
                            }
                        }

                        _ = shutdown_rx.changed() => {
                            if *shutdown_rx.borrow() {
                                info!("IRC adapter shutting down");
                                let _ = writer.write_all(b"QUIT :LibreFang shutting down\r\n").await;
                                return;
                            }
                        }
                    }
                };

                if !should_reconnect || *shutdown_rx.borrow() {
                    break;
                }

                warn!("IRC: reconnecting in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(max_backoff);
            }

            info!("IRC connection loop stopped");
        });

        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let write_tx = self.write_tx.read().await;
        let write_tx = write_tx
            .as_ref()
            .ok_or("IRC adapter not started — call start() first")?;

        let target = &user.platform_id; // channel name or nick
        let text = match content {
            ChannelContent::Text(t) => t,
            _ => "(Unsupported content type)".to_string(),
        };

        let chunks = split_message(&text, MAX_PRIVMSG_PAYLOAD);
        for chunk in chunks {
            let raw = format!("PRIVMSG {target} :{chunk}\r\n");
            if raw.len() > MAX_MESSAGE_LEN + 2 {
                // Shouldn't happen with MAX_PRIVMSG_PAYLOAD, but be safe
                warn!("IRC message exceeds 512 bytes, truncating");
            }
            write_tx.send(raw).await.map_err(|e| {
                Box::<dyn std::error::Error + Send + Sync>::from(format!(
                    "IRC write channel closed: {e}"
                ))
            })?;
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

    #[test]
    fn test_irc_adapter_creation() {
        let adapter = IrcAdapter::new(
            "irc.libera.chat".to_string(),
            6667,
            "librefang".to_string(),
            None,
            vec!["#librefang".to_string()],
            false,
        );
        assert_eq!(adapter.name(), "irc");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("irc".to_string())
        );
    }

    #[test]
    fn test_irc_addr() {
        let adapter = IrcAdapter::new(
            "irc.libera.chat".to_string(),
            6667,
            "bot".to_string(),
            None,
            vec![],
            false,
        );
        assert_eq!(adapter.addr(), "irc.libera.chat:6667");
    }

    #[test]
    fn test_irc_addr_custom_port() {
        let adapter = IrcAdapter::new(
            "localhost".to_string(),
            6697,
            "bot".to_string(),
            Some("secret".to_string()),
            vec!["#test".to_string()],
            true,
        );
        assert_eq!(adapter.addr(), "localhost:6697");
    }

    #[test]
    fn test_parse_irc_line_ping() {
        let line = parse_irc_line("PING :server.example.com").unwrap();
        assert!(line.prefix.is_none());
        assert_eq!(line.command, "PING");
        assert_eq!(line.trailing.as_deref(), Some("server.example.com"));
    }

    #[test]
    fn test_parse_irc_line_privmsg() {
        let line = parse_irc_line(":alice!alice@host PRIVMSG #librefang :Hello everyone!").unwrap();
        assert_eq!(line.prefix.as_deref(), Some("alice!alice@host"));
        assert_eq!(line.command, "PRIVMSG");
        assert_eq!(line.params, vec!["#librefang"]);
        assert_eq!(line.trailing.as_deref(), Some("Hello everyone!"));
    }

    #[test]
    fn test_parse_irc_line_numeric() {
        let line = parse_irc_line(":server 001 botnick :Welcome to the IRC network").unwrap();
        assert_eq!(line.prefix.as_deref(), Some("server"));
        assert_eq!(line.command, "001");
        assert_eq!(line.params, vec!["botnick"]);
        assert_eq!(line.trailing.as_deref(), Some("Welcome to the IRC network"));
    }

    #[test]
    fn test_parse_irc_line_no_trailing() {
        let line = parse_irc_line(":alice!alice@host JOIN #librefang").unwrap();
        assert_eq!(line.command, "JOIN");
        assert_eq!(line.params, vec!["#librefang"]);
        assert!(line.trailing.is_none());
    }

    #[test]
    fn test_parse_irc_line_empty() {
        assert!(parse_irc_line("").is_none());
        assert!(parse_irc_line("   ").is_none());
    }

    #[test]
    fn test_nick_from_prefix_full() {
        assert_eq!(nick_from_prefix("alice!alice@host.example.com"), "alice");
    }

    #[test]
    fn test_nick_from_prefix_nick_only() {
        assert_eq!(nick_from_prefix("alice"), "alice");
    }

    #[test]
    fn test_parse_privmsg_channel() {
        let line = IrcLine {
            prefix: Some("alice!alice@host".to_string()),
            command: "PRIVMSG".to_string(),
            params: vec!["#librefang".to_string()],
            trailing: Some("Hello from IRC!".to_string()),
        };

        let msg = parse_privmsg(&line, "librefang-bot").unwrap();
        assert_eq!(msg.channel, ChannelType::Custom("irc".to_string()));
        assert_eq!(msg.sender.display_name, "alice");
        assert_eq!(msg.sender.platform_id, "#librefang");
        assert!(msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello from IRC!"));
    }

    #[test]
    fn test_parse_privmsg_dm() {
        let line = IrcLine {
            prefix: Some("bob!bob@host".to_string()),
            command: "PRIVMSG".to_string(),
            params: vec!["librefang-bot".to_string()],
            trailing: Some("Private message".to_string()),
        };

        let msg = parse_privmsg(&line, "librefang-bot").unwrap();
        assert!(!msg.is_group);
        assert_eq!(msg.sender.platform_id, "bob"); // DM replies go to sender
    }

    #[test]
    fn test_parse_privmsg_skips_self() {
        let line = IrcLine {
            prefix: Some("librefang-bot!bot@host".to_string()),
            command: "PRIVMSG".to_string(),
            params: vec!["#librefang".to_string()],
            trailing: Some("My own message".to_string()),
        };

        let msg = parse_privmsg(&line, "librefang-bot");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_privmsg_command() {
        let line = IrcLine {
            prefix: Some("alice!alice@host".to_string()),
            command: "PRIVMSG".to_string(),
            params: vec!["#librefang".to_string()],
            trailing: Some("/agent hello-world".to_string()),
        };

        let msg = parse_privmsg(&line, "librefang-bot").unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_privmsg_empty_text() {
        let line = IrcLine {
            prefix: Some("alice!alice@host".to_string()),
            command: "PRIVMSG".to_string(),
            params: vec!["#librefang".to_string()],
            trailing: Some("".to_string()),
        };

        let msg = parse_privmsg(&line, "librefang-bot");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_privmsg_no_trailing() {
        let line = IrcLine {
            prefix: Some("alice!alice@host".to_string()),
            command: "PRIVMSG".to_string(),
            params: vec!["#librefang".to_string()],
            trailing: None,
        };

        let msg = parse_privmsg(&line, "librefang-bot");
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_privmsg_not_privmsg() {
        let line = IrcLine {
            prefix: Some("alice!alice@host".to_string()),
            command: "NOTICE".to_string(),
            params: vec!["#librefang".to_string()],
            trailing: Some("Notice text".to_string()),
        };

        let msg = parse_privmsg(&line, "librefang-bot");
        assert!(msg.is_none());
    }

    // ----- send() path tests (issue #3820) -----
    //
    // IRC send() does not open a socket — it writes IRC frames to an
    // internal `mpsc::Sender<String>` (`write_tx`) which the start()
    // loop drains onto a `TcpStream`. That mpsc boundary is the natural
    // tap point: we seed `write_tx` with a test channel and then assert
    // the exact IRC `PRIVMSG` lines that send() emits. No socket, no
    // wiremock — and no production code change.

    fn irc_user(target: &str) -> ChannelUser {
        ChannelUser {
            platform_id: target.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn irc_send_writes_privmsg_frame_to_internal_mpsc() {
        let adapter = IrcAdapter::new(
            "irc.example".to_string(),
            6667,
            "librefang-bot".to_string(),
            None,
            vec!["#librefang".to_string()],
            false,
        );

        // Tap the internal write channel before calling send().
        let (tx, mut rx) = mpsc::channel::<String>(8);
        *adapter.write_tx.write().await = Some(tx);

        adapter
            .send(
                &irc_user("#librefang"),
                ChannelContent::Text("hello irc".into()),
            )
            .await
            .expect("irc send must succeed when write_tx is wired up");

        let frame = rx
            .recv()
            .await
            .expect("send() must push exactly one PRIVMSG frame");
        assert_eq!(
            frame, "PRIVMSG #librefang :hello irc\r\n",
            "frame must match the IRC PRIVMSG wire format"
        );
        assert!(
            rx.try_recv().is_err(),
            "no extra frames expected for a single short message"
        );
    }

    #[tokio::test]
    async fn irc_send_returns_err_when_not_started() {
        // write_tx is None ⇒ adapter has not been start()ed; send() must
        // bubble that up instead of silently dropping the message.
        let adapter = IrcAdapter::new(
            "irc.example".to_string(),
            6667,
            "librefang-bot".to_string(),
            None,
            vec![],
            false,
        );
        let err = adapter
            .send(&irc_user("alice"), ChannelContent::Text("x".into()))
            .await
            .expect_err("irc send must error when start() has not run");
        assert!(
            err.to_string().to_lowercase().contains("not started"),
            "error should mention not-started state, got: {err}"
        );
    }
}
