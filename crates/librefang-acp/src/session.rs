// Wired into `server.rs` in the next milestone of #3313; the workspace
// `warnings = "deny"` lint would otherwise reject the scaffolding-only
// state of this PR.
#![allow(dead_code)]

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

    pub(crate) fn remove(&self, id: &AcpSessionId) -> Option<SessionState> {
        self.inner.remove(id).map(|(_, v)| v)
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
        store.insert(id.clone(), state.clone());
        let fetched = store.get(&id).expect("session should exist");
        assert_eq!(fetched.cwd, PathBuf::from("/tmp/proj"));
        assert!(store.remove(&id).is_some());
        assert!(store.get(&id).is_none());
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
}
