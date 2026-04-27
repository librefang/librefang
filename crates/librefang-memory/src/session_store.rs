//! Pluggable session-store abstraction.
//!
//! `SessionStore` is a thin trait covering the session-CRUD subset of
//! [`MemorySubstrate`](crate::MemorySubstrate). It exists so future memory
//! backends (in-memory, file-based, lancedb, …) can provide the same API
//! without forcing every consumer to take the concrete substrate type.
//!
//! The current SQLite-backed substrate implements this trait by delegating
//! straight to its existing inherent methods — semantics are identical.
//!
//! Note: this trait is intentionally narrow. Sibling traits for KV,
//! proactive memory, etc. can be added in follow-up PRs without disturbing
//! existing callers. Consumers in the workspace continue to use
//! [`MemorySubstrate`](crate::MemorySubstrate) directly; migration to this
//! trait is a separate, opt-in step.

use crate::session::Session;
use crate::substrate::MemorySubstrate;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::error::LibreFangResult;

/// Session-CRUD subset of the memory substrate API.
///
/// All methods preserve the exact signatures and error semantics of the
/// equivalently named inherent methods on [`MemorySubstrate`].
pub trait SessionStore {
    /// Load a session by id. Returns `Ok(None)` when the session does not exist.
    fn get_session(&self, session_id: SessionId) -> LibreFangResult<Option<Session>>;

    /// Persist a session. Implementations must be idempotent — calling
    /// `save_session` repeatedly with the same payload must not corrupt state.
    fn save_session(&self, session: &Session) -> LibreFangResult<()>;

    /// Return all session ids belonging to `agent_id`, newest first.
    fn get_agent_session_ids(&self, agent_id: AgentId) -> LibreFangResult<Vec<SessionId>>;

    /// Delete a session by id. Implementations should treat a missing id as
    /// a no-op (do not return an error when the row is already gone).
    fn delete_session(&self, session_id: SessionId) -> LibreFangResult<()>;
}

impl SessionStore for MemorySubstrate {
    fn get_session(&self, session_id: SessionId) -> LibreFangResult<Option<Session>> {
        MemorySubstrate::get_session(self, session_id)
    }

    fn save_session(&self, session: &Session) -> LibreFangResult<()> {
        MemorySubstrate::save_session(self, session)
    }

    fn get_agent_session_ids(&self, agent_id: AgentId) -> LibreFangResult<Vec<SessionId>> {
        MemorySubstrate::get_agent_session_ids(self, agent_id)
    }

    fn delete_session(&self, session_id: SessionId) -> LibreFangResult<()> {
        MemorySubstrate::delete_session(self, session_id)
    }
}

#[cfg(test)]
mod tests {
    //! Trait-level tests: exercise `MemorySubstrate` exclusively through
    //! `&dyn SessionStore` so the abstraction itself is covered, not just
    //! the concrete impl. These tests fail to compile if the trait drifts
    //! away from the substrate API.

    use super::SessionStore;
    use crate::MemorySubstrate;
    use librefang_types::agent::AgentId;
    use librefang_types::message::Message;

    fn store() -> MemorySubstrate {
        MemorySubstrate::open_in_memory(0.1).expect("in-memory substrate")
    }

    #[test]
    fn save_and_get_through_trait() {
        let substrate = store();
        let store: &dyn SessionStore = &substrate;
        let agent_id = AgentId::new();

        // Round-trip an empty session created via the substrate's helper,
        // then persist+load through the trait.
        let mut session = substrate
            .create_session(agent_id)
            .expect("create empty session");
        session.messages.push(Message::user("hello via trait"));
        session.messages.push(Message::assistant("ack"));
        store.save_session(&session).expect("save through trait");

        let loaded = store
            .get_session(session.id)
            .expect("get through trait")
            .expect("session present");
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.agent_id, agent_id);
        assert_eq!(loaded.messages.len(), 2);
    }

    #[test]
    fn get_missing_session_returns_none() {
        let substrate = store();
        let store: &dyn SessionStore = &substrate;
        let missing = librefang_types::agent::SessionId::new();
        assert!(store.get_session(missing).expect("get").is_none());
    }

    #[test]
    fn list_agent_session_ids_through_trait() {
        let substrate = store();
        let agent_id = AgentId::new();
        let s1 = substrate.create_session(agent_id).expect("s1");
        let s2 = substrate.create_session(agent_id).expect("s2");

        let store: &dyn SessionStore = &substrate;
        let ids = store
            .get_agent_session_ids(agent_id)
            .expect("list ids through trait");
        assert!(ids.contains(&s1.id));
        assert!(ids.contains(&s2.id));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn delete_session_through_trait() {
        let substrate = store();
        let agent_id = AgentId::new();
        let session = substrate.create_session(agent_id).expect("create");

        let store: &dyn SessionStore = &substrate;
        assert!(store
            .get_session(session.id)
            .expect("pre-delete get")
            .is_some());
        store
            .delete_session(session.id)
            .expect("delete through trait");
        assert!(store
            .get_session(session.id)
            .expect("post-delete get")
            .is_none());
    }
}
