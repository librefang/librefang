//! [`kernel_handle::AcpTerminalBridge`] — editor-backed `terminal/*`
//! reverse-RPC routing (#3313). Mirrors the `acp_fs_bridge` impl.

use std::sync::Arc;

use librefang_runtime::kernel_handle;
use librefang_types::agent::SessionId;

use super::super::LibreFangKernel;

impl kernel_handle::AcpTerminalBridge for LibreFangKernel {
    fn register_acp_terminal_client(
        &self,
        session_id: SessionId,
        client: Arc<dyn kernel_handle::AcpTerminalClient>,
    ) {
        self.acp_terminal_clients.insert(session_id, client);
    }

    fn unregister_acp_terminal_client(&self, session_id: SessionId) {
        self.acp_terminal_clients.remove(&session_id);
    }

    fn acp_terminal_client(
        &self,
        session_id: SessionId,
    ) -> Option<Arc<dyn kernel_handle::AcpTerminalClient>> {
        self.acp_terminal_clients
            .get(&session_id)
            .map(|entry| Arc::clone(entry.value()))
    }
}
