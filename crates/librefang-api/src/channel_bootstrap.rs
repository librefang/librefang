//! File-backed ownership records for channel bootstrap sessions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapKind {
    QrLogin,
    PairingCode,
    SessionReauth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapStatus {
    Pending,
    Confirmed,
    Expired,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelBootstrapSession {
    pub bootstrap_id: String,
    pub channel_type: String,
    pub instance_key: String,
    pub account_id: String,
    pub bootstrap_kind: BootstrapKind,
    pub provider_handle: Option<String>,
    pub provider_qr_payload: Option<String>,
    pub provider_qr_url: Option<String>,
    pub provider_pairing_code: Option<String>,
    pub status: BootstrapStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_by: String,
    pub last_error: Option<String>,
}

pub struct ChannelBootstrapStore {
    persist_path: PathBuf,
    sessions: RwLock<Vec<ChannelBootstrapSession>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChannelBootstrapFile {
    sessions: Vec<ChannelBootstrapSession>,
}

impl ChannelBootstrapStore {
    pub fn new(home_dir: &Path) -> Self {
        Self {
            persist_path: home_dir.join("channel_bootstrap_sessions.json"),
            sessions: RwLock::new(Vec::new()),
        }
    }

    pub fn load(&self) -> Result<usize, String> {
        if !self.persist_path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(&self.persist_path)
            .map_err(|e| format!("Failed to read bootstrap sessions: {e}"))?;
        let file: ChannelBootstrapFile = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse bootstrap sessions: {e}"))?;
        let count = file.sessions.len();
        let mut sessions = if let Ok(sessions) = self.sessions.try_write() {
            sessions
        } else {
            self.sessions.blocking_write()
        };
        sessions.clear();
        sessions.extend(file.sessions);
        Ok(count)
    }

    pub async fn create(
        &self,
        session: ChannelBootstrapSession,
    ) -> Result<ChannelBootstrapSession, String> {
        validate_session(&session)?;
        let mut sessions = self.sessions.write().await;
        if sessions
            .iter()
            .any(|existing| existing.bootstrap_id == session.bootstrap_id)
        {
            return Err(format!(
                "bootstrap session '{}' already exists",
                session.bootstrap_id
            ));
        }
        if has_conflicting_pending_session(&sessions, &session.channel_type, &session.instance_key)
        {
            return Err(format!(
                "conflicting pending bootstrap already exists for instance '{}:{}'",
                session.channel_type, session.instance_key
            ));
        }
        sessions.push(session.clone());
        let snapshot = sessions.clone();
        drop(sessions);
        self.persist_snapshot(snapshot)?;
        Ok(session)
    }

    pub async fn get_by_bootstrap_id(&self, bootstrap_id: &str) -> Option<ChannelBootstrapSession> {
        let sessions = self.sessions.read().await;
        sessions
            .iter()
            .find(|session| session.bootstrap_id == bootstrap_id)
            .cloned()
    }

    pub async fn get_pending_by_instance(
        &self,
        channel_type: &str,
        instance_key: &str,
    ) -> Option<ChannelBootstrapSession> {
        let sessions = self.sessions.read().await;
        sessions
            .iter()
            .find(|session| {
                session.channel_type == channel_type
                    && session.instance_key == instance_key
                    && session.status == BootstrapStatus::Pending
            })
            .cloned()
    }

    pub async fn get_latest_by_instance(
        &self,
        channel_type: &str,
        instance_key: &str,
    ) -> Option<ChannelBootstrapSession> {
        let sessions = self.sessions.read().await;
        sessions
            .iter()
            .filter(|session| {
                session.channel_type == channel_type && session.instance_key == instance_key
            })
            .max_by_key(|session| session.updated_at)
            .cloned()
    }

    pub async fn cancel(
        &self,
        bootstrap_id: &str,
        cancelled_at: DateTime<Utc>,
    ) -> Result<ChannelBootstrapSession, String> {
        self.transition(bootstrap_id, cancelled_at, BootstrapStatus::Cancelled, None)
            .await
    }

    pub async fn expire(
        &self,
        bootstrap_id: &str,
        expired_at: DateTime<Utc>,
    ) -> Result<ChannelBootstrapSession, String> {
        self.transition(bootstrap_id, expired_at, BootstrapStatus::Expired, None)
            .await
    }

    pub async fn confirm(
        &self,
        bootstrap_id: &str,
        confirmed_at: DateTime<Utc>,
    ) -> Result<ChannelBootstrapSession, String> {
        self.transition(bootstrap_id, confirmed_at, BootstrapStatus::Confirmed, None)
            .await
    }

    pub async fn fail(
        &self,
        bootstrap_id: &str,
        failed_at: DateTime<Utc>,
        last_error: String,
    ) -> Result<ChannelBootstrapSession, String> {
        self.transition(
            bootstrap_id,
            failed_at,
            BootstrapStatus::Failed,
            Some(last_error),
        )
        .await
    }

    #[allow(dead_code)]
    pub(crate) async fn get_by_provider_handle(
        &self,
        provider_handle: &str,
    ) -> Option<ChannelBootstrapSession> {
        let sessions = self.sessions.read().await;
        sessions
            .iter()
            .find(|session| session.provider_handle.as_deref() == Some(provider_handle))
            .cloned()
    }

    fn persist_snapshot(&self, sessions: Vec<ChannelBootstrapSession>) -> Result<(), String> {
        let file = ChannelBootstrapFile { sessions };
        let data = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("Failed to serialize bootstrap sessions: {e}"))?;
        if let Some(parent) = self.persist_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create bootstrap store directory: {e}"))?;
        }
        let tmp_path = self.persist_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, data.as_bytes())
            .map_err(|e| format!("Failed to write bootstrap sessions temp file: {e}"))?;
        std::fs::rename(&tmp_path, &self.persist_path)
            .map_err(|e| format!("Failed to rename bootstrap sessions file: {e}"))?;
        Ok(())
    }

    async fn transition(
        &self,
        bootstrap_id: &str,
        transitioned_at: DateTime<Utc>,
        next_status: BootstrapStatus,
        last_error: Option<String>,
    ) -> Result<ChannelBootstrapSession, String> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .iter_mut()
            .find(|session| session.bootstrap_id == bootstrap_id)
            .ok_or_else(|| format!("bootstrap session '{bootstrap_id}' not found"))?;
        ensure_transition_allowed(session.status, next_status)?;
        session.status = next_status;
        session.updated_at = transitioned_at;
        if last_error.is_some() {
            session.last_error = last_error;
        }
        let updated = session.clone();
        let snapshot = sessions.clone();
        drop(sessions);
        self.persist_snapshot(snapshot)?;
        Ok(updated)
    }
}

fn validate_session(session: &ChannelBootstrapSession) -> Result<(), String> {
    if session.bootstrap_id.trim().is_empty() {
        return Err("bootstrap_id must not be empty".to_string());
    }
    if session.channel_type.trim().is_empty() {
        return Err("channel_type must not be empty".to_string());
    }
    if session.instance_key.trim().is_empty() {
        return Err("instance_key must not be empty".to_string());
    }
    if session.account_id.trim().is_empty() {
        return Err("account_id must not be empty".to_string());
    }
    if session.created_by.trim().is_empty() {
        return Err("created_by must not be empty".to_string());
    }
    Ok(())
}

fn has_conflicting_pending_session(
    sessions: &[ChannelBootstrapSession],
    channel_type: &str,
    instance_key: &str,
) -> bool {
    sessions.iter().any(|existing| {
        existing.channel_type == channel_type
            && existing.instance_key == instance_key
            && existing.status == BootstrapStatus::Pending
    })
}

fn ensure_transition_allowed(
    current: BootstrapStatus,
    next: BootstrapStatus,
) -> Result<(), String> {
    if current != BootstrapStatus::Pending {
        return Err(format!(
            "cannot transition bootstrap session from {current:?} to {next:?}"
        ));
    }
    if next == BootstrapStatus::Pending {
        return Err("cannot transition bootstrap session back to pending".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_session(
        bootstrap_id: &str,
        channel_type: &str,
        instance_key: &str,
        account_id: &str,
    ) -> ChannelBootstrapSession {
        let now = Utc::now();
        ChannelBootstrapSession {
            bootstrap_id: bootstrap_id.to_string(),
            channel_type: channel_type.to_string(),
            instance_key: instance_key.to_string(),
            account_id: account_id.to_string(),
            bootstrap_kind: BootstrapKind::QrLogin,
            provider_handle: Some(format!("provider-{bootstrap_id}")),
            provider_qr_payload: Some(format!("payload-{bootstrap_id}")),
            provider_qr_url: Some(format!("https://example.com/{bootstrap_id}.png")),
            provider_pairing_code: None,
            status: BootstrapStatus::Pending,
            created_at: now,
            updated_at: now,
            expires_at: Some(now + chrono::Duration::minutes(5)),
            created_by: "operator@example.com".to_string(),
            last_error: None,
        }
    }

    #[tokio::test]
    async fn bootstrap_records_persist_concrete_account_id() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());
        let created = store
            .create(sample_session(
                "bootstrap-1",
                "wechat",
                "wechat:tenant-a",
                "tenant-a",
            ))
            .await
            .unwrap();

        assert_eq!(created.account_id, "tenant-a");

        let reloaded = ChannelBootstrapStore::new(temp.path());
        reloaded.load().unwrap();
        let stored = reloaded.get_by_bootstrap_id("bootstrap-1").await.unwrap();
        assert_eq!(stored.account_id, "tenant-a");
    }

    #[tokio::test]
    async fn bootstrap_records_persist_instance_identity_and_lifecycle_status() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());

        store
            .create(sample_session(
                "bootstrap-1",
                "wechat",
                "wechat:tenant-a",
                "tenant-a",
            ))
            .await
            .unwrap();

        let stored = store.get_by_bootstrap_id("bootstrap-1").await.unwrap();
        assert_eq!(stored.channel_type, "wechat");
        assert_eq!(stored.instance_key, "wechat:tenant-a");
        assert_eq!(stored.status, BootstrapStatus::Pending);
    }

    #[tokio::test]
    async fn bootstrap_records_survive_reload_round_trip() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());
        let original = sample_session("bootstrap-1", "wechat", "wechat:tenant-a", "tenant-a");
        let original_created_at = original.created_at;
        let original_updated_at = original.updated_at;

        store.create(original).await.unwrap();

        let reloaded = ChannelBootstrapStore::new(temp.path());
        let count = reloaded.load().unwrap();
        assert_eq!(count, 1);

        let stored = reloaded.get_by_bootstrap_id("bootstrap-1").await.unwrap();
        assert_eq!(stored.created_at, original_created_at);
        assert_eq!(stored.updated_at, original_updated_at);
        assert_eq!(
            stored.provider_handle.as_deref(),
            Some("provider-bootstrap-1")
        );
    }

    #[tokio::test]
    async fn lookup_by_instance_returns_only_the_owning_record() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());

        store
            .create(sample_session(
                "bootstrap-a",
                "wechat",
                "wechat:tenant-a",
                "tenant-a",
            ))
            .await
            .unwrap();
        store
            .create(sample_session(
                "bootstrap-b",
                "wechat",
                "wechat:tenant-b",
                "tenant-b",
            ))
            .await
            .unwrap();
        store
            .create(sample_session(
                "bootstrap-c",
                "whatsapp",
                "whatsapp:tenant-a",
                "tenant-a",
            ))
            .await
            .unwrap();

        let stored = store
            .get_pending_by_instance("wechat", "wechat:tenant-b")
            .await
            .unwrap();

        assert_eq!(stored.bootstrap_id, "bootstrap-b");
        assert_eq!(stored.account_id, "tenant-b");
    }

    #[tokio::test]
    async fn conflicting_pending_bootstrap_for_same_local_instance_is_rejected() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());

        store
            .create(sample_session(
                "bootstrap-a",
                "wechat",
                "wechat:tenant-a",
                "tenant-a",
            ))
            .await
            .unwrap();

        let err = store
            .create(sample_session(
                "bootstrap-b",
                "wechat",
                "wechat:tenant-a",
                "tenant-a",
            ))
            .await
            .unwrap_err();

        assert!(err.contains("conflicting pending bootstrap"));
    }

    #[tokio::test]
    async fn cancelling_a_bootstrap_session_transitions_state_correctly() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());
        let session = sample_session("bootstrap-1", "wechat", "wechat:tenant-a", "tenant-a");
        store.create(session).await.unwrap();

        let cancelled_at = Utc::now() + chrono::Duration::minutes(1);
        let cancelled = store.cancel("bootstrap-1", cancelled_at).await.unwrap();

        assert_eq!(cancelled.status, BootstrapStatus::Cancelled);
        assert_eq!(cancelled.updated_at, cancelled_at);
        assert!(store
            .get_pending_by_instance("wechat", "wechat:tenant-a")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn expiring_a_bootstrap_session_transitions_state_correctly() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());
        let session = sample_session("bootstrap-1", "wechat", "wechat:tenant-a", "tenant-a");
        store.create(session).await.unwrap();

        let expired_at = Utc::now() + chrono::Duration::minutes(6);
        let expired = store.expire("bootstrap-1", expired_at).await.unwrap();

        assert_eq!(expired.status, BootstrapStatus::Expired);
        assert_eq!(expired.updated_at, expired_at);
        assert!(store
            .get_pending_by_instance("wechat", "wechat:tenant-a")
            .await
            .is_none());
    }

    #[tokio::test]
    async fn provider_handle_is_stored_as_data_not_primary_ownership_key() {
        let temp = TempDir::new().unwrap();
        let store = ChannelBootstrapStore::new(temp.path());
        let mut first = sample_session("bootstrap-a", "wechat", "wechat:tenant-a", "tenant-a");
        first.provider_handle = Some("shared-handle".to_string());
        store.create(first).await.unwrap();

        let mut second = sample_session("bootstrap-b", "wechat", "wechat:tenant-b", "tenant-b");
        second.provider_handle = Some("shared-handle".to_string());
        store.create(second).await.unwrap();

        let tenant_a = store
            .get_pending_by_instance("wechat", "wechat:tenant-a")
            .await
            .unwrap();
        let tenant_b = store
            .get_pending_by_instance("wechat", "wechat:tenant-b")
            .await
            .unwrap();

        assert_eq!(tenant_a.bootstrap_id, "bootstrap-a");
        assert_eq!(tenant_b.bootstrap_id, "bootstrap-b");
        assert_eq!(tenant_a.provider_handle.as_deref(), Some("shared-handle"));
        assert_eq!(tenant_b.provider_handle.as_deref(), Some("shared-handle"));
    }
}
