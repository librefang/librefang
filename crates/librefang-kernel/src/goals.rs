//! Tenant-owned goal storage and persistence.

use chrono::Utc;
use librefang_types::goal::{Goal, GoalId, GoalStatus};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GoalFile {
    goals: Vec<Goal>,
}

/// Persisted goal store keyed by goal ID.
pub struct GoalStore {
    goals: RwLock<HashMap<GoalId, Goal>>,
    persist_path: PathBuf,
}

impl GoalStore {
    pub fn new(home_dir: &Path) -> Self {
        Self {
            goals: RwLock::new(HashMap::new()),
            persist_path: home_dir.join("goals.json"),
        }
    }

    pub fn load(&self) -> Result<usize, String> {
        if !self.persist_path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(&self.persist_path)
            .map_err(|e| format!("Failed to read goals: {e}"))?;
        let file: GoalFile =
            serde_json::from_str(&data).map_err(|e| format!("Failed to parse goals: {e}"))?;
        let count = file.goals.len();
        let mut goals = self.goals.blocking_write();
        goals.clear();
        for goal in file.goals {
            goals.insert(goal.id, goal);
        }
        Ok(count)
    }

    fn persist_snapshot(&self, goals: Vec<Goal>) -> Result<(), String> {
        let file = GoalFile { goals };
        let data = serde_json::to_string_pretty(&file)
            .map_err(|e| format!("Failed to serialize goals: {e}"))?;
        let tmp_path = self.persist_path.with_extension("json.tmp");
        std::fs::write(&tmp_path, data.as_bytes())
            .map_err(|e| format!("Failed to write goals temp file: {e}"))?;
        std::fs::rename(&tmp_path, &self.persist_path)
            .map_err(|e| format!("Failed to rename goals file: {e}"))?;
        Ok(())
    }

    pub fn persist(&self) -> Result<(), String> {
        let goals = self.goals.blocking_read();
        self.persist_snapshot(goals.values().cloned().collect())
    }

    async fn persist_async(&self) -> Result<(), String> {
        let goals = self.goals.read().await;
        self.persist_snapshot(goals.values().cloned().collect())
    }

    pub async fn list_by_account(&self, account_id: &str) -> Vec<Goal> {
        self.goals
            .read()
            .await
            .values()
            .filter(|goal| goal.account_id == account_id)
            .cloned()
            .collect()
    }

    pub async fn get_scoped(&self, goal_id: GoalId, account_id: &str) -> Option<Goal> {
        self.goals
            .read()
            .await
            .get(&goal_id)
            .filter(|goal| goal.account_id == account_id)
            .cloned()
    }

    pub async fn children_scoped(&self, parent_id: GoalId, account_id: &str) -> Vec<Goal> {
        self.goals
            .read()
            .await
            .values()
            .filter(|goal| goal.account_id == account_id && goal.parent_id == Some(parent_id))
            .cloned()
            .collect()
    }

    pub async fn create(&self, goal: Goal) -> Result<Goal, String> {
        goal.validate()?;
        let mut goals = self.goals.write().await;
        if let Some(parent_id) = goal.parent_id {
            let parent = goals
                .get(&parent_id)
                .ok_or_else(|| format!("Parent goal '{parent_id}' not found"))?;
            if parent.account_id != goal.account_id {
                return Err("Parent goal not found".to_string());
            }
        }
        goals.insert(goal.id, goal.clone());
        drop(goals);
        self.persist_async().await?;
        Ok(goal)
    }

    pub async fn update_scoped(
        &self,
        goal_id: GoalId,
        account_id: &str,
        updates: GoalUpdate,
    ) -> Result<Goal, String> {
        let mut goals = self.goals.write().await;
        let current = goals
            .get(&goal_id)
            .cloned()
            .ok_or_else(|| format!("Goal '{goal_id}' not found"))?;
        if current.account_id != account_id {
            return Err(format!("Goal '{goal_id}' not found"));
        }

        if let Some(parent_id) = updates.parent_id {
            if parent_id == Some(goal_id) {
                return Err("A goal cannot be its own parent".to_string());
            }
            if let Some(parent_id) = parent_id {
                let parent = goals
                    .get(&parent_id)
                    .ok_or_else(|| format!("Parent goal '{parent_id}' not found"))?;
                if parent.account_id != account_id {
                    return Err(format!("Parent goal '{parent_id}' not found"));
                }

                let mut ancestor = Some(parent_id);
                let mut seen = HashSet::from([goal_id]);
                while let Some(ancestor_id) = ancestor {
                    if !seen.insert(ancestor_id) {
                        break;
                    }
                    ancestor = goals.get(&ancestor_id).and_then(|goal| goal.parent_id);
                    if ancestor == Some(goal_id) {
                        return Err("Circular parent reference detected".to_string());
                    }
                }
            }
        }

        let mut updated = current.clone();
        if let Some(title) = updates.title {
            updated.title = title;
        }
        if let Some(description) = updates.description {
            updated.description = description;
        }
        if let Some(status) = updates.status {
            updated.status = status;
        }
        if let Some(progress) = updates.progress {
            updated.progress = progress;
        }
        if let Some(parent_id) = updates.parent_id {
            updated.parent_id = parent_id;
        }
        if let Some(agent_id) = updates.agent_id {
            updated.agent_id = agent_id;
        }
        updated.updated_at = Utc::now();
        updated.validate()?;

        goals.insert(goal_id, updated.clone());
        drop(goals);
        self.persist_async().await?;
        Ok(updated)
    }

    pub async fn delete_scoped(&self, goal_id: GoalId, account_id: &str) -> Result<usize, String> {
        let mut goals = self.goals.write().await;
        let target = goals
            .get(&goal_id)
            .cloned()
            .ok_or_else(|| format!("Goal '{goal_id}' not found"))?;
        if target.account_id != account_id {
            return Err(format!("Goal '{goal_id}' not found"));
        }

        let mut ids_to_remove = HashSet::from([goal_id]);
        let mut queue = vec![goal_id];
        while let Some(current_id) = queue.pop() {
            for goal in goals.values() {
                if goal.account_id == account_id
                    && goal.parent_id == Some(current_id)
                    && ids_to_remove.insert(goal.id)
                {
                    queue.push(goal.id);
                }
            }
        }

        let removed = ids_to_remove.len();
        goals.retain(|id, _| !ids_to_remove.contains(id));
        drop(goals);
        self.persist_async().await?;
        Ok(removed)
    }

    pub async fn list_active_for_account(
        &self,
        account_id: &str,
        agent_id_filter: Option<&str>,
    ) -> Vec<Goal> {
        self.goals
            .read()
            .await
            .values()
            .filter(|goal| {
                goal.account_id == account_id
                    && matches!(goal.status, GoalStatus::Pending | GoalStatus::InProgress)
                    && match agent_id_filter {
                        Some(agent_id) => {
                            goal.agent_id.map(|id| id.to_string()) == Some(agent_id.to_string())
                        }
                        None => true,
                    }
            })
            .cloned()
            .collect()
    }

    pub fn list_active_for_account_blocking(
        &self,
        account_id: &str,
        agent_id_filter: Option<&str>,
    ) -> Vec<Goal> {
        self.goals
            .blocking_read()
            .values()
            .filter(|goal| {
                goal.account_id == account_id
                    && matches!(goal.status, GoalStatus::Pending | GoalStatus::InProgress)
                    && match agent_id_filter {
                        Some(agent_id) => {
                            goal.agent_id.map(|id| id.to_string()) == Some(agent_id.to_string())
                        }
                        None => true,
                    }
            })
            .cloned()
            .collect()
    }

    pub async fn update_status_scoped(
        &self,
        goal_id: GoalId,
        account_id: &str,
        status: Option<GoalStatus>,
        progress: Option<u8>,
    ) -> Result<Goal, String> {
        let mut goals = self.goals.write().await;
        let goal = goals
            .get_mut(&goal_id)
            .ok_or_else(|| format!("Goal '{goal_id}' not found"))?;
        if goal.account_id != account_id {
            return Err(format!("Goal '{goal_id}' not found"));
        }
        if let Some(status) = status {
            goal.status = status;
        }
        if let Some(progress) = progress {
            goal.progress = progress;
        }
        goal.updated_at = Utc::now();
        goal.validate()?;
        let updated = goal.clone();
        drop(goals);
        self.persist_async().await?;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_goal(account_id: &str, title: &str) -> Goal {
        Goal {
            id: GoalId::new(),
            title: title.to_string(),
            description: "".to_string(),
            parent_id: None,
            status: GoalStatus::Pending,
            progress: 0,
            agent_id: None,
            account_id: account_id.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_goal_store_persists_account_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let store = GoalStore::new(tmp.path());
        let goal = test_goal("tenant-a", "Ship feature");
        let goal_id = goal.id;
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(store.create(goal)).unwrap();

        let loaded = GoalStore::new(tmp.path());
        let count = loaded.load().unwrap();
        assert_eq!(count, 1);
        let stored = runtime
            .block_on(loaded.get_scoped(goal_id, "tenant-a"))
            .unwrap();
        assert_eq!(stored.account_id, "tenant-a");
        assert!(runtime
            .block_on(loaded.get_scoped(goal_id, "tenant-b"))
            .is_none());
    }
}

#[derive(Default)]
pub struct GoalUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<GoalStatus>,
    pub progress: Option<u8>,
    pub parent_id: Option<Option<GoalId>>,
    pub agent_id: Option<Option<librefang_types::agent::AgentId>>,
}
