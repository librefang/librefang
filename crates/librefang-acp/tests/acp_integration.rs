//! End-to-end integration tests for the ACP adapter.
//!
//! Each test wires `librefang_acp::run_with_transport` to one end of a
//! `tokio::io::duplex` pipe and drives the matching `Client.builder()`
//! on the other end with a stub [`AcpKernel`] impl. This lets us assert
//! the on-the-wire JSON-RPC behaviour (request → response, notification
//! ordering, permission round-trip) without booting a real LibreFang
//! kernel or spawning a real LLM provider.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::{
    ContentBlock, CreateTerminalRequest, CreateTerminalResponse, InitializeRequest,
    InitializeResponse, NewSessionRequest, NewSessionResponse, PermissionOptionId, PromptRequest,
    PromptResponse, ProtocolVersion, ReadTextFileRequest, ReadTextFileResponse,
    ReleaseTerminalRequest, ReleaseTerminalResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, StopReason, TerminalExitStatus, TerminalId,
    TerminalOutputRequest, TerminalOutputResponse, TextContent, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse,
};
use agent_client_protocol::{ConnectionTo, JsonRpcResponse, Responder, SentRequest};
use async_trait::async_trait;
use librefang_acp::TerminalClientHandle;
use librefang_acp::{AcpKernel, AcpResult, FsClientHandle};
use librefang_llm_driver::StreamEvent;
use librefang_types::agent::{AgentId, SessionId as LfSessionId};
use librefang_types::approval::{ApprovalDecision, ApprovalEvent, ApprovalRequest, RiskLevel};
use librefang_types::message::{StopReason as LfStopReason, TokenUsage};
use tokio::sync::{broadcast, mpsc, Mutex as AsyncMutex};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Mock kernel
// ---------------------------------------------------------------------------

struct MockKernel {
    canned_events: AsyncMutex<Vec<StreamEvent>>,
    approval_tx: broadcast::Sender<ApprovalEvent>,
    resolves: AsyncMutex<Vec<(Uuid, ApprovalDecision)>>,
    last_session_id: AsyncMutex<Option<LfSessionId>>,
    /// Captured at `initialize` time so tests can pull the handle
    /// out and exercise the reverse-RPC against the live connection
    /// directly.
    fs_client: std::sync::Mutex<Option<FsClientHandle>>,
    terminal_client: std::sync::Mutex<Option<TerminalClientHandle>>,
}

impl MockKernel {
    fn new(canned: Vec<StreamEvent>) -> Arc<Self> {
        let (approval_tx, _) = broadcast::channel(16);
        Arc::new(Self {
            canned_events: AsyncMutex::new(canned),
            approval_tx,
            resolves: AsyncMutex::new(Vec::new()),
            last_session_id: AsyncMutex::new(None),
            fs_client: std::sync::Mutex::new(None),
            terminal_client: std::sync::Mutex::new(None),
        })
    }

    fn fs_client_handle(&self) -> Option<FsClientHandle> {
        self.fs_client.lock().ok().and_then(|g| g.clone())
    }

    fn terminal_client_handle(&self) -> Option<TerminalClientHandle> {
        self.terminal_client.lock().ok().and_then(|g| g.clone())
    }

    /// Inject an approval into the broadcast as if the kernel had just
    /// queued one. The bridge should pick it up and dispatch a
    /// `session/request_permission` to the connected client.
    fn fire_approval(&self, lf_session_id: LfSessionId) -> Uuid {
        let id = Uuid::new_v4();
        let req = ApprovalRequest {
            id,
            agent_id: "test-agent".to_string(),
            tool_name: "bash".to_string(),
            description: "execute shell command".to_string(),
            action_summary: "ls /tmp".to_string(),
            risk_level: RiskLevel::Medium,
            requested_at: chrono::Utc::now(),
            timeout_secs: 60,
            sender_id: None,
            channel: None,
            route_to: vec![],
            escalation_count: 0,
            session_id: Some(lf_session_id.0.to_string()),
            // Pin a non-None tool_use_id so the bridge exercises the
            // primary path (use the LLM-assigned id as ToolCallId)
            // rather than the `approval-{req_id}` fallback. The
            // round-trip assertions don't check the id, so it stays
            // synthetic.
            tool_use_id: Some("toolu_acp_integration_test".into()),
        };
        let _ = self.approval_tx.send(ApprovalEvent::Created(req));
        id
    }
}

#[async_trait]
impl AcpKernel for MockKernel {
    async fn resolve_agent(&self, _name_or_id: &str) -> AcpResult<AgentId> {
        Ok(AgentId(Uuid::nil()))
    }

    async fn send_prompt(
        &self,
        _agent_id: AgentId,
        _message: String,
        librefang_session_id: LfSessionId,
    ) -> AcpResult<mpsc::Receiver<StreamEvent>> {
        *self.last_session_id.lock().await = Some(librefang_session_id);
        let evs = std::mem::take(&mut *self.canned_events.lock().await);
        let (tx, rx) = mpsc::channel(16);
        tokio::spawn(async move {
            for ev in evs {
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    fn subscribe_approvals(&self) -> broadcast::Receiver<ApprovalEvent> {
        self.approval_tx.subscribe()
    }

    async fn resolve_approval(
        &self,
        request_id: Uuid,
        decision: ApprovalDecision,
        _decided_by: Option<String>,
    ) -> AcpResult<()> {
        self.resolves.lock().await.push((request_id, decision));
        Ok(())
    }

    fn set_fs_client(&self, handle: FsClientHandle) {
        if let Ok(mut guard) = self.fs_client.lock() {
            *guard = Some(handle);
        }
    }

    fn set_terminal_client(&self, handle: TerminalClientHandle) {
        if let Ok(mut guard) = self.terminal_client.lock() {
            *guard = Some(handle);
        }
    }
}

// ---------------------------------------------------------------------------
// Test harness
// ---------------------------------------------------------------------------

async fn recv<T: JsonRpcResponse + Send + 'static>(
    sent: SentRequest<T>,
) -> Result<T, agent_client_protocol::Error> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    sent.on_receiving_result(async move |result| {
        tx.send(result)
            .map_err(|_| agent_client_protocol::Error::internal_error())
    })?;
    rx.await
        .map_err(|_| agent_client_protocol::Error::internal_error())?
}

/// Build a duplex stream pair suitable for ACP framed JSON-RPC.
fn duplex_pair() -> (
    impl futures::AsyncRead + Send + 'static,
    impl futures::AsyncWrite + Send + 'static,
    impl futures::AsyncRead + Send + 'static,
    impl futures::AsyncWrite + Send + 'static,
) {
    let (a, b) = tokio::io::duplex(8192);
    let (c, d) = tokio::io::duplex(8192);
    // server reads from `a`, writes to `d`; client reads from `c`, writes to `b`
    (a.compat(), d.compat_write(), c.compat(), b.compat_write())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn initialize_and_prompt_emits_text_chunks_and_end_turn() {
    use tokio::task::LocalSet;

    let local = LocalSet::new();
    local
        .run_until(async {
            let kernel = MockKernel::new(vec![
                StreamEvent::TextDelta {
                    text: "Hello".into(),
                },
                StreamEvent::TextDelta {
                    text: " world".into(),
                },
                StreamEvent::ContentComplete {
                    stop_reason: LfStopReason::EndTurn,
                    usage: TokenUsage::default(),
                },
            ]);

            let (server_reader, server_writer, client_reader, client_writer) = duplex_pair();
            let server_transport =
                agent_client_protocol::ByteStreams::new(server_writer, server_reader);
            let client_transport =
                agent_client_protocol::ByteStreams::new(client_writer, client_reader);

            tokio::task::spawn_local(async move {
                let _ = librefang_acp::run_with_transport(
                    kernel.clone(),
                    AgentId(Uuid::nil()),
                    server_transport,
                )
                .await;
            });

            let updates: Arc<AsyncMutex<Vec<SessionNotification>>> =
                Arc::new(AsyncMutex::new(Vec::new()));
            let updates_capture = updates.clone();

            let client = agent_client_protocol::Client.builder().on_receive_notification(
                async move |notif: SessionNotification, _cx| {
                    updates_capture.lock().await.push(notif);
                    Ok(())
                },
                agent_client_protocol::on_receive_notification!(),
            );

            let result = client
                .connect_with(client_transport, async |cx: ConnectionTo<agent_client_protocol::Agent>| -> Result<(), agent_client_protocol::Error> {
                    let init: InitializeResponse =
                        recv(cx.send_request(InitializeRequest::new(ProtocolVersion::LATEST)))
                            .await?;
                    assert!(
                        init.agent_info.as_ref().map(|i| i.name.as_str()) == Some("librefang"),
                        "agent_info should advertise 'librefang', got {:?}",
                        init.agent_info
                    );

                    let new_resp: NewSessionResponse =
                        recv(cx.send_request(NewSessionRequest::new(PathBuf::from("/tmp/proj"))))
                            .await?;
                    let session_id = new_resp.session_id;

                    let prompt_resp: PromptResponse = recv(cx.send_request(PromptRequest::new(
                        session_id.clone(),
                        vec![ContentBlock::Text(TextContent::new("hi"))],
                    )))
                    .await?;
                    assert_eq!(prompt_resp.stop_reason, StopReason::EndTurn);

                    // Give the connection a moment to flush queued notifications.
                    tokio::time::sleep(Duration::from_millis(50)).await;

                    let captured = updates.lock().await;
                    let texts: Vec<String> = captured
                        .iter()
                        .filter_map(|n| match &n.update {
                            SessionUpdate::AgentMessageChunk(chunk) => match &chunk.content {
                                ContentBlock::Text(t) => Some(t.text.clone()),
                                _ => None,
                            },
                            _ => None,
                        })
                        .collect();
                    assert_eq!(texts, vec!["Hello".to_string(), " world".to_string()]);
                    Ok(())
                })
                .await;

            assert!(result.is_ok(), "client driver failed: {result:?}");
        })
        .await;
}

#[tokio::test(flavor = "current_thread")]
async fn permission_round_trip_resolves_kernel_approval() {
    use tokio::task::LocalSet;

    let local = LocalSet::new();
    local
        .run_until(async {
            // Canned events: empty — we just need a session/prompt that
            // hangs long enough for the approval round-trip.
            let kernel = MockKernel::new(vec![StreamEvent::ContentComplete {
                stop_reason: LfStopReason::EndTurn,
                usage: TokenUsage::default(),
            }]);
            let (server_reader, server_writer, client_reader, client_writer) = duplex_pair();
            let server_transport =
                agent_client_protocol::ByteStreams::new(server_writer, server_reader);
            let client_transport =
                agent_client_protocol::ByteStreams::new(client_writer, client_reader);

            let kernel_for_server = kernel.clone();
            tokio::task::spawn_local(async move {
                let _ = librefang_acp::run_with_transport(
                    kernel_for_server,
                    AgentId(Uuid::nil()),
                    server_transport,
                )
                .await;
            });

            let client = agent_client_protocol::Client
                .builder()
                .on_receive_request(
                    async move |req: RequestPermissionRequest, responder: Responder<RequestPermissionResponse>, _cx| {
                        // Always pick allow_once — the test asserts the
                        // kernel sees `Approved` with the right uuid.
                        let outcome = RequestPermissionOutcome::Selected(
                            SelectedPermissionOutcome::new(PermissionOptionId::new("allow_once")),
                        );
                        // Sanity check the request shape.
                        assert_eq!(req.options.len(), 4);
                        responder.respond(RequestPermissionResponse::new(outcome))
                    },
                    agent_client_protocol::on_receive_request!(),
                );

            let kernel_for_driver = kernel.clone();
            let result = client
                .connect_with(client_transport, async move |cx: ConnectionTo<agent_client_protocol::Agent>| -> Result<(), agent_client_protocol::Error> {
                    let _: InitializeResponse =
                        recv(cx.send_request(InitializeRequest::new(ProtocolVersion::LATEST)))
                            .await?;
                    let new_resp: NewSessionResponse =
                        recv(cx.send_request(NewSessionRequest::new(PathBuf::from("/tmp/proj"))))
                            .await?;

                    // Kick off a prompt — the pump will keep the bridge
                    // active long enough for the approval to round-trip.
                    let prompt_handle = tokio::task::spawn_local({
                        let cx = cx.clone();
                        let session_id = new_resp.session_id.clone();
                        async move {
                            recv(cx.send_request(PromptRequest::new(
                                session_id,
                                vec![ContentBlock::Text(TextContent::new("trigger"))],
                            )))
                            .await
                        }
                    });

                    // Wait for the kernel adapter to record the
                    // librefang_session_id mapped to this ACP session.
                    let lf_id = wait_for_session_id(&kernel_for_driver).await;

                    // Fire an approval into the broadcast.
                    let req_id = kernel_for_driver.fire_approval(lf_id);

                    // Wait for the bridge to call resolve_approval.
                    let resolved = wait_for_resolve(&kernel_for_driver, req_id).await;
                    assert_eq!(resolved, ApprovalDecision::Approved);

                    // The prompt eventually returns end_turn.
                    let _ = prompt_handle.await.unwrap()?;
                    Ok(())
                })
                .await;

            assert!(result.is_ok(), "client driver failed: {result:?}");
        })
        .await;
}

async fn wait_for_session_id(kernel: &MockKernel) -> LfSessionId {
    for _ in 0..40 {
        if let Some(id) = *kernel.last_session_id.lock().await {
            return id;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("kernel never received send_prompt");
}

async fn wait_for_resolve(kernel: &MockKernel, req_id: Uuid) -> ApprovalDecision {
    for _ in 0..40 {
        let guard = kernel.resolves.lock().await;
        if let Some((_, d)) = guard.iter().find(|(id, _)| *id == req_id) {
            return d.clone();
        }
        drop(guard);
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("bridge never resolved approval {req_id}");
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_session_id_returns_invalid_params() {
    use tokio::task::LocalSet;

    let local = LocalSet::new();
    local
        .run_until(async {
            let kernel = MockKernel::new(vec![]);
            let (server_reader, server_writer, client_reader, client_writer) = duplex_pair();
            let server_transport =
                agent_client_protocol::ByteStreams::new(server_writer, server_reader);
            let client_transport =
                agent_client_protocol::ByteStreams::new(client_writer, client_reader);

            tokio::task::spawn_local(async move {
                let _ = librefang_acp::run_with_transport(
                    kernel.clone(),
                    AgentId(Uuid::nil()),
                    server_transport,
                )
                .await;
            });

            let client = agent_client_protocol::Client.builder();
            let result = client
                .connect_with(client_transport, async |cx: ConnectionTo<agent_client_protocol::Agent>| -> Result<(), agent_client_protocol::Error> {
                    let _: InitializeResponse =
                        recv(cx.send_request(InitializeRequest::new(ProtocolVersion::LATEST)))
                            .await?;

                    let bogus = agent_client_protocol::schema::SessionId::new("does-not-exist");
                    let prompt_result = recv(cx.send_request(PromptRequest::new(
                        bogus,
                        vec![ContentBlock::Text(TextContent::new("hi"))],
                    )))
                    .await;
                    assert!(
                        prompt_result.is_err(),
                        "prompt against unknown session should error"
                    );
                    Ok(())
                })
                .await;
            assert!(result.is_ok(), "client driver failed: {result:?}");
        })
        .await;
}

/// `fs/read_text_file` round-trip: the server-side `FsClientHandle`
/// (set on the mock kernel at `initialize` time) issues a
/// `fs/read_text_file` request; the client builder's
/// `on_receive_request` handler answers with a canned body; the
/// kernel-side helper returns it back to the test driver. Verifies
/// the reverse-RPC path end-to-end without booting a real LibreFang
/// kernel.
#[tokio::test(flavor = "current_thread")]
async fn fs_read_text_file_round_trip() {
    use tokio::task::LocalSet;

    let local = LocalSet::new();
    local
        .run_until(async {
            let kernel = MockKernel::new(vec![]);
            let (server_reader, server_writer, client_reader, client_writer) = duplex_pair();
            let server_transport =
                agent_client_protocol::ByteStreams::new(server_writer, server_reader);
            let client_transport =
                agent_client_protocol::ByteStreams::new(client_writer, client_reader);

            let kernel_for_server = kernel.clone();
            tokio::task::spawn_local(async move {
                let _ = librefang_acp::run_with_transport(
                    kernel_for_server,
                    AgentId(Uuid::nil()),
                    server_transport,
                )
                .await;
            });

            // Client side: declare fs.read_text_file capability and
            // answer with a canned body when the server asks for it.
            let client = agent_client_protocol::Client.builder().on_receive_request(
                async move |req: ReadTextFileRequest,
                            responder: Responder<ReadTextFileResponse>,
                            _cx| {
                    assert_eq!(req.path, PathBuf::from("/tmp/hello.txt"));
                    responder.respond(ReadTextFileResponse::new("canned editor content"))
                },
                agent_client_protocol::on_receive_request!(),
            );

            let kernel_for_driver = kernel.clone();
            let result = client
                .connect_with(
                    client_transport,
                    async move |cx: ConnectionTo<agent_client_protocol::Agent>| -> Result<(), agent_client_protocol::Error> {
                        // Initialize with read_text_file capability so the
                        // server-side FsCapabilities snapshot reports it.
                        let mut init_req = InitializeRequest::new(ProtocolVersion::LATEST);
                        init_req.client_capabilities.fs.read_text_file = true;
                        let _: InitializeResponse = recv(cx.send_request(init_req)).await?;

                        // Pull the FsClientHandle the server stashed at
                        // initialize-time and exercise it directly.
                        let handle = poll_for(|| kernel_for_driver.fs_client_handle()).await;
                        assert!(handle.capabilities().read_text_file);
                        let content = handle
                            .read_text_file(
                                agent_client_protocol::schema::SessionId::new("test-session"),
                                PathBuf::from("/tmp/hello.txt"),
                                None,
                                None,
                            )
                            .await
                            .expect("read_text_file should succeed");
                        assert_eq!(content, "canned editor content");
                        Ok(())
                    },
                )
                .await;
            assert!(result.is_ok(), "client driver failed: {result:?}");
        })
        .await;
}

async fn poll_for<T, F: FnMut() -> Option<T>>(mut f: F) -> T {
    for _ in 0..40 {
        if let Some(v) = f() {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("poll_for: condition never satisfied within 1s");
}

/// `terminal/*` round-trip: server-side `TerminalClientHandle` runs
/// the full create→wait_for_exit→output→release dance, and the
/// client builder answers each method with a stub. Verifies the five
/// reverse-RPCs interleave cleanly + the `AcpTerminalRunResult` the
/// runtime sees carries the expected exit code, output, and
/// truncation flag.
#[tokio::test(flavor = "current_thread")]
async fn terminal_run_command_round_trip() {
    use tokio::task::LocalSet;

    let local = LocalSet::new();
    local
        .run_until(async {
            let kernel = MockKernel::new(vec![]);
            let (server_reader, server_writer, client_reader, client_writer) = duplex_pair();
            let server_transport =
                agent_client_protocol::ByteStreams::new(server_writer, server_reader);
            let client_transport =
                agent_client_protocol::ByteStreams::new(client_writer, client_reader);

            let kernel_for_server = kernel.clone();
            tokio::task::spawn_local(async move {
                let _ = librefang_acp::run_with_transport(
                    kernel_for_server,
                    AgentId(Uuid::nil()),
                    server_transport,
                )
                .await;
            });

            // Client side: stub all four `terminal/*` requests the
            // run_command dance issues.
            let client = agent_client_protocol::Client
                .builder()
                .on_receive_request(
                    async move |_req: CreateTerminalRequest,
                                responder: Responder<CreateTerminalResponse>,
                                _cx| {
                        responder.respond(CreateTerminalResponse::new(TerminalId::new("term-1")))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_req: WaitForTerminalExitRequest,
                                responder: Responder<WaitForTerminalExitResponse>,
                                _cx| {
                        let mut exit = TerminalExitStatus::default();
                        exit.exit_code = Some(0);
                        responder.respond(WaitForTerminalExitResponse::new(exit))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_req: TerminalOutputRequest,
                                responder: Responder<TerminalOutputResponse>,
                                _cx| {
                        responder.respond(TerminalOutputResponse::new("hello world\n", false))
                    },
                    agent_client_protocol::on_receive_request!(),
                )
                .on_receive_request(
                    async move |_req: ReleaseTerminalRequest,
                                responder: Responder<ReleaseTerminalResponse>,
                                _cx| { responder.respond(ReleaseTerminalResponse::default()) },
                    agent_client_protocol::on_receive_request!(),
                );

            let kernel_for_driver = kernel.clone();
            let result = client
                .connect_with(
                    client_transport,
                    async move |cx: ConnectionTo<agent_client_protocol::Agent>| -> Result<(), agent_client_protocol::Error> {
                        let mut init_req = InitializeRequest::new(ProtocolVersion::LATEST);
                        init_req.client_capabilities.terminal = true;
                        let _: InitializeResponse = recv(cx.send_request(init_req)).await?;

                        let handle = poll_for(|| kernel_for_driver.terminal_client_handle()).await;
                        assert!(handle.capabilities().terminal);

                        // Drive `run_command` through the
                        // `AcpTerminalClient` trait — the same path the
                        // runtime's `shell_exec` arm uses.
                        use librefang_kernel_handle::AcpTerminalClient;
                        let result = handle
                            .run_command(
                                "echo".to_string(),
                                vec!["hello".to_string()],
                                Vec::new(),
                                None,
                                None,
                            )
                            .await
                            .expect("terminal run_command should succeed");
                        assert_eq!(result.output, "hello world\n");
                        assert!(!result.truncated);
                        assert_eq!(result.exit_code, Some(0));
                        assert_eq!(result.signal, None);
                        Ok(())
                    },
                )
                .await;
            assert!(result.is_ok(), "client driver failed: {result:?}");
        })
        .await;
}
