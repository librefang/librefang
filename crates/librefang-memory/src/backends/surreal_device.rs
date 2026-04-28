//! SurrealDB-backed implementation of [`DeviceBackend`].
//!
//! Persists paired-device records to the `paired_devices` SurrealDB table
//! (defined in `009_paired_devices.surql`).  Device IDs are used directly
//! as SurrealDB record IDs for O(1) upserts.

use std::sync::Arc;

use surrealdb::{engine::any::Any, Surreal};

use librefang_types::error::{LibreFangError, LibreFangResult};

use librefang_storage::SurrealSession;

use crate::backend::DeviceBackend;

/// Run a future on the current Tokio runtime or spin up a temporary one.
fn block_on<F: std::future::Future>(f: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(f)),
        Err(_) => tokio::runtime::Runtime::new()
            .expect("tokio runtime")
            .block_on(f),
    }
}

/// SurrealDB implementation of [`DeviceBackend`].
#[derive(Clone)]
pub struct SurrealDeviceStore {
    db: Arc<Surreal<Any>>,
}

impl SurrealDeviceStore {
    /// Open against an existing [`SurrealSession`].
    pub fn open(session: &SurrealSession) -> Self {
        Self {
            db: Arc::new(session.client().clone()),
        }
    }

    /// Wrap an existing connected SurrealDB instance.
    pub fn new(db: Arc<Surreal<Any>>) -> Self {
        Self { db }
    }
}

impl DeviceBackend for SurrealDeviceStore {
    fn load_paired_devices(&self) -> LibreFangResult<Vec<serde_json::Value>> {
        block_on(async {
            let mut res = self
                .db
                .query("SELECT * FROM paired_devices")
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB load_devices: {e}")))?;
            let rows: Vec<serde_json::Value> = res
                .take(0)
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB load_devices: {e}")))?;
            // Normalize the SurrealDB rows to plain JSON objects matching the
            // SQLite column layout expected by the kernel.
            Ok(rows
                .into_iter()
                .filter_map(|row| {
                    let device_id = row.get("device_id")?.as_str()?.to_string();
                    let display_name = row
                        .get("display_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let platform = row
                        .get("platform")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let paired_at = row
                        .get("paired_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let last_seen = row
                        .get("last_seen")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let push_token = row
                        .get("push_token")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());

                    let api_key_hash = row
                        .get("api_key_hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    Some(serde_json::json!({
                        "device_id": device_id,
                        "display_name": display_name,
                        "platform": platform,
                        "paired_at": paired_at,
                        "last_seen": last_seen,
                        "push_token": push_token,
                        "api_key_hash": api_key_hash,
                    }))
                })
                .collect())
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn save_paired_device(
        &self,
        device_id: &str,
        display_name: &str,
        platform: &str,
        paired_at: &str,
        last_seen: &str,
        push_token: Option<&str>,
        api_key_hash: &str,
    ) -> LibreFangResult<()> {
        let row = serde_json::json!({
            "device_id": device_id,
            "display_name": display_name,
            "platform": platform,
            "paired_at": paired_at,
            "last_seen": last_seen,
            "push_token": push_token,
            "api_key_hash": api_key_hash,
        });
        // Use device_id as record ID for idempotent upserts
        let safe_id = device_id.replace([':', '/'], "_");
        block_on(async {
            self.db
                .upsert::<Option<serde_json::Value>>(("paired_devices", safe_id))
                .content(row)
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB save_device: {e}")))?;
            Ok(())
        })
    }

    fn remove_paired_device(&self, device_id: &str) -> LibreFangResult<()> {
        let safe_id = device_id.replace([':', '/'], "_");
        block_on(async {
            self.db
                .delete::<Option<serde_json::Value>>(("paired_devices", safe_id))
                .await
                .map_err(|e| LibreFangError::Memory(format!("SurrealDB remove_device: {e}")))?;
            Ok(())
        })
    }
}
