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

use crate::types::ParticipantRef;
use dashmap::DashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Maximum distinct chats retained by one in-memory roster store.
/// Older chat rosters are evicted when a new active chat crosses this bound.
pub const MAX_TRACKED_CHATS: usize = 1_000;

/// Maximum known human members retained for one chat.
/// Existing members can still refresh their display data at the limit.
pub const MAX_MEMBERS_PER_CHAT: usize = 1_000;

/// Composite key identifying a specific chat on a specific channel.
///
/// For Telegram the `chat_id` is the group's negative chat ID (or the user's
/// ID for DMs). For Discord it's the channel ID, and so on per platform.
type RosterKey = (String, String);

/// Thread-safe in-memory store of known group members per chat.
#[derive(Debug, Default, Clone)]
pub struct GroupRosterStore {
    inner: Arc<RosterInner>,
}

#[derive(Debug, Default)]
struct RosterInner {
    rosters: DashMap<RosterKey, Arc<ChatRoster>>,
    chat_admission: Mutex<()>,
    access_clock: AtomicU64,
}

#[derive(Debug)]
struct ChatRoster {
    members: DashMap<String, ParticipantRef>,
    member_admission: Mutex<()>,
    last_seen: AtomicU64,
}

impl ChatRoster {
    fn new(last_seen: u64) -> Self {
        Self {
            members: DashMap::new(),
            member_admission: Mutex::new(()),
            last_seen: AtomicU64::new(last_seen),
        }
    }
}

impl GroupRosterStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record (or update) a member in the given chat roster.
    ///
    /// Idempotent: subsequent calls with the same `user_id` simply refresh the
    /// display name and username. Returns `false` for empty identifiers or when
    /// a new member would exceed [`MAX_MEMBERS_PER_CHAT`].
    pub fn upsert(&self, channel: &str, chat_id: &str, member: ParticipantRef) -> bool {
        self.upsert_with_limits(
            channel,
            chat_id,
            member,
            MAX_TRACKED_CHATS,
            MAX_MEMBERS_PER_CHAT,
        )
    }

    fn upsert_with_limits(
        &self,
        channel: &str,
        chat_id: &str,
        member: ParticipantRef,
        max_chats: usize,
        max_members_per_chat: usize,
    ) -> bool {
        if channel.is_empty() || chat_id.is_empty() || member.jid.is_empty() {
            return false;
        }
        let key = (channel.to_string(), chat_id.to_string());
        let tick = self.inner.access_clock.fetch_add(1, Ordering::Relaxed);
        let roster = if let Some(roster) = self.inner.rosters.get(&key) {
            let roster = Arc::clone(roster.value());
            roster.last_seen.store(tick, Ordering::Relaxed);
            roster
        } else {
            let _admission = self
                .inner
                .chat_admission
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(roster) = self.inner.rosters.get(&key) {
                let roster = Arc::clone(roster.value());
                roster.last_seen.store(tick, Ordering::Relaxed);
                roster
            } else {
                self.make_chat_room(max_chats.max(1));
                let roster = Arc::new(ChatRoster::new(tick));
                self.inner.rosters.insert(key, Arc::clone(&roster));
                roster
            }
        };

        if let Some(mut existing) = roster.members.get_mut(&member.jid) {
            *existing = member;
            return true;
        }

        let _admission = roster
            .member_admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !roster.members.contains_key(&member.jid)
            && roster.members.len() >= max_members_per_chat.max(1)
        {
            return false;
        }
        roster.members.insert(member.jid.clone(), member);
        true
    }

    /// Return all known members for a chat, sorted by display name for stable
    /// rendering. Returns an empty vector if the chat is unknown.
    pub fn members(&self, channel: &str, chat_id: &str) -> Vec<ParticipantRef> {
        let key = (channel.to_string(), chat_id.to_string());
        let Some(entry) = self.inner.rosters.get(&key) else {
            return Vec::new();
        };
        let roster = Arc::clone(entry.value());
        drop(entry);
        let mut out: Vec<ParticipantRef> = roster
            .members
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        out.sort_by(|a, b| {
            a.display_name
                .cmp(&b.display_name)
                .then_with(|| a.jid.cmp(&b.jid))
        });
        out
    }

    /// Number of members in a specific chat (0 if unknown).
    pub fn member_count(&self, channel: &str, chat_id: &str) -> usize {
        let key = (channel.to_string(), chat_id.to_string());
        self.inner
            .rosters
            .get(&key)
            .map(|entry| entry.members.len())
            .unwrap_or(0)
    }

    /// Total number of chats being tracked.
    pub fn chat_count(&self) -> usize {
        self.inner.rosters.len()
    }

    /// Remove one member from a chat roster.
    pub fn remove(&self, channel: &str, chat_id: &str, user_id: &str) -> bool {
        let key = (channel.to_string(), chat_id.to_string());
        self.inner
            .rosters
            .get(&key)
            .is_some_and(|roster| roster.members.remove(user_id).is_some())
    }

    /// Drop a complete chat roster, returning its retained member count.
    pub fn drop_chat(&self, channel: &str, chat_id: &str) -> usize {
        let key = (channel.to_string(), chat_id.to_string());
        self.inner
            .rosters
            .remove(&key)
            .map_or(0, |(_, roster)| roster.members.len())
    }

    /// Clear every retained chat roster.
    pub fn clear(&self) {
        self.inner.rosters.clear();
    }

    fn make_chat_room(&self, max_chats: usize) {
        while self.inner.rosters.len() >= max_chats {
            let Some((oldest_key, observed_tick)) = self
                .inner
                .rosters
                .iter()
                .map(|entry| {
                    (
                        entry.key().clone(),
                        entry.value().last_seen.load(Ordering::Relaxed),
                    )
                })
                .min_by_key(|(_, tick)| *tick)
            else {
                return;
            };
            self.inner.rosters.remove_if(&oldest_key, |_, roster| {
                roster.last_seen.load(Ordering::Relaxed) == observed_tick
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn mk_member(jid: &str, display: &str) -> ParticipantRef {
        ParticipantRef {
            jid: jid.to_string(),
            display_name: display.to_string(),
        }
    }

    #[test]
    fn upsert_and_list_sorted() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "-100123", mk_member("1", "Jorge"));
        store.upsert("telegram", "-100123", mk_member("2", "Pakman"));
        store.upsert("telegram", "-100123", mk_member("3", "Ana"));

        let members = store.members("telegram", "-100123");
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].display_name, "Ana");
        assert_eq!(members[1].display_name, "Jorge");
        assert_eq!(members[2].display_name, "Pakman");
    }

    #[test]
    fn upsert_idempotent_and_updates() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "-100123", mk_member("1", "Jorge"));
        store.upsert("telegram", "-100123", mk_member("1", "Jorge Pablo"));
        let members = store.members("telegram", "-100123");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].display_name, "Jorge Pablo");
        assert_eq!(members[0].jid, "1");
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
        assert!(!store.upsert("", "-100", mk_member("1", "Nobody")));
        assert!(!store.upsert("telegram", "", mk_member("1", "Nobody")));
        assert!(!store.upsert("telegram", "-100", mk_member("", "Nameless")));
        assert_eq!(store.chat_count(), 0);
    }

    #[test]
    fn separate_chats_are_isolated() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "-100", mk_member("1", "Alice"));
        store.upsert("telegram", "-200", mk_member("2", "Bob"));
        assert_eq!(store.members("telegram", "-100").len(), 1);
        assert_eq!(store.members("telegram", "-200").len(), 1);
        assert_eq!(store.chat_count(), 2);
    }

    #[test]
    fn separate_channels_are_isolated() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "123", mk_member("1", "Alice"));
        store.upsert("discord", "123", mk_member("2", "Bob"));
        assert_eq!(store.members("telegram", "123").len(), 1);
        assert_eq!(store.members("discord", "123").len(), 1);
    }

    #[test]
    fn limits_members_without_blocking_existing_member_updates() {
        let store = GroupRosterStore::new();
        assert!(store.upsert_with_limits("telegram", "-100", mk_member("1", "Alice"), 2, 2,));
        assert!(store.upsert_with_limits("telegram", "-100", mk_member("2", "Bob"), 2, 2,));
        assert!(!store.upsert_with_limits("telegram", "-100", mk_member("3", "Carol"), 2, 2,));
        assert!(store.upsert_with_limits(
            "telegram",
            "-100",
            mk_member("1", "Alice Updated"),
            2,
            2,
        ));
        assert_eq!(store.member_count("telegram", "-100"), 2);
        assert_eq!(
            store.members("telegram", "-100")[0].display_name,
            "Alice Updated"
        );
    }

    #[test]
    fn evicts_the_least_recently_updated_chat_at_capacity() {
        let store = GroupRosterStore::new();
        let add = |chat: &str, jid: &str| {
            store.upsert_with_limits("telegram", chat, mk_member(jid, jid), 2, 2)
        };

        assert!(add("chat-1", "1"));
        assert!(add("chat-2", "2"));
        assert!(add("chat-1", "1"));
        assert!(add("chat-3", "3"));

        assert_eq!(store.chat_count(), 2);
        assert_eq!(store.member_count("telegram", "chat-2"), 0);
        assert_eq!(store.member_count("telegram", "chat-1"), 1);
        assert_eq!(store.member_count("telegram", "chat-3"), 1);
    }

    #[test]
    fn removal_apis_release_retained_rosters() {
        let store = GroupRosterStore::new();
        store.upsert("telegram", "chat-1", mk_member("1", "Same"));
        store.upsert("telegram", "chat-1", mk_member("2", "Same"));
        store.upsert("telegram", "chat-2", mk_member("3", "Other"));

        let members = store.members("telegram", "chat-1");
        assert_eq!(members[0].jid, "1");
        assert_eq!(members[1].jid, "2");
        assert!(store.remove("telegram", "chat-1", "1"));
        assert!(!store.remove("telegram", "chat-1", "missing"));
        assert_eq!(store.drop_chat("telegram", "chat-1"), 1);
        store.clear();
        assert_eq!(store.chat_count(), 0);
    }

    #[test]
    fn concurrent_chat_admission_never_exceeds_the_limit() {
        let store = GroupRosterStore::new();
        let barrier = Arc::new(Barrier::new(32));
        let mut threads = Vec::new();
        for index in 0..32 {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                store.upsert_with_limits(
                    "telegram",
                    &format!("chat-{index}"),
                    mk_member(&format!("user-{index}"), "Member"),
                    4,
                    4,
                )
            }));
        }

        for thread in threads {
            assert!(thread.join().unwrap());
        }
        assert_eq!(store.chat_count(), 4);
    }

    #[test]
    fn concurrent_member_admission_never_exceeds_the_limit() {
        let store = GroupRosterStore::new();
        let barrier = Arc::new(Barrier::new(32));
        let mut threads = Vec::new();
        for index in 0..32 {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                store.upsert_with_limits(
                    "telegram",
                    "chat",
                    mk_member(&format!("user-{index}"), "Member"),
                    4,
                    4,
                )
            }));
        }

        let accepted = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|accepted| *accepted)
            .count();
        assert_eq!(accepted, 4);
        assert_eq!(store.member_count("telegram", "chat"), 4);
    }
}
