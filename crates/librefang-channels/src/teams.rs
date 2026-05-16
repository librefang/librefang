//! Microsoft Teams channel adapter for the LibreFang channel bridge.
//!
//! Uses Bot Framework v3 REST API for sending messages and a lightweight axum
//! HTTP webhook server for receiving inbound activities. OAuth2 client credentials
//! flow is used to obtain and cache access tokens for outbound API calls.

use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use chrono::Utc;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};
use zeroize::Zeroizing;

/// Verify a Microsoft Teams outgoing-webhook HMAC-SHA256 signature.
///
/// The `Authorization` header carries `HMAC <base64-digest>`.
/// The expected digest is `Base64(HMAC-SHA256(security_token_bytes, raw_body))`.
///
/// `key_bytes` is the *decoded* security-token raw key — Teams provides the
/// token base64-encoded in the portal, but `TeamsAdapter::new` decodes it
/// once at construction so verify path stays hot-loop-cheap.
fn verify_teams_signature(key_bytes: &[u8], body: &[u8], auth_header: &str) -> bool {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let claimed_b64 = match auth_header.strip_prefix("HMAC ") {
        Some(s) => s.trim(),
        None => return false,
    };

    let Ok(claimed_bytes) = base64::engine::general_purpose::STANDARD.decode(claimed_b64) else {
        warn!("Teams: invalid base64 in Authorization header");
        return false;
    };

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(key_bytes) else {
        warn!("Teams: failed to create HMAC-SHA256 instance");
        return false;
    };
    mac.update(body);
    let result = mac.finalize().into_bytes();

    crate::http_client::ct_eq(&result, &claimed_bytes)
}

/// OAuth2 token endpoint for Bot Framework.
const OAUTH_TOKEN_URL: &str =
    "https://login.microsoftonline.com/botframework.com/oauth2/v2.0/token";

/// Default Bot Framework service URL used when no per-conversation URL is available.
const DEFAULT_SERVICE_URL: &str = "https://smba.trafficmanager.net/teams/";

/// Maximum Teams message length (characters).
const MAX_MESSAGE_LEN: usize = 4096;

/// OAuth2 token refresh buffer — refresh 5 minutes before actual expiry.
const TOKEN_REFRESH_BUFFER_SECS: u64 = 300;

/// Microsoft Teams Bot Framework v3 adapter.
///
/// Inbound messages arrive via an axum HTTP webhook on `POST /api/messages`.
/// Outbound messages are sent via the Bot Framework v3 REST API using a
/// cached OAuth2 bearer token (client credentials flow).
pub struct TeamsAdapter {
    /// Bot Framework App ID (also called "Microsoft App ID").
    app_id: String,
    /// SECURITY: App password is zeroized on drop to prevent memory disclosure.
    app_password: Zeroizing<String>,
    /// SECURITY: Decoded outgoing-webhook security-token raw bytes used as
    /// the HMAC-SHA256 key for inbound webhook verification.
    ///
    /// `None` means no token configured (verification is skipped, with a
    /// loud warning logged at construction). The base64 form from the
    /// Teams portal is decoded once in `new()`; misconfigurations
    /// (non-base64 input) collapse to `None` so the per-request hot path
    /// stays branch-free.
    security_token_key: Option<Zeroizing<Vec<u8>>>,
    /// Restrict inbound activities to specific Azure AD tenant IDs (empty = allow all).
    allowed_tenants: Vec<String>,
    /// HTTP client for outbound API calls.
    client: reqwest::Client,
    /// Optional account identifier for multi-bot routing.
    account_id: Option<String>,
    /// Cached OAuth2 bearer token and its expiry instant.
    cached_token: Arc<RwLock<Option<(String, Instant)>>>,
    /// OAuth2 token endpoint. Defaults to the Bot Framework Microsoft login URL.
    /// Overridable in tests via `with_oauth_token_url()` to point at a wiremock server.
    oauth_token_url: String,
    /// Bot Framework service URL used for outbound API calls when no per-conversation
    /// URL is available. Overridable in tests via `with_service_url()`.
    service_url: String,
}

impl TeamsAdapter {
    /// Create a new Teams adapter.
    ///
    /// * `app_id` — Bot Framework application ID.
    /// * `app_password` — Bot Framework application password (client secret).
    /// * `security_token` — Base64-encoded outgoing webhook security token from the
    ///   Teams portal. Used to verify HMAC-SHA256 signatures on inbound webhooks.
    ///   Pass an empty string to disable signature verification (logs a warning).
    /// * `allowed_tenants` — Azure AD tenant IDs to accept (empty = accept all).
    pub fn new(
        app_id: String,
        app_password: String,
        security_token: String,
        _webhook_port: u16,
        allowed_tenants: Vec<String>,
    ) -> Self {
        use base64::Engine;
        let security_token_key = if security_token.is_empty() {
            warn!(
                "Teams: no security_token configured — webhook signature \
                 verification is DISABLED. Set security_token_env to harden \
                 this endpoint."
            );
            None
        } else {
            match base64::engine::general_purpose::STANDARD.decode(&security_token) {
                Ok(bytes) => Some(Zeroizing::new(bytes)),
                Err(e) => {
                    warn!(
                        "Teams: configured security_token is not valid base64 \
                         ({e}); webhook signature verification is DISABLED."
                    );
                    None
                }
            }
        };
        Self {
            app_id,
            app_password: Zeroizing::new(app_password),
            security_token_key,
            allowed_tenants,
            client: crate::http_client::new_client(),
            account_id: None,
            cached_token: Arc::new(RwLock::new(None)),
            oauth_token_url: OAUTH_TOKEN_URL.to_string(),
            service_url: DEFAULT_SERVICE_URL.to_string(),
        }
    }

    /// Set the account_id for multi-bot routing. Returns self for builder chaining.
    pub fn with_account_id(mut self, account_id: Option<String>) -> Self {
        self.account_id = account_id;
        self
    }

    /// Override the OAuth2 token endpoint URL. Intended for tests that point the
    /// adapter at a wiremock server instead of the Microsoft login endpoint.
    #[cfg(test)]
    pub fn with_oauth_token_url(mut self, url: String) -> Self {
        self.oauth_token_url = url;
        self
    }

    /// Override the Bot Framework service URL. Intended for tests that point the
    /// adapter at a wiremock server instead of the default trafficmanager endpoint.
    #[cfg(test)]
    pub fn with_service_url(mut self, url: String) -> Self {
        self.service_url = url;
        self
    }

    /// Obtain a valid OAuth2 bearer token, refreshing if expired or missing.
    async fn get_token(&self) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        // Check cache first
        {
            let guard = self.cached_token.read().await;
            if let Some((ref token, expiry)) = *guard {
                if Instant::now() < expiry {
                    return Ok(token.clone());
                }
            }
        }

        // Fetch a new token via client credentials flow
        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", &self.app_id),
            ("client_secret", self.app_password.as_str()),
            ("scope", "https://api.botframework.com/.default"),
        ];

        let resp = self
            .client
            .post(&self.oauth_token_url)
            .form(&params)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Teams OAuth2 token error {status}: {body}").into());
        }

        let body: serde_json::Value = resp.json().await?;
        let access_token = body["access_token"]
            .as_str()
            .ok_or("Missing access_token in OAuth2 response")?
            .to_string();
        let expires_in = body["expires_in"].as_u64().unwrap_or(3600);

        // Cache with a safety buffer
        let expiry = Instant::now()
            + Duration::from_secs(expires_in.saturating_sub(TOKEN_REFRESH_BUFFER_SECS));
        *self.cached_token.write().await = Some((access_token.clone(), expiry));

        Ok(access_token)
    }

    /// Send a text reply to a Teams conversation via Bot Framework v3.
    ///
    /// * `service_url` — The per-conversation service URL provided in inbound activities.
    /// * `conversation_id` — The Teams conversation ID.
    /// * `text` — The message text to send.
    async fn api_send_message(
        &self,
        service_url: &str,
        conversation_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/v3/conversations/{}/activities",
            service_url.trim_end_matches('/'),
            conversation_id
        );

        let chunks = split_message(text, MAX_MESSAGE_LEN);
        for chunk in chunks {
            let body = serde_json::json!({
                "type": "message",
                "text": chunk,
            });

            let resp = self
                .client
                .post(&url)
                .bearer_auth(&token)
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let status = resp.status();
                let resp_body = resp.text().await.unwrap_or_default();
                warn!("Teams API error {status}: {resp_body}");
            }
        }

        Ok(())
    }

    /// Check whether a tenant ID is allowed (empty list = allow all).
    #[allow(dead_code)]
    fn is_allowed_tenant(&self, tenant_id: &str) -> bool {
        self.allowed_tenants.is_empty() || self.allowed_tenants.iter().any(|t| t == tenant_id)
    }
}

/// Parse an inbound Bot Framework activity JSON into a `ChannelMessage`.
///
/// Returns `None` for activities that should be ignored (non-message types,
/// activities from the bot itself, activities from disallowed tenants, etc.).
fn parse_teams_activity(
    activity: &serde_json::Value,
    app_id: &str,
    allowed_tenants: &[String],
) -> Option<ChannelMessage> {
    let activity_type = activity["type"].as_str().unwrap_or("");
    if activity_type != "message" {
        return None;
    }

    // Extract sender info
    let from = activity.get("from")?;
    let from_id = from["id"].as_str().unwrap_or("");
    let from_name = from["name"].as_str().unwrap_or("Unknown");

    // Skip messages from the bot itself
    if from_id == app_id {
        return None;
    }

    // Tenant filtering
    if !allowed_tenants.is_empty() {
        let tenant_id = activity["channelData"]["tenant"]["id"]
            .as_str()
            .unwrap_or("");
        if !allowed_tenants.iter().any(|t| t == tenant_id) {
            return None;
        }
    }

    let text = activity["text"].as_str().unwrap_or("");
    if text.is_empty() {
        return None;
    }

    let conversation_id = activity["conversation"]["id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let activity_id = activity["id"].as_str().unwrap_or("").to_string();
    let service_url = activity["serviceUrl"].as_str().unwrap_or("").to_string();

    // Determine if this is a group conversation
    let is_group = activity["conversation"]["isGroup"]
        .as_bool()
        .unwrap_or(false);

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

    let mut metadata = HashMap::new();
    // Store serviceUrl in metadata so outbound replies can use it
    if !service_url.is_empty() {
        metadata.insert(
            "serviceUrl".to_string(),
            serde_json::Value::String(service_url),
        );
    }

    Some(ChannelMessage {
        channel: ChannelType::Teams,
        platform_message_id: activity_id,
        sender: ChannelUser {
            platform_id: conversation_id,
            display_name: from_name.to_string(),
            librefang_user: None,
        },
        content,
        target_agent: None,
        timestamp: Utc::now(),
        is_group,
        thread_id: None,
        metadata,
    })
}

#[async_trait]
impl ChannelAdapter for TeamsAdapter {
    fn name(&self) -> &str {
        "teams"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Teams
    }

    async fn create_webhook_routes(
        &self,
    ) -> Option<(
        axum::Router,
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
    )> {
        // Verify credentials before registering routes
        if let Err(e) = self.get_token().await {
            tracing::error!("Teams adapter authentication failed: {e}");
            return None;
        }
        tracing::info!("Teams adapter authenticated (app_id: {})", self.app_id);

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);
        let tx = Arc::new(tx);
        let app_id = Arc::new(self.app_id.clone());
        let allowed_tenants = Arc::new(self.allowed_tenants.clone());
        let account_id = Arc::new(self.account_id.clone());
        // Clone the decoded HMAC key once for the route closure. `None`
        // means "no token configured" — verification was already warned
        // about at construction time, the request path stays silent.
        let key: Option<Arc<Zeroizing<Vec<u8>>>> = self
            .security_token_key
            .as_ref()
            .map(|k| Arc::new(k.clone()));

        let app = axum::Router::new().route(
            "/webhook",
            axum::routing::post({
                let app_id = Arc::clone(&app_id);
                let tenants = Arc::clone(&allowed_tenants);
                let tx = Arc::clone(&tx);
                let account_id = Arc::clone(&account_id);
                let key = key.clone();
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let app_id = Arc::clone(&app_id);
                    let tenants = Arc::clone(&tenants);
                    let tx = Arc::clone(&tx);
                    let account_id = Arc::clone(&account_id);
                    let key = key.clone();
                    async move {
                        if let Some(key) = key.as_ref() {
                            let Some(auth) =
                                headers.get("authorization").and_then(|v| v.to_str().ok())
                            else {
                                warn!("Teams: missing Authorization header");
                                return axum::http::StatusCode::BAD_REQUEST;
                            };
                            if !verify_teams_signature(key, &body, auth) {
                                warn!("Teams: invalid HMAC-SHA256 signature");
                                return axum::http::StatusCode::UNAUTHORIZED;
                            }
                        }

                        let json_body: serde_json::Value = match serde_json::from_slice(&body) {
                            Ok(v) => v,
                            Err(_) => return axum::http::StatusCode::BAD_REQUEST,
                        };

                        if let Some(mut msg) = parse_teams_activity(&json_body, &app_id, &tenants) {
                            // Inject account_id for multi-bot routing
                            if let Some(ref aid) = *account_id {
                                msg.metadata
                                    .insert("account_id".to_string(), serde_json::json!(aid));
                            }
                            let _ = tx.send(msg).await;
                        }
                        axum::http::StatusCode::OK
                    }
                }
            }),
        );

        info!("Teams adapter registered on shared server");

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
        // When using the shared webhook server, create_webhook_routes() is called
        // instead. This start() is only reached as a fallback (shouldn't happen
        // in normal operation since BridgeManager prefers create_webhook_routes).
        let (_tx, rx) = mpsc::channel::<ChannelMessage>(1);
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let conversation_id = &user.platform_id;

        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(&self.service_url, conversation_id, &text)
                    .await?;
            }
            _ => {
                self.api_send_message(
                    &self.service_url,
                    conversation_id,
                    "(Unsupported content type)",
                )
                .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(
        &self,
        user: &ChannelUser,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let token = self.get_token().await?;
        let url = format!(
            "{}/v3/conversations/{}/activities",
            self.service_url.trim_end_matches('/'),
            user.platform_id
        );

        let body = serde_json::json!({
            "type": "typing",
        });

        let _ = self
            .client
            .post(&url)
            .bearer_auth(&token)
            .json(&body)
            .send()
            .await;

        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    /// Expose the configured multi-bot `account_id` (typically the Teams
    /// tenant / app identifier) so the bridge approval listener builds the
    /// same `teams:<account_id>` key the router stores in `channel_defaults`,
    /// scoping ApprovalRequested delivery to the tenant bound to the
    /// requesting agent (#5003, follow-up to #4985 / #4994).
    fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- send() path tests (issue #3820) -----
    //
    // Uses `wiremock` to stand up a local HTTP server. The adapter is pointed at
    // the mock via `with_oauth_token_url()` (so the client-credentials token
    // fetch goes to wiremock) and `with_service_url()` (so the Bot Framework
    // activities POST goes to wiremock). This mirrors the pattern used for the
    // gotify slice (PR #4447) and the slack slice (PR #4545).

    use wiremock::matchers::{body_json, body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Returns a `TeamsAdapter` configured to talk to `server` for both OAuth
    /// token acquisition and Bot Framework API calls.
    async fn make_adapter(server: &MockServer) -> TeamsAdapter {
        TeamsAdapter::new(
            "test-app-id".to_string(),
            "test-app-password".to_string(),
            "".to_string(), // no signature verification in tests
            0,
            vec![],
        )
        .with_oauth_token_url(format!("{}/oauth2/token", server.uri()))
        .with_service_url(server.uri())
    }

    /// Mount a mock that returns a synthetic OAuth2 token. Every test that
    /// exercises `send()` must call `get_token()` first, so this must be
    /// mounted before the activities mock.
    async fn mount_token_mock(server: &MockServer) {
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .and(body_string_contains("client_credentials"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "test-bearer-token",
                "expires_in": 3600,
                "token_type": "Bearer",
            })))
            .mount(server)
            .await;
    }

    fn dummy_user(conversation_id: &str) -> ChannelUser {
        ChannelUser {
            platform_id: conversation_id.to_string(),
            display_name: "tester".to_string(),
            librefang_user: None,
        }
    }

    #[tokio::test]
    async fn teams_send_text_posts_message_activity() {
        let server = MockServer::start().await;
        mount_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/v3/conversations/conv-abc/activities"))
            .and(header("Authorization", "Bearer test-bearer-token"))
            .and(body_json(serde_json::json!({
                "type": "message",
                "text": "hello from librefang",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "activity-1",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(&server).await;
        adapter
            .send(
                &dummy_user("conv-abc"),
                ChannelContent::Text("hello from librefang".into()),
            )
            .await
            .expect("send must succeed against mock");
    }

    #[tokio::test]
    async fn teams_send_non_text_content_uses_placeholder() {
        let server = MockServer::start().await;
        mount_token_mock(&server).await;

        Mock::given(method("POST"))
            .and(path("/v3/conversations/conv-xyz/activities"))
            .and(header("Authorization", "Bearer test-bearer-token"))
            .and(body_json(serde_json::json!({
                "type": "message",
                "text": "(Unsupported content type)",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "activity-2",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let adapter = make_adapter(&server).await;
        adapter
            .send(
                &dummy_user("conv-xyz"),
                ChannelContent::Command {
                    name: "noop".into(),
                    args: vec![],
                },
            )
            .await
            .expect("send with unsupported content must succeed");
    }

    #[tokio::test]
    async fn teams_send_reuses_cached_oauth_token() {
        let server = MockServer::start().await;

        // Token endpoint should only be called once even though we send twice.
        Mock::given(method("POST"))
            .and(path("/oauth2/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "cached-token",
                "expires_in": 3600,
                "token_type": "Bearer",
            })))
            .expect(1) // exactly one token fetch for two send() calls
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v3/conversations/conv-cache/activities"))
            .and(header("Authorization", "Bearer cached-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "activity-3",
            })))
            .expect(2) // two messages sent
            .mount(&server)
            .await;

        let adapter = make_adapter(&server).await;
        adapter
            .send(
                &dummy_user("conv-cache"),
                ChannelContent::Text("first".into()),
            )
            .await
            .expect("first send must succeed");
        adapter
            .send(
                &dummy_user("conv-cache"),
                ChannelContent::Text("second".into()),
            )
            .await
            .expect("second send must succeed");
    }

    #[test]
    fn test_teams_adapter_creation() {
        let adapter = TeamsAdapter::new(
            "app-id-123".to_string(),
            "app-password".to_string(),
            "".to_string(),
            3978,
            vec![],
        );
        assert_eq!(adapter.name(), "teams");
        assert_eq!(adapter.channel_type(), ChannelType::Teams);
    }

    #[test]
    fn test_teams_allowed_tenants() {
        let adapter = TeamsAdapter::new(
            "app-id".to_string(),
            "password".to_string(),
            "".to_string(),
            3978,
            vec!["tenant-abc".to_string()],
        );
        assert!(adapter.is_allowed_tenant("tenant-abc"));
        assert!(!adapter.is_allowed_tenant("tenant-xyz"));

        let open = TeamsAdapter::new(
            "app-id".to_string(),
            "password".to_string(),
            "".to_string(),
            3978,
            vec![],
        );
        assert!(open.is_allowed_tenant("any-tenant"));
    }

    #[test]
    fn test_verify_teams_signature_valid() {
        use base64::Engine;
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let key_bytes: &[u8] = b"test-teams-key-bytes-16bytes!!xx";

        let body = b"teams webhook body";

        let mut mac = Hmac::<Sha256>::new_from_slice(key_bytes).unwrap();
        mac.update(body);
        let result = mac.finalize().into_bytes();
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(result);
        let auth_header = format!("HMAC {sig_b64}");

        assert!(verify_teams_signature(key_bytes, body, &auth_header));
    }

    #[test]
    fn test_verify_teams_signature_invalid() {
        let key_bytes: &[u8] = b"test-teams-key-bytes-16bytes!!xx";
        let body = b"teams webhook body";
        assert!(!verify_teams_signature(key_bytes, body, "HMAC badsig=="));
        assert!(!verify_teams_signature(key_bytes, body, "Bearer token"));
        assert!(!verify_teams_signature(key_bytes, body, ""));
    }

    /// Constructor must record the warning *and* leave verification disabled
    /// (key set to `None`) when the operator misconfigures the env var with
    /// non-base64 input. Otherwise the per-request path would silently fail.
    #[test]
    fn test_teams_invalid_base64_token_disables_verification() {
        let adapter = TeamsAdapter::new(
            "app".into(),
            "pw".into(),
            "this is not base64!!!".into(),
            3978,
            vec![],
        );
        assert!(adapter.security_token_key.is_none());
    }

    #[test]
    fn test_parse_teams_activity_basic() {
        let activity = serde_json::json!({
            "type": "message",
            "id": "activity-1",
            "text": "Hello from Teams!",
            "from": {
                "id": "user-456",
                "name": "Alice"
            },
            "conversation": {
                "id": "conv-789",
                "isGroup": false
            },
            "serviceUrl": "https://smba.trafficmanager.net/teams/",
            "channelData": {
                "tenant": {
                    "id": "tenant-abc"
                }
            }
        });

        let msg = parse_teams_activity(&activity, "app-id-123", &[]).unwrap();
        assert_eq!(msg.channel, ChannelType::Teams);
        assert_eq!(msg.sender.display_name, "Alice");
        assert_eq!(msg.sender.platform_id, "conv-789");
        assert!(!msg.is_group);
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello from Teams!"));
        assert!(msg.metadata.contains_key("serviceUrl"));
    }

    #[test]
    fn test_parse_teams_activity_skips_bot_self() {
        let activity = serde_json::json!({
            "type": "message",
            "id": "activity-1",
            "text": "Bot reply",
            "from": {
                "id": "app-id-123",
                "name": "LibreFang Bot"
            },
            "conversation": {
                "id": "conv-789"
            },
            "serviceUrl": "https://smba.trafficmanager.net/teams/"
        });

        let msg = parse_teams_activity(&activity, "app-id-123", &[]);
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_teams_activity_tenant_filter() {
        let activity = serde_json::json!({
            "type": "message",
            "id": "activity-1",
            "text": "Hello",
            "from": {
                "id": "user-1",
                "name": "Bob"
            },
            "conversation": {
                "id": "conv-1"
            },
            "serviceUrl": "https://smba.trafficmanager.net/teams/",
            "channelData": {
                "tenant": {
                    "id": "tenant-xyz"
                }
            }
        });

        // Not in allowed tenants
        let msg = parse_teams_activity(&activity, "app-id", &["tenant-abc".to_string()]);
        assert!(msg.is_none());

        // In allowed tenants
        let msg = parse_teams_activity(&activity, "app-id", &["tenant-xyz".to_string()]);
        assert!(msg.is_some());
    }

    #[test]
    fn test_parse_teams_activity_command() {
        let activity = serde_json::json!({
            "type": "message",
            "id": "activity-1",
            "text": "/agent hello-world",
            "from": {
                "id": "user-1",
                "name": "Alice"
            },
            "conversation": {
                "id": "conv-1"
            },
            "serviceUrl": "https://smba.trafficmanager.net/teams/"
        });

        let msg = parse_teams_activity(&activity, "app-id", &[]).unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_teams_activity_non_message() {
        let activity = serde_json::json!({
            "type": "conversationUpdate",
            "id": "activity-1",
            "from": { "id": "user-1", "name": "Alice" },
            "conversation": { "id": "conv-1" },
            "serviceUrl": "https://smba.trafficmanager.net/teams/"
        });

        let msg = parse_teams_activity(&activity, "app-id", &[]);
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_teams_activity_empty_text() {
        let activity = serde_json::json!({
            "type": "message",
            "id": "activity-1",
            "text": "",
            "from": { "id": "user-1", "name": "Alice" },
            "conversation": { "id": "conv-1" },
            "serviceUrl": "https://smba.trafficmanager.net/teams/"
        });

        let msg = parse_teams_activity(&activity, "app-id", &[]);
        assert!(msg.is_none());
    }

    #[test]
    fn test_parse_teams_activity_group() {
        let activity = serde_json::json!({
            "type": "message",
            "id": "activity-1",
            "text": "Group hello",
            "from": { "id": "user-1", "name": "Alice" },
            "conversation": {
                "id": "conv-1",
                "isGroup": true
            },
            "serviceUrl": "https://smba.trafficmanager.net/teams/"
        });

        let msg = parse_teams_activity(&activity, "app-id", &[]).unwrap();
        assert!(msg.is_group);
    }

    #[test]
    fn test_teams_account_id_default_none() {
        let adapter = TeamsAdapter::new(
            "app-id".to_string(),
            "app-password".to_string(),
            String::new(),
            0,
            vec![],
        );
        assert_eq!(adapter.account_id(), None);
    }

    #[test]
    fn test_teams_account_id_returns_configured_value() {
        // #5003: two Teams tenants must resolve under distinct
        // `teams:<account_id>` keys via the trait override.
        let adapter = TeamsAdapter::new(
            "app-id".to_string(),
            "app-password".to_string(),
            String::new(),
            0,
            vec![],
        )
        .with_account_id(Some("tenant-42".to_string()));
        assert_eq!(adapter.account_id(), Some("tenant-42"));
    }
}
