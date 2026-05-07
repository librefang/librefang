//! [`kernel_handle::AcpFsBridge`] — editor-backed `fs/*` reverse-RPC
//! routing (#3313).
//!
//! The kernel keeps a `DashMap<SessionId, Arc<dyn AcpFsClient>>`
//! populated by the ACP adapter at `session/new` time. This impl just
//! exposes register / unregister / lookup over that map; the read /
//! write path picks up the trait-default `acp_read_text_file` /
//! `acp_write_text_file` from `librefang-kernel-handle`, which calls
//! the looked-up client.
//!
//! Sessions without an attached editor (dashboard / TUI / cron / channel
//! bridge) get `Unavailable` from the default impls — runtime tools that
//! opt into ACP backing should treat that as "fall back to local fs",
//! not as a hard error.

use std::sync::Arc;

use librefang_runtime::kernel_handle;
use librefang_types::agent::SessionId;
use tracing::warn;

use super::super::LibreFangKernel;

impl kernel_handle::AcpFsBridge for LibreFangKernel {
    fn register_acp_fs_client(
        &self,
        session_id: SessionId,
        client: Arc<dyn kernel_handle::AcpFsClient>,
    ) {
        // `SessionId` is derived deterministically from the editor's
        // ACP session id (UUIDv5 in `librefang-acp::session::SessionState::for_acp_id`),
        // so two editor tabs that load the same ACP session id collide
        // on the same kernel-side `SessionId`. Without this guard the
        // second `register` would silently displace the first tab's
        // client handle and start routing tab A's tool calls to tab B's
        // editor. Log loudly so the operator can correlate against an
        // editor-side reconnect log; we still allow the displacement
        // because (a) reconnect after a transient disconnect is
        // legitimate and (b) the displaced handle's transport is
        // typically already dead.
        if let Some(_prior) = self.acp_fs_clients.insert(session_id, client) {
            warn!(
                session_id = %session_id.0,
                "ACP fs client re-registered for same session id — \
                 prior tab's tool calls will route to the new editor connection"
            );
        }
    }

    fn unregister_acp_fs_client(&self, session_id: SessionId) {
        self.acp_fs_clients.remove(&session_id);
    }

    fn acp_fs_client(&self, session_id: SessionId) -> Option<Arc<dyn kernel_handle::AcpFsClient>> {
        self.acp_fs_clients
            .get(&session_id)
            .map(|entry| Arc::clone(entry.value()))
    }
}
