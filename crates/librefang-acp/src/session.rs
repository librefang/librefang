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

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::SessionId as AcpSessionId;
use dashmap::DashMap;
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

impl SessionState {
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
pub(crate) struct SessionStore {
    inner: DashMap<AcpSessionId, SessionState>,
}

impl SessionStore {
    pub(crate) fn new_arc() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(crate) fn insert(&self, id: AcpSessionId, state: SessionState) {
        self.inner.insert(id, state);
    }

    pub(crate) fn get(&self, id: &AcpSessionId) -> Option<SessionState> {
        self.inner.get(id).map(|r| r.value().clone())
    }

    /// Reverse lookup used by the permission bridge to translate a kernel
    /// `ApprovalRequest.session_id` (LibreFang `SessionId` serialised as
    /// a UUID string) back to the ACP `SessionId` we should target.
    pub(crate) fn find_by_librefang_id(&self, lf_id: &LfSessionId) -> Option<AcpSessionId> {
        self.inner
            .iter()
            .find(|entry| entry.value().librefang_session_id == *lf_id)
            .map(|entry| entry.key().clone())
    }

    /// Trigger the cancel token for `id` if it exists. Returns `true`
    /// if a session was found, regardless of whether it was already
    /// cancelled — ACP `session/cancel` is fire-and-forget.
    pub(crate) fn cancel(&self, id: &AcpSessionId) -> bool {
        match self.inner.get(id) {
            Some(state) => {
                state.value().cancel.cancel();
                true
            }
            None => false,
        }
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
        store.insert(id.clone(), state.clone());
        let fetched = store.get(&id).expect("session should exist");
        assert_eq!(fetched.cwd, PathBuf::from("/tmp/proj"));
        let reverse = store.find_by_librefang_id(&lf_id).expect("reverse lookup");
        assert_eq!(reverse, id);
    }

    #[test]
    fn cancel_flips_token() {
        let store = SessionStore::default();
        let id: AcpSessionId = "sess-2".into();
        let state = SessionState::new(PathBuf::from("/tmp"));
        let token = state.cancel.clone();
        store.insert(id.clone(), state);
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
}
