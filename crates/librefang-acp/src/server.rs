//! Top-level ACP server: builder assembly + stdio entry point.
//!
//! [`run`] is the public entry. It clones the kernel + session-store
//! into each handler closure, chains them onto an
//! [`agent_client_protocol::Agent`] builder, runs the permission bridge
//! as a background task spawned with [`agent_client_protocol::Builder::with_spawned`],
//! and finally hands the whole thing to [`agent_client_protocol_tokio::Stdio`]
//! to drive the JSON-RPC loop until stdin EOF.

use std::sync::Arc;

use agent_client_protocol::schema::{
    AgentCapabilities, CancelNotification, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest,
};
use agent_client_protocol::{Agent, Client, Dispatch};
use agent_client_protocol_tokio::Stdio;
use librefang_types::agent::AgentId;
use tracing::debug;

use crate::permission;
use crate::prompt;
use crate::session::{SessionState, SessionStore};
use crate::{AcpKernel, AcpResult};

/// Run the ACP server bound to `kernel` and `agent_id` on stdio.
///
/// Returns when stdin closes (the editor has disconnected) or the
/// transport hits an unrecoverable error. The function is intended to
/// be called once per process from the `librefang acp` CLI subcommand;
/// daemon-attached mode lands in Phase 2.
pub async fn run<K: AcpKernel>(kernel: Arc<K>, agent_id: AgentId) -> AcpResult<()> {
    let sessions = SessionStore::new_arc();

    // Each builder method consumes the builder, so we clone the Arcs
    // we want each handler to own up front.
    let kernel_for_perm = Arc::clone(&kernel);
    let kernel_for_prompt = Arc::clone(&kernel);
    let sessions_for_new = Arc::clone(&sessions);
    let sessions_for_prompt = Arc::clone(&sessions);
    let sessions_for_cancel = Arc::clone(&sessions);
    let sessions_for_perm = Arc::clone(&sessions);

    Agent
        .builder()
        .name("librefang")
        // initialize ----------------------------------------------------
        .on_receive_request(
            async move |req: InitializeRequest, responder, _cx| {
                debug!(client = ?req.client_info, "ACP initialize");
                responder.respond(
                    InitializeResponse::new(req.protocol_version)
                        .agent_capabilities(AgentCapabilities::new())
                        .agent_info(agent_client_protocol::schema::Implementation::new(
                            "librefang",
                            env!("CARGO_PKG_VERSION"),
                        )),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        // session/new ---------------------------------------------------
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                let new_id = next_session_id();
                let state = SessionState::new(req.cwd);
                debug!(session_id = %new_id.0, librefang_id = %state.librefang_session_id.0,
                       "ACP session/new");
                sessions_for_new.insert(new_id.clone(), state);
                responder.respond(NewSessionResponse::new(new_id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        // session/prompt ------------------------------------------------
        .on_receive_request(
            async move |req: PromptRequest, responder, cx: agent_client_protocol::ConnectionTo<Client>| {
                let kernel = Arc::clone(&kernel_for_prompt);
                let sessions = Arc::clone(&sessions_for_prompt);
                prompt::handle(kernel, sessions, agent_id, req, responder, cx).await
            },
            agent_client_protocol::on_receive_request!(),
        )
        // session/cancel (notification) ---------------------------------
        .on_receive_notification(
            async move |notif: CancelNotification, _cx| {
                debug!(session_id = %notif.session_id.0, "ACP session/cancel");
                sessions_for_cancel.cancel(&notif.session_id);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        // Catch-all for methods we don't yet implement (authenticate,
        // session/load, session/list, terminal/*, fs/*, …) so the
        // editor gets a clean method_not_found instead of hanging.
        .on_receive_dispatch(
            async move |message: Dispatch, cx: agent_client_protocol::ConnectionTo<Client>| {
                message.respond_with_error(
                    agent_client_protocol::util::internal_error(
                        "method not supported by librefang ACP Phase 1",
                    ),
                    cx,
                )
            },
            agent_client_protocol::on_receive_dispatch!(),
        )
        // Background: permission bridge --------------------------------
        .with_spawned(move |cx| async move {
            let kernel = kernel_for_perm;
            let sessions = sessions_for_perm;
            permission::run_bridge(kernel, sessions, cx).await
        })
        .connect_to(Stdio::new())
        .await?;

    Ok(())
}

/// Mint a fresh ACP `SessionId`. Phase 1 uses a UUID v4; we don't
/// derive deterministic ids since each `librefang acp` invocation
/// gets its own fresh kernel and prior sessions don't survive
/// process exit.
fn next_session_id() -> agent_client_protocol::schema::SessionId {
    let uuid = uuid::Uuid::new_v4();
    agent_client_protocol::schema::SessionId::new(uuid.to_string())
}
