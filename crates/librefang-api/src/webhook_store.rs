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
        // Validate URL format
        if url::Url::parse(&self.url).is_err() {
            return Err("url is not a valid URL".to_string());
        }
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
        std::fs::write(&self.path, json).map_err(|e| format!("write error: {e}"))?;
        Ok(())
    }

    /// List all webhook subscriptions.
    pub fn list(&self) -> Vec<WebhookSubscription> {
        self.data.read().unwrap_or_else(|e| e.into_inner()).webhooks.clone()
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
            if url::Url::parse(url_str).is_err() {
                return Err("url is not a valid URL".to_string());
            }
            webhook.url = url_str.clone();
        }
        if let Some(ref secret) = req.secret {
            if secret.len() > MAX_SECRET_LEN {
                return Err(format!(
                    "secret exceeds maximum length of {} chars",
                    MAX_SECRET_LEN
                ));
            }
            webhook.secret = Some(secret.clone());
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
    fn create_validates_empty_events() {
        let (store, _dir) = temp_store();
        let mut req = valid_create_req();
        req.events = vec![];
        let err = store.create(req).unwrap_err();
        assert!(err.contains("events must not be empty"));
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
}
