//! SQLite-backed channel-instance binding store (#5671).
//!
//! Holds the two tables that drive Model A inbound dispatch:
//!
//! - `channel_instance_defaults` — one row per `[[sidecar_channels]]`
//!   instance, seeded from config at boot, giving the agent a channel
//!   instance routes to by default.
//! - `conversation_bindings` — a per-`(instance, conversation)` override
//!   written by the `/agent` command; supersedes the instance default.
//!
//! [`ChannelBindingStore::resolve`] performs the two-level lookup
//! (conversation override first, then instance default) that replaces the
//! non-deterministic `list_agents().first()` fallback the bridge used to
//! reach for. Both tables store the agent *name*, not the per-spawn
//! `AgentId` uuid: config and the agent registry resolve agents by stable
//! name, and the bridge maps name -> id at dispatch.

use librefang_types::error::{LibreFangError, LibreFangResult};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

#[derive(Clone)]
pub struct ChannelBindingStore {
    pool: Pool<SqliteConnectionManager>,
}

impl ChannelBindingStore {
    /// Caller must have run `migration::run_migrations` first so the
    /// `channel_instance_defaults` / `conversation_bindings` tables exist.
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    // --- instance defaults ---------------------------------------------------

    /// Seed (or refresh) the instance default from config at boot.
    ///
    /// Upserts via `ON CONFLICT DO UPDATE` (not `INSERT OR REPLACE`, which
    /// would delete+reinsert the row) because config is the source of truth
    /// for the instance default: a changed `[[sidecar_channels]] agent` value
    /// must win on the next boot. Per-conversation overrides live in a
    /// separate table and are never touched here.
    pub fn seed_instance_default(&self, instance: &str, agent: &str) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute(
            "INSERT INTO channel_instance_defaults (instance_name, agent_name, bound_by)
             VALUES (?1, ?2, 'config')
             ON CONFLICT(instance_name) DO UPDATE SET
                agent_name = excluded.agent_name,
                bound_at   = datetime('now'),
                bound_by   = 'config'",
            rusqlite::params![instance, agent],
        )
        .map_err(|e| {
            LibreFangError::memory_msg(format!("channel instance default seed failed: {e}"))
        })?;
        Ok(())
    }

    /// Operator/runtime rebind of an instance default (e.g. via the REST
    /// mirror). `bound_by` records who set it for the audit trail.
    pub fn set_instance_default(
        &self,
        instance: &str,
        agent: &str,
        bound_by: &str,
    ) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute(
            "INSERT INTO channel_instance_defaults (instance_name, agent_name, bound_by)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(instance_name) DO UPDATE SET
                agent_name = excluded.agent_name,
                bound_at   = datetime('now'),
                bound_by   = excluded.bound_by",
            rusqlite::params![instance, agent, bound_by],
        )
        .map_err(|e| {
            LibreFangError::memory_msg(format!("channel instance default set failed: {e}"))
        })?;
        Ok(())
    }

    /// The bound agent name for an instance, if one is configured.
    pub fn instance_default(&self, instance: &str) -> LibreFangResult<Option<String>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.query_row(
            "SELECT agent_name FROM channel_instance_defaults WHERE instance_name = ?1",
            rusqlite::params![instance],
            |row| row.get::<_, String>(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(LibreFangError::memory_msg(format!(
                "channel instance default get failed: {other}"
            ))),
        })
    }

    /// Remove an instance default. Returns true if a row was deleted.
    pub fn clear_instance_default(&self, instance: &str) -> LibreFangResult<bool> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let affected = c
            .execute(
                "DELETE FROM channel_instance_defaults WHERE instance_name = ?1",
                rusqlite::params![instance],
            )
            .map_err(|e| {
                LibreFangError::memory_msg(format!("channel instance default clear failed: {e}"))
            })?;
        Ok(affected > 0)
    }

    /// Count configured instance defaults. Used by boot logging and tests.
    pub fn count_instance_defaults(&self) -> LibreFangResult<usize> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM channel_instance_defaults",
                [],
                |row| row.get(0),
            )
            .map_err(|e| {
                LibreFangError::memory_msg(format!("channel instance default count failed: {e}"))
            })?;
        Ok(n as usize)
    }

    // --- per-conversation overrides -----------------------------------------

    /// Write a per-conversation override (the `/agent <name>` action).
    pub fn set_conversation_binding(
        &self,
        instance: &str,
        conversation_id: &str,
        agent: &str,
        bound_by: &str,
    ) -> LibreFangResult<()> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.execute(
            "INSERT INTO conversation_bindings (instance_name, conversation_id, agent_name, bound_by)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(instance_name, conversation_id) DO UPDATE SET
                agent_name = excluded.agent_name,
                bound_at   = datetime('now'),
                bound_by   = excluded.bound_by",
            rusqlite::params![instance, conversation_id, agent, bound_by],
        )
        .map_err(|e| {
            LibreFangError::memory_msg(format!("conversation binding set failed: {e}"))
        })?;
        Ok(())
    }

    /// The override agent name for a conversation, if one exists.
    pub fn conversation_binding(
        &self,
        instance: &str,
        conversation_id: &str,
    ) -> LibreFangResult<Option<String>> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        c.query_row(
            "SELECT agent_name FROM conversation_bindings
             WHERE instance_name = ?1 AND conversation_id = ?2",
            rusqlite::params![instance, conversation_id],
            |row| row.get::<_, String>(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(LibreFangError::memory_msg(format!(
                "conversation binding get failed: {other}"
            ))),
        })
    }

    /// Clear a conversation override (the `/agent reset` action). Returns
    /// true if a row was deleted.
    pub fn clear_conversation_binding(
        &self,
        instance: &str,
        conversation_id: &str,
    ) -> LibreFangResult<bool> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let affected = c
            .execute(
                "DELETE FROM conversation_bindings
                 WHERE instance_name = ?1 AND conversation_id = ?2",
                rusqlite::params![instance, conversation_id],
            )
            .map_err(|e| {
                LibreFangError::memory_msg(format!("conversation binding clear failed: {e}"))
            })?;
        Ok(affected > 0)
    }

    /// Count per-conversation overrides. Used by tests and operator tooling.
    pub fn count_conversation_bindings(&self) -> LibreFangResult<usize> {
        let c = self.pool.get().map_err(LibreFangError::memory)?;
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM conversation_bindings", [], |row| {
                row.get(0)
            })
            .map_err(|e| {
                LibreFangError::memory_msg(format!("conversation binding count failed: {e}"))
            })?;
        Ok(n as usize)
    }

    // --- the two-level dispatch lookup --------------------------------------

    /// Resolve the agent name bound to `(instance, conversation_id)`:
    /// the per-conversation override wins, falling back to the instance
    /// default, then `None` (the bridge falls through to its legacy chain).
    pub fn resolve(
        &self,
        instance: &str,
        conversation_id: &str,
    ) -> LibreFangResult<Option<String>> {
        if let Some(agent) = self.conversation_binding(instance, conversation_id)? {
            return Ok(Some(agent));
        }
        self.instance_default(instance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_store() -> ChannelBindingStore {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        {
            let conn = pool.get().unwrap();
            crate::migration::run_migrations(&conn).unwrap();
        }
        ChannelBindingStore::new(pool)
    }

    #[test]
    fn instance_default_seed_then_get() {
        let store = in_memory_store();
        store.seed_instance_default("tg-bot", "researcher").unwrap();
        assert_eq!(
            store.instance_default("tg-bot").unwrap().as_deref(),
            Some("researcher")
        );
        assert_eq!(store.instance_default("unknown").unwrap(), None);
    }

    #[test]
    fn reseed_overwrites_default_in_place() {
        let store = in_memory_store();
        store.seed_instance_default("tg-bot", "researcher").unwrap();
        store.seed_instance_default("tg-bot", "legal").unwrap();
        assert_eq!(store.count_instance_defaults().unwrap(), 1);
        assert_eq!(
            store.instance_default("tg-bot").unwrap().as_deref(),
            Some("legal")
        );
    }

    #[test]
    fn conversation_override_wins_over_instance_default() {
        let store = in_memory_store();
        store.seed_instance_default("tg-bot", "researcher").unwrap();
        // No override yet -> resolve returns the instance default.
        assert_eq!(
            store.resolve("tg-bot", "peer-42").unwrap().as_deref(),
            Some("researcher")
        );
        // Override this one conversation -> resolve returns the override.
        store
            .set_conversation_binding("tg-bot", "peer-42", "legal", "user:admin")
            .unwrap();
        assert_eq!(
            store.resolve("tg-bot", "peer-42").unwrap().as_deref(),
            Some("legal")
        );
        // A different conversation on the same instance still gets the default.
        assert_eq!(
            store.resolve("tg-bot", "peer-99").unwrap().as_deref(),
            Some("researcher")
        );
    }

    #[test]
    fn resolve_with_no_binding_is_none() {
        let store = in_memory_store();
        assert_eq!(store.resolve("tg-bot", "peer-42").unwrap(), None);
    }

    #[test]
    fn clear_conversation_falls_back_to_default() {
        let store = in_memory_store();
        store.seed_instance_default("tg-bot", "researcher").unwrap();
        store
            .set_conversation_binding("tg-bot", "peer-42", "legal", "user:admin")
            .unwrap();
        assert!(store
            .clear_conversation_binding("tg-bot", "peer-42")
            .unwrap());
        assert!(!store
            .clear_conversation_binding("tg-bot", "peer-42")
            .unwrap());
        assert_eq!(
            store.resolve("tg-bot", "peer-42").unwrap().as_deref(),
            Some("researcher")
        );
    }

    #[test]
    fn conversation_binding_is_scoped_per_instance() {
        // The #5672 cross-bot leak guard: the same conversation id on two
        // different instances must resolve independently.
        let store = in_memory_store();
        store
            .set_conversation_binding("bot-a", "shared-peer", "agent-a", "config")
            .unwrap();
        store
            .set_conversation_binding("bot-b", "shared-peer", "agent-b", "config")
            .unwrap();
        assert_eq!(
            store.resolve("bot-a", "shared-peer").unwrap().as_deref(),
            Some("agent-a")
        );
        assert_eq!(
            store.resolve("bot-b", "shared-peer").unwrap().as_deref(),
            Some("agent-b")
        );
    }

    #[test]
    fn set_then_clear_instance_default() {
        let store = in_memory_store();
        store
            .set_instance_default("tg-bot", "researcher", "user:admin")
            .unwrap();
        assert_eq!(
            store.instance_default("tg-bot").unwrap().as_deref(),
            Some("researcher")
        );
        assert!(store.clear_instance_default("tg-bot").unwrap());
        assert_eq!(store.instance_default("tg-bot").unwrap(), None);
    }
}
