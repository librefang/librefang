//! SQLite-backed group roster store.
//!
//! Tracks which users have been seen in each group chat, persisting across
//! daemon restarts. Agents query this via the `group_members` tool instead
//! of having the roster injected into the system prompt (saving tokens).

use rusqlite::Connection;
use std::sync::{Arc, Mutex};

/// Persistent roster of group chat members, backed by SQLite.
pub struct RosterStore {
    conn: Arc<Mutex<Connection>>,
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
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Insert or update a member in the roster.
    pub fn upsert(
        &self,
        channel: &str,
        chat_id: &str,
        user_id: &str,
        display_name: &str,
        username: Option<&str>,
    ) {
        if chat_id.is_empty() || user_id.is_empty() {
            return;
        }
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "INSERT INTO group_roster (channel_type, chat_id, user_id, display_name, username, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, strftime('%s','now'), strftime('%s','now'))
             ON CONFLICT(channel_type, chat_id, user_id) DO UPDATE SET
               display_name = excluded.display_name,
               username = COALESCE(excluded.username, group_roster.username),
               last_seen = strftime('%s','now')",
            rusqlite::params![channel, chat_id, user_id, display_name, username],
        );
    }

    /// List all members of a group chat, ordered by display name.
    pub fn members(&self, channel: &str, chat_id: &str) -> Vec<(String, String, Option<String>)> {
        let c = self.conn.lock().unwrap();
        let mut stmt = c
            .prepare(
                "SELECT user_id, display_name, username FROM group_roster
                 WHERE channel_type = ?1 AND chat_id = ?2
                 ORDER BY display_name",
            )
            .unwrap();
        stmt.query_map(rusqlite::params![channel, chat_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Remove a single member from the roster.
    pub fn remove_member(&self, channel: &str, chat_id: &str, user_id: &str) {
        let c = self.conn.lock().unwrap();
        let _ = c.execute(
            "DELETE FROM group_roster WHERE channel_type = ?1 AND chat_id = ?2 AND user_id = ?3",
            rusqlite::params![channel, chat_id, user_id],
        );
    }

    /// Count the members in a group chat.
    pub fn member_count(&self, channel: &str, chat_id: &str) -> usize {
        let c = self.conn.lock().unwrap();
        c.query_row(
            "SELECT COUNT(*) FROM group_roster WHERE channel_type = ?1 AND chat_id = ?2",
            rusqlite::params![channel, chat_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_store() -> RosterStore {
        let conn = Connection::open_in_memory().unwrap();
        crate::migration::run_migrations(&conn).expect("migrations must apply");
        RosterStore::new(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn upsert_and_list() {
        let store = in_memory_store();
        store.upsert("telegram", "-100", "1", "Alice", Some("alice"));
        store.upsert("telegram", "-100", "2", "Bob", None);

        let members = store.members("telegram", "-100");
        assert_eq!(members.len(), 2);
        assert_eq!(members[0].1, "Alice");
        assert_eq!(members[1].1, "Bob");
        assert_eq!(members[0].2, Some("alice".to_string()));
        assert_eq!(members[1].2, None);
    }

    #[test]
    fn idempotent_upsert_updates_display_name() {
        let store = in_memory_store();
        store.upsert("telegram", "-100", "1", "Alice", Some("alice"));
        store.upsert("telegram", "-100", "1", "Alice Updated", Some("alice"));

        let members = store.members("telegram", "-100");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].1, "Alice Updated");
    }

    #[test]
    fn remove_member() {
        let store = in_memory_store();
        store.upsert("telegram", "-100", "1", "Alice", None);
        store.upsert("telegram", "-100", "2", "Bob", None);
        assert_eq!(store.member_count("telegram", "-100"), 2);

        store.remove_member("telegram", "-100", "1");
        assert_eq!(store.member_count("telegram", "-100"), 1);
        let members = store.members("telegram", "-100");
        assert_eq!(members[0].1, "Bob");
    }

    #[test]
    fn empty_chat_returns_nothing() {
        let store = in_memory_store();
        let members = store.members("telegram", "-999");
        assert!(members.is_empty());
        assert_eq!(store.member_count("telegram", "-999"), 0);
    }

    #[test]
    fn different_chats_are_isolated() {
        let store = in_memory_store();
        store.upsert("telegram", "-100", "1", "Alice", None);
        store.upsert("telegram", "-200", "2", "Bob", None);

        assert_eq!(store.member_count("telegram", "-100"), 1);
        assert_eq!(store.member_count("telegram", "-200"), 1);
    }

    #[test]
    fn empty_ids_are_ignored() {
        let store = in_memory_store();
        store.upsert("telegram", "", "1", "Alice", None);
        store.upsert("telegram", "-100", "", "Bob", None);
        assert_eq!(store.member_count("telegram", "-100"), 0);
        assert_eq!(store.member_count("telegram", ""), 0);
    }
}
