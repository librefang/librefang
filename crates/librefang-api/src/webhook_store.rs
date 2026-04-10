//! In-process webhook subscription store with file persistence.
//!
//! Manages outbound webhook subscriptions — when system events occur,
//! registered webhooks receive HTTP POST notifications.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::RwLock;
use uuid::Uuid;

/// Unique identifier for a webhook subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WebhookId(pub Uuid);

impl std::fmt::Display for WebhookId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Events that can trigger a webhook notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEvent {
    /// Agent spawned.
    AgentSpawned,
    /// Agent stopped/killed.
    AgentStopped,
    /// Message received by an agent.
    MessageReceived,
    /// Message response completed.
    MessageCompleted,
    /// Agent error occurred.
    AgentError,
    /// Cron job fired.
    CronFired,
    /// Trigger fired.
    TriggerFired,
    /// Wildcard — all events.
    All,
}

impl std::fmt::Display for WebhookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AgentSpawned => write!(f, "agent_spawned"),
            Self::AgentStopped => write!(f, "agent_stopped"),
            Self::MessageReceived => write!(f, "message_received"),
            Self::MessageCompleted => write!(f, "message_completed"),
            Self::AgentError => write!(f, "agent_error"),
            Self::CronFired => write!(f, "cron_fired"),
            Self::TriggerFired => write!(f, "trigger_fired"),
            Self::All => write!(f, "all"),
        }
    }
}

/// A webhook subscription.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookSubscription {
    pub id: WebhookId,
    /// Tenant account ID for multi-tenant isolation.
    #[serde(default = "default_account_id")]
    pub account_id: String,
    /// Human-readable label.
    pub name: String,
    /// URL to POST event payloads to.
    pub url: String,
    /// Optional shared secret for HMAC-SHA256 signature verification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    /// Events this webhook subscribes to.
    pub events: Vec<WebhookEvent>,
    /// Whether the webhook is active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// When the subscription was created.
    pub created_at: DateTime<Utc>,
    /// When the subscription was last updated.
    pub updated_at: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

/// Default account_id for legacy webhooks during migration.
fn default_account_id() -> String {
    "default".to_string()
}

/// Return a copy of a webhook with its secret redacted for API responses.
pub fn redact_webhook_secret(wh: &WebhookSubscription) -> WebhookSubscription {
    let mut redacted = wh.clone();
    if redacted.secret.is_some() {
        redacted.secret = Some("***".to_string());
    }
    redacted
}

/// Compute HMAC-SHA256 signature for a payload using the given secret.
pub fn compute_hmac_signature(secret: &str, payload: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
    mac.update(payload);
    let result = mac.finalize();
    let bytes = result.into_bytes();
    format!("sha256={}", hex::encode(bytes))
}

/// Validate that a URL is safe to send webhooks to (mitigate SSRF).
/// Only allows http and https schemes, blocks private/link-local IPs.
pub fn validate_webhook_url(url_str: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url_str).map_err(|_| "url is not a valid URL".to_string())?;

    match parsed.scheme() {
        "http" | "https" => {}
        other => {
            return Err(format!(
                "url scheme '{}' is not allowed, only http/https",
                other
            ))
        }
    }

    // Block private/link-local IPs to mitigate SSRF
    if let Some(host) = parsed.host_str() {
        if let Ok(ip) = host.parse::<std::net::IpAddr>() {
            if ip.is_loopback() || is_private_ip(ip) || is_link_local(ip) {
                return Err(
                    "url must not point to a private, loopback, or link-local address".to_string(),
                );
            }
        }
        // Also block common internal hostnames
        let lower = host.to_lowercase();
        if lower == "localhost"
            || lower == "metadata.google.internal"
            || lower.ends_with(".internal")
        {
            return Err("url must not point to an internal/localhost address".to_string());
        }
    }

    Ok(())
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64
            // 100.64.0.0/10
        }
        std::net::IpAddr::V6(_) => false, // Simplified; production should check IPv6 ULA
    }
}

fn is_link_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_link_local() || v4.octets()[0] == 169,
        std::net::IpAddr::V6(_) => false,
    }
}

/// Request body for creating a webhook.
#[derive(Debug, Deserialize)]
pub struct CreateWebhookRequest {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub secret: Option<String>,
    pub events: Vec<WebhookEvent>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// Request body for updating a webhook.
#[derive(Debug, Deserialize)]
pub struct UpdateWebhookRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub secret: Option<String>,
    #[serde(default)]
    pub events: Option<Vec<WebhookEvent>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

/// Maximum number of webhook subscriptions.
const MAX_WEBHOOKS: usize = 100;
/// Maximum name length.
const MAX_NAME_LEN: usize = 128;
/// Maximum URL length.
const MAX_URL_LEN: usize = 2048;
/// Maximum secret length.
const MAX_SECRET_LEN: usize = 256;

impl CreateWebhookRequest {
    /// Validate the create request.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("name must not be empty".to_string());
        }
        if self.name.len() > MAX_NAME_LEN {
            return Err(format!(
                "name exceeds maximum length of {} chars",
                MAX_NAME_LEN
            ));
        }
        if self.url.trim().is_empty() {
            return Err("url must not be empty".to_string());
        }
        if self.url.len() > MAX_URL_LEN {
            return Err(format!(
                "url exceeds maximum length of {} chars",
                MAX_URL_LEN
            ));
        }
        validate_webhook_url(&self.url)?;
        if let Some(ref s) = self.secret {
            if s.len() > MAX_SECRET_LEN {
                return Err(format!(
                    "secret exceeds maximum length of {} chars",
                    MAX_SECRET_LEN
                ));
            }
        }
        if self.events.is_empty() {
            return Err("events must not be empty".to_string());
        }
        Ok(())
    }
}

/// Persisted webhook store.
#[derive(Debug, Serialize, Deserialize, Default)]
struct StoreData {
    webhooks: Vec<WebhookSubscription>,
}

/// Thread-safe webhook subscription store with file persistence.
pub struct WebhookStore {
    data: RwLock<StoreData>,
    path: PathBuf,
}

impl WebhookStore {
    /// Load or create a webhook store at the given path.
    pub fn load(path: PathBuf) -> Self {
        let data = if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
                Err(_) => StoreData::default(),
            }
        } else {
            StoreData::default()
        };
        let legacy_count = data
            .webhooks
            .iter()
            .filter(|webhook| webhook.account_id == default_account_id())
            .count();
        Self {
            data: RwLock::new(data),
            path,
        }
        .with_legacy_migration_log(legacy_count)
    }

    fn with_legacy_migration_log(self, legacy_count: usize) -> Self {
        if legacy_count > 0 {
            tracing::info!(
                legacy_webhooks = legacy_count,
                default_account = %default_account_id(),
                "Loaded legacy webhooks without account_id; assigned to default account"
            );
            if let Ok(guard) = self.data.read() {
                if let Err(e) = self.persist(&guard) {
                    tracing::warn!("Failed to persist migrated legacy webhooks: {e}");
                }
            }
        }
        self
    }

    /// Persist current state to disk.
    fn persist(&self, data: &StoreData) -> Result<(), String> {
        let json =
            serde_json::to_string_pretty(data).map_err(|e| format!("serialize error: {e}"))?;
        // Ensure parent directory exists
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Set restrictive permissions on the file (contains secrets)
        std::fs::write(&self.path, json).map_err(|e| format!("write error: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    /// List webhook subscriptions scoped to a specific account (tenant).
    pub fn list_scoped(&self, account_id: &str) -> Vec<WebhookSubscription> {
        self.data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .webhooks
            .iter()
            .filter(|w| w.account_id == account_id)
            .cloned()
            .collect()
    }

    /// Get a single webhook by ID, verifying it belongs to the account (tenant).
    pub fn get_scoped(&self, id: WebhookId, account_id: &str) -> Option<WebhookSubscription> {
        self.data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .webhooks
            .iter()
            .find(|w| w.id == id && w.account_id == account_id)
            .cloned()
    }

    /// Create a new webhook subscription with account (tenant) association.
    pub fn create_scoped(
        &self,
        req: CreateWebhookRequest,
        account_id: String,
    ) -> Result<WebhookSubscription, String> {
        req.validate()?;
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        if data.webhooks.len() >= MAX_WEBHOOKS {
            return Err(format!(
                "maximum number of webhooks ({}) reached",
                MAX_WEBHOOKS
            ));
        }
        let now = Utc::now();
        let webhook = WebhookSubscription {
            id: WebhookId(Uuid::new_v4()),
            account_id,
            name: req.name,
            url: req.url,
            secret: req.secret,
            events: req.events,
            enabled: req.enabled,
            created_at: now,
            updated_at: now,
        };
        data.webhooks.push(webhook.clone());
        if let Err(e) = self.persist(&data) {
            tracing::warn!("Failed to persist webhook store: {e}");
        }
        Ok(webhook)
    }

    /// Update a webhook subscription, verifying it belongs to the account (tenant).
    pub fn update_scoped(
        &self,
        id: WebhookId,
        account_id: &str,
        req: UpdateWebhookRequest,
    ) -> Result<WebhookSubscription, String> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        let webhook = data
            .webhooks
            .iter_mut()
            .find(|w| w.id == id && w.account_id == account_id)
            .ok_or_else(|| "webhook not found".to_string())?;

        if let Some(ref name) = req.name {
            if name.trim().is_empty() {
                return Err("name must not be empty".to_string());
            }
            if name.len() > MAX_NAME_LEN {
                return Err(format!(
                    "name exceeds maximum length of {} chars",
                    MAX_NAME_LEN
                ));
            }
            webhook.name = name.clone();
        }
        if let Some(ref url_str) = req.url {
            if url_str.trim().is_empty() {
                return Err("url must not be empty".to_string());
            }
            if url_str.len() > MAX_URL_LEN {
                return Err(format!(
                    "url exceeds maximum length of {} chars",
                    MAX_URL_LEN
                ));
            }
            validate_webhook_url(url_str)?;
            webhook.url = url_str.clone();
        }
        if let Some(ref secret) = req.secret {
            if secret.is_empty() {
                webhook.secret = None;
            } else if secret.len() > MAX_SECRET_LEN {
                return Err(format!(
                    "secret exceeds maximum length of {} chars",
                    MAX_SECRET_LEN
                ));
            } else {
                webhook.secret = Some(secret.clone());
            }
        }
        if let Some(ref events) = req.events {
            if events.is_empty() {
                return Err("events must not be empty".to_string());
            }
            webhook.events = events.clone();
        }
        if let Some(enabled) = req.enabled {
            webhook.enabled = enabled;
        }
        webhook.updated_at = Utc::now();
        let updated = webhook.clone();
        if let Err(e) = self.persist(&data) {
            tracing::warn!("Failed to persist webhook store: {e}");
        }
        Ok(updated)
    }

    /// Delete a webhook subscription, verifying it belongs to the account (tenant).
    pub fn delete_scoped(&self, id: WebhookId, account_id: &str) -> bool {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        let before = data.webhooks.len();
        data.webhooks
            .retain(|w| !(w.id == id && w.account_id == account_id));
        let removed = data.webhooks.len() < before;
        if removed {
            if let Err(e) = self.persist(&data) {
                tracing::warn!("Failed to persist webhook store: {e}");
            }
        }
        removed
    }
}

// hex encoding helper (avoids pulling in another crate)
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (WebhookStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webhooks.json");
        (WebhookStore::load(path), dir)
    }

    fn valid_create_req() -> CreateWebhookRequest {
        CreateWebhookRequest {
            name: "test-hook".to_string(),
            url: "https://example.com/hook".to_string(),
            secret: Some("my-secret".to_string()),
            events: vec![WebhookEvent::AgentSpawned],
            enabled: true,
        }
    }

    fn account_id() -> String {
        "test-account".to_string()
    }

    #[test]
    fn create_and_list() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        assert!(store.list_scoped(&acct).is_empty());
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        assert_eq!(wh.name, "test-hook");
        assert_eq!(wh.account_id, acct);
        assert_eq!(store.list_scoped(&acct).len(), 1);
    }

    #[test]
    fn create_validates_empty_name() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.name = String::new();
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("name must not be empty"));
    }

    #[test]
    fn create_validates_empty_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = String::new();
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("url must not be empty"));
    }

    #[test]
    fn create_validates_invalid_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "not a url".to_string();
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("not a valid URL"));
    }

    #[test]
    fn create_validates_empty_events() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.events = vec![];
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("events must not be empty"));
    }

    #[test]
    fn create_rejects_private_ip_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "http://192.168.1.1/hook".to_string();
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("private"));
    }

    #[test]
    fn create_rejects_localhost_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "http://localhost:8080/hook".to_string();
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("internal/localhost"));
    }

    #[test]
    fn create_rejects_link_local_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "http://169.254.169.254/metadata".to_string();
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("private") || err.contains("link-local"));
    }

    #[test]
    fn get_by_id() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        let found = store.get_scoped(wh.id, &acct).unwrap();
        assert_eq!(found.name, "test-hook");
        assert!(store.get_scoped(WebhookId(Uuid::new_v4()), &acct).is_none());
    }

    #[test]
    fn update_webhook() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        let updated = store
            .update_scoped(
                wh.id,
                &acct,
                UpdateWebhookRequest {
                    name: Some("renamed".to_string()),
                    url: None,
                    secret: None,
                    events: None,
                    enabled: Some(false),
                },
            )
            .unwrap();
        assert_eq!(updated.name, "renamed");
        assert!(!updated.enabled);
        assert!(updated.updated_at > wh.updated_at);
    }

    #[test]
    fn update_clears_secret_with_empty_string() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        assert!(wh.secret.is_some());
        let updated = store
            .update_scoped(
                wh.id,
                &acct,
                UpdateWebhookRequest {
                    name: None,
                    url: None,
                    secret: Some(String::new()),
                    events: None,
                    enabled: None,
                },
            )
            .unwrap();
        assert!(updated.secret.is_none());
    }

    #[test]
    fn update_not_found() {
        let (store, _dir) = temp_store();
        let err = store
            .update_scoped(
                WebhookId(Uuid::new_v4()),
                &account_id(),
                UpdateWebhookRequest {
                    name: Some("x".to_string()),
                    url: None,
                    secret: None,
                    events: None,
                    enabled: None,
                },
            )
            .unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn delete_webhook() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        assert!(store.delete_scoped(wh.id, &acct));
        assert!(store.list_scoped(&acct).is_empty());
        assert!(!store.delete_scoped(wh.id, &acct));
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webhooks.json");
        let acct = account_id();

        // Create and persist
        {
            let store = WebhookStore::load(path.clone());
            store
                .create_scoped(valid_create_req(), acct.clone())
                .unwrap();
        }

        // Reload and verify
        {
            let store = WebhookStore::load(path);
            let scoped = store.list_scoped(&acct);
            assert_eq!(scoped.len(), 1);
            assert_eq!(scoped[0].name, "test-hook");
        }
    }

    #[test]
    fn max_webhooks_enforced() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        for i in 0..MAX_WEBHOOKS {
            let req = CreateWebhookRequest {
                name: format!("hook-{i}"),
                url: format!("https://example.com/hook/{i}"),
                secret: None,
                events: vec![WebhookEvent::All],
                enabled: true,
            };
            store.create_scoped(req, acct.clone()).unwrap();
        }
        let err = store.create_scoped(valid_create_req(), acct).unwrap_err();
        assert!(err.contains("maximum number of webhooks"));
    }

    #[test]
    fn webhook_event_serde_roundtrip() {
        let events = vec![
            WebhookEvent::AgentSpawned,
            WebhookEvent::AgentStopped,
            WebhookEvent::MessageReceived,
            WebhookEvent::MessageCompleted,
            WebhookEvent::AgentError,
            WebhookEvent::CronFired,
            WebhookEvent::TriggerFired,
            WebhookEvent::All,
        ];
        let json = serde_json::to_string(&events).unwrap();
        let back: Vec<WebhookEvent> = serde_json::from_str(&json).unwrap();
        assert_eq!(events, back);
    }

    #[test]
    fn name_too_long() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.name = "x".repeat(MAX_NAME_LEN + 1);
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("name exceeds maximum length"));
    }

    #[test]
    fn url_too_long() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = format!("https://example.com/{}", "x".repeat(MAX_URL_LEN));
        let err = store.create_scoped(req, account_id()).unwrap_err();
        assert!(err.contains("url exceeds maximum length"));
    }

    #[test]
    fn update_validates_empty_name() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        let err = store
            .update_scoped(
                wh.id,
                &acct,
                UpdateWebhookRequest {
                    name: Some(String::new()),
                    url: None,
                    secret: None,
                    events: None,
                    enabled: None,
                },
            )
            .unwrap_err();
        assert!(err.contains("name must not be empty"));
    }

    #[test]
    fn update_validates_invalid_url() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        let err = store
            .update_scoped(
                wh.id,
                &acct,
                UpdateWebhookRequest {
                    name: None,
                    url: Some("not-a-url".to_string()),
                    secret: None,
                    events: None,
                    enabled: None,
                },
            )
            .unwrap_err();
        assert!(err.contains("not a valid URL"));
    }

    #[test]
    fn update_validates_empty_events() {
        let (store, _dir) = temp_store();
        let acct = account_id();
        let wh = store
            .create_scoped(valid_create_req(), acct.clone())
            .unwrap();
        let err = store
            .update_scoped(
                wh.id,
                &acct,
                UpdateWebhookRequest {
                    name: None,
                    url: None,
                    secret: None,
                    events: Some(vec![]),
                    enabled: None,
                },
            )
            .unwrap_err();
        assert!(err.contains("events must not be empty"));
    }

    #[test]
    fn redact_secret_works() {
        let wh = WebhookSubscription {
            id: WebhookId(Uuid::new_v4()),
            account_id: account_id(),
            name: "test".to_string(),
            url: "https://example.com".to_string(),
            secret: Some("super-secret".to_string()),
            events: vec![WebhookEvent::All],
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let redacted = redact_webhook_secret(&wh);
        assert_eq!(redacted.secret, Some("***".to_string()));

        let no_secret = WebhookSubscription { secret: None, ..wh };
        let redacted2 = redact_webhook_secret(&no_secret);
        assert!(redacted2.secret.is_none());
    }

    #[test]
    fn hmac_signature_is_deterministic() {
        let sig1 = compute_hmac_signature("secret", b"payload");
        let sig2 = compute_hmac_signature("secret", b"payload");
        assert_eq!(sig1, sig2);
        assert!(sig1.starts_with("sha256="));

        let sig3 = compute_hmac_signature("other", b"payload");
        assert_ne!(sig1, sig3);
    }

    #[test]
    fn cross_tenant_isolation() {
        let (store, _dir) = temp_store();
        let acct_a = "tenant-a".to_string();
        let acct_b = "tenant-b".to_string();

        // Tenant A creates a webhook
        let wh_a = store
            .create_scoped(valid_create_req(), acct_a.clone())
            .unwrap();

        // Tenant B creates a webhook
        let mut req_b = valid_create_req();
        req_b.name = "tenant-b-hook".to_string();
        let wh_b = store.create_scoped(req_b, acct_b.clone()).unwrap();

        // Verify each tenant only sees their own webhooks
        assert_eq!(store.list_scoped(&acct_a).len(), 1);
        assert_eq!(store.list_scoped(&acct_b).len(), 1);

        // Verify tenant A cannot see tenant B's webhook
        assert!(store.get_scoped(wh_b.id, &acct_a).is_none());
        assert_eq!(store.get_scoped(wh_a.id, &acct_a).unwrap().id, wh_a.id);

        // Verify tenant A cannot delete tenant B's webhook
        assert!(!store.delete_scoped(wh_b.id, &acct_a));
        assert_eq!(store.list_scoped(&acct_b).len(), 1);

        // Verify tenant A can delete their own webhook
        assert!(store.delete_scoped(wh_a.id, &acct_a));
        assert!(store.list_scoped(&acct_a).is_empty());
        assert_eq!(store.list_scoped(&acct_b).len(), 1);
    }

    #[test]
    fn update_respects_tenant_boundary() {
        let (store, _dir) = temp_store();
        let acct_a = "tenant-a".to_string();
        let acct_b = "tenant-b".to_string();

        let wh_a = store
            .create_scoped(valid_create_req(), acct_a.clone())
            .unwrap();

        // Tenant B cannot update tenant A's webhook
        let err = store
            .update_scoped(
                wh_a.id,
                &acct_b,
                UpdateWebhookRequest {
                    name: Some("hacked".to_string()),
                    url: None,
                    secret: None,
                    events: None,
                    enabled: None,
                },
            )
            .unwrap_err();
        assert!(err.contains("not found"));

        // Verify the webhook was not modified
        let unchanged = store.get_scoped(wh_a.id, &acct_a).unwrap();
        assert_eq!(unchanged.name, "test-hook");
    }

    #[test]
    fn legacy_webhooks_load_with_default_account_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webhooks.json");
        let legacy_json = serde_json::json!({
            "webhooks": [{
                "id": Uuid::new_v4().to_string(),
                "name": "legacy-hook",
                "url": "https://example.com/legacy",
                "secret": "legacy-secret",
                "events": ["agent_spawned"],
                "enabled": true,
                "created_at": Utc::now(),
                "updated_at": Utc::now()
            }]
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&legacy_json).unwrap()).unwrap();

        let store = WebhookStore::load(path.clone());
        let migrated = store.list_scoped(&default_account_id());
        assert_eq!(migrated.len(), 1);
        assert_eq!(migrated[0].name, "legacy-hook");
        assert_eq!(migrated[0].account_id, default_account_id());

        let persisted: StoreData = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.webhooks.len(), 1);
        assert_eq!(persisted.webhooks[0].account_id, default_account_id());
    }
}
