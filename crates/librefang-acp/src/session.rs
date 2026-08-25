//! ACP session state map.
//!
//! ACP sessions are created via `session/new`. Each ACP session is a
//! distinct conversation surface even when multiple sessions multiplex
//! over the same stdio connection. We map each ACP `SessionId` (a string
//! arc on the wire) to:
//!
//! 1. The LibreFang [`librefang_types::agent::SessionId`] that backs the
//!    underlying agent loop. Phase 1 derives one fresh per ACP session
//!    via `SessionId::new()` so prior chat history doesn't leak across
//!    `librefang acp` invocations.
//! 2. The cwd the editor declared at `session/new` time, surfaced to the
//!    agent loop so file-relative paths in tool calls resolve against
//!    the editor's project root.
//! 3. A [`tokio_util::sync::CancellationToken`] used by `session/cancel`
//!    notifications to interrupt the active prompt pump.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use agent_client_protocol::schema::v1::SessionId as AcpSessionId;
use librefang_types::agent::SessionId as LfSessionId;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// Per-ACP-session state.
#[derive(Debug, Clone)]
pub(crate) struct SessionState {
    pub librefang_session_id: LfSessionId,
    #[allow(dead_code)] // surfaced to the agent loop in Phase 2 via SenderContext
    pub cwd: PathBuf,
    /// Cancelled by the `session/cancel` notification. Cloned by the
    /// prompt pump so a tokio `select!` can short-circuit on cancel.
    pub cancel: CancellationToken,
}

/// Namespace UUID used to derive a stable LibreFang `SessionId` from
/// an ACP session id string. Same string ⇒ same kernel-side session,
/// so a `session/load` after the editor reopens picks up the same
/// LibreFang session — and therefore the same persisted message
/// history — that the previous `session/new` minted.
const ACP_SESSION_NS: Uuid = Uuid::from_bytes([
    0xa3, 0x0c, 0x71, 0x3a, 0x4b, 0x1c, 0x4f, 0x6e, 0xb5, 0x12, 0x9c, 0x7f, 0x88, 0xd0, 0xa1, 0x42,
]);

impl SessionState {
    /// Build a `SessionState` for the given ACP session id with a
    /// `Uuid::new_v5`-derived LibreFang session id. Stable: same input
    /// always produces the same kernel-side id, so a reconnecting
    /// editor's `session/load` rejoins the existing session.
    ///
    /// The ACP id is a same-user bearer capability, not an independent
    /// authorization token. Production ids are random UUID v4 values minted
    /// by `session/new`, and the daemon transports admit only the daemon's OS
    /// user (owner-only UDS permissions plus peer UID on Unix, owner-only pipe
    /// DACL on Windows). A future remote or multi-user transport must bind
    /// load/resume to an authenticated principal instead of reusing this
    /// deterministic derivation as an access-control decision.
    pub(crate) fn for_acp_id(acp_id: &AcpSessionId, cwd: PathBuf) -> Self {
        let lf_uuid = Uuid::new_v5(&ACP_SESSION_NS, acp_id.0.as_bytes());
        Self {
            librefang_session_id: LfSessionId(lf_uuid),
            cwd,
            cancel: CancellationToken::new(),
        }
    }

    /// Random-id constructor kept for tests that don't care about
    /// cross-restart stability. Production paths should use
    /// [`Self::for_acp_id`].
    #[cfg(test)]
    pub(crate) fn new(cwd: PathBuf) -> Self {
        Self {
            librefang_session_id: LfSessionId(Uuid::new_v4()),
            cwd,
            cancel: CancellationToken::new(),
        }
    }
}

/// Concurrent map of ACP `SessionId` -> `SessionState`.
///
/// `Arc<SessionStore>` is cloned into every handler closure so all
/// handlers see the same map.
#[derive(Debug, Default)]
struct SessionMaps {
    by_acp_id: HashMap<AcpSessionId, Arc<SessionState>>,
    /// Permission events carry only the LibreFang session id. Keep the
    /// reverse mapping alongside the primary store so approval routing is an
    /// O(1) lookup instead of scanning all active sessions.
    by_librefang_id: HashMap<LfSessionId, AcpSessionId>,
}

#[derive(Debug, Default)]
pub(crate) struct SessionStore {
    /// Both indexes share one lock so insert, replacement, removal, and drain
    /// publish atomically to readers.
    maps: RwLock<SessionMaps>,
}

impl SessionStore {
    pub(crate) fn new_arc() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn read_maps(&self) -> RwLockReadGuard<'_, SessionMaps> {
        self.maps.read().unwrap_or_else(|poisoned| {
            tracing::warn!("ACP session store read lock poisoned; recovering state");
            let guard = poisoned.into_inner();
            self.maps.clear_poison();
            guard
        })
    }

    fn write_maps(&self) -> RwLockWriteGuard<'_, SessionMaps> {
        self.maps.write().unwrap_or_else(|poisoned| {
            tracing::warn!("ACP session store write lock poisoned; recovering state");
            let guard = poisoned.into_inner();
            self.maps.clear_poison();
            guard
        })
    }

    /// Insert or replace a session.
    ///
    /// Replacement cancels the displaced state's prompt token before
    /// returning it to the caller for downstream handle cleanup.
    pub(crate) fn insert(
        &self,
        id: AcpSessionId,
        state: SessionState,
    ) -> Option<Arc<SessionState>> {
        let librefang_id = state.librefang_session_id;
        let mut maps = self.write_maps();
        let previous = maps.by_acp_id.insert(id.clone(), Arc::new(state));
        if let Some(previous) = previous.as_ref() {
            let previous_librefang_id = previous.librefang_session_id;
            if previous_librefang_id != librefang_id
                && maps
                    .by_librefang_id
                    .get(&previous_librefang_id)
                    .is_some_and(|mapped| mapped == &id)
            {
                maps.by_librefang_id.remove(&previous_librefang_id);
            }
        }
        maps.by_librefang_id.insert(librefang_id, id);
        drop(maps);

        if let Some(previous) = previous.as_ref() {
            previous.cancel.cancel();
        }
        previous
    }

    pub(crate) fn get(&self, id: &AcpSessionId) -> Option<Arc<SessionState>> {
        self.read_maps().by_acp_id.get(id).map(Arc::clone)
    }

    /// Reverse lookup used by the permission bridge to translate a kernel
    /// `ApprovalRequest.session_id` (LibreFang `SessionId` serialised as
    /// a UUID string) back to the ACP `SessionId` we should target.
    pub(crate) fn find_by_librefang_id(&self, lf_id: &LfSessionId) -> Option<AcpSessionId> {
        self.read_maps().by_librefang_id.get(lf_id).cloned()
    }

    /// Enumerate `(acp_id, cwd)` for every active session. Used by the
    /// `session/list` handler. Cheap enough to do un-paginated for Phase 1
    /// — daemon-attached mode in Phase 2 will need a real cursor scheme.
    pub(crate) fn list(&self) -> Vec<(AcpSessionId, std::path::PathBuf)> {
        self.read_maps()
            .by_acp_id
            .iter()
            .map(|(id, state)| (id.clone(), state.cwd.clone()))
            .collect()
    }

    /// Remove a session by id. Returns the removed state if it
    /// existed so callers (`session/close`) can pull the
    /// LibreFang session id back out for downstream cleanup.
    pub(crate) fn remove(&self, id: &AcpSessionId) -> Option<Arc<SessionState>> {
        let mut maps = self.write_maps();
        let state = maps.by_acp_id.remove(id)?;
        let librefang_id = state.librefang_session_id;
        if maps
            .by_librefang_id
            .get(&librefang_id)
            .is_some_and(|mapped| mapped == id)
        {
            maps.by_librefang_id.remove(&librefang_id);
        }
        drop(maps);
        state.cancel.cancel();
        Some(state)
    }

    /// Trigger the cancel token for `id` if it exists. Returns `true`
    /// if a session was found, regardless of whether it was already
    /// cancelled — ACP `session/cancel` is fire-and-forget.
    pub(crate) fn cancel(&self, id: &AcpSessionId) -> bool {
        match self.read_maps().by_acp_id.get(id) {
            Some(state) => {
                state.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Empty the store, returning every `(acp_id, librefang_session_id)`
    /// pair that was active. Used by `run_with_transport`'s cleanup
    /// path: when the JSON-RPC loop ends without `session/close` for
    /// every session (editor crash, network drop, kill -9), we still
    /// need to unregister the per-session `fs/*` and `terminal/*`
    /// clients in the kernel registry. Otherwise a subsequent
    /// `register_session_fs` against a recycled `SessionId` would land
    /// alongside the dead handle and tool calls would race against a
    /// closed transport for `FS_RPC_TIMEOUT` (60s) before falling
    /// back. Returning the LibreFang ids — not just the ACP ids —
    /// lets the caller drive `unregister_session_fs` /
    /// `unregister_session_terminal` directly without re-looking-up
    /// each entry.
    pub(crate) fn drain_active(&self) -> Vec<(AcpSessionId, LfSessionId)> {
        let mut maps = self.write_maps();
        maps.by_librefang_id.clear();
        let states = std::mem::take(&mut maps.by_acp_id);
        drop(maps);
        states
            .into_iter()
            .map(|(id, state)| {
                state.cancel.cancel();
                (id, state.librefang_session_id)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_get_remove_roundtrip() {
        let store = SessionStore::default();
        let id: AcpSessionId = "sess-1".into();
        let state = SessionState::new(PathBuf::from("/tmp/proj"));
        let lf_id = state.librefang_session_id;
        let cancel = state.cancel.clone();
        assert!(store.insert(id.clone(), state).is_none());
        let fetched = store.get(&id).expect("session should exist");
        assert_eq!(fetched.cwd, PathBuf::from("/tmp/proj"));
        let fetched_again = store.get(&id).expect("session should still exist");
        assert!(
            Arc::ptr_eq(&fetched, &fetched_again),
            "reads should clone only the shared state Arc"
        );
        let reverse = store.find_by_librefang_id(&lf_id).expect("reverse lookup");
        assert_eq!(reverse, id);

        let removed = store.remove(&id).expect("remove session");
        assert_eq!(removed.librefang_session_id, lf_id);
        assert!(cancel.is_cancelled());
        assert!(store.get(&id).is_none());
        assert!(store.find_by_librefang_id(&lf_id).is_none());
    }

    #[test]
    fn cancel_flips_token() {
        let store = SessionStore::default();
        let id: AcpSessionId = "sess-2".into();
        let state = SessionState::new(PathBuf::from("/tmp"));
        let token = state.cancel.clone();
        assert!(store.insert(id.clone(), state).is_none());
        assert!(!token.is_cancelled());
        assert!(store.cancel(&id));
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancel_unknown_session_returns_false() {
        let store = SessionStore::default();
        let unknown: AcpSessionId = "nope".into();
        assert!(!store.cancel(&unknown));
    }

    #[test]
    fn reverse_lookup_misses_when_unknown() {
        let store = SessionStore::default();
        let phantom = LfSessionId(Uuid::new_v4());
        assert!(store.find_by_librefang_id(&phantom).is_none());
    }

    #[test]
    fn replacing_a_session_removes_its_stale_reverse_entry() {
        let store = SessionStore::default();
        let id: AcpSessionId = "replace-me".into();
        let first = SessionState::new(PathBuf::from("/first"));
        let first_lf_id = first.librefang_session_id;
        let second = SessionState::new(PathBuf::from("/second"));
        let second_lf_id = second.librefang_session_id;

        let first_cancel = first.cancel.clone();
        assert!(store.insert(id.clone(), first).is_none());
        let displaced = store.insert(id.clone(), second).expect("displaced state");

        assert_eq!(displaced.librefang_session_id, first_lf_id);
        assert!(first_cancel.is_cancelled());
        assert!(store.find_by_librefang_id(&first_lf_id).is_none());
        assert_eq!(store.find_by_librefang_id(&second_lf_id), Some(id));
    }

    #[test]
    fn drain_active_clears_both_indexes() {
        let store = SessionStore::default();
        let id: AcpSessionId = "drain-me".into();
        let state = SessionState::new(PathBuf::from("/tmp"));
        let lf_id = state.librefang_session_id;
        let cancel = state.cancel.clone();
        assert!(store.insert(id.clone(), state).is_none());

        assert_eq!(store.drain_active(), vec![(id.clone(), lf_id)]);
        assert!(cancel.is_cancelled());
        assert!(store.get(&id).is_none());
        assert!(store.find_by_librefang_id(&lf_id).is_none());
    }

    #[test]
    fn deterministic_ids_are_stable_and_distinct() {
        let first_id: AcpSessionId = "server-minted-id-1".into();
        let second_id: AcpSessionId = "server-minted-id-2".into();
        let first = SessionState::for_acp_id(&first_id, PathBuf::from("/one"));
        let first_again = SessionState::for_acp_id(&first_id, PathBuf::from("/two"));
        let second = SessionState::for_acp_id(&second_id, PathBuf::from("/one"));

        assert_eq!(first.librefang_session_id, first_again.librefang_session_id);
        assert_ne!(first.librefang_session_id, second.librefang_session_id);
    }

    #[test]
    fn poisoned_store_lock_recovers_both_indexes() {
        let store = SessionStore::new_arc();
        let panicking_store = Arc::clone(&store);
        let panic = std::thread::spawn(move || {
            let _guard = panicking_store.maps.write().expect("initial write lock");
            panic!("poison session store");
        });
        assert!(panic.join().is_err());
        assert!(store.maps.is_poisoned());

        let id: AcpSessionId = "after-poison".into();
        let state = SessionState::new(PathBuf::from("/tmp"));
        let lf_id = state.librefang_session_id;
        assert!(store.insert(id.clone(), state).is_none());

        assert!(!store.maps.is_poisoned());
        assert!(store.get(&id).is_some());
        assert_eq!(store.find_by_librefang_id(&lf_id), Some(id));
    }
}
