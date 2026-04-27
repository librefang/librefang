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
//! This module adds an in-memory single-process claim registry keyed
//! `(channel, thread)` with a TTL. The bridge consults it after routing and
//! before dispatch, suppressing any agent that isn't the current claim
//! holder. An explicit @-mention re-claims for the new agent.
//!
//! Multi-process / multi-daemon coordination (sharing the registry across
//! processes via a forwarder API) is out of scope — see issue #3334. DMs
//! bypass the registry entirely (no overlap risk by definition).

use dashmap::DashMap;
use librefang_types::agent::AgentId;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Default TTL for a fresh claim. After this many seconds without a refresh,
/// the next agent to dispatch can take ownership.
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);

/// Identity of a single (channel, account, thread) tuple. Built per-message
/// from the canonical channel-type slug, the optional multi-tenant account
/// identifier, and the platform's thread identifier.
///
/// `account_id` is part of the key because thread identifiers are not
/// globally unique across workspaces / guilds / orgs on most platforms
/// (Slack `thread_ts` is monotonic-ish but reused across workspaces;
/// Discord thread IDs are workspace-scoped). Without it, a claim from
/// account A's thread `T123` would shadow account B's thread `T123` on the
/// same channel slug. See #3334 review.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct ThreadKey {
    /// Adapter-qualified channel slug (e.g. `"slack"`, `"discord"`).
    pub channel: String,
    /// Multi-tenant account / workspace / guild identifier when the channel
    /// supports multi-tenant deployments. `None` for single-tenant channels
    /// or when the adapter does not surface an account id.
    pub account_id: Option<String>,
    /// Platform thread identifier (Slack `thread_ts`, Discord thread ID,
    /// etc.). Empty string is invalid; callers should not invoke the
    /// registry without a real thread.
    pub thread: String,
}

impl ThreadKey {
    /// Build a key from a channel slug, optional account id, and thread id.
    /// Trims whitespace; channel and thread must be non-empty after trimming
    /// or the call is meaningless. An empty `account_id` (after trimming)
    /// is treated as `None`.
    pub fn new(channel: &str, account_id: Option<&str>, thread: &str) -> Option<Self> {
        let channel = channel.trim();
        let thread = thread.trim();
        if channel.is_empty() || thread.is_empty() {
            return None;
        }
        let account_id = account_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        Some(Self {
            channel: channel.to_string(),
            account_id,
            thread: thread.to_string(),
        })
    }
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

    /// Test seam: like `decide` but with a caller-supplied `now`.
    pub fn decide_at(
        &self,
        key: ThreadKey,
        candidate: AgentId,
        was_mentioned: bool,
        now: Instant,
    ) -> DispatchDecision {
        let mut entry = self.claims.entry(key).or_insert_with(|| Claim {
            agent_id: candidate,
            claimed_at: now,
            ttl: self.default_ttl,
        });

        // Existing entry path. Three cases: same holder (extend), expired
        // (take over), different live holder (suppress unless mentioned).
        if entry.agent_id == candidate {
            entry.claimed_at = now;
            return DispatchDecision::Allow {
                agent_id: candidate,
            };
        }

        if entry.is_expired(now) {
            *entry = Claim {
                agent_id: candidate,
                claimed_at: now,
                ttl: self.default_ttl,
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
                ttl: self.default_ttl,
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
        ThreadKey::new("slack", None, thread).expect("key")
    }

    fn key_in_account(account: &str, thread: &str) -> ThreadKey {
        ThreadKey::new("slack", Some(account), thread).expect("key")
    }

    #[test]
    fn empty_thread_key_rejected() {
        assert!(ThreadKey::new("", None, "T123").is_none());
        assert!(ThreadKey::new("slack", None, "").is_none());
        assert!(ThreadKey::new("  ", None, "T123").is_none());
        assert!(ThreadKey::new("slack", None, "  ").is_none());
        assert!(ThreadKey::new("slack", None, "T123").is_some());
    }

    #[test]
    fn empty_account_id_normalized_to_none() {
        // Adapters that surface account_id as "" rather than absent should
        // produce the same key as the absent variant — otherwise
        // single-tenant traffic would split into two distinct claim slots.
        let absent = ThreadKey::new("slack", None, "T1").unwrap();
        let blank_some = ThreadKey::new("slack", Some(""), "T1").unwrap();
        let whitespace_some = ThreadKey::new("slack", Some("  "), "T1").unwrap();
        assert_eq!(absent, blank_some);
        assert_eq!(absent, whitespace_some);
    }

    #[test]
    fn distinct_accounts_do_not_collide_on_same_thread_id() {
        // Slack `thread_ts` is not globally unique across workspaces. If two
        // workspaces happen to produce the same `thread_ts`, the registry
        // must keep their claims independent.
        let reg = ThreadOwnershipRegistry::new();
        let alice = fresh_id();
        let bob = fresh_id();
        let now = Instant::now();
        let _ = reg.decide_at(key_in_account("acctA", "T1"), alice, false, now);
        match reg.decide_at(key_in_account("acctB", "T1"), bob, false, now) {
            DispatchDecision::Allow { agent_id } => assert_eq!(agent_id, bob),
            other => panic!(
                "expected Allow on different account with same thread_id, got {:?}",
                other
            ),
        }
        // Holders are independent — re-querying acctA must still see alice.
        assert_eq!(
            reg.current_holder(&key_in_account("acctA", "T1")),
            Some(alice)
        );
        assert_eq!(
            reg.current_holder(&key_in_account("acctB", "T1")),
            Some(bob)
        );
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
}
