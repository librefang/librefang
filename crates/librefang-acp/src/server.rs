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
    AgentCapabilities, CancelNotification, CloseSessionRequest, CloseSessionResponse,
    InitializeRequest, InitializeResponse, ListSessionsRequest, ListSessionsResponse,
    LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PromptCapabilities, PromptRequest, ResumeSessionRequest, ResumeSessionResponse,
    SessionCapabilities, SessionCloseCapabilities, SessionInfo, SessionListCapabilities,
    SessionResumeCapabilities,
};
use agent_client_protocol::{Agent, Client, ConnectTo, Dispatch};
use agent_client_protocol_tokio::Stdio;
use librefang_types::agent::AgentId;
use tracing::debug;

use crate::fs::{FsCapabilities, FsClientHandle};
use crate::permission;
use crate::prompt;
use crate::session::{SessionState, SessionStore};
use crate::terminal::{TerminalCapabilities, TerminalClientHandle};
use crate::{AcpKernel, AcpResult};

/// Run the ACP server bound to `kernel` and `agent_id` on stdio.
///
/// Returns when stdin closes (the editor has disconnected) or the
/// transport hits an unrecoverable error. Used by the `librefang acp`
/// CLI subcommand for the in-process execution mode; the
/// daemon-attached UDS path uses [`run_with_transport`] directly with
/// the connection's framed stream.
pub async fn run<K: AcpKernel>(kernel: Arc<K>, agent_id: AgentId) -> AcpResult<()> {
    run_with_transport(kernel, agent_id, Stdio::new()).await
}

/// Same as [`run`] but with an explicit transport. Used by integration
/// tests in `tests/acp_integration.rs` to drive the server over a
/// `tokio::io::duplex` pipe instead of real stdio.
pub async fn run_with_transport<K, T>(
    kernel: Arc<K>,
    agent_id: AgentId,
    transport: T,
) -> AcpResult<()>
where
    K: AcpKernel,
    T: ConnectTo<Agent> + Send + 'static,
{
    let sessions = SessionStore::new_arc();

    // Each builder method consumes the builder, so we clone the Arcs
    // we want each handler to own up front.
    let kernel_for_init = Arc::clone(&kernel);
    let kernel_for_new = Arc::clone(&kernel);
    let kernel_for_load = Arc::clone(&kernel);
    let kernel_for_resume = Arc::clone(&kernel);
    let kernel_for_close = Arc::clone(&kernel);
    let kernel_for_perm = Arc::clone(&kernel);
    let kernel_for_prompt = Arc::clone(&kernel);
    let sessions_for_new = Arc::clone(&sessions);
    let sessions_for_load = Arc::clone(&sessions);
    let sessions_for_resume = Arc::clone(&sessions);
    let sessions_for_list = Arc::clone(&sessions);
    let sessions_for_close = Arc::clone(&sessions);
    let sessions_for_prompt = Arc::clone(&sessions);
    let sessions_for_cancel = Arc::clone(&sessions);
    let sessions_for_perm = Arc::clone(&sessions);

    Agent
        .builder()
        .name("librefang")
        // initialize ----------------------------------------------------
        .on_receive_request(
            async move |req: InitializeRequest, responder, cx: agent_client_protocol::ConnectionTo<Client>| {
                debug!(client = ?req.client_info, "ACP initialize");
                // Hand the kernel a `fs/*` reverse-RPC handle so any
                // tool the runtime later runs can read / write through
                // the editor instead of the local filesystem (#3313).
                // The handle captures the editor's declared
                // capabilities so the runtime can short-circuit when
                // the editor doesn't support the operation, instead of
                // round-tripping a `method_not_found`.
                let fs_caps = FsCapabilities::from_client(&req.client_capabilities);
                kernel_for_init.set_fs_client(FsClientHandle::new(cx.clone(), fs_caps));
                let term_caps = TerminalCapabilities::from_client(&req.client_capabilities);
                kernel_for_init
                    .set_terminal_client(TerminalClientHandle::new(cx.clone(), term_caps));
                let session_caps = SessionCapabilities::new()
                    .list(SessionListCapabilities::default())
                    .resume(SessionResumeCapabilities::default())
                    .close(SessionCloseCapabilities::default());
                // Explicit declaration: this build does not yet pipe
                // image / audio / embedded resource content blocks
                // through the agent loop. `PromptCapabilities::new()`
                // defaults all three to `false`, which is what we
                // want — telling the editor up front lets it downgrade
                // or warn instead of silently dropping multimodal
                // input on the floor.
                let prompt_caps = PromptCapabilities::new();
                let agent_caps = AgentCapabilities::new()
                    .load_session(true)
                    .session_capabilities(session_caps)
                    .prompt_capabilities(prompt_caps);
                responder.respond(
                    InitializeResponse::new(req.protocol_version)
                        .agent_capabilities(agent_caps)
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
                let state = SessionState::for_acp_id(&new_id, req.cwd);
                let lf_id = state.librefang_session_id;
                debug!(session_id = %new_id.0, librefang_id = %lf_id.0,
                       "ACP session/new");
                sessions_for_new.insert(new_id.clone(), state);
                // Bind the editor's `fs/*` client (set at `initialize`)
                // to this session so runtime tools dispatched on it can
                // route through the editor (#3313). No-op for kernels
                // without an attached editor.
                kernel_for_new.register_session_fs(lf_id);
                kernel_for_new.register_session_terminal(lf_id);
                responder.respond(NewSessionResponse::new(new_id))
            },
            agent_client_protocol::on_receive_request!(),
        )
        // session/load --------------------------------------------------
        // Phase 1 doesn't persist session history across `librefang acp`
        // invocations, so loading is a no-op on history but does honour
        // the client-supplied session id so multi-tab editors can
        // reconnect with a stable id within one process. A real history
        // replay (including past `session/update` notifications) is
        // tracked separately under #3313 phase 2.
        .on_receive_request(
            async move |req: LoadSessionRequest, responder, _cx| {
                let state = SessionState::for_acp_id(&req.session_id, req.cwd);
                let lf_id = state.librefang_session_id;
                debug!(session_id = %req.session_id.0, librefang_id = %lf_id.0,
                       "ACP session/load (no history replay yet)");
                sessions_for_load.insert(req.session_id, state);
                kernel_for_load.register_session_fs(lf_id);
                kernel_for_load.register_session_terminal(lf_id);
                responder.respond(LoadSessionResponse::default())
            },
            agent_client_protocol::on_receive_request!(),
        )
        // session/resume ------------------------------------------------
        // Identical to load in Phase 1 — both create-or-replace the
        // mapping. The protocol distinction (resume MUST NOT replay
        // history) is moot until we have history to replay.
        .on_receive_request(
            async move |req: ResumeSessionRequest, responder, _cx| {
                let state = SessionState::for_acp_id(&req.session_id, req.cwd);
                let lf_id = state.librefang_session_id;
                debug!(session_id = %req.session_id.0, librefang_id = %lf_id.0,
                       "ACP session/resume");
                sessions_for_resume.insert(req.session_id, state);
                kernel_for_resume.register_session_fs(lf_id);
                kernel_for_resume.register_session_terminal(lf_id);
                responder.respond(ResumeSessionResponse::default())
            },
            agent_client_protocol::on_receive_request!(),
        )
        // session/list --------------------------------------------------
        .on_receive_request(
            async move |req: ListSessionsRequest, responder, _cx| {
                let mut sessions: Vec<SessionInfo> = sessions_for_list
                    .list()
                    .into_iter()
                    .filter(|(_, cwd)| match req.cwd.as_ref() {
                        Some(filter) => cwd == filter,
                        None => true,
                    })
                    .map(|(id, cwd)| SessionInfo::new(id, cwd))
                    .collect();
                // Stable order so test fixtures don't flap on DashMap
                // iteration nondeterminism.
                sessions.sort_by(|a, b| a.session_id.0.cmp(&b.session_id.0));
                responder.respond(ListSessionsResponse::new(sessions))
            },
            agent_client_protocol::on_receive_request!(),
        )
        // session/close -------------------------------------------------
        .on_receive_request(
            async move |req: CloseSessionRequest, responder, _cx| {
                let removed = sessions_for_close.remove(&req.session_id);
                if let Some(state) = removed.as_ref() {
                    kernel_for_close.unregister_session_fs(state.librefang_session_id);
                    kernel_for_close.unregister_session_terminal(state.librefang_session_id);
                }
                debug!(
                    session_id = %req.session_id.0,
                    removed = removed.is_some(),
                    "ACP session/close",
                );
                responder.respond(CloseSessionResponse::default())
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
        // terminal/*, fs/*, …) so the editor gets a JSON-RPC
        // `method_not_found` (-32601) instead of `internal_error`
        // (-32603). Editors typically handle the former by silently
        // skipping the optional feature; `internal_error` looks like a
        // bug and surfaces a user-visible diagnostic.
        //
        // Matches all three [`Dispatch`] variants explicitly so that
        // *responses* to our own outgoing requests (the permission
        // bridge's `cx.send_request(RequestPermissionRequest)` round
        // trips, etc.) don't get rewrapped as JSON-RPC errors and
        // propagated back to the bridge.
        .on_receive_dispatch(
            async move |message: Dispatch, _cx: agent_client_protocol::ConnectionTo<Client>| {
                match message {
                    Dispatch::Request(_, responder) => responder
                        .respond_with_error(agent_client_protocol::Error::method_not_found()),
                    Dispatch::Notification(_) => Ok(()),
                    Dispatch::Response(result, router) => router.respond_with_result(result),
                }
            },
            agent_client_protocol::on_receive_dispatch!(),
        )
        // Background: permission bridge --------------------------------
        .with_spawned(move |cx| async move {
            let kernel = kernel_for_perm;
            let sessions = sessions_for_perm;
            permission::run_bridge(kernel, sessions, cx).await
        })
        .connect_to(transport)
        .await?;

    Ok(())
}

/// Mint a fresh ACP `SessionId`. UUID v4; we don't derive
/// deterministic ids because session-history replay across daemon
/// restarts (which would benefit from a stable id) is tracked as a
/// follow-up — see the doc on `session/load` in the handler chain.
fn next_session_id() -> agent_client_protocol::schema::SessionId {
    let uuid = uuid::Uuid::new_v4();
    agent_client_protocol::schema::SessionId::new(uuid.to_string())
}
