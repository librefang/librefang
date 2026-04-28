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

    // Block private/link-local IPs to mitigate SSRF.
    //
    // Use the typed `url::Host` enum rather than `host_str().parse::<IpAddr>()`:
    // `host_str()` returns IPv6 literals wrapped in brackets (e.g.
    // `"[::ffff:7f00:1]"`), which `IpAddr::from_str` rejects — meaning the
    // string-parse path silently skipped every IPv6 URL. `parsed.host()`
    // returns `Host::Ipv6(Ipv6Addr)` / `Host::Ipv4(Ipv4Addr)` directly, with
    // the url crate already having normalised the address.
    match parsed.host() {
        Some(url::Host::Ipv4(v4)) => {
            let ip = std::net::IpAddr::V4(v4);
            if ip.is_loopback() || is_private_ip(ip) || is_link_local(ip) {
                return Err(
                    "url must not point to a private, loopback, or link-local address".to_string(),
                );
            }
        }
        Some(url::Host::Ipv6(v6)) => {
            // Canonicalise IPv4-mapped IPv6 (::ffff:X.X.X.X) so the OS-level
            // transparent connect to the embedded IPv4 target can't bypass
            // these checks — ip.is_loopback() and the V6 arms of
            // is_private_ip / is_link_local do not recognise the mapped form.
            let ip = canonical_ip(std::net::IpAddr::V6(v6));
            if ip.is_loopback() || is_private_ip(ip) || is_link_local(ip) {
                return Err(
                    "url must not point to a private, loopback, or link-local address".to_string(),
                );
            }
        }
        Some(url::Host::Domain(host)) => {
            // Also block common internal hostnames
            let lower = host.to_lowercase();
            if lower == "localhost"
                || lower == "metadata.google.internal"
                || lower.ends_with(".internal")
            {
                return Err("url must not point to an internal/localhost address".to_string());
            }
        }
        None => {}
    }

    Ok(())
}

/// Unwrap IPv4-mapped IPv6 (`::ffff:X.X.X.X`) to its IPv4 form. All other
/// addresses are returned unchanged.
fn canonical_ip(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => std::net::IpAddr::V4(v4),
            None => std::net::IpAddr::V6(v6),
        },
        std::net::IpAddr::V4(_) => ip,
    }
}

fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match canonical_ip(ip) {
        std::net::IpAddr::V4(v4) => {
            v4.is_private() || v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64
            // 100.64.0.0/10
        }
        std::net::IpAddr::V6(_) => false, // Simplified; production should check IPv6 ULA
    }
}

fn is_link_local(ip: std::net::IpAddr) -> bool {
    match canonical_ip(ip) {
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
            if s.is_empty() {
                return Err(
                    "secret must not be empty; omit the field entirely to create a webhook without authentication".to_string(),
                );
            }
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
        Self {
            data: RwLock::new(data),
            path,
        }
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

    /// List all webhook subscriptions.
    pub fn list(&self) -> Vec<WebhookSubscription> {
        self.data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .webhooks
            .clone()
    }

    /// Get a single webhook by ID.
    pub fn get(&self, id: WebhookId) -> Option<WebhookSubscription> {
        self.data
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .webhooks
            .iter()
            .find(|w| w.id == id)
            .cloned()
    }

    /// Create a new webhook subscription.
    pub fn create(&self, req: CreateWebhookRequest) -> Result<WebhookSubscription, String> {
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

    /// Update an existing webhook subscription.
    pub fn update(
        &self,
        id: WebhookId,
        req: UpdateWebhookRequest,
    ) -> Result<WebhookSubscription, String> {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        let webhook = data
            .webhooks
            .iter_mut()
            .find(|w| w.id == id)
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
                // Treat empty string as "clear the secret"
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

    /// Delete a webhook subscription.
    pub fn delete(&self, id: WebhookId) -> bool {
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        let before = data.webhooks.len();
        data.webhooks.retain(|w| w.id != id);
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

    #[test]
    fn create_and_list() {
        let (store, _dir) = temp_store();
        assert!(store.list().is_empty());
        let wh = store.create(valid_create_req()).unwrap();
        assert_eq!(wh.name, "test-hook");
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn validate_webhook_url_blocks_ipv4_mapped_ipv6_loopback() {
        // OS-level transparent connect means ::ffff:127.0.0.1 reaches
        // loopback, but ip.is_loopback() + the V6 _ => false arms of
        // is_private_ip / is_link_local miss it. canonical_ip must unwrap
        // these before the guard runs.
        assert!(validate_webhook_url("http://[::ffff:127.0.0.1]/hook").is_err());
        assert!(validate_webhook_url("http://[::ffff:7f00:1]/hook").is_err());
    }

    #[test]
    fn validate_webhook_url_blocks_ipv4_mapped_ipv6_metadata() {
        assert!(validate_webhook_url("http://[::ffff:169.254.169.254]/hook").is_err());
        assert!(validate_webhook_url("http://[::ffff:a9fe:a9fe]/hook").is_err());
    }

    #[test]
    fn validate_webhook_url_blocks_ipv4_mapped_ipv6_private() {
        assert!(validate_webhook_url("http://[::ffff:10.0.0.1]/hook").is_err());
        assert!(validate_webhook_url("http://[::ffff:192.168.1.1]/hook").is_err());
    }

    #[test]
    fn create_validates_empty_name() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.name = String::new();
        let err = store.create(req).unwrap_err();
        assert!(err.contains("name must not be empty"));
    }

    #[test]
    fn create_validates_empty_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = String::new();
        let err = store.create(req).unwrap_err();
        assert!(err.contains("url must not be empty"));
    }

    #[test]
    fn create_validates_invalid_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "not a url".to_string();
        let err = store.create(req).unwrap_err();
        assert!(err.contains("not a valid URL"));
    }

    #[test]
    fn create_rejects_empty_secret() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.secret = Some(String::new());
        let err = store.create(req).unwrap_err();
        assert!(
            err.contains("secret must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn create_validates_empty_events() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.events = vec![];
        let err = store.create(req).unwrap_err();
        assert!(err.contains("events must not be empty"));
    }

    #[test]
    fn create_rejects_private_ip_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "http://192.168.1.1/hook".to_string();
        let err = store.create(req).unwrap_err();
        assert!(err.contains("private"));
    }

    #[test]
    fn create_rejects_localhost_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "http://localhost:8080/hook".to_string();
        let err = store.create(req).unwrap_err();
        assert!(err.contains("internal/localhost"));
    }

    #[test]
    fn create_rejects_link_local_url() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = "http://169.254.169.254/metadata".to_string();
        let err = store.create(req).unwrap_err();
        assert!(err.contains("private") || err.contains("link-local"));
    }

    #[test]
    fn get_by_id() {
        let (store, _dir) = temp_store();
        let wh = store.create(valid_create_req()).unwrap();
        let found = store.get(wh.id).unwrap();
        assert_eq!(found.name, "test-hook");
        assert!(store.get(WebhookId(Uuid::new_v4())).is_none());
    }

    #[test]
    fn update_webhook() {
        let (store, _dir) = temp_store();
        let wh = store.create(valid_create_req()).unwrap();
        let updated = store
            .update(
                wh.id,
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
        let wh = store.create(valid_create_req()).unwrap();
        assert!(wh.secret.is_some());
        let updated = store
            .update(
                wh.id,
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
            .update(
                WebhookId(Uuid::new_v4()),
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
        let wh = store.create(valid_create_req()).unwrap();
        assert!(store.delete(wh.id));
        assert!(store.list().is_empty());
        assert!(!store.delete(wh.id));
    }

    #[test]
    fn persistence_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("webhooks.json");

        // Create and persist
        {
            let store = WebhookStore::load(path.clone());
            store.create(valid_create_req()).unwrap();
        }

        // Reload and verify
        {
            let store = WebhookStore::load(path);
            assert_eq!(store.list().len(), 1);
            assert_eq!(store.list()[0].name, "test-hook");
        }
    }

    #[test]
    fn max_webhooks_enforced() {
        let (store, _dir) = temp_store();
        for i in 0..MAX_WEBHOOKS {
            let req = CreateWebhookRequest {
                name: format!("hook-{i}"),
                url: format!("https://example.com/hook/{i}"),
                secret: None,
                events: vec![WebhookEvent::All],
                enabled: true,
            };
            store.create(req).unwrap();
        }
        let err = store.create(valid_create_req()).unwrap_err();
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
        let err = store.create(req).unwrap_err();
        assert!(err.contains("name exceeds maximum length"));
    }

    #[test]
    fn url_too_long() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.url = format!("https://example.com/{}", "x".repeat(MAX_URL_LEN));
        let err = store.create(req).unwrap_err();
        assert!(err.contains("url exceeds maximum length"));
    }

    #[test]
    fn update_validates_empty_name() {
        let (store, _dir) = temp_store();
        let wh = store.create(valid_create_req()).unwrap();
        let err = store
            .update(
                wh.id,
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
        let wh = store.create(valid_create_req()).unwrap();
        let err = store
            .update(
                wh.id,
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
        let wh = store.create(valid_create_req()).unwrap();
        let err = store
            .update(
                wh.id,
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
}
