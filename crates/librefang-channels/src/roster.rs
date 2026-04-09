//! In-memory group roster store.
//!
//! Tracks the human members seen in each group chat so that agents can be given
//! a structured "who is in this group" context in their system prompt. Without
//! this, an agent receiving a message like `@pepe dile algo a @jose` has no way
//! to know who `@pepe` and `@jose` are — they look like opaque text.
//!
//! The store is a simple in-memory map keyed by `(channel_type, chat_id)`. It
//! does not persist to disk: on daemon restart it is empty and repopulates
//! naturally as members send messages. A persistent backend can be added later
//! without changing the public API.

use crate::types::GroupMember;
use dashmap::DashMap;
use std::sync::Arc;

/// Composite key identifying a specific chat on a specific channel.
///
/// For Telegram the `chat_id` is the group's negative chat ID (or the user's
/// ID for DMs). For Discord it's the channel ID, and so on per platform.
type RosterKey = (String, String);

/// Thread-safe in-memory store of known group members per chat.
#[derive(Debug, Default, Clone)]
pub struct GroupRosterStore {
    rosters: Arc<DashMap<RosterKey, DashMap<String, GroupMember>>>,
}

impl GroupRosterStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or update) a member in the given chat roster.
    ///
    /// Idempotent: subsequent calls with the same `user_id` simply refresh the
    /// display name and username.
    pub fn upsert(&self, channel: &str, chat_id: &str, member: GroupMember) {
        if chat_id.is_empty() || member.user_id.is_empty() {
            return;
        }
        let key = (channel.to_string(), chat_id.to_string());
        let members = self.rosters.entry(key).or_insert_with(DashMap::new);
        members.insert(member.user_id.clone(), member);
    }

    /// Return all known members for a chat, sorted by display name for stable
    /// rendering. Returns an empty vector if the chat is unknown.
    pub fn members(&self, channel: &str, chat_id: &str) -> Vec<GroupMember> {
        let key = (channel.to_string(), chat_id.to_string());
        let Some(entry) = self.rosters.get(&key) else {
            return Vec::new();
        };
        let mut out: Vec<GroupMember> = entry.iter().map(|e| e.value().clone()).collect();
        out.sort_by(|a, b| a.display_name.cmp(&b.display_name));
        out
    }

    /// Number of members in a specific chat (0 if unknown).
    pub fn member_count(&self, channel: &str, chat_id: &str) -> usize {
        let key = (channel.to_string(), chat_id.to_string());
        self.rosters.get(&key).map(|e| e.len()).unwrap_or(0)
    }

    /// Total number of chats being tracked.
    pub fn chat_count(&self) -> usize {
        self.rosters.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_member(user_id: &str, display: &str, username: Option<&str>) -> GroupMember {
        GroupMember {
            user_id: user_id.to_string(),
            display_name: display.to_string(),
            username: username.map(String::from),
        }
    }

    #[test]
    fn upsert_and_list_sorted() {
        let store = GroupRosterStore::new();
        store.upsert(
            "telegram",
            "-100123",
            mk_member("1", "Jorge", Some("jorge")),
        );
        store.upsert(
            "telegram",
            "-100123",
            mk_member("2", "Pakman", Some("pakman")),
        );
        store.upsert("telegram", "-100123", mk_member("3", "Ana", Some("ana")));

        let members = store.members("telegram", "-100123");
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].display_name, "Ana");
        assert_eq!(members[1].display_name, "Jorge");
        assert_eq!(members[2].display_name, "Pakman");
    }

    #[test]
    fn upsert_idempotent_and_updates() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "-100123", mk_member("1", "Jorge", None));
        store.upsert(
            "telegram",
            "-100123",
            mk_member("1", "Jorge Pablo", Some("jorgepablo")),
        );
        let members = store.members("telegram", "-100123");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].display_name, "Jorge Pablo");
        assert_eq!(members[0].username.as_deref(), Some("jorgepablo"));
    }

    #[test]
    fn unknown_chat_returns_empty() {
        let store = GroupRosterStore::new();
        assert!(store.members("telegram", "-999").is_empty());
        assert_eq!(store.member_count("telegram", "-999"), 0);
    }

    #[test]
    fn ignores_empty_ids() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "", mk_member("1", "Nobody", None));
        store.upsert("telegram", "-100", mk_member("", "Nameless", None));
        assert_eq!(store.chat_count(), 0);
    }

    #[test]
    fn separate_chats_are_isolated() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "-100", mk_member("1", "Alice", None));
        store.upsert("telegram", "-200", mk_member("2", "Bob", None));
        assert_eq!(store.members("telegram", "-100").len(), 1);
        assert_eq!(store.members("telegram", "-200").len(), 1);
        assert_eq!(store.chat_count(), 2);
    }

    #[test]
    fn separate_channels_are_isolated() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "123", mk_member("1", "Alice", None));
        store.upsert("discord", "123", mk_member("2", "Bob", None));
        assert_eq!(store.members("telegram", "123").len(), 1);
        assert_eq!(store.members("discord", "123").len(), 1);
    }
}
