//! Tenant-owned provider configuration and defaults.

use librefang_types::config::DefaultModelConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProviderAccountsFile {
    records: Vec<TenantProviderRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantProviderRecord {
    pub account_id: String,
    pub provider: String,
    pub api_key_env: Option<String>,
    pub base_url: Option<String>,
    pub default_model: Option<String>,
    pub is_default: bool,
}

impl TenantProviderRecord {
    fn is_empty(&self) -> bool {
        self.api_key_env.is_none()
            && self.base_url.is_none()
            && self.default_model.is_none()
            && !self.is_default
    }
}

#[derive(Default)]
pub struct TenantProviderUpdate {
    pub api_key_env: Option<Option<String>>,
    pub base_url: Option<Option<String>>,
}

pub struct ProviderAccountStore {
    records: RwLock<HashMap<(String, String), TenantProviderRecord>>,
    persist_path: PathBuf,
}

impl ProviderAccountStore {
    fn with_sync_read<T>(
        &self,
        f: impl FnOnce(&HashMap<(String, String), TenantProviderRecord>) -> T,
    ) -> T {
        if let Ok(records) = self.records.try_read() {
            return f(&records);
        }
        let records = self.records.blocking_read();
        f(&records)
    }

    pub fn new(home_dir: &Path) -> Self {
        Self {
            records: RwLock::new(HashMap::new()),
            persist_path: home_dir.join("provider_accounts.json"),
        }
    }

    pub fn load(&self) -> Result<usize, String> {
        if !self.persist_path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(&self.persist_path)
            .map_err(|e| format!("Failed to read provider accounts: {e}"))?;
        let file: ProviderAccountsFile = serde_json::from_str(&data)
            .map_err(|e| format!("Failed to parse provider accounts: {e}"))?;
        let count = file.records.len();
        let mut records = self.records.blocking_write();
        records.clear();
        for record in file.records {
            records.insert((record.account_id.clone(), record.provider.clone()), record);
        }
        Ok(count)
    }

    fn persist_snapshot(&self, records: Vec<TenantProviderRecord>) -> Result<(), String> {
        let file = ProviderAccountsFile { records };
        let data = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("Failed to serialize provider accounts: {e}"))?;
        let tmp_path = self.persist_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, data.as_bytes())
            .map_err(|e| format!("Failed to write provider accounts temp file: {e}"))?;
        std::fs::rename(&tmp_path, &self.persist_path)
            .map_err(|e| format!("Failed to rename provider accounts file: {e}"))?;
        Ok(())
    }

    async fn persist_async(&self) -> Result<(), String> {
        let records = self.records.read().await;
        self.persist_snapshot(records.values().cloned().collect())
    }

    pub async fn list_by_account(&self, account_id: &str) -> Vec<TenantProviderRecord> {
        self.records
            .read()
            .await
            .values()
            .filter(|record| record.account_id == account_id)
            .cloned()
            .collect()
    }

    pub fn list_by_account_blocking(&self, account_id: &str) -> Vec<TenantProviderRecord> {
        self.with_sync_read(|records| {
            records
                .values()
                .filter(|record| record.account_id == account_id)
                .cloned()
                .collect()
        })
    }

    pub async fn get_scoped(
        &self,
        account_id: &str,
        provider: &str,
    ) -> Option<TenantProviderRecord> {
        self.records
            .read()
            .await
            .get(&(account_id.to_string(), provider.to_string()))
            .cloned()
    }

    pub async fn effective_default_for_account_async(
        &self,
        account_id: &str,
    ) -> Option<DefaultModelConfig> {
        self.records
            .read()
            .await
            .values()
            .find(|record| record.account_id == account_id && record.is_default)
            .and_then(|record| {
                record
                    .default_model
                    .as_ref()
                    .map(|model| DefaultModelConfig {
                        provider: record.provider.clone(),
                        model: model.clone(),
                        api_key_env: record.api_key_env.clone().unwrap_or_default(),
                        base_url: record.base_url.clone(),
                        ..Default::default()
                    })
            })
    }

    pub fn get_scoped_blocking(
        &self,
        account_id: &str,
        provider: &str,
    ) -> Option<TenantProviderRecord> {
        self.with_sync_read(|records| {
            records
                .get(&(account_id.to_string(), provider.to_string()))
                .cloned()
        })
    }

    pub async fn upsert_scoped(
        &self,
        account_id: &str,
        provider: &str,
        update: TenantProviderUpdate,
    ) -> Result<TenantProviderRecord, String> {
        let key = (account_id.to_string(), provider.to_string());
        let mut records = self.records.write().await;
        let record = records
            .entry(key.clone())
            .or_insert_with(|| TenantProviderRecord {
                account_id: account_id.to_string(),
                provider: provider.to_string(),
                api_key_env: None,
                base_url: None,
                default_model: None,
                is_default: false,
            });
        if let Some(api_key_env) = update.api_key_env {
            record.api_key_env = api_key_env;
        }
        if let Some(base_url) = update.base_url {
            record.base_url = base_url;
        }
        let updated = record.clone();
        if updated.is_empty() {
            records.remove(&key);
        }
        drop(records);
        self.persist_async().await?;
        Ok(updated)
    }

    pub async fn set_default_scoped(
        &self,
        account_id: &str,
        provider: &str,
        model_id: String,
    ) -> Result<TenantProviderRecord, String> {
        let mut records = self.records.write().await;
        for record in records.values_mut() {
            if record.account_id == account_id {
                record.is_default = false;
                record.default_model = None;
            }
        }
        let key = (account_id.to_string(), provider.to_string());
        let record = records.entry(key).or_insert_with(|| TenantProviderRecord {
            account_id: account_id.to_string(),
            provider: provider.to_string(),
            api_key_env: None,
            base_url: None,
            default_model: None,
            is_default: false,
        });
        record.is_default = true;
        record.default_model = Some(model_id);
        let updated = record.clone();
        drop(records);
        self.persist_async().await?;
        Ok(updated)
    }

    pub fn effective_default_for_account(&self, account_id: &str) -> Option<DefaultModelConfig> {
        self.with_sync_read(|records| {
            records
                .values()
                .find(|record| record.account_id == account_id && record.is_default)
                .and_then(|record| {
                    record
                        .default_model
                        .as_ref()
                        .map(|model| DefaultModelConfig {
                            provider: record.provider.clone(),
                            model: model.clone(),
                            api_key_env: record.api_key_env.clone().unwrap_or_default(),
                            base_url: record.base_url.clone(),
                            ..Default::default()
                        })
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_account_store_persists_account_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ProviderAccountStore::new(tmp.path());
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime
            .block_on(store.upsert_scoped(
                "tenant-a",
                "openai",
                TenantProviderUpdate {
                    api_key_env: Some(Some("OPENAI_API_KEY__ACCT_TENANT_A".to_string())),
                    base_url: Some(Some("https://tenant-a.example/v1".to_string())),
                },
            ))
            .unwrap();
        runtime
            .block_on(store.set_default_scoped("tenant-a", "openai", "gpt-4o-mini".to_string()))
            .unwrap();

        let loaded = ProviderAccountStore::new(tmp.path());
        let count = loaded.load().unwrap();
        assert_eq!(count, 1);
        let record = loaded
            .get_scoped_blocking("tenant-a", "openai")
            .expect("tenant record");
        assert_eq!(record.account_id, "tenant-a");
        assert!(record.is_default);
        assert_eq!(
            record.api_key_env.as_deref(),
            Some("OPENAI_API_KEY__ACCT_TENANT_A")
        );
        assert!(loaded.get_scoped_blocking("tenant-b", "openai").is_none());
    }

    #[tokio::test]
    async fn sync_lookups_do_not_panic_inside_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ProviderAccountStore::new(tmp.path());
        store
            .upsert_scoped(
                "tenant-a",
                "openai",
                TenantProviderUpdate {
                    api_key_env: Some(Some("OPENAI_API_KEY__TENANT_A".to_string())),
                    base_url: Some(Some("https://tenant-a.example/v1".to_string())),
                },
            )
            .await
            .unwrap();
        store
            .set_default_scoped("tenant-a", "openai", "gpt-4o-mini".to_string())
            .await
            .unwrap();

        let record = store
            .get_scoped_blocking("tenant-a", "openai")
            .expect("scoped record should be readable from async context");
        assert_eq!(record.account_id, "tenant-a");

        let default = store
            .effective_default_for_account("tenant-a")
            .expect("default should be readable from async context");
        assert_eq!(default.provider, "openai");
        assert_eq!(default.model, "gpt-4o-mini");
    }
}
