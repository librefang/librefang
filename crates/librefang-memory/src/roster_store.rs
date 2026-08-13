//! SQLite-backed group roster store.
//!
//! Tracks which users have been seen in each group chat, persisting across
//! daemon restarts. Agents query this via the `group_members` tool instead
//! of having the roster injected into the system prompt (saving tokens).

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use librefang_types::error::{LibreFangError, LibreFangResult};

/// Persistent roster of group chat members, backed by SQLite.
pub struct RosterStore {
    pool: Pool<SqliteConnectionManager>,
}

impl RosterStore {
    /// Wrap an existing SQLite connection.
    ///
    /// The `group_roster` table is created by `migration::migrate_v28`,
    /// which `MemorySubstrate::open` runs before constructing the store.
    /// We deliberately don't run schema DDL here so a) every memory
    /// table goes through the single migration ladder and b)
    /// constructing a `RosterStore` can never panic on a locked /
    /// read-only DB — the failure surfaces from `MemorySubstrate::open`
    /// at boot instead.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    /// Insert or update a member in the roster.
    pub fn upsert(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        display_name: &str,
        username: Option<&str>,
    ) -> LibreFangResult<()> {
        if chat_id.is_empty() || user_id.is_empty() {
            return Ok(());
        }
        let c = self.pool.get().map_err(|error| {
            metrics::counter!(
                "librefang_memory_pool_get_failed_total",
                "store" => "roster",
                "op" => "upsert",
            )
            .increment(1);
            LibreFangError::memory(error)
        })?;
        c.execute(
            "INSERT INTO group_roster (channel_type, chat_id, user_id, display_name, username, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'), strftime('%s','now'))
             ON CONFLICT(channel_type, chat_id, user_id) DO UPDATE SET
               display_name = excluded.display_name,
               username = COALESCE(excluded.username, group_roster.username),
               last_seen = strftime('%s','now')",
            rusqlite::params![channel, chat_id, user_id, display_name, username],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// List all members of a group chat, ordered by display name.
    pub fn members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> LibreFangResult<Vec<(String, String, Option<String>)>> {
        let c = self.pool.get().map_err(|error| {
            metrics::counter!(
                "librefang_memory_pool_get_failed_total",
                "store" => "roster",
                "op" => "members",
            )
            .increment(1);
            LibreFangError::memory(error)
        })?;
        let mut stmt = c
            .prepare(
                "SELECT user_id, display_name, username FROM group_roster
                 WHERE channel_type = ?1 AND chat_id = ?2
                 ORDER BY display_name",
            )
            .map_err(LibreFangError::memory)?;
        let rows = stmt
            .query_map(rusqlite::params![channel, chat_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(LibreFangError::memory)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(LibreFangError::memory)
    }

    /// Remove a single member from the roster.
    pub fn remove_member(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
    ) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(|error| {
            metrics::counter!(
                "librefang_memory_pool_get_failed_total",
                "store" => "roster",
                "op" => "remove_member",
            )
            .increment(1);
            LibreFangError::memory(error)
        })?;
        c.execute(
            "DELETE FROM group_roster WHERE channel_type = ?1 AND chat_id = ?2 AND user_id = ?3",
            rusqlite::params![channel, chat_id, user_id],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Count the members in a group chat.
    pub fn member_count(&self, channel: &str, chat_id: &str) -> LibreFangResult<usize> {
        let c = self.pool.get().map_err(|error| {
            metrics::counter!(
                "librefang_memory_pool_get_failed_total",
                "store" => "roster",
                "op" => "member_count",
            )
            .increment(1);
            LibreFangError::memory(error)
        })?;
        let count = c
            .query_row(
                "SELECT COUNT(*) FROM group_roster WHERE channel_type = ?1 AND chat_id = ?2",
                rusqlite::params![channel, chat_id],
                |row| row.get::<_, i64>(0),
            )
            .map_err(LibreFangError::memory)?;
        usize::try_from(count).map_err(|_| {
            LibreFangError::memory_msg(format!("invalid roster member count: {count}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_store() -> RosterStore {
        let pool = Pool::builder()
            .max_size(1)
            .build(SqliteConnectionManager::memory())
            .unwrap();
        crate::migration::run_migrations(&pool.get().unwrap()).expect("migrations must apply");
        RosterStore::new(pool)
    }

    #[test]
    fn upsert_and_list() {
        let store = in_memory_store();
        store
            .upsert("telegram", "-100", "1", "Alice", Some("alice"))
            .unwrap();
        store.upsert("telegram", "-100", "2", "Bob", None).unwrap();

        let members = store.members("telegram", "-100").unwrap();
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].1, "Alice");
        assert_eq!(members[1].1, "Bob");
        assert_eq!(members[0].2, Some("alice".to_string()));
        assert_eq!(members[1].2, None);
    }

    #[test]
    fn idempotent_upsert_updates_display_name() {
        let store = in_memory_store();
        store
            .upsert("telegram", "-100", "1", "Alice", Some("alice"))
            .unwrap();
        store
            .upsert("telegram", "-100", "1", "Alice Updated", Some("alice"))
            .unwrap();

        let members = store.members("telegram", "-100").unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].1, "Alice Updated");
    }

    #[test]
    fn remove_member() {
        let store = in_memory_store();
        store
            .upsert("telegram", "-100", "1", "Alice", None)
            .unwrap();
        store.upsert("telegram", "-100", "2", "Bob", None).unwrap();
        assert_eq!(store.member_count("telegram", "-100").unwrap(), 2);

        store.remove_member("telegram", "-100", "1").unwrap();
        assert_eq!(store.member_count("telegram", "-100").unwrap(), 1);
        let members = store.members("telegram", "-100").unwrap();
        assert_eq!(members[0].1, "Bob");
    }

    #[test]
    fn empty_chat_returns_nothing() {
        let store = in_memory_store();
        let members = store.members("telegram", "-999").unwrap();
        assert!(members.is_empty());
        assert_eq!(store.member_count("telegram", "-999").unwrap(), 0);
    }

    #[test]
    fn different_chats_are_isolated() {
        let store = in_memory_store();
        store
            .upsert("telegram", "-100", "1", "Alice", None)
            .unwrap();
        store.upsert("telegram", "-200", "2", "Bob", None).unwrap();

        assert_eq!(store.member_count("telegram", "-100").unwrap(), 1);
        assert_eq!(store.member_count("telegram", "-200").unwrap(), 1);
    }

    #[test]
    fn empty_ids_are_ignored() {
        let store = in_memory_store();
        store.upsert("telegram", "", "1", "Alice", None).unwrap();
        store.upsert("telegram", "-100", "", "Bob", None).unwrap();
        assert_eq!(store.member_count("telegram", "-100").unwrap(), 0);
        assert_eq!(store.member_count("telegram", "").unwrap(), 0);
    }

    #[test]
    fn storage_errors_are_returned_to_callers() {
        let store = in_memory_store();
        store
            .pool
            .get()
            .unwrap()
            .execute("DROP TABLE group_roster", [])
            .unwrap();

        assert!(store
            .upsert("telegram", "-100", "1", "Alice", None)
            .is_err());
        assert!(store.members("telegram", "-100").is_err());
        assert!(store.remove_member("telegram", "-100", "1").is_err());
        assert!(store.member_count("telegram", "-100").is_err());
    }

    #[test]
    fn corrupt_member_rows_are_not_silently_dropped() {
        let store = in_memory_store();
        store
            .pool
            .get()
            .unwrap()
            .execute(
                "INSERT INTO group_roster (channel_type, chat_id, user_id, display_name) \
                 VALUES ('telegram', '-100', '1', X'00')",
                [],
            )
            .unwrap();

        assert!(store.members("telegram", "-100").is_err());
    }
}
