//! SQLite-backed group roster store.
//!
//! Tracks which users are known in each group chat, persisting across daemon restarts.
//! Agents query this on demand through the `channel_members` tool (#7865) rather than having the roster injected into every system prompt, which keeps the token cost proportional to the questions asked instead of to the number of turns.
//!
//! # Two kinds of membership, one table
//!
//! Every row carries a [`RosterSource`], and the distinction is a security boundary rather than bookkeeping.
//!
//! * [`RosterSource::Observed`] — this person spoke in this chat and the bridge recorded them.
//!   This is the set `channel_dm` authorizes a private message against, and [`RosterStore::observed_members`] is the only read that answers it.
//! * [`RosterSource::Enumerated`] — a platform listed this person as a member (Slack's `conversations.members`), and the agent has never heard from them.
//!   Reportable by `channel_members`, never addressable by `channel_dm`.
//!
//! Bulk-filling the observational rows from a platform member list would have been the obvious way to answer "who is in this channel?", and it would have silently widened `channel_dm`'s authorization set from "people this agent has interacted with" to "everyone the workspace lists" (#7086).
//! Hence the column, and hence the separate reader: the narrowing is an explicit `WHERE source = 'observed'`, not a property something else happens to preserve.
//!
//! One column rather than a second table because the natural key `(channel_type, chat_id, user_id)` is identical in both sets.
//! A member who was enumerated and then speaks is one row changing classification — an `UPDATE` — instead of two rows in two tables that every read would have to `UNION` and de-duplicate with a priority rule, which is exactly the kind of implicit reasoning the split exists to avoid.

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;

use librefang_types::error::{LibreFangError, LibreFangResult};

/// Why a roster row exists.
///
/// Persisted verbatim in `group_roster.source`, so the string forms are schema and not a display detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RosterSource {
    /// The daemon saw this person send a message in this chat.
    ///
    /// The authorization set for `channel_dm`: an agent may privately address someone who has spoken to it, and nobody else.
    Observed,
    /// A platform's member list named this person; the daemon has never heard from them here.
    ///
    /// Reportable, not addressable.
    Enumerated,
}

impl RosterSource {
    /// The value stored in `group_roster.source`.
    pub const fn as_str(self) -> &'static str {
        match self {
            RosterSource::Observed => "observed",
            RosterSource::Enumerated => "enumerated",
        }
    }

    /// Parse a stored value, treating anything unrecognised as [`RosterSource::Enumerated`].
    ///
    /// The fallback direction is deliberate and is the fail-closed one: an unreadable `source` must never be mistaken for "observed", because that is the value that authorizes a private message.
    /// A row written by a future version with a source this build has never heard of therefore degrades to reportable-but-not-addressable.
    pub fn from_stored(raw: &str) -> Self {
        if raw == RosterSource::Observed.as_str() {
            RosterSource::Observed
        } else {
            RosterSource::Enumerated
        }
    }
}

/// One roster row as the read paths hand it out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterMember {
    /// Platform user id — the value `channel_dm` addresses.
    pub user_id: String,
    /// Human-readable name, or the raw platform id when the adapter resolves no better one.
    pub display_name: String,
    /// Platform `@handle`, where the platform has one.
    pub username: Option<String>,
    /// Why this row exists. See [`RosterSource`].
    pub source: RosterSource,
}

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

    /// Record a member the daemon has **observed speaking** in this chat.
    ///
    /// Writes [`RosterSource::Observed`], and promotes an existing enumerated row to observed: someone the platform merely listed has now addressed the agent, which is exactly the event that earns them a place in `channel_dm`'s authorization set.
    /// The promotion is one-way — see [`RosterStore::upsert_enumerated`], which never demotes.
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
            "INSERT INTO group_roster (channel_type, chat_id, user_id, display_name, username, source, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, 'observed', strftime('%s','now'), strftime('%s','now'))
             ON CONFLICT(channel_type, chat_id, user_id) DO UPDATE SET
               display_name = excluded.display_name,
               username = COALESCE(excluded.username, group_roster.username),
               source = 'observed',
               last_seen = strftime('%s','now')",
            rusqlite::params![channel, chat_id, user_id, display_name, username],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// Record a member a platform's member list named, whom the daemon has never heard from here.
    ///
    /// Writes [`RosterSource::Enumerated`] and **must never demote an observed row**, which is why the conflict clause pins `source = group_roster.source` instead of taking `excluded.source`.
    /// An enumeration sweep runs over the whole channel, so without that pin the first sweep after someone spoke would quietly revoke their `channel_dm` reachability.
    ///
    /// `last_seen` is left alone on conflict for the same reason it is not a general-purpose "touched at": for an observed row it means *last heard from*, and a retention sweep keyed on it would otherwise treat a channel-wide enumeration as everyone having just spoken.
    /// A brand-new enumerated row does get both timestamps, because "first listed" is the only time it has.
    pub fn upsert_enumerated(
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
                "op" => "upsert_enumerated",
            )
            .increment(1);
            LibreFangError::memory(error)
        })?;
        c.execute(
            "INSERT INTO group_roster (channel_type, chat_id, user_id, display_name, username, source, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, 'enumerated', strftime('%s','now'), strftime('%s','now'))
             ON CONFLICT(channel_type, chat_id, user_id) DO UPDATE SET
               display_name = excluded.display_name,
               username = COALESCE(excluded.username, group_roster.username),
               source = group_roster.source",
            rusqlite::params![channel, chat_id, user_id, display_name, username],
        )
        .map_err(LibreFangError::memory)?;
        Ok(())
    }

    /// List **every** member of a group chat — observed and enumerated alike — ordered by display name then user id.
    ///
    /// This is the `channel_members` read, and it is deliberately **not** an authorization set.
    /// Anything deciding who an agent may privately address must call [`RosterStore::observed_members`]; the rows here include people a platform listed who have never interacted with the agent (#7086).
    ///
    /// The tiebreak is load-bearing, not cosmetic: this list reaches an LLM prompt through the `channel_members` tool, and two members sharing a display name would otherwise come back in whatever order SQLite happened to produce, invalidating the provider prompt cache on unchanged content (#3298).
    /// `source` is not part of the ordering, so a member being reclassified does not reshuffle the list around them.
    /// The in-memory `GroupRosterStore::members` in `librefang-channels` already breaks the same tie on the participant id.
    pub fn members(&self, channel: &str, chat_id: &str) -> LibreFangResult<Vec<RosterMember>> {
        self.query_members(channel, chat_id, None, "members")
    }

    /// List only the members the daemon has **observed speaking** in this chat.
    ///
    /// This is the `channel_dm` authorization set, and the `AND source = ?3` predicate below is the whole of the guarantee — remove it and an agent can privately address anyone a platform enumerated into the channel, which is the escalation the `source` column exists to prevent (#7086).
    /// `crates/librefang-kernel/tests/kernel_handle_contract_broader.rs::enumerated_members_are_reportable_but_never_dm_authorized` fails if it goes.
    ///
    /// Separate method rather than a flag on [`RosterStore::members`], so a call site cannot end up with the wrong set by defaulting an argument.
    pub fn observed_members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> LibreFangResult<Vec<RosterMember>> {
        self.query_members(
            channel,
            chat_id,
            Some(RosterSource::Observed),
            "observed_members",
        )
    }

    /// Shared body of [`RosterStore::members`] and [`RosterStore::observed_members`].
    ///
    /// `source` is `None` for "every row" and `Some(_)` for a single classification; the two callers are the only ones, and both name their intent in their own signature.
    fn query_members(
        &self,
        channel: &str,
        chat_id: &str,
        source: Option<RosterSource>,
        op: &'static str,
    ) -> LibreFangResult<Vec<RosterMember>> {
        let c = self.pool.get().map_err(|error| {
            metrics::counter!(
                "librefang_memory_pool_get_failed_total",
                "store" => "roster",
                "op" => op,
            )
            .increment(1);
            LibreFangError::memory(error)
        })?;
        let sql = if source.is_some() {
            "SELECT user_id, display_name, username, source FROM group_roster
             WHERE channel_type = ?1 AND chat_id = ?2 AND source = ?3
             ORDER BY display_name, user_id"
        } else {
            "SELECT user_id, display_name, username, source FROM group_roster
             WHERE channel_type = ?1 AND chat_id = ?2
             ORDER BY display_name, user_id"
        };
        let mut stmt = c.prepare(sql).map_err(LibreFangError::memory)?;
        let to_member = |row: &rusqlite::Row<'_>| {
            Ok(RosterMember {
                user_id: row.get::<_, String>(0)?,
                display_name: row.get::<_, String>(1)?,
                username: row.get::<_, Option<String>>(2)?,
                source: RosterSource::from_stored(&row.get::<_, String>(3)?),
            })
        };
        let rows = match source {
            Some(source) => stmt.query_map(
                rusqlite::params![channel, chat_id, source.as_str()],
                to_member,
            ),
            None => stmt.query_map(rusqlite::params![channel, chat_id], to_member),
        }
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
        assert_eq!(members[0].display_name, "Alice");
        assert_eq!(members[1].display_name, "Bob");
        assert_eq!(members[0].username, Some("alice".to_string()));
        assert_eq!(members[1].username, None);
        assert_eq!(members[0].source, RosterSource::Observed);
        assert_eq!(members[1].source, RosterSource::Observed);
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
        assert_eq!(members[0].display_name, "Alice Updated");
    }

    // Two members with the same display name must come back in a fixed order regardless of insertion order: the list is rendered into an LLM prompt by the `channel_members` tool, so an unstable tail silently invalidates the provider prompt cache (#3298).
    #[test]
    fn same_display_name_is_ordered_by_user_id_across_insertion_orders() {
        let forward = in_memory_store();
        forward.upsert("slack", "C1", "U1", "Alex", None).unwrap();
        forward.upsert("slack", "C1", "U2", "Alex", None).unwrap();

        let reverse = in_memory_store();
        reverse.upsert("slack", "C1", "U2", "Alex", None).unwrap();
        reverse.upsert("slack", "C1", "U1", "Alex", None).unwrap();

        let expected = vec![
            RosterMember {
                user_id: "U1".to_string(),
                display_name: "Alex".to_string(),
                username: None,
                source: RosterSource::Observed,
            },
            RosterMember {
                user_id: "U2".to_string(),
                display_name: "Alex".to_string(),
                username: None,
                source: RosterSource::Observed,
            },
        ];
        assert_eq!(forward.members("slack", "C1").unwrap(), expected);
        assert_eq!(reverse.members("slack", "C1").unwrap(), expected);
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
        assert_eq!(members[0].display_name, "Bob");
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
        assert!(store
            .upsert_enumerated("telegram", "-100", "1", "Alice", None)
            .is_err());
        assert!(store.members("telegram", "-100").is_err());
        assert!(store.observed_members("telegram", "-100").is_err());
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
    // --- source boundary (#7086) -------------------------------------------

    /// The `channel_members` read reports both sets; the `channel_dm` read reports only the observational one.
    /// If these two ever return the same rows, bulk enumeration has become a DM authorization, which is the whole thing the column prevents.
    #[test]
    fn enumerated_members_are_listed_but_not_observed() {
        let store = in_memory_store();
        store.upsert("slack", "C1", "U1", "Ada", None).unwrap();
        store
            .upsert_enumerated("slack", "C1", "U2", "Bo", None)
            .unwrap();

        let all = store.members("slack", "C1").unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].user_id, "U1");
        assert_eq!(all[0].source, RosterSource::Observed);
        assert_eq!(all[1].user_id, "U2");
        assert_eq!(all[1].source, RosterSource::Enumerated);

        let observed = store.observed_members("slack", "C1").unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].user_id, "U1");
    }

    /// Speaking earns a promotion: someone the platform merely listed becomes addressable the moment they address the agent.
    #[test]
    fn speaking_promotes_an_enumerated_member_to_observed() {
        let store = in_memory_store();
        store
            .upsert_enumerated("slack", "C1", "U2", "U2", None)
            .unwrap();
        assert!(store.observed_members("slack", "C1").unwrap().is_empty());

        store.upsert("slack", "C1", "U2", "Bo", Some("bo")).unwrap();

        let observed = store.observed_members("slack", "C1").unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].display_name, "Bo");
        assert_eq!(observed[0].username, Some("bo".to_string()));
    }

    /// The promotion is one-way.
    /// An enumeration sweep runs over every member of a channel, so a sweep that overwrote `source` would revoke `channel_dm` for everyone who had spoken since the last one — a security control that silently switches itself off on a timer.
    #[test]
    fn enumeration_never_demotes_an_observed_member() {
        let store = in_memory_store();
        store
            .upsert("slack", "C1", "U1", "Ada", Some("ada"))
            .unwrap();
        store
            .upsert_enumerated("slack", "C1", "U1", "Ada Lovelace", None)
            .unwrap();

        let observed = store.observed_members("slack", "C1").unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].user_id, "U1");
        // The refreshed name is still taken — enumeration is allowed to improve the label, just not the classification.
        assert_eq!(observed[0].display_name, "Ada Lovelace");
        assert_eq!(observed[0].username, Some("ada".to_string()));
    }

    /// Enumeration must not look like activity.
    /// `last_seen` is what a retention sweep would prune on, and a channel-wide member list refreshing it would keep every listed person alive forever.
    #[test]
    fn enumeration_does_not_touch_last_seen_of_an_existing_row() {
        let store = in_memory_store();
        store.upsert("slack", "C1", "U1", "Ada", None).unwrap();
        let conn = store.pool.get().unwrap();
        conn.execute(
            "UPDATE group_roster SET last_seen = 1000 WHERE user_id = 'U1'",
            [],
        )
        .unwrap();
        drop(conn);

        store
            .upsert_enumerated("slack", "C1", "U1", "Ada", None)
            .unwrap();

        let last_seen: i64 = store
            .pool
            .get()
            .unwrap()
            .query_row(
                "SELECT last_seen FROM group_roster WHERE user_id = 'U1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(last_seen, 1000);
    }

    /// Every row that predates the `source` column was written by the sender upsert, so the v52 default is the only correct one.
    /// Reading it as anything else would have retroactively revoked `channel_dm` for every existing deployment.
    #[test]
    fn pre_migration_rows_default_to_observed() {
        let store = in_memory_store();
        store
            .pool
            .get()
            .unwrap()
            .execute(
                "INSERT INTO group_roster (channel_type, chat_id, user_id, display_name) \
                 VALUES ('telegram', '-100', '1', 'Alice')",
                [],
            )
            .unwrap();

        let observed = store.observed_members("telegram", "-100").unwrap();
        assert_eq!(observed.len(), 1);
        assert_eq!(observed[0].user_id, "1");
    }

    /// A `source` this build does not recognise must not be read as "observed".
    /// The fallback is the fail-closed direction: an unknown classification loses DM reachability rather than gaining it.
    #[test]
    fn an_unknown_source_is_not_dm_authorized() {
        let store = in_memory_store();
        store.upsert("slack", "C1", "U1", "Ada", None).unwrap();
        store
            .pool
            .get()
            .unwrap()
            .execute(
                "UPDATE group_roster SET source = 'directory_sync' WHERE user_id = 'U1'",
                [],
            )
            .unwrap();

        assert!(store.observed_members("slack", "C1").unwrap().is_empty());
        let all = store.members("slack", "C1").unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].source, RosterSource::Enumerated);
    }
}
