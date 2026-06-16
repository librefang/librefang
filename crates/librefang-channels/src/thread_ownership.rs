//! Thread-ownership registry — prevents multi-agent duplicate replies in a
//! shared group thread.
//!
//! When two or more LibreFang agents are bound to the same channel (e.g. one
//! Slack workspace with both a "support" agent and a "research" agent), each
//! incoming group-thread message would otherwise be routed to whichever agent
//! the router resolves — and that resolution can flip turn-to-turn (last
//! @-mention wins, sticky-TTL falls off, etc.). The user sees both agents
//! reply, contradict each other, and run up cost.
//!
//! This module adds an in-memory single-process claim registry keyed by a `ThreadKey` with a TTL.
//! The bridge consults it after routing and before dispatch, suppressing any agent that isn't the current claim holder.
//! An explicit @-mention re-claims for the new agent.
//!
//! The key carries more than `(channel, thread)` (#5323): an optional `account_id` keeps two bot accounts on the same channel-type from colliding (multi-tenant), an optional `chat_id` distinguishes two chats that happen to share a forum-topic id, and an optional `peer_id` scopes a claim to one conversational partner so two users talking to two different agents in the same thread keep independent stickiness.
//! All three extra slots default to `None`, which reproduces the original single-account `(channel, thread)` behaviour byte-for-byte.
//!
//! Multi-process / multi-daemon coordination (sharing the registry across processes via a forwarder API) is out of scope — see issue #3334.
//! DMs bypass the registry unless `conversation_ownership_include_dms` is set on the channel override.

use dashmap::DashMap;
use librefang_types::agent::AgentId;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Default TTL for a fresh claim. After this many seconds without a refresh,
/// the next agent to dispatch can take ownership.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Identity of one ownership slice. Built per-message from the canonical
/// channel-type slug, the platform's thread identifier, and — when known —
/// the bot account, chat, and conversational peer that further narrow it.
///
/// `channel` and `thread` are mandatory; the remaining three are `None` on single-account installs and on adapters that do not surface them, which keeps the key equal to the historical `(channel, thread)` pair.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ThreadKey {
    /// Adapter-qualified channel slug (e.g. `"slack"`, `"discord"`).
    pub channel: String,
    /// Bot account this message reached, when the adapter is multi-tenant (e.g. one Slack workspace id, one Telegram bot token slug).
    /// Two accounts on the same channel-type never share a claim.
    pub account_id: Option<String>,
    /// Chat / group / DM container id.
    /// Distinguishes two chats that reuse the same platform-side `thread` id (rare but possible for forum topics).
    pub chat_id: Option<String>,
    /// Platform thread identifier (Slack `thread_ts`, Discord thread ID, a forum-topic id, …).
    /// Callers without a forum topic pass the `chat_id` here so a topic-less group still gets a stable claim.
    /// Empty string is invalid; callers should not invoke the registry without a real thread.
    pub thread: String,
    /// Conversational partner (the individual sender).
    /// Scoping the claim to a peer lets two users in the same thread talk to two different agents without contaminating each other.
    /// `None` => thread-wide claim.
    pub peer_id: Option<String>,
}

impl ThreadKey {
    /// Build a key from a channel slug and thread id, leaving the `account_id` / `chat_id` / `peer_id` slices unset.
    /// Trims whitespace; both fields must be non-empty after trimming or the call is meaningless.
    /// Use the `with_*` builders to narrow the key further.
    pub fn new(channel: &str, thread: &str) -> Option<Self> {
        let channel = channel.trim();
        let thread = thread.trim();
        if channel.is_empty() || thread.is_empty() {
            return None;
        }
        Some(Self {
            channel: channel.to_string(),
            account_id: None,
            chat_id: None,
            thread: thread.to_string(),
            peer_id: None,
        })
    }

    /// Narrow the key to a specific bot account.
    /// An empty / whitespace value clears the slot rather than storing a meaningless empty string.
    pub fn with_account_id(mut self, account_id: Option<&str>) -> Self {
        self.account_id = normalize_opt(account_id);
        self
    }

    /// Narrow the key to a specific chat / group container.
    pub fn with_chat_id(mut self, chat_id: Option<&str>) -> Self {
        self.chat_id = normalize_opt(chat_id);
        self
    }

    /// Narrow the key to a specific conversational peer.
    pub fn with_peer_id(mut self, peer_id: Option<&str>) -> Self {
        self.peer_id = normalize_opt(peer_id);
        self
    }
}

/// Trim an optional string, collapsing empties to `None` so a blank metadata value never silently widens or splinters a key.
fn normalize_opt(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// One ownership record. Stored values are immutable — `extend` writes a new
/// claim. `claimed_at` is monotonic time so wall-clock changes don't break
/// TTL.
#[derive(Debug, Clone)]
struct Claim {
    agent_id: AgentId,
    claimed_at: Instant,
    ttl: Duration,
}

impl Claim {
    fn is_expired(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.claimed_at) >= self.ttl
    }
}

/// Outcome of asking the registry whether an agent may dispatch in a thread.
///
/// `Allow` carries the agent that will hold the claim after this call; the
/// caller should proceed to dispatch as normal. `Suppress` carries the
/// existing claim holder so the bridge can log a meaningful skip reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchDecision {
    /// Dispatch is permitted. The candidate agent now owns the thread (claim
    /// fresh-set or extended).
    Allow { agent_id: AgentId },
    /// Dispatch must be suppressed — another agent owns the thread and the
    /// current candidate is not the @-mentioned override. Caller should drop
    /// without sending anything.
    Suppress { holder: AgentId },
}

/// In-memory claim registry, single-process. Cheap to clone (`Arc`-style via
/// `DashMap`), so a single instance is shared by every adapter through the
/// channel bridge.
#[derive(Debug, Default)]
pub struct ThreadOwnershipRegistry {
    claims: Arc<DashMap<ThreadKey, Claim>>,
    default_ttl: Duration,
}

impl ThreadOwnershipRegistry {
    /// Build a registry with the default TTL.
    pub fn new() -> Self {
        Self::with_ttl(DEFAULT_TTL)
    }

    /// Build a registry with a custom TTL. A TTL of zero is meaningless —
    /// this clamps to one second to avoid every claim expiring immediately.
    pub fn with_ttl(ttl: Duration) -> Self {
        let ttl = if ttl.is_zero() {
            Duration::from_secs(1)
        } else {
            ttl
        };
        Self {
            claims: Arc::new(DashMap::new()),
            default_ttl: ttl,
        }
    }

    /// Decide whether `candidate` may dispatch in `key`.
    ///
    /// Logic:
    /// 1. No claim or expired claim → fresh-claim for `candidate`, return
    ///    `Allow`.
    /// 2. Existing claim, candidate is the holder → extend (refresh
    ///    `claimed_at`), return `Allow`.
    /// 3. Existing claim, different agent, `was_mentioned = true` → re-claim
    ///    for `candidate`, return `Allow`.
    /// 4. Existing claim, different agent, `was_mentioned = false` →
    ///    `Suppress { holder }`.
    pub fn decide(
        &self,
        key: ThreadKey,
        candidate: AgentId,
        was_mentioned: bool,
    ) -> DispatchDecision {
        self.decide_at(key, candidate, was_mentioned, Instant::now())
    }

    /// Like `decide`, but use `ttl` for any claim this call creates or refreshes instead of the registry default.
    /// Lets the bridge honour a per-channel `conversation_ownership_ttl_seconds` override while every channel keeps sharing one registry.
    pub fn decide_with_ttl(
        &self,
        key: ThreadKey,
        candidate: AgentId,
        was_mentioned: bool,
        ttl: Duration,
    ) -> DispatchDecision {
        self.decide_at_with_ttl(key, candidate, was_mentioned, Instant::now(), ttl)
    }

    /// Test seam: like `decide` but with a caller-supplied `now`.
    pub fn decide_at(
        &self,
        key: ThreadKey,
        candidate: AgentId,
        was_mentioned: bool,
        now: Instant,
    ) -> DispatchDecision {
        self.decide_at_with_ttl(key, candidate, was_mentioned, now, self.default_ttl)
    }

    /// Test seam: like `decide_with_ttl` but with a caller-supplied `now`.
    pub fn decide_at_with_ttl(
        &self,
        key: ThreadKey,
        candidate: AgentId,
        was_mentioned: bool,
        now: Instant,
        ttl: Duration,
    ) -> DispatchDecision {
        let ttl = if ttl.is_zero() {
            Duration::from_secs(1)
        } else {
            ttl
        };
        let mut entry = self.claims.entry(key).or_insert_with(|| Claim {
            agent_id: candidate,
            claimed_at: now,
            ttl,
        });

        // Existing entry path. Three cases: same holder (extend), expired
        // (take over), different live holder (suppress unless mentioned).
        if entry.agent_id == candidate {
            entry.claimed_at = now;
            entry.ttl = ttl;
            return DispatchDecision::Allow {
                agent_id: candidate,
            };
        }

        if entry.is_expired(now) {
            *entry = Claim {
                agent_id: candidate,
                claimed_at: now,
                ttl,
            };
            return DispatchDecision::Allow {
                agent_id: candidate,
            };
        }

        if was_mentioned {
            let _previous = entry.agent_id;
            *entry = Claim {
                agent_id: candidate,
                claimed_at: now,
                ttl,
            };
            return DispatchDecision::Allow {
                agent_id: candidate,
            };
        }

        DispatchDecision::Suppress {
            holder: entry.agent_id,
        }
    }

    /// Drop expired claims. Cheap O(n) sweep; intended to be called
    /// occasionally (e.g. once a minute by the bridge). Not required for
    /// correctness — `decide` handles expiry inline — but keeps memory bounded
    /// in long-lived deployments with many ephemeral threads.
    pub fn sweep_expired(&self) -> usize {
        self.sweep_expired_at(Instant::now())
    }

    /// Test seam: like `sweep_expired` but with a caller-supplied `now`.
    pub fn sweep_expired_at(&self, now: Instant) -> usize {
        let before = self.claims.len();
        self.claims.retain(|_, claim| !claim.is_expired(now));
        before - self.claims.len()
    }

    /// Read the current holder for a thread, if any. Used for log lines and
    /// observability — does not mutate the entry.
    pub fn current_holder(&self, key: &ThreadKey) -> Option<AgentId> {
        self.claims.get(key).and_then(|c| {
            if c.is_expired(Instant::now()) {
                None
            } else {
                Some(c.agent_id)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_id() -> AgentId {
        AgentId::new()
    }

    fn key(thread: &str) -> ThreadKey {
        ThreadKey::new("slack", thread).expect("key")
    }

    #[test]
    fn empty_thread_key_rejected() {
        assert!(ThreadKey::new("", "T123").is_none());
        assert!(ThreadKey::new("slack", "").is_none());
        assert!(ThreadKey::new("  ", "T123").is_none());
        assert!(ThreadKey::new("slack", "  ").is_none());
        assert!(ThreadKey::new("slack", "T123").is_some());
    }

    #[test]
    fn first_dispatch_claims_the_thread() {
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let now = Instant::now();
        match reg.decide_at(key("T1"), alice, false, now) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, alice),
            other => panic!("expected Allow, got {:?}", other),
        }
        assert_eq!(reg.current_holder(&key("T1")), Some(alice));
    }

    #[test]
    fn second_agent_without_mention_is_suppressed() {
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let bob = fresh_id();
        let now = Instant::now();
        let _ = reg.decide_at(key("T1"), alice, false, now);
        match reg.decide_at(key("T1"), bob, false, now) {
            DispatchDecision::Suppress { holder } => assert_eq!(holder, alice),
            other => panic!("expected Suppress, got {:?}", other),
        }
    }

    #[test]
    fn at_mention_overrides_existing_claim() {
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let bob = fresh_id();
        let now = Instant::now();
        let _ = reg.decide_at(key("T1"), alice, false, now);
        match reg.decide_at(key("T1"), bob, true, now) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, bob),
            other => panic!("expected Allow, got {:?}", other),
        }
        assert_eq!(reg.current_holder(&key("T1")), Some(bob));
    }

    #[test]
    fn same_agent_extends_claim_in_place() {
        let reg = ThreadOwnershipRegistry::with_ttl(Duration::from_secs(10));
        let alice = fresh_id();
        let t0 = Instant::now();
        let _ = reg.decide_at(key("T1"), alice, false, t0);
        let t1 = t0 + Duration::from_secs(8);
        match reg.decide_at(key("T1"), alice, false, t1) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, alice),
            other => panic!("expected Allow, got {:?}", other),
        }
        // Still the holder a second past the *original* TTL because the
        // second decide refreshed `claimed_at`.
        assert_eq!(
            reg.current_holder(&key("T1")),
            Some(alice),
            "extended claim should survive past original TTL window"
        );
    }

    #[test]
    fn expired_claim_yields_to_next_agent_without_mention() {
        let reg = ThreadOwnershipRegistry::with_ttl(Duration::from_secs(10));
        let alice = fresh_id();
        let bob = fresh_id();
        let t0 = Instant::now();
        let _ = reg.decide_at(key("T1"), alice, false, t0);
        let after_ttl = t0 + Duration::from_secs(11);
        match reg.decide_at(key("T1"), bob, false, after_ttl) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, bob),
            other => panic!("expected Allow, got {:?}", other),
        }
    }

    #[test]
    fn sweep_expired_drops_only_expired_entries() {
        let reg = ThreadOwnershipRegistry::with_ttl(Duration::from_secs(10));
        let alice = fresh_id();
        let bob = fresh_id();
        let t0 = Instant::now();
        let _ = reg.decide_at(key("T1"), alice, false, t0);
        let _ = reg.decide_at(key("T2"), bob, false, t0 + Duration::from_secs(5));

        // Sweep at t0 + 11s: T1 expired (Δ=11s >= 10s), T2 still alive (Δ=6s).
        let dropped = reg.sweep_expired_at(t0 + Duration::from_secs(11));
        assert_eq!(dropped, 1);
        assert!(reg.current_holder(&key("T1")).is_none());
        assert_eq!(reg.current_holder(&key("T2")), Some(bob));
    }

    #[test]
    fn ttl_zero_clamps_to_one_second() {
        let reg = ThreadOwnershipRegistry::with_ttl(Duration::ZERO);
        let alice = fresh_id();
        let bob = fresh_id();
        let t0 = Instant::now();
        let _ = reg.decide_at(key("T1"), alice, false, t0);
        // Within 1s, alice still owns it.
        match reg.decide_at(key("T1"), bob, false, t0 + Duration::from_millis(500)) {
            DispatchDecision::Suppress { holder } => assert_eq!(holder, alice),
            other => panic!("expected Suppress, got {:?}", other),
        }
    }

    #[test]
    fn distinct_threads_do_not_collide() {
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let bob = fresh_id();
        let now = Instant::now();
        let _ = reg.decide_at(key("T1"), alice, false, now);
        match reg.decide_at(key("T2"), bob, false, now) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, bob),
            other => panic!("expected Allow on disjoint thread, got {:?}", other),
        }
    }

    #[test]
    fn same_thread_distinct_account_id_does_not_collide() {
        // Regression for #3419/#3420: two bot accounts on the same
        // channel-type with an identical platform thread id must not share a
        // claim.
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let bob = fresh_id();
        let now = Instant::now();
        let k_a = key("T1").with_account_id(Some("acct-a"));
        let k_b = key("T1").with_account_id(Some("acct-b"));
        let _ = reg.decide_at(k_a.clone(), alice, false, now);
        match reg.decide_at(k_b, bob, false, now) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, bob),
            other => panic!("expected Allow across accounts, got {:?}", other),
        }
        // The first account's claim is untouched.
        assert_eq!(reg.current_holder(&k_a), Some(alice));
    }

    #[test]
    fn same_thread_distinct_peer_does_not_collide() {
        // Per-peer stickiness (#5323): two users in the same thread can talk
        // to two different agents without contaminating each other.
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let bob = fresh_id();
        let now = Instant::now();
        let k_u1 = key("T1").with_peer_id(Some("user-1"));
        let k_u2 = key("T1").with_peer_id(Some("user-2"));
        let _ = reg.decide_at(k_u1.clone(), alice, false, now);
        match reg.decide_at(k_u2.clone(), bob, false, now) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, bob),
            other => panic!("expected Allow across peers, got {:?}", other),
        }
        assert_eq!(reg.current_holder(&k_u1), Some(alice));
        assert_eq!(reg.current_holder(&k_u2), Some(bob));
    }

    #[test]
    fn blank_optional_slices_collapse_to_none() {
        // A blank metadata value must not splinter the key away from the
        // unset form.
        let bare = key("T1");
        let blanked = key("T1")
            .with_account_id(Some("  "))
            .with_chat_id(Some(""))
            .with_peer_id(None);
        assert_eq!(bare, blanked);
    }

    #[test]
    fn per_channel_ttl_overrides_default() {
        // A 10s override expires the claim where the 300s default would not.
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let bob = fresh_id();
        let t0 = Instant::now();
        let ttl = Duration::from_secs(10);
        let _ = reg.decide_at_with_ttl(key("T1"), alice, false, t0, ttl);
        let after_ttl = t0 + Duration::from_secs(11);
        match reg.decide_at_with_ttl(key("T1"), bob, false, after_ttl, ttl) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, bob),
            other => panic!("expected Allow after per-call TTL expiry, got {:?}", other),
        }
    }
}
