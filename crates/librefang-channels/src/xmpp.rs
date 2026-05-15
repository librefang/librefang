//! XMPP channel adapter (stub).
//!
//! This is a stub adapter for XMPP/Jabber messaging. A full XMPP implementation
//! requires the `tokio-xmpp` crate (or equivalent) for proper SASL authentication,
//! TLS negotiation, XML stream parsing, and MUC (Multi-User Chat) support.
//!
//! The adapter struct is fully defined so it can be constructed and configured, but
//! `start()` returns an error explaining that the `tokio-xmpp` dependency is needed.
//! This allows the adapter to be wired into the channel system without adding
//! heavyweight dependencies to the workspace.

use crate::types::{ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser};
use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::warn;
use zeroize::Zeroizing;

/// XMPP/Jabber channel adapter (stub implementation).
///
/// Holds all configuration needed for a full XMPP client but defers actual
/// connection to when the `tokio-xmpp` dependency is added.
pub struct XmppAdapter {
    /// JID (Jabber ID) of the bot (e.g., "bot@example.com").
    jid: String,
    /// SECURITY: Password is zeroized on drop.
    #[allow(dead_code)]
    password: Zeroizing<String>,
    /// XMPP server hostname.
    server: String,
    /// XMPP server port (default 5222 for STARTTLS, 5223 for direct TLS).
    port: u16,
    /// MUC rooms to join (e.g., "room@conference.example.com").
    rooms: Vec<String>,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Shutdown signal.
    shutdown_tx: Arc<watch::Sender<bool>>,
    #[allow(dead_code)]
    shutdown_rx: watch::Receiver<bool>,
}

impl XmppAdapter {
    /// Create a new XMPP adapter.
    ///
    /// # Arguments
    /// * `jid` - Full JID of the bot (user@domain).
    /// * `password` - XMPP account password.
    /// * `server` - Server hostname (may differ from JID domain).
    /// * `port` - Server port (typically 5222).
    /// * `rooms` - MUC room JIDs to auto-join.
    pub fn new(
        jid: String,
        password: String,
        server: String,
        port: u16,
        rooms: Vec<String>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            jid,
            password: Zeroizing::new(password),
            server,
            port,
            rooms,
            account_id: None,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }
    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Get the bare JID (without resource).
    #[allow(dead_code)]
    pub fn bare_jid(&self) -> &str {
        self.jid.split('/').next().unwrap_or(&self.jid)
    }

    /// Get the configured server endpoint.
    #[allow(dead_code)]
    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.server, self.port)
    }

    /// Get the list of configured rooms.
    #[allow(dead_code)]
    pub fn rooms(&self) -> &[String] {
        &self.rooms
    }
}

#[async_trait]
impl ChannelAdapter for XmppAdapter {
    fn name(&self) -> &str {
        "xmpp"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Custom("xmpp".to_string())
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        warn!(
            "XMPP adapter for {}@{}:{} cannot start: \
             full XMPP support requires the tokio-xmpp dependency which is not \
             currently included in the workspace. Add tokio-xmpp to Cargo.toml \
             and implement the SASL/TLS/XML stream handling to enable this adapter.",
            self.jid, self.server, self.port
        );

        Err(format!(
            "XMPP adapter requires tokio-xmpp dependency (not yet added to workspace). \
             Configured for JID '{}' on {}:{} with {} room(s).",
            self.jid,
            self.server,
            self.port,
            self.rooms.len()
        )
        .into())
    }

    async fn send(
        &self,
        _user: &ChannelUser,
        _content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Err("XMPP adapter not started: tokio-xmpp dependency required".into())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    /// Expose the configured multi-bot `account_id` (typically the XMPP
    /// JID / resource identifier) so the bridge approval listener builds
    /// the same `xmpp:<account_id>` key the router stores in
    /// `channel_defaults`, scoping ApprovalRequested delivery to the JID
    /// bound to the requesting agent (#5003, follow-up to #4985 / #4994).
    fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xmpp_adapter_creation() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "secret-password".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec!["room@conference.example.com".to_string()],
        );
        assert_eq!(adapter.name(), "xmpp");
        assert_eq!(
            adapter.channel_type(),
            ChannelType::Custom("xmpp".to_string())
        );
    }

    #[test]
    fn test_xmpp_bare_jid() {
        let adapter = XmppAdapter::new(
            "bot@example.com/resource".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec![],
        );
        assert_eq!(adapter.bare_jid(), "bot@example.com");

        let adapter_no_resource = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec![],
        );
        assert_eq!(adapter_no_resource.bare_jid(), "bot@example.com");
    }

    #[test]
    fn test_xmpp_endpoint() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec![],
        );
        assert_eq!(adapter.endpoint(), "xmpp.example.com:5222");
    }

    #[test]
    fn test_xmpp_rooms() {
        let rooms = vec![
            "room1@conference.example.com".to_string(),
            "room2@conference.example.com".to_string(),
        ];
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            rooms.clone(),
        );
        assert_eq!(adapter.rooms(), &rooms);
    }

    #[tokio::test]
    async fn test_xmpp_start_returns_error() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec!["room@conference.example.com".to_string()],
        );
        let result = adapter.start().await;
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("tokio-xmpp"));
    }

    #[tokio::test]
    async fn test_xmpp_send_returns_error() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec![],
        );
        let user = ChannelUser {
            platform_id: "user@example.com".to_string(),
            display_name: "Test User".to_string(),
            librefang_user: None,
        };
        let result = adapter
            .send(&user, ChannelContent::Text("hello".to_string()))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn test_xmpp_password_zeroized() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "my-secret-pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec![],
        );
        // Verify accessible before drop (zeroized on drop)
        assert_eq!(adapter.password.as_str(), "my-secret-pass");
    }

    #[test]
    fn test_xmpp_custom_port() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5223,
            vec![],
        );
        assert_eq!(adapter.port, 5223);
        assert_eq!(adapter.endpoint(), "xmpp.example.com:5223");
    }

    // ----- send() path tests (issue #3820) -----
    //
    // The XMPP adapter is intentionally a stub today (`send()` returns
    // a contract-pinning Err naming the missing tokio-xmpp dependency).
    // The existing `test_send_returns_error` already asserts `is_err()`;
    // this extra case pins the **error message**, which callers and
    // dashboards may surface verbatim — so a future implementation
    // cannot silently change the wording while still being a no-op.

    #[tokio::test]
    async fn xmpp_send_error_message_pins_tokio_xmpp_pending_marker() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "xmpp.example.com".to_string(),
            5222,
            vec![],
        );
        let user = ChannelUser {
            platform_id: "alice@example.com".to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        };
        let err = adapter
            .send(&user, ChannelContent::Text("ignored — stub".into()))
            .await
            .expect_err("xmpp send must remain a stub Err until tokio-xmpp lands");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("not started") && msg.contains("tokio-xmpp"),
            "stub error must mention not-started + tokio-xmpp, got: {err}"
        );
    }

    #[test]
    fn test_xmpp_account_id_default_none() {
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "example.com".to_string(),
            5222,
            vec![],
        );
        assert_eq!(adapter.account_id(), None);
    }

    #[test]
    fn test_xmpp_account_id_returns_configured_value() {
        // #5003: two XMPP JIDs must resolve under distinct
        // `xmpp:<account_id>` keys via the trait override.
        let adapter = XmppAdapter::new(
            "bot@example.com".to_string(),
            "pass".to_string(),
            "example.com".to_string(),
            5222,
            vec![],
        )
        .with_account_id(Some("jid-42".to_string()));
        assert_eq!(adapter.account_id(), Some("jid-42"));
    }
}
