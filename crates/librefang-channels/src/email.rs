//! Email channel adapter (IMAP + SMTP).
//!
//! Polls IMAP for new emails and sends responses via SMTP using `lettre`.
//! Uses the subject line for agent routing (e.g., "\[coder\] Fix this bug").

use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use futures::Stream;
use lettre::message::Mailbox;
use lettre::transport::smtp::authentication::Credentials;
use lettre::AsyncSmtpTransport;
use lettre::AsyncTransport;
use lettre::Tokio1Executor;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

/// SASL PLAIN authenticator for IMAP servers that reject LOGIN
/// (e.g., Lark/Larksuite which only advertise AUTH=PLAIN).
struct PlainAuthenticator {
    username: String,
    password: String,
}

impl imap::Authenticator for PlainAuthenticator {
    type Response = String;
    fn process(&self, _data: &[u8]) -> Self::Response {
        // SASL PLAIN: \0<username>\0<password>
        format!("\x00{}\x00{}", self.username, self.password)
    }
}

/// Reply context for email threading (In-Reply-To / Subject continuity).
#[derive(Debug, Clone)]
struct ReplyCtx {
    subject: String,
    message_id: String,
}

/// Email channel adapter using IMAP for receiving and SMTP for sending.
pub struct EmailAdapter {
    /// IMAP server host.
    imap_host: String,
    /// IMAP port (993 for TLS).
    imap_port: u16,
    /// SMTP server host.
    smtp_host: String,
    /// SMTP port (587 for STARTTLS, 465 for implicit TLS).
    smtp_port: u16,
    /// Email address (used for both IMAP and SMTP).
    username: String,
    /// SECURITY: Password is zeroized on drop.
    password: Zeroizing<String>,
    /// How often to check for new emails.
    poll_interval: Duration,
    /// Which IMAP folders to monitor.
    folders: Vec<String>,
    /// Only process emails from these senders (empty = all).
    allowed_senders: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Tracks reply context per sender for email threading.
    reply_ctx: Arc<DashMap<String, ReplyCtx>>,
    /// When `true`, `build_smtp_transport` builds a plain (no-TLS,
    /// no-AUTH) SMTP transport via `builder_dangerous`. `#[cfg(test)]`
    /// only path: tests set this via `with_plain_smtp` so `send()`
    /// can talk to a hand-rolled local SMTP fixture without TLS.
    smtp_use_plain: bool,
}

impl EmailAdapter {
    /// Create a new email adapter.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        imap_host: String,
        imap_port: u16,
        smtp_host: String,
        smtp_port: u16,
        username: String,
        password: String,
        poll_interval_secs: u64,
        folders: Vec<String>,
        allowed_senders: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            imap_host,
            imap_port,
            smtp_host,
            smtp_port,
            username,
            password: Zeroizing::new(password),
            poll_interval: Duration::from_secs(poll_interval_secs),
            folders: if folders.is_empty() {
                vec!["INBOX".to_string()]
            } else {
                folders
            },
            allowed_senders,
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            reply_ctx: Arc::new(DashMap::new()),
            smtp_use_plain: false,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Switch the SMTP transport into plain (no-TLS) mode for tests.
    /// `#[cfg(test)]`-only — used to point the adapter at a local
    /// hand-rolled SMTP fixture without standing up TLS.
    #[cfg(test)]
    pub fn with_plain_smtp(mut self) -> Self {
        self.smtp_use_plain = true;
        self
    }

    /// Check if a sender is in the allowlist (empty = allow all). Used in tests.
    #[allow(dead_code)]
    fn is_allowed_sender(&self, sender: &str) -> bool {
        if self.allowed_senders.is_empty() {
            return true;
        }
        sender_matches_allowlist(sender, &self.allowed_senders)
    }

    /// Extract agent name from subject line brackets, e.g., "[coder] Fix the bug" -> Some("coder")
    fn extract_agent_from_subject(subject: &str) -> Option<String> {
        let subject = subject.trim();
        if subject.starts_with('[') {
            if let Some(end) = subject.find(']') {
                let agent = &subject[1..end];
                if !agent.is_empty() {
                    return Some(agent.to_string());
                }
            }
        }
        None
    }

    /// Strip the agent tag from a subject line.
    fn strip_agent_tag(subject: &str) -> String {
        let subject = subject.trim();
        if subject.starts_with('[') {
            if let Some(end) = subject.find(']') {
                return subject[end + 1..].trim().to_string();
            }
        }
        subject.to_string()
    }

    /// Build an async SMTP transport for sending emails.
    async fn build_smtp_transport(
        &self,
    ) -> Result<AsyncSmtpTransport<Tokio1Executor>, Box<dyn std::error::Error + Send + Sync>> {
        if self.smtp_use_plain {
            // Test-only path: no TLS, no AUTH. Lets `send()` talk to a
            // hand-rolled in-process SMTP listener that doesn't speak
            // TLS. Production paths leave `smtp_use_plain == false`.
            return Ok(AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(
                &self.smtp_host,
            )
            .port(self.smtp_port)
            .build());
        }

        let creds = Credentials::new(self.username.clone(), self.password.as_str().to_string());

        let transport = if self.smtp_port == 465 {
            // Implicit TLS (port 465)
            AsyncSmtpTransport::<Tokio1Executor>::relay(&self.smtp_host)?
                .port(self.smtp_port)
                .credentials(creds)
                .build()
        } else {
            // STARTTLS (port 587 or other)
            AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&self.smtp_host)?
                .port(self.smtp_port)
                .credentials(creds)
                .build()
        };

        Ok(transport)
    }
}

/// Extract `user@domain` from a potentially formatted email string like `"Name <user@domain>"`.
fn extract_email_addr(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(start) = raw.find('<') {
        if let Some(end) = raw.find('>') {
            if end > start {
                return raw[start + 1..end].trim().to_string();
            }
        }
    }
    raw.to_string()
}

/// Exact-address or `@domain` allowlist match (no substring — fixes #3463).
fn sender_matches_allowlist(sender: &str, allowed: &[String]) -> bool {
    let addr = extract_email_addr(sender);
    let addr = addr.trim();
    let Some(at_idx) = addr.rfind('@') else {
        return false;
    };
    let domain = &addr[at_idx + 1..];
    if domain.is_empty() {
        return false;
    }
    for entry in allowed {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some(allowed_domain) = entry.strip_prefix('@') {
            if !allowed_domain.is_empty() && domain.eq_ignore_ascii_case(allowed_domain) {
                return true;
            }
        } else if addr.eq_ignore_ascii_case(entry) {
            return true;
        }
    }
    false
}

/// Get a specific header value from a parsed email.
fn get_header(parsed: &mailparse::ParsedMail<'_>, name: &str) -> Option<String> {
    parsed
        .headers
        .iter()
        .find(|h| h.get_key().eq_ignore_ascii_case(name))
        .map(|h| h.get_value())
}

/// Extract the text/plain body from a parsed email (handles multipart).
fn extract_text_body(parsed: &mailparse::ParsedMail<'_>) -> String {
    if parsed.subparts.is_empty() {
        return parsed.get_body().unwrap_or_default();
    }
    // Walk subparts looking for text/plain
    for part in &parsed.subparts {
        let ct = part.ctype.mimetype.to_lowercase();
        if ct == "text/plain" {
            return part.get_body().unwrap_or_default();
        }
    }
    // Fallback: first subpart body
    parsed
        .subparts
        .first()
        .and_then(|p| p.get_body().ok())
        .unwrap_or_default()
}

/// One successfully-parsed email pulled from IMAP, with the `(folder, uid)`
/// pair we need to flag it Seen / Quarantine after downstream processing.
struct FetchedEmail {
    folder: String,
    uid: u32,
    from_addr: String,
    subject: String,
    message_id: String,
    body: String,
}

/// UID outcome for `mark_uids_outcome`.
enum UidOutcome {
    /// Mark `\Seen` — the message was successfully delivered.
    Processed,
    /// Mark a custom `Librefang-Quarantine` keyword AND `\Seen` so the
    /// user's inbox shows it but the agent will ignore it on the next poll.
    Quarantined,
}

/// Fetch unseen emails from IMAP using blocking I/O.
///
/// IMPORTANT: This does NOT flag any UID as `\Seen`. The caller is responsible
/// for invoking `mark_uids_outcome` after successfully processing each
/// message — this prevents permanent message loss when parsing fails or a
/// sender is denied (issue #3481).
fn fetch_unseen_emails(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    folders: &[String],
) -> Result<Vec<FetchedEmail>, String> {
    let tcp = std::net::TcpStream::connect((host, port))
        .map_err(|e| format!("TCP connect failed: {e}"))?;
    let tls = rustls_connector::RustlsConnector::new_with_native_certs()
        .map_err(|e| format!("TLS connector error: {e}"))?;
    let tls_stream = tls
        .connect(host, tcp)
        .map_err(|e| format!("TLS handshake failed: {e}"))?;

    let client = imap::Client::new(tls_stream);

    // Try LOGIN first; fall back to AUTHENTICATE PLAIN for servers like Lark
    // that reject LOGIN and only support AUTH=PLAIN (SASL).
    let mut session = match client.login(username, password) {
        Ok(s) => s,
        Err((login_err, client)) => {
            let authenticator = PlainAuthenticator {
                username: username.to_string(),
                password: password.to_string(),
            };
            client
                .authenticate("PLAIN", &authenticator)
                .map_err(|(e, _)| {
                    format!("IMAP login failed: {login_err}; AUTH=PLAIN also failed: {e}")
                })?
        }
    };

    let mut results = Vec::new();

    for folder in folders {
        if let Err(e) = session.select(folder) {
            warn!(folder, error = %e, "IMAP SELECT failed, skipping folder");
            continue;
        }

        // Skip messages already quarantined by a previous poll. Fall back to
        // plain UNSEEN if the server rejects custom keyword search.
        let uids = match session.uid_search("UNSEEN UNKEYWORD Librefang-Quarantine") {
            Ok(uids) => uids,
            Err(_) => match session.uid_search("UNSEEN") {
                Ok(uids) => uids,
                Err(e) => {
                    warn!(folder, error = %e, "IMAP SEARCH UNSEEN failed");
                    continue;
                }
            },
        };

        if uids.is_empty() {
            debug!(folder, "No unseen emails");
            continue;
        }

        // Fetch in batches of up to 50 to avoid huge responses
        let uid_list: Vec<u32> = uids.into_iter().take(50).collect();
        let uid_set: String = uid_list
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let fetches = match session.uid_fetch(&uid_set, "(UID RFC822)") {
            Ok(f) => f,
            Err(e) => {
                warn!(folder, error = %e, "IMAP FETCH failed");
                continue;
            }
        };

        // Track which UIDs in this batch parsed successfully so we can
        // quarantine the rest — leaving them UNSEEN would loop forever on the
        // same poison pill, but marking Seen would silently drop them.
        let mut parsed_uids = std::collections::HashSet::new();

        for fetch in fetches.iter() {
            let Some(uid) = fetch.uid else { continue };
            let body_bytes = match fetch.body() {
                Some(b) => b,
                None => continue,
            };

            let parsed = match mailparse::parse_mail(body_bytes) {
                Ok(p) => p,
                Err(e) => {
                    warn!(folder, uid, error = %e, "Failed to parse email — quarantining");
                    continue;
                }
            };

            let from = get_header(&parsed, "From").unwrap_or_default();
            let subject = get_header(&parsed, "Subject").unwrap_or_default();
            let message_id = get_header(&parsed, "Message-ID").unwrap_or_default();
            let text_body = extract_text_body(&parsed);

            let from_addr = extract_email_addr(&from);
            parsed_uids.insert(uid);
            results.push(FetchedEmail {
                folder: folder.clone(),
                uid,
                from_addr,
                subject,
                message_id,
                body: text_body,
            });
        }

        // UIDs we couldn't parse → quarantine so we don't reprocess them
        // forever (but keep them visible to the user).
        let unparsed: Vec<u32> = uid_list
            .iter()
            .copied()
            .filter(|u| !parsed_uids.contains(u))
            .collect();
        if !unparsed.is_empty() {
            mark_uids_on_session(&mut session, &unparsed, UidOutcome::Quarantined);
        }
    }

    let _ = session.logout();
    Ok(results)
}

/// Apply a UID outcome on an already-selected mailbox session.
fn mark_uids_on_session<T: std::io::Read + std::io::Write>(
    session: &mut imap::Session<T>,
    uids: &[u32],
    outcome: UidOutcome,
) {
    if uids.is_empty() {
        return;
    }
    let uid_set: String = uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let flags = match outcome {
        UidOutcome::Processed => "+FLAGS (\\Seen)",
        UidOutcome::Quarantined => "+FLAGS (\\Seen Librefang-Quarantine)",
    };
    if let Err(e) = session.uid_store(&uid_set, flags) {
        warn!(uids = %uid_set, error = %e, "Failed to update IMAP flags");
    }
}

/// Mark a set of `(folder, uid)` pairs with the given outcome on a fresh IMAP
/// session — used by the poller after downstream processing decides whether
/// each message was delivered or denied.
fn mark_uids_outcome(
    host: &str,
    port: u16,
    username: &str,
    password: &str,
    items: Vec<(String, u32, UidOutcome)>,
) -> Result<(), String> {
    if items.is_empty() {
        return Ok(());
    }
    let tcp = std::net::TcpStream::connect((host, port))
        .map_err(|e| format!("TCP connect failed: {e}"))?;
    let tls = rustls_connector::RustlsConnector::new_with_native_certs()
        .map_err(|e| format!("TLS connector error: {e}"))?;
    let tls_stream = tls
        .connect(host, tcp)
        .map_err(|e| format!("TLS handshake failed: {e}"))?;
    let client = imap::Client::new(tls_stream);
    let mut session = match client.login(username, password) {
        Ok(s) => s,
        Err((login_err, client)) => {
            let authenticator = PlainAuthenticator {
                username: username.to_string(),
                password: password.to_string(),
            };
            client
                .authenticate("PLAIN", &authenticator)
                .map_err(|(e, _)| {
                    format!("IMAP login failed: {login_err}; AUTH=PLAIN also failed: {e}")
                })?
        }
    };

    // Group by folder so we can SELECT once per folder.
    use std::collections::BTreeMap;
    let mut by_folder: BTreeMap<String, (Vec<u32>, Vec<u32>)> = BTreeMap::new();
    for (folder, uid, outcome) in items {
        let entry = by_folder.entry(folder).or_default();
        match outcome {
            UidOutcome::Processed => entry.0.push(uid),
            UidOutcome::Quarantined => entry.1.push(uid),
        }
    }

    for (folder, (processed, quarantined)) in by_folder {
        if let Err(e) = session.select(&folder) {
            warn!(folder, error = %e, "IMAP SELECT failed during flag update");
            continue;
        }
        mark_uids_on_session(&mut session, &processed, UidOutcome::Processed);
        mark_uids_on_session(&mut session, &quarantined, UidOutcome::Quarantined);
    }

    let _ = session.logout();
    Ok(())
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    fn name(&self) -> &str {
        "email"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Email
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let poll_interval = self.poll_interval;
        let imap_host = self.imap_host.clone();
        let imap_port = self.imap_port;
        let username = self.username.clone();
        let password = self.password.clone();
        let folders = self.folders.clone();
        let allowed_senders = self.allowed_senders.clone();
        let mut shutdown_rx = self.shutdown_rx.clone();
        let reply_ctx = self.reply_ctx.clone();
        let account_id = self.account_id.clone();

        info!(
            "Starting email adapter (IMAP: {}:{}, SMTP: {}:{}, polling every {:?})",
            imap_host, imap_port, self.smtp_host, self.smtp_port, poll_interval
        );

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        info!("Email adapter shutting down");
                        break;
                    }
                    _ = tokio::time::sleep(poll_interval) => {}
                }

                // IMAP operations are blocking I/O — run in spawn_blocking
                let host = imap_host.clone();
                let port = imap_port;
                let user = username.clone();
                let pass = password.clone();
                let fldrs = folders.clone();

                let emails = tokio::task::spawn_blocking(move || {
                    fetch_unseen_emails(&host, port, &user, pass.as_str(), &fldrs)
                })
                .await;

                let emails = match emails {
                    Ok(Ok(emails)) => emails,
                    Ok(Err(e)) => {
                        error!("IMAP poll error: {e}");
                        continue;
                    }
                    Err(e) => {
                        error!("IMAP spawn_blocking panic: {e}");
                        continue;
                    }
                };

                let mut flag_updates: Vec<(String, u32, UidOutcome)> = Vec::new();
                for FetchedEmail {
                    folder,
                    uid,
                    from_addr,
                    subject,
                    message_id,
                    body,
                } in emails
                {
                    // Exact-match allowlist (substring match would let
                    // evil-trusted.com bypass). Denied senders go to
                    // quarantine (#3481): marking Seen would silently drop
                    // them, but leaving UNSEEN would loop forever.
                    if !allowed_senders.is_empty()
                        && !sender_matches_allowlist(&from_addr, &allowed_senders)
                    {
                        debug!(from = %from_addr, "Email from non-allowed sender, quarantining");
                        flag_updates.push((folder, uid, UidOutcome::Quarantined));
                        continue;
                    }

                    // Store reply context for threading
                    if !message_id.is_empty() {
                        reply_ctx.insert(
                            from_addr.clone(),
                            ReplyCtx {
                                subject: subject.clone(),
                                message_id: message_id.clone(),
                            },
                        );
                    }

                    // Extract target agent from subject brackets (stored in metadata for router)
                    let _target_agent = EmailAdapter::extract_agent_from_subject(&subject);
                    let clean_subject = EmailAdapter::strip_agent_tag(&subject);

                    // Build the message body: prepend subject context
                    let text = if clean_subject.is_empty() {
                        body.trim().to_string()
                    } else {
                        format!("Subject: {clean_subject}\n\n{}", body.trim())
                    };

                    let mut msg = ChannelMessage {
                        channel: ChannelType::Email,
                        platform_message_id: message_id.clone(),
                        sender: ChannelUser {
                            platform_id: from_addr.clone(),
                            display_name: from_addr.clone(),
                            librefang_user: None,
                        },
                        content: ChannelContent::Text(text),
                        target_agent: None, // Routing handled by bridge AgentRouter
                        timestamp: Utc::now(),
                        is_group: false,
                        thread_id: None,
                        metadata: std::collections::HashMap::new(),
                    };

                    // Inject account_id for multi-bot routing
                    if let Some(ref aid) = account_id {
                        msg.metadata
                            .insert("account_id".to_string(), serde_json::json!(aid));
                    }
                    if tx.send(msg).await.is_err() {
                        info!("Email channel receiver dropped, stopping poll");
                        // Best-effort flush of accumulated flag updates.
                        if !flag_updates.is_empty() {
                            let h = imap_host.clone();
                            let u = username.clone();
                            let p = password.clone();
                            let updates = std::mem::take(&mut flag_updates);
                            let _ = tokio::task::spawn_blocking(move || {
                                mark_uids_outcome(&h, imap_port, &u, p.as_str(), updates)
                            })
                            .await;
                        }
                        return;
                    }
                    // Successfully delivered to bridge — safe to mark Seen.
                    flag_updates.push((folder, uid, UidOutcome::Processed));
                }

                // Apply flag updates in a fresh blocking call.
                if !flag_updates.is_empty() {
                    let h = imap_host.clone();
                    let u = username.clone();
                    let p = password.clone();
                    let updates = std::mem::take(&mut flag_updates);
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        mark_uids_outcome(&h, imap_port, &u, p.as_str(), updates)
                    })
                    .await
                    .unwrap_or_else(|join_err| Err(format!("spawn_blocking panic: {join_err}")))
                    {
                        warn!("Failed to apply IMAP flag updates: {e}");
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
                // Parse recipient address
                let to_addr = extract_email_addr(&user.platform_id);
                let to_mailbox: Mailbox = to_addr
                    .parse()
                    .map_err(|e| format!("Invalid recipient email '{}': {}", to_addr, e))?;

                let from_mailbox: Mailbox = self
                    .username
                    .parse()
                    .map_err(|e| format!("Invalid sender email '{}': {}", self.username, e))?;

                // Extract subject from text body convention: "Subject: ...\n\n..."
                let (subject, body) = if text.starts_with("Subject: ") {
                    if let Some(pos) = text.find("\n\n") {
                        let subj = text[9..pos].trim().to_string();
                        let body = text[pos + 2..].to_string();
                        (subj, body)
                    } else {
                        ("LibreFang Reply".to_string(), text)
                    }
                } else {
                    // Check reply context for subject continuity
                    let subj = self
                        .reply_ctx
                        .get(&to_addr)
                        .map(|ctx| format!("Re: {}", ctx.subject))
                        .unwrap_or_else(|| "LibreFang Reply".to_string());
                    (subj, text)
                };

                // Build email message
                let mut builder = lettre::Message::builder()
                    .from(from_mailbox)
                    .to(to_mailbox)
                    .subject(&subject);

                // Add In-Reply-To header for threading
                if let Some(ctx) = self.reply_ctx.get(&to_addr) {
                    if !ctx.message_id.is_empty() {
                        builder = builder.in_reply_to(ctx.message_id.clone());
                    }
                }

                let email = builder
                    .body(body)
                    .map_err(|e| format!("Failed to build email: {e}"))?;

                // Send via SMTP
                let transport = self.build_smtp_transport().await?;
                transport
                    .send(email)
                    .await
                    .map_err(|e| format!("SMTP send failed: {e}"))?;

                info!(
                    to = %to_addr,
                    subject = %subject,
                    "Email sent successfully via SMTP"
                );
            }
            _ => {
                warn!(
                    "Unsupported email content type for {}, only text is supported",
                    user.platform_id
                );
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

    #[test]
    fn test_email_adapter_creation() {
        let adapter = EmailAdapter::new(
            "imap.gmail.com".to_string(),
            993,
            "smtp.gmail.com".to_string(),
            587,
            "user@gmail.com".to_string(),
            "password".to_string(),
            30,
            vec![],
            vec![],
        );
        assert_eq!(adapter.name(), "email");
        assert_eq!(adapter.folders, vec!["INBOX".to_string()]);
    }

    #[test]
    fn test_allowed_senders() {
        let adapter = EmailAdapter::new(
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "bot@example.com".to_string(),
            "pass".to_string(),
            30,
            vec![],
            vec!["boss@company.com".to_string()],
        );
        assert!(adapter.is_allowed_sender("boss@company.com"));
        assert!(!adapter.is_allowed_sender("random@other.com"));

        let open = EmailAdapter::new(
            "imap.example.com".to_string(),
            993,
            "smtp.example.com".to_string(),
            587,
            "bot@example.com".to_string(),
            "pass".to_string(),
            30,
            vec![],
            vec![],
        );
        assert!(open.is_allowed_sender("anyone@anywhere.com"));
    }

    #[test]
    fn test_allowed_senders_domain_exact_match() {
        let allowed = vec!["@example.com".to_string()];
        // Exact domain match passes
        assert!(sender_matches_allowlist("alice@example.com", &allowed));
        assert!(sender_matches_allowlist("ALICE@EXAMPLE.COM", &allowed));
        assert!(sender_matches_allowlist(
            "Alice <alice@example.com>",
            &allowed
        ));
        // Sibling-domain spoofing is rejected (the original substring bug).
        assert!(!sender_matches_allowlist(
            "attacker@example.com.evil.com",
            &allowed
        ));
        assert!(!sender_matches_allowlist(
            "attacker@notexample.com",
            &allowed
        ));
        assert!(!sender_matches_allowlist("attacker@evil.com", &allowed));
        assert!(!sender_matches_allowlist("malformed", &allowed));
        assert!(!sender_matches_allowlist("trailing@", &allowed));
    }

    #[test]
    fn test_allowed_senders_full_address_exact_match() {
        let allowed = vec!["alice@example.com".to_string()];
        assert!(sender_matches_allowlist("alice@example.com", &allowed));
        assert!(sender_matches_allowlist("ALICE@example.com", &allowed));
        assert!(!sender_matches_allowlist(
            "alice@example.com.evil.com",
            &allowed
        ));
        assert!(!sender_matches_allowlist("bob@example.com", &allowed));
        assert!(!sender_matches_allowlist(
            "alice+spoof@example.com",
            &allowed
        ));
    }

    #[test]
    fn test_allowed_senders_mixed_entries() {
        let allowed = vec!["@example.com".to_string(), "bob@partner.com".to_string()];
        assert!(sender_matches_allowlist("anyone@example.com", &allowed));
        assert!(sender_matches_allowlist("bob@partner.com", &allowed));
        assert!(!sender_matches_allowlist("alice@partner.com", &allowed));
        assert!(!sender_matches_allowlist(
            "bob@partner.com.evil.com",
            &allowed
        ));
    }

    #[test]
    fn test_extract_agent_from_subject() {
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("[coder] Fix the bug"),
            Some("coder".to_string())
        );
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("[researcher] Find papers on AI"),
            Some("researcher".to_string())
        );
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("No brackets here"),
            None
        );
        assert_eq!(
            EmailAdapter::extract_agent_from_subject("[] Empty brackets"),
            None
        );
    }

    #[test]
    fn test_strip_agent_tag() {
        assert_eq!(
            EmailAdapter::strip_agent_tag("[coder] Fix the bug"),
            "Fix the bug"
        );
        assert_eq!(EmailAdapter::strip_agent_tag("No brackets"), "No brackets");
    }

    #[test]
    fn test_extract_email_addr() {
        assert_eq!(
            extract_email_addr("John Doe <john@example.com>"),
            "john@example.com"
        );
        assert_eq!(extract_email_addr("user@example.com"), "user@example.com");
        assert_eq!(extract_email_addr("<user@test.com>"), "user@test.com");
    }

    #[test]
    fn test_subject_extraction_from_body() {
        let text = "Subject: Test Subject\n\nThis is the body.";
        assert!(text.starts_with("Subject: "));
        let pos = text.find("\n\n").unwrap();
        let subject = &text[9..pos];
        let body = &text[pos + 2..];
        assert_eq!(subject, "Test Subject");
        assert_eq!(body, "This is the body.");
    }

    #[test]
    fn test_reply_ctx_threading() {
        let ctx_map: DashMap<String, ReplyCtx> = DashMap::new();
        ctx_map.insert(
            "user@test.com".to_string(),
            ReplyCtx {
                subject: "Original Subject".to_string(),
                message_id: "<msg-123@test.com>".to_string(),
            },
        );
        let ctx = ctx_map.get("user@test.com").unwrap();
        assert_eq!(ctx.subject, "Original Subject");
        assert_eq!(ctx.message_id, "<msg-123@test.com>");
    }

    // ----- send() path tests (issue #3820) -----
    //
    // Email's outbound path goes through `lettre::AsyncSmtpTransport`
    // and a real SMTP handshake. Faking that boundary requires either a
    // trait abstraction over `AsyncTransport` or an in-process SMTP
    // server (e.g. `mailhog` via `testcontainers-rs`); both are larger
    // architectural changes than this PR's wiremock-coverage scope.
    //
    // What we *can* pin without standing up a real SMTP server is the
    // input-validation path that runs before any TCP I/O: an
    // unparseable recipient address must surface as an Err before the
    // adapter touches the network. That is the early-return contract
    // future SMTP-fixture tests will rely on.

    fn email_user(addr: &str) -> ChannelUser {
        ChannelUser {
            platform_id: addr.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn email_send_returns_err_for_invalid_recipient_before_smtp_io() {
        // Use a syntactically invalid email so `to_addr.parse::<Mailbox>()`
        // bails inside `send()` before any SMTP handshake is attempted.
        // The SMTP host below is purposefully unreachable; if the
        // pre-flight parse check were ever removed, this test would
        // start hanging on DNS and the regression would be visible.
        let adapter = EmailAdapter::new(
            "imap.invalid.tld".to_string(),
            993,
            "smtp.invalid.tld".to_string(),
            587,
            "bot@example.com".to_string(),
            "password".to_string(),
            30,
            vec![],
            vec![],
        );

        let err = adapter
            .send(
                &email_user("not-an-email-at-all"),
                ChannelContent::Text("x".into()),
            )
            .await
            .expect_err("email send must reject malformed recipient before SMTP");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("invalid recipient"),
            "error should mention invalid recipient, got: {err}"
        );
    }

    // -- happy-path against a hand-rolled plain-SMTP fixture --
    //
    // No public crate ships a "tap-the-DATA-body" mock for `lettre`.
    // The fixture below speaks just enough SMTP to satisfy lettre's
    // builder_dangerous transport: 220 banner → EHLO 250 (no STARTTLS,
    // no AUTH so lettre doesn't try to upgrade) → MAIL FROM 250 →
    // RCPT TO 250 → DATA 354 → capture body until `\r\n.\r\n` → 250 →
    // QUIT 221 → close. The captured `(from, recipient, body)` tuple
    // is forwarded through a oneshot and asserted against what
    // `EmailAdapter::send()` was asked to emit.

    #[tokio::test]
    async fn email_send_writes_rfc5322_message_through_smtp_fixture() {
        use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
        use tokio::net::TcpListener;
        use tokio::sync::oneshot;

        #[derive(Debug)]
        struct CapturedSmtp {
            mail_from: String,
            rcpt_to: String,
            data: String,
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = addr.ip().to_string();
        let port = addr.port();

        let (tx, rx) = oneshot::channel::<CapturedSmtp>();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let (r, mut w) = stream.into_split();
            let mut reader = BufReader::new(r);

            // Greet.
            w.write_all(b"220 mock.smtp ESMTP\r\n").await.unwrap();

            let mut mail_from = String::new();
            let mut rcpt_to = String::new();
            let mut data_body = String::new();
            let mut tx_opt = Some(tx);

            loop {
                let mut line = String::new();
                let n = reader.read_line(&mut line).await.unwrap();
                if n == 0 {
                    return;
                }
                let trimmed = line.trim_end_matches(['\r', '\n']);
                let upper = trimmed.to_uppercase();
                if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                    // multi-line 250: do NOT advertise STARTTLS or AUTH.
                    w.write_all(b"250-mock.smtp\r\n").await.unwrap();
                    w.write_all(b"250-SIZE 65536\r\n").await.unwrap();
                    w.write_all(b"250 8BITMIME\r\n").await.unwrap();
                } else if upper.starts_with("MAIL FROM:") {
                    mail_from = trimmed.to_string();
                    w.write_all(b"250 OK\r\n").await.unwrap();
                } else if upper.starts_with("RCPT TO:") {
                    rcpt_to = trimmed.to_string();
                    w.write_all(b"250 OK\r\n").await.unwrap();
                } else if upper == "DATA" {
                    w.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n")
                        .await
                        .unwrap();
                    // Read body lines until we see "\r\n.\r\n". Each
                    // SMTP body line is a discrete \r\n-terminated read.
                    loop {
                        let mut body_line = String::new();
                        let n = reader.read_line(&mut body_line).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        if body_line == ".\r\n" {
                            break;
                        }
                        data_body.push_str(&body_line);
                    }
                    w.write_all(b"250 OK: queued as MOCK-1\r\n").await.unwrap();
                    if let Some(tx) = tx_opt.take() {
                        let _ = tx.send(CapturedSmtp {
                            mail_from: mail_from.clone(),
                            rcpt_to: rcpt_to.clone(),
                            data: data_body.clone(),
                        });
                    }
                } else if upper.starts_with("QUIT") {
                    w.write_all(b"221 Bye\r\n").await.unwrap();
                    // Drain the rest of the connection so the client
                    // sees the QUIT response before EOF.
                    let mut sink = Vec::new();
                    let _ = reader.read_to_end(&mut sink).await;
                    return;
                } else if upper.starts_with("RSET") || upper.starts_with("NOOP") {
                    w.write_all(b"250 OK\r\n").await.unwrap();
                } else {
                    // Unknown verb — keep talking.
                    w.write_all(b"250 OK\r\n").await.unwrap();
                }
            }
        });

        let adapter = EmailAdapter::new(
            "imap.invalid.tld".to_string(),
            993,
            host,
            port,
            "bot@example.com".to_string(),
            "password".to_string(),
            30,
            vec![],
            vec![],
        )
        .with_plain_smtp();

        adapter
            .send(
                &email_user("alice@example.com"),
                ChannelContent::Text("Subject: Test Message\n\nhello smtp".into()),
            )
            .await
            .expect("email send must succeed against plain-SMTP fixture");

        let captured = tokio::time::timeout(Duration::from_secs(5), rx)
            .await
            .expect("fixture must capture DATA body within 5s")
            .expect("oneshot must not be dropped");

        assert!(
            captured.mail_from.contains("bot@example.com"),
            "MAIL FROM must reference the configured sender, got: {}",
            captured.mail_from
        );
        assert!(
            captured.rcpt_to.contains("alice@example.com"),
            "RCPT TO must reference the recipient, got: {}",
            captured.rcpt_to
        );
        assert!(
            captured.data.contains("Subject: Test Message"),
            "DATA body must carry the subject, got: ---\n{}\n---",
            captured.data
        );
        assert!(
            captured.data.contains("hello smtp"),
            "DATA body must carry the message body, got: ---\n{}\n---",
            captured.data
        );

        server.abort();
    }
}
