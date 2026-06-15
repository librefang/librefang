//! SQLite-backed MCP server config store; each entry is JSON-encoded so the schema doesn't migrate on field additions (#6021).

use librefang_types::config::McpServerConfigEntry;
use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

#[derive(Clone)]
pub struct McpConfigStore {
    pool: Pool<SqliteConnectionManager>,
}

impl McpConfigStore {
    /// Caller must have run `migration::run_migrations` first so `mcp_server_configs` exists.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// `ON CONFLICT DO UPDATE` (not `INSERT OR REPLACE`) so `created_at` survives updates.
    pub fn upsert(&self, entry: &McpServerConfigEntry) -> LibreFangResult<()> {
        let entry_json = serde_json::to_string(entry)
            .map_err(|e| LibreFangError::memory_msg(format!("mcp config serialize failed: {e}")))?;
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute(
            "INSERT INTO mcp_server_configs (name, entry_json)
             VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET
                entry_json = excluded.entry_json,
                updated_at = datetime('now')",
            rusqlite::params![entry.name, entry_json],
        )
        .map_err(|e| LibreFangError::memory_msg(format!("mcp config upsert failed: {e}")))?;
        Ok(())
    }

    /// Deserialization failure surfaces as `Err`, not `Ok(None)`, so a corrupt entry is visible.
    pub fn get(&self, name: &str) -> LibreFangResult<Option<McpServerConfigEntry>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let json: Option<String> = c
            .query_row(
                "SELECT entry_json FROM mcp_server_configs WHERE name = ?1",
                rusqlite::params![name],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(LibreFangError::memory_msg(format!(
                    "mcp config get failed: {other}"
                ))),
            })?;
        match json {
            Some(j) => {
                let entry: McpServerConfigEntry = serde_json::from_str(&j).map_err(|e| {
                    LibreFangError::memory_msg(format!(
                        "mcp config '{name}' deserialize failed: {e}"
                    ))
                })?;
                Ok(Some(entry))
            }
            None => Ok(None),
        }
    }

    /// Ordered by `name` so the boot-time merge is deterministic across processes.
    pub fn load_all(&self) -> LibreFangResult<Vec<McpServerConfigEntry>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let mut stmt = c
            .prepare("SELECT name, entry_json FROM mcp_server_configs ORDER BY name ASC")
            .map_err(|e| {
                LibreFangError::memory_msg(format!("mcp config load_all prepare failed: {e}"))
            })?;
        let rows = stmt
            .query_map([], |row| {
                let name: String = row.get(0)?;
                let json: String = row.get(1)?;
                Ok((name, json))
            })
            .map_err(|e| {
                LibreFangError::memory_msg(format!("mcp config load_all query failed: {e}"))
            })?;
        let mut result = Vec::new();
        for row in rows {
            let (name, json) = row.map_err(|e| {
                LibreFangError::memory_msg(format!("mcp config load_all row read failed: {e}"))
            })?;
            let entry: McpServerConfigEntry = serde_json::from_str(&json).map_err(|e| {
                LibreFangError::memory_msg(format!("mcp config '{name}' deserialize failed: {e}"))
            })?;
            result.push(entry);
        }
        Ok(result)
    }

    /// Delete an MCP server config by name. Returns true if a row was deleted.
    pub fn delete(&self, name: &str) -> LibreFangResult<bool> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let affected = c
            .execute(
                "DELETE FROM mcp_server_configs WHERE name = ?1",
                rusqlite::params![name],
            )
            .map_err(|e| LibreFangError::memory_msg(format!("mcp config delete failed: {e}")))?;
        Ok(affected > 0)
    }

    /// Count stored MCP server configs. Used by tests and operator tooling.
    pub fn count(&self) -> LibreFangResult<usize> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let count: i64 = c
            .query_row("SELECT COUNT(*) FROM mcp_server_configs", [], |row| {
                row.get(0)
            })
            .map_err(|e| LibreFangError::memory_msg(format!("mcp config count failed: {e}")))?;
        Ok(count as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::config::McpTransportEntry;

    fn in_memory_store() -> McpConfigStore {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        {
            let conn = pool.get().unwrap();
            crate::migration::run_migrations(&conn).unwrap();
        }
        McpConfigStore::new(pool)
    }

    fn sample_entry(name: &str, command: &str) -> McpServerConfigEntry {
        McpServerConfigEntry {
            name: name.to_string(),
            template_id: None,
            transport: Some(McpTransportEntry::Stdio {
                command: command.to_string(),
                args: vec!["--flag".to_string()],
            }),
            timeout_secs: 30,
            env: vec!["TOKEN".to_string()],
            headers: vec![],
            oauth: None,
            taint_scanning: true,
            taint_policy: None,
        }
    }

    #[test]
    fn upsert_then_get_roundtrips_entry() {
        let store = in_memory_store();
        let entry = sample_entry("fs", "filesystem-server");
        store.upsert(&entry).unwrap();

        let got = store.get("fs").unwrap().expect("entry should exist");
        assert_eq!(got.name, "fs");
        assert_eq!(got.env, vec!["TOKEN".to_string()]);
        match got.transport {
            Some(McpTransportEntry::Stdio { command, args }) => {
                assert_eq!(command, "filesystem-server");
                assert_eq!(args, vec!["--flag".to_string()]);
            }
            other => panic!("unexpected transport: {other:?}"),
        }
    }

    #[test]
    fn get_missing_returns_none() {
        let store = in_memory_store();
        assert!(store.get("nope").unwrap().is_none());
    }

    #[test]
    fn upsert_same_name_replaces_in_place() {
        let store = in_memory_store();
        store.upsert(&sample_entry("srv", "v1")).unwrap();
        store.upsert(&sample_entry("srv", "v2")).unwrap();

        assert_eq!(store.count().unwrap(), 1);
        let got = store.get("srv").unwrap().unwrap();
        match got.transport {
            Some(McpTransportEntry::Stdio { command, .. }) => assert_eq!(command, "v2"),
            other => panic!("unexpected transport: {other:?}"),
        }
    }

    #[test]
    fn load_all_is_sorted_by_name() {
        let store = in_memory_store();
        store.upsert(&sample_entry("zeta", "z")).unwrap();
        store.upsert(&sample_entry("alpha", "a")).unwrap();
        store.upsert(&sample_entry("mike", "m")).unwrap();

        let names: Vec<String> = store
            .load_all()
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["alpha", "mike", "zeta"]);
    }

    #[test]
    fn delete_removes_and_reports() {
        let store = in_memory_store();
        store.upsert(&sample_entry("srv", "v1")).unwrap();
        assert!(store.delete("srv").unwrap());
        assert!(!store.delete("srv").unwrap());
        assert!(store.get("srv").unwrap().is_none());
        assert_eq!(store.count().unwrap(), 0);
    }

    #[test]
    fn empty_store_loads_nothing() {
        let store = in_memory_store();
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.load_all().unwrap().is_empty());
    }
}
