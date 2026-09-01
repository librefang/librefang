//! Event system: crossterm polling, tick timer, streaming bridges.

use crate::commands::automation::WorkflowRunOutcome;
use librefang_kernel::AgentSubsystemApi;
use librefang_kernel::LibreFangKernel;
use librefang_kernel::McpSubsystemApi;
use librefang_kernel::SkillsSubsystemApi;
use librefang_runtime::agent_loop::AgentLoopResult;
use librefang_runtime::llm_driver::StreamEvent;
use librefang_types::agent::AgentId;
use ratatui::crossterm::event::{self, Event as CtEvent, KeyEvent, KeyEventKind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, OnceLock};
use std::time::Duration;

use super::screens::{
    audit::AuditEntry,
    channels::{ChannelAdapterInfo, ChannelFieldInfo, ChannelInstance, ConfigureRequest},
    dashboard::AuditRow,
    extensions::{ExtensionHealthInfo, ExtensionInfo},
    groups::GroupInfo,
    hands::{HandInfo, HandInstanceInfo},
    logs::LogEntry,
    memory::{AgentEntry, KvPair},
    models::ModelRow,
    peers::PeerInfo,
    security::SecurityFeature,
    sessions::SessionInfo,
    settings::{BackupInfo, ModelInfo, ProviderInfo, TestResult, ToolInfo},
    skills::{ClawHubResult, McpServerInfo, SkillInfo},
    templates::{self, ProviderAuth, TemplateInfo, TemplateSource},
    triggers::TriggerInfo,
    usage::{AgentUsage, ModelUsage, UsageSummary},
    workflows::{WorkflowInfo, WorkflowRun},
};

// ── BackendRef ──────────────────────────────────────────────────────────────

/// Lightweight reference to the active backend, for passing to spawn functions.
#[derive(Clone)]
pub enum BackendRef {
    Daemon {
        base_url: String,
        /// API key for `Authorization: Bearer` auth, if the daemon requires it.
        api_key: Option<String>,
    },
    InProcess(Arc<LibreFangKernel>),
}

// ── AppEvent ────────────────────────────────────────────────────────────────

/// Unified application event.
#[allow(clippy::large_enum_variant)] // AgentLoopResult is inherently large
pub enum AppEvent {
    /// A crossterm key press event (filtered to Press only).
    Key(KeyEvent),
    /// Periodic tick for animations (spinners, etc.).
    Tick,
    /// Terminal was resized to the given (width, height).
    Resize(u16, u16),
    /// Bracketed-paste text received from the terminal.
    Paste(String),
    /// A streaming event from the LLM (daemon SSE or kernel mpsc).
    Stream(StreamEvent),
    /// The streaming agent loop finished.
    StreamDone(Result<AgentLoopResult, String>),
    /// The kernel finished booting in the background.
    KernelReady(Arc<LibreFangKernel>),
    /// The kernel failed to boot.
    KernelError(String),
    /// An agent was successfully spawned (daemon mode).
    AgentSpawned {
        id: String,
        name: String,
    },
    /// Agent spawn failed.
    AgentSpawnError(String),
    /// Daemon detection result from background thread.
    DaemonDetected {
        url: Option<String>,
        agent_count: u64,
    },

    // ── New tab events ──────────────────────────────────────────────────────
    /// Dashboard data loaded.
    DashboardData {
        agent_count: u64,
        uptime_secs: u64,
        version: String,
        provider: String,
        model: String,
    },
    /// Audit trail loaded.
    AuditLoaded(Vec<AuditRow>),
    /// Auto-dream status loaded. Surfaces on the Dashboard's dream strip.
    DreamsLoaded {
        enabled: bool,
        rows: Vec<crate::tui::screens::dashboard::DreamRow>,
    },
    /// Channel instances and the sidecar catalog loaded together — both come
    /// from the single `GET /api/channels` listing, split by `configured`.
    ChannelListLoaded {
        instances: Vec<ChannelInstance>,
        adapters: Vec<ChannelAdapterInfo>,
    },
    /// A `[[sidecar_channels]]` instance was written. Carries the instance
    /// name, not the adapter, because that is what the operator named.
    ChannelInstanceSaved {
        instance_name: String,
        restart_required: bool,
        /// Secret field keys already exported in the daemon's own process
        /// environment. `build_spawn_env` resolves those ahead of the
        /// `secrets.env` line this save just wrote, so the value the operator
        /// typed does not take effect until the variable is unset and the
        /// daemon restarted — a "saved" with no warning would be a lie.
        shadowed_secrets: Vec<String>,
    },
    /// A `[[sidecar_channels]]` instance was removed.
    ChannelInstanceDeleted(String),
    /// A manual channel reload finished, with the number of adapters started.
    ChannelsReloaded(u64),
    /// Workflow list loaded.
    WorkflowListLoaded(Vec<WorkflowInfo>),
    /// Workflow runs loaded for a specific workflow.
    WorkflowRunsLoaded(Vec<WorkflowRun>),
    /// Workflow run completed.
    WorkflowRunResult(String),
    /// Workflow created successfully.
    WorkflowCreated(String),
    /// Trigger list loaded.
    TriggerListLoaded(Vec<TriggerInfo>),
    /// Trigger created.
    TriggerCreated(String),
    /// Trigger deleted.
    TriggerDeleted(String),
    /// Agent killed successfully.
    AgentKilled {
        id: String,
    },
    /// Agent kill failed.
    AgentKillError(String),
    /// Generic fetch error for any tab.
    FetchError(String),

    // ── New screen events ──────────────────────────────────────────────────
    /// Sessions loaded.
    SessionsLoaded(Vec<SessionInfo>),
    /// Session deleted.
    SessionDeleted(String),
    /// Memory agents loaded (for agent selector).
    MemoryAgentsLoaded(Vec<AgentEntry>),
    MemoryConfigLoaded(crate::tui::screens::memory::MemoryConfigView),
    /// Memory KV pairs loaded.
    MemoryKvLoaded(Vec<KvPair>),
    /// Memory KV saved.
    MemoryKvSaved {
        key: String,
    },
    /// Memory KV deleted.
    MemoryKvDeleted(String),
    /// Skills loaded.
    SkillsLoaded(Vec<SkillInfo>),
    /// ClawHub results loaded.
    ClawHubLoaded(Vec<ClawHubResult>),
    /// Skill installed.
    SkillInstalled(String),
    /// Skill uninstalled.
    SkillUninstalled(String),
    /// MCP servers loaded.
    McpServersLoaded(Vec<McpServerInfo>),
    /// Templates providers loaded (auth status).
    TemplateProvidersLoaded(Vec<ProviderAuth>),
    /// Manifest-backed agent types from `GET /api/templates` (#7760).
    AgentTemplatesLoaded(Vec<TemplateInfo>),
    /// The verbatim `agent.toml` for one manifest-backed agent type.
    /// `None` means it could not be read; the screen reports that rather than spawning something it made up.
    TemplateTomlLoaded {
        name: String,
        toml: Option<String>,
    },
    /// Security features loaded.
    SecurityLoaded(Vec<SecurityFeature>),
    /// Security chain verification result.
    SecurityChainVerified {
        valid: bool,
        message: String,
    },
    /// Audit entries loaded (full audit screen).
    AuditEntriesLoaded(Vec<AuditEntry>),
    /// Audit chain verified.
    AuditChainVerified(bool),
    /// Usage summary loaded.
    UsageSummaryLoaded(UsageSummary),
    /// Usage by model loaded.
    UsageByModelLoaded(Vec<ModelUsage>),
    /// Usage by agent loaded.
    UsageByAgentLoaded(Vec<AgentUsage>),
    /// Settings providers loaded.
    SettingsProvidersLoaded(Vec<ProviderInfo>),
    /// Settings models loaded.
    SettingsModelsLoaded(Vec<ModelInfo>),
    /// Settings tools loaded.
    SettingsToolsLoaded(Vec<ToolInfo>),
    /// Provider key saved.
    ProviderKeySaved(String),
    /// Provider key deleted.
    ProviderKeyDeleted(String),
    /// Provider test result.
    ProviderTestResult(TestResult),
    /// Model catalogue loaded for the Models screen (refs #7774).
    ModelCatalogLoaded(Vec<ModelRow>),
    /// One model's operator capacity limits were persisted; carries the
    /// `provider:model_id` override key.
    ModelLimitsSaved(String),
    /// One model's operator capacity limits were dropped back to the catalog.
    ModelLimitsReset(String),
    /// Backup archives listed.
    BackupsLoaded(Vec<BackupInfo>),
    /// A new archive was written.
    BackupCreated(String),
    /// An archive was deleted.
    BackupDeleted(String),
    /// A restore finished. `errors` counts entries the daemon could not write.
    BackupRestored {
        filename: String,
        restored_files: u64,
        errors: usize,
    },
    /// Peers loaded.
    PeersLoaded(Vec<PeerInfo>),
    /// User groups loaded (#7745).
    GroupsLoaded(Vec<GroupInfo>),
    /// Log entries loaded.
    LogsLoaded(Vec<LogEntry>),
    /// Hand definitions loaded (marketplace).
    HandsLoaded(Vec<HandInfo>),
    /// Active hand instances loaded.
    ActiveHandsLoaded(Vec<HandInstanceInfo>),
    /// Hand activated.
    HandActivated(String),
    /// Hand deactivated.
    HandDeactivated(String),
    /// Hand paused.
    HandPaused(String),
    /// Hand resumed.
    HandResumed(String),
    /// Extensions loaded (available + installed).
    ExtensionsLoaded(Vec<ExtensionInfo>),
    /// Extension health loaded.
    ExtensionHealthLoaded(Vec<ExtensionHealthInfo>),
    /// Extension installed.
    ExtensionInstalled(String),
    /// Extension removed.
    ExtensionRemoved(String),
    /// Extension reconnected.
    ExtensionReconnected(String, usize),
    /// Agent skills loaded (for edit screen).
    AgentSkillsLoaded {
        assigned: Vec<String>,
        available: Vec<String>,
    },
    /// Agent MCP servers loaded (for edit screen).
    AgentMcpServersLoaded {
        assigned: Vec<String>,
        available: Vec<String>,
    },
    /// Agent skills updated.
    AgentSkillsUpdated(String),
    /// Agent MCP servers updated.
    AgentMcpServersUpdated(String),
    /// Agent channel allowlist loaded (for edit screen).
    AgentChannelsLoaded {
        assigned: Vec<String>,
        available: Vec<String>,
    },
    /// Agent channel allowlist updated.
    AgentChannelsUpdated(String),
    /// Comms topology loaded.
    CommsTopologyLoaded {
        nodes: Vec<super::screens::comms::CommsNode>,
        edges: Vec<super::screens::comms::CommsEdge>,
    },
    /// Comms events loaded.
    CommsEventsLoaded(Vec<super::screens::comms::CommsEventItem>),
    /// Comms send result.
    CommsSendResult(String),
    /// Comms task post result.
    CommsTaskResult(String),

    // ── Async chat helpers (previously blocking on the event-loop thread) ──
    /// Agent model label fetched for chat header.
    ChatModelLabelLoaded {
        agent_id: String,
        label: String,
    },
    /// Model list loaded for the model picker in chat.
    ChatModelsForPicker(Vec<super::screens::chat::ModelEntry>),
    /// Agent list loaded for the /agents chat command.
    ChatAgentListLoaded(Vec<String>),
}

/// Spawn the crossterm polling + tick thread. Returns sender + receiver.
pub fn spawn_event_thread(
    tick_rate: Duration,
) -> (mpsc::Sender<AppEvent>, mpsc::Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel();
    let poll_tx = tx.clone();

    std::thread::spawn(move || {
        loop {
            if event::poll(tick_rate).unwrap_or(false) {
                if let Ok(ev) = event::read() {
                    let sent = match ev {
                        // CRITICAL: only forward Press events — Windows sends
                        // Release and Repeat too, which causes double/triple input
                        CtEvent::Key(key) if key.kind == KeyEventKind::Press => {
                            poll_tx.send(AppEvent::Key(key))
                        }
                        CtEvent::Resize(w, h) => poll_tx.send(AppEvent::Resize(w, h)),
                        CtEvent::Paste(text) => poll_tx.send(AppEvent::Paste(text)),
                        _ => Ok(()),
                    };
                    if sent.is_err() {
                        break;
                    }
                }
            } else {
                // No event within tick_rate → send tick for spinner animations
                if poll_tx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    (tx, rx)
}

// ── Stream cancellation ─────────────────────────────────────────────────────

/// A cancellation token for a background SSE/in-process stream thread.
///
/// Setting the flag to `true` causes the thread to stop reading and exit on
/// its next iteration. Dropping the token without cancelling is a no-op.
#[derive(Clone)]
pub struct StreamCancelToken(Arc<AtomicBool>);

impl StreamCancelToken {
    fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    /// Signal the stream thread to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

// ── Original spawn functions ────────────────────────────────────────────────

/// Detect daemon in a background thread (non-blocking).
pub fn spawn_daemon_detect(tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let url = crate::find_daemon();
        let mut agent_count = 0u64;

        if let Some(ref u) = url {
            if let Ok(client) = crate::http_client::client_builder()
                .timeout(Duration::from_secs(2))
                .build()
            {
                if let Ok(resp) = client.get(format!("{u}/api/status")).send() {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        agent_count = body["agent_count"].as_u64().unwrap_or(0);
                    }
                }
            }
        }

        let _ = tx.send(AppEvent::DaemonDetected { url, agent_count });
    });
}

/// Process-lifetime tokio runtime for the TUI's long-lived background tasks.
///
/// The TUI's `main()` is plain `fn main()` and its event loop is synchronous, so async work is normally done on throwaway per-operation runtimes.
/// That does not work for the kernel's `spawn_*` sweep loops: they must be spawned from a runtime context (`Handle::current()`) **and** the runtime has to outlive the spawn call, otherwise the loop is aborted the moment the throwaway runtime drops.
///
/// The `OnceLock` is never dropped (statics aren't), so tasks spawned onto this handle live until the process exits.
fn tui_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("librefang-tui-bg")
            // Same 2 MiB -> 8 MiB worker stack rationale as the daemon and the desktop embedded server (refs #6659): the TUI's in-process mode (`InProcess(Arc<LibreFangKernel>)`) runs the same `agent_send` / workflow-step turn chain as those, so it can restack the same fat frames.
            .thread_stack_size(8 * 1024 * 1024)
            .build()
            .expect("Failed to create Tokio runtime")
    })
}

/// Run `fut` to completion on the shared TUI runtime.
///
/// The replacement for the per-operation `Runtime::new()` the TUI used to build.
/// Besides the obvious cost of standing up a thread pool per click, a throwaway runtime **aborts every task detached during the call** the moment it drops — `reset_session` spawns the session-summary write that way, so `/new` was silently dropping summaries.
/// Running on a runtime that outlives the call lets those tasks finish.
///
/// # Panics
///
/// `block_on` panics when called from a thread that is already driving tokio tasks ("Cannot start a runtime from within a runtime").
/// Every TUI caller is either the main thread or a `std::thread::spawn`'d worker, neither of which is a runtime worker thread, so this is safe here — but do not call it from inside an async task.
pub(crate) fn block_on_tui<F: std::future::Future>(fut: F) -> F::Output {
    tui_runtime().block_on(fut)
}

/// Spawn the kernel's long-lived background sweep loops onto the TUI runtime.
///
/// Call this from the sync TUI event loop once the kernel is ready.
/// The `EnterGuard` is scoped to this function because that is all that needs a runtime context — the sweep loop itself runs on the runtime's own worker threads once spawned.
pub fn spawn_kernel_background_tasks(kernel: Arc<LibreFangKernel>) {
    let _guard = tui_runtime().enter();
    kernel.spawn_approval_sweep_task();
}

/// Spawn a background thread that boots the kernel.
pub fn spawn_kernel_boot(config: Option<std::path::PathBuf>, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        // Boot inside the process-lifetime runtime's context so any tokio::spawn calls during boot (e.g. publish_event via set_self_handle) find the reactor *and* survive past this thread — a throwaway runtime here would abort them on drop.
        let _guard = tui_runtime().enter();

        match LibreFangKernel::boot(config.as_deref()) {
            Ok(k) => {
                let k = Arc::new(k);
                k.set_self_handle();
                let _ = tx.send(AppEvent::KernelReady(k));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::KernelError(format!("{e}")));
            }
        }
    });
}

/// Spawn a background thread for in-process streaming.
///
/// Returns a [`StreamCancelToken`] that the caller can use to abort the stream
/// when the user navigates away or starts a new chat session.
pub fn spawn_inprocess_stream(
    kernel: Arc<LibreFangKernel>,
    agent_id: AgentId,
    message: String,
    tx: mpsc::Sender<AppEvent>,
) -> StreamCancelToken {
    let token = StreamCancelToken::new();
    let cancel = token.clone();
    std::thread::Builder::new()
        .name("librefang-tui-stream".into())
        // This thread runs `block_on_tui`, so the outermost agent turn is polled on *its* stack, not on `tui_runtime()`'s worker threads -- `thread_stack_size` on that runtime's builder does not cover it, same gap the desktop server thread had (refs #6659).
        // `send_message_streaming_with_routing` is the same `agent_send` / workflow-step turn chain the daemon and desktop size for, so give this thread the same headroom.
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            // The shared runtime, not a throwaway: the agent loop detaches tasks (tool executions, memory writes) that must outlive this turn.
            match block_on_tui(kernel.send_message_streaming_with_routing(agent_id, &message, None))
            {
                Ok((mut rx, handle)) => {
                    block_on_tui(async {
                        while let Some(ev) = rx.recv().await {
                            if cancel.is_cancelled() {
                                break;
                            }
                            if tx.send(AppEvent::Stream(ev)).is_err() {
                                return;
                            }
                        }
                        if cancel.is_cancelled() {
                            return;
                        }
                        let result = handle
                            .await
                            .map_err(|e| e.to_string())
                            .and_then(|r| r.map_err(|e| e.to_string()));
                        let _ = tx.send(AppEvent::StreamDone(result));
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::StreamDone(Err(format!("{e}"))));
                }
            }
        })
        .expect("failed to spawn librefang-tui-stream thread");
    token
}

/// Spawn a background thread for daemon SSE streaming.
///
/// Returns a [`StreamCancelToken`] that the caller can use to abort the stream
/// when the user navigates away or starts a new chat session.
pub fn spawn_daemon_stream(
    base_url: String,
    agent_id: String,
    message: String,
    api_key: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) -> StreamCancelToken {
    let token = StreamCancelToken::new();
    let cancel = token.clone();
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Read};

        let client = make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(300));

        let url = format!("{base_url}/api/agents/{agent_id}/message/stream");
        let resp = client
            .post(&url)
            .json(&serde_json::json!({"message": message}))
            .send();

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(_) => {
                let fallback = daemon_fallback(&base_url, &agent_id, &message, api_key.as_deref());
                let _ = tx.send(AppEvent::StreamDone(fallback));
                return;
            }
            Err(e) => {
                let _ = tx.send(AppEvent::StreamDone(Err(crate::i18n::t_args(
                    "tui-event-stream-connection-failed",
                    &[("error", &e.to_string())],
                ))));
                return;
            }
        };

        struct RespReader(reqwest::blocking::Response);
        impl Read for RespReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(buf)
            }
        }

        // Accumulate usage across all iterations (tool-use loops send
        // multiple ContentComplete events — one per LLM call).  Do NOT
        // return early on "done": true — the SSE stream continues until
        // the server closes the connection after the agent loop finishes.
        let mut total_input_tokens: u64 = 0;
        let mut total_output_tokens: u64 = 0;

        let reader = BufReader::new(RespReader(resp));
        for line in reader.lines() {
            if cancel.is_cancelled() {
                return;
            }
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            if line.is_empty() || line.starts_with("event:") {
                continue;
            }
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = json.get("content").and_then(|c| c.as_str()) {
                        let _ = tx.send(AppEvent::Stream(StreamEvent::TextDelta {
                            text: content.to_string(),
                        }));
                    }
                    if let Some(tool) = json.get("tool").and_then(|t| t.as_str()) {
                        if json.get("input").is_none() {
                            let _ = tx.send(AppEvent::Stream(StreamEvent::ToolUseStart {
                                id: String::new(),
                                name: tool.to_string(),
                            }));
                        } else {
                            let _ = tx.send(AppEvent::Stream(StreamEvent::ToolUseEnd {
                                id: String::new(),
                                name: tool.to_string(),
                                input: json["input"].clone(),
                            }));
                        }
                    }
                    if json.get("done").and_then(|d| d.as_bool()) == Some(true) {
                        let usage = json.get("usage").cloned().unwrap_or_default();
                        total_input_tokens += usage
                            .get("input_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        total_output_tokens += usage
                            .get("output_tokens")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0);
                        // Forward as ContentComplete so the UI can update
                        // token display, but do NOT terminate — the agent
                        // loop may continue with tool results.
                        let _ = tx.send(AppEvent::Stream(StreamEvent::ContentComplete {
                            stop_reason: librefang_types::message::StopReason::EndTurn,
                            usage: librefang_types::message::TokenUsage {
                                input_tokens: total_input_tokens,
                                output_tokens: total_output_tokens,
                                ..Default::default()
                            },
                        }));
                    }
                }
            }
        }

        // Connection closed — agent loop is truly done.
        let _ = tx.send(AppEvent::StreamDone(Ok(AgentLoopResult {
            response: String::new(),
            total_usage: librefang_types::message::TokenUsage {
                input_tokens: total_input_tokens,
                output_tokens: total_output_tokens,
                ..Default::default()
            },
            iterations: 0,
            cost_usd: None,
            silent: false,
            directives: Default::default(),
            decision_traces: Vec::new(),
            memories_saved: Vec::new(),
            memories_used: Vec::new(),
            memory_conflicts: Vec::new(),
            provider_not_configured: false,
            experiment_context: None,
            latency_ms: 0,
            // TUI doesn't use the session-slice index; N/A.
            new_messages_start: 0,
            skill_evolution_suggested: false,
            owner_notice: None,
            // The TUI streams over the daemon's SSE bridge, which does
            // not surface the fallback-chain provider tag. Metering on
            // the daemon side has already billed against the right
            // provider; no value to forward here.
            actual_provider: None,
            actual_model: None,
        })));
    });
    token
}

/// Blocking fallback for daemon chat (non-streaming).
fn daemon_fallback(
    base_url: &str,
    agent_id: &str,
    message: &str,
    api_key: Option<&str>,
) -> Result<AgentLoopResult, String> {
    let client = make_daemon_client_with_timeout(api_key, Duration::from_secs(120));

    let resp = client
        .post(format!("{base_url}/api/agents/{agent_id}/message"))
        .json(&serde_json::json!({"message": message}))
        .send()
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().map_err(|e| e.to_string())?;

    if let Some(response) = body.get("response").and_then(|r| r.as_str()) {
        let input_tokens = body["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = body["output_tokens"].as_u64().unwrap_or(0);
        Ok(AgentLoopResult {
            response: response.to_string(),
            total_usage: librefang_types::message::TokenUsage {
                input_tokens,
                output_tokens,
                ..Default::default()
            },
            iterations: body["iterations"].as_u64().unwrap_or(0) as u32,
            cost_usd: body["cost_usd"].as_f64(),
            silent: false,
            directives: Default::default(),
            decision_traces: Vec::new(),
            memories_saved: Vec::new(),
            memories_used: Vec::new(),
            memory_conflicts: Vec::new(),
            provider_not_configured: false,
            experiment_context: None,
            latency_ms: 0,
            // TUI doesn't use the session-slice index; N/A.
            new_messages_start: 0,
            skill_evolution_suggested: false,
            owner_notice: None,
            // Daemon's `POST /agents/<id>/message` JSON shape does not
            // include the fallback-chain provider tag. Metering on the
            // daemon side has already billed against the right
            // provider; no value to forward here.
            actual_provider: None,
            actual_model: None,
        })
    } else {
        Err(body["error"]
            .as_str()
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// Spawn a background thread that spawns an agent on the daemon.
pub fn spawn_daemon_agent(
    base_url: String,
    toml_content: String,
    api_key: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let client = make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(30));

        let resp = client
            .post(format!("{base_url}/api/agents"))
            .json(&serde_json::json!({"manifest_toml": toml_content}))
            .send();

        match resp {
            Ok(r) => {
                let body: serde_json::Value = r.json().unwrap_or_default();
                if let Some(id) = body.get("agent_id").and_then(|v| v.as_str()) {
                    let name = body["name"].as_str().unwrap_or("agent").to_string();
                    let _ = tx.send(AppEvent::AgentSpawned {
                        id: id.to_string(),
                        name,
                    });
                } else {
                    let _ = tx.send(AppEvent::AgentSpawnError(
                        body["error"]
                            .as_str()
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| {
                                crate::i18n::t("tui-event-agent-spawn-failed-fallback")
                            }),
                    ));
                }
            }
            Err(e) => {
                let _ = tx.send(AppEvent::AgentSpawnError(format!("{e}")));
            }
        }
    });
}

// ── New spawn functions for tabs ────────────────────────────────────────────

/// Fetch dashboard data in background.
pub fn spawn_fetch_dashboard(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());

            if let Ok(resp) = client.get(format!("{base_url}/api/status")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let _ = tx.send(AppEvent::DashboardData {
                        agent_count: body["agent_count"].as_u64().unwrap_or(0),
                        uptime_secs: body["uptime_secs"].as_u64().unwrap_or(0),
                        version: body["version"].as_str().unwrap_or("?").to_string(),
                        provider: body["provider"].as_str().unwrap_or("").to_string(),
                        model: body["model"].as_str().unwrap_or("").to_string(),
                    });
                }
            }

            // Try to fetch audit trail
            if let Ok(resp) = client.get(format!("{base_url}/api/audit/recent")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let rows: Vec<AuditRow> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|r| AuditRow {
                                    timestamp: r["timestamp"].as_str().unwrap_or("").to_string(),
                                    agent: r["agent"].as_str().unwrap_or("").to_string(),
                                    action: r["action"].as_str().unwrap_or("").to_string(),
                                    detail: r["detail"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AuditLoaded(rows));
                }
            }

            // Try to fetch auto-dream status. Silent skip on any failure —
            // this endpoint is optional and the dashboard should keep
            // working if auto-dream is not wired up.
            if let Ok(resp) = client
                .get(format!("{base_url}/api/auto-dream/status"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let enabled = body
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let rows: Vec<crate::tui::screens::dashboard::DreamRow> = body
                        .get("agents")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|a| {
                                    let progress = a.get("progress")?;
                                    if progress.is_null() {
                                        return None;
                                    }
                                    Some(crate::tui::screens::dashboard::DreamRow {
                                        agent_name: a
                                            .get("agent_name")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?")
                                            .to_string(),
                                        status: progress
                                            .get("status")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("?")
                                            .to_string(),
                                        phase: progress
                                            .get("phase")
                                            .and_then(|v| v.as_str())
                                            .unwrap_or("")
                                            .to_string(),
                                        memories_touched: progress
                                            .get("memories_touched")
                                            .and_then(|v| v.as_array())
                                            .map(|a| a.len() as u32)
                                            .unwrap_or(0),
                                        tool_use_count: progress
                                            .get("tool_use_count")
                                            .and_then(|v| v.as_u64())
                                            .unwrap_or(0)
                                            as u32,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::DreamsLoaded { enabled, rows });
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let count = kernel.agent_registry_ref().count() as u64;
            let _ = tx.send(AppEvent::DashboardData {
                agent_count: count,
                uptime_secs: 0,
                version: env!("CARGO_PKG_VERSION").to_string(),
                provider: String::new(),
                model: String::new(),
            });
            // In-process mode doesn't have a REST audit endpoint yet
            let _ = tx.send(AppEvent::AuditLoaded(Vec::new()));

            // Pull auto-dream status directly off the kernel.
            // Without this the DREAMS strip never receives data in standalone TUI mode (no daemon), even though the local kernel's dream flow is fully active.
            // `current_status` is async, so drive it on the shared TUI runtime.
            let status = block_on_tui(librefang_kernel::auto_dream::current_status(&kernel));
            let rows: Vec<crate::tui::screens::dashboard::DreamRow> = status
                .agents
                .iter()
                .filter_map(|a| {
                    let progress = a.progress.as_ref()?;
                    let status_str = match progress.status {
                        librefang_kernel::auto_dream::DreamStatus::Running => "running",
                        librefang_kernel::auto_dream::DreamStatus::Completed => "completed",
                        librefang_kernel::auto_dream::DreamStatus::Failed => "failed",
                        librefang_kernel::auto_dream::DreamStatus::Aborted => "aborted",
                    };
                    Some(crate::tui::screens::dashboard::DreamRow {
                        agent_name: a.agent_name.clone(),
                        status: status_str.to_string(),
                        phase: progress.phase.clone(),
                        memories_touched: progress.memories_touched.len() as u32,
                        tool_use_count: progress.tool_use_count,
                    })
                })
                .collect();
            let _ = tx.send(AppEvent::DreamsLoaded {
                enabled: status.enabled,
                rows,
            });
        }
    });
}

/// Split a `GET /api/channels` listing into configured instances and the
/// catalog of adapters an operator can add.
///
/// The listing mixes two row shapes under one `items` array and `configured`
/// is the only discriminator. It matters which one a row is, because `name`
/// means different things in each: on a configured row it is the
/// `[[sidecar_channels]].name` the operator chose, on a catalog row it is the
/// adapter key. Reading a configured row's `name` as an adapter is the #8055 /
/// #8063 bug, so the split happens here, once, and the two shapes land in
/// separate types that cannot be confused downstream.
///
/// Pure so it can be tested without a daemon.
pub fn parse_channel_list(
    body: &serde_json::Value,
) -> (Vec<ChannelInstance>, Vec<ChannelAdapterInfo>) {
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut instances = Vec::new();
    let mut adapters = Vec::new();
    for row in &items {
        let name = row["name"].as_str().unwrap_or_default().to_string();
        if name.is_empty() {
            continue;
        }
        let fields = parse_channel_fields(&row["fields"]);
        if row["configured"].as_bool() == Some(true) {
            instances.push(ChannelInstance {
                // `channel_type` is absent on entries that never set it, and
                // the daemon then treats the name as the type — mirror that
                // fallback rather than leaving the adapter blank.
                adapter: row["channel_type"]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .unwrap_or(name.as_str())
                    .to_string(),
                name,
                agent: row["agent"].as_str().map(str::to_string),
                supervised: row["supervised"].as_bool().unwrap_or(false),
                connected: row["connected"].as_bool().unwrap_or(false),
                last_error: row["last_error"].as_str().map(str::to_string),
                messages_received: row["messages_received"].as_u64().unwrap_or(0),
                messages_sent: row["messages_sent"].as_u64().unwrap_or(0),
                fields,
            });
        } else {
            adapters.push(ChannelAdapterInfo {
                display_name: row["display_name"]
                    .as_str()
                    .filter(|d| !d.is_empty())
                    .unwrap_or(name.as_str())
                    .to_string(),
                name,
                fields,
                schema_error: row["schema_error"].as_str().map(str::to_string),
            });
        }
    }
    // The catalog is served in declaration order; sort it so the picker is
    // stable and alphabetical regardless of how the daemon listed it.
    adapters.sort_by(|a, b| a.name.cmp(&b.name));
    (instances, adapters)
}

fn parse_channel_fields(fields: &serde_json::Value) -> Vec<ChannelFieldInfo> {
    fields
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|f| {
                    let key = f["key"].as_str().unwrap_or_default().to_string();
                    ChannelFieldInfo {
                        label: f["label"]
                            .as_str()
                            .filter(|l| !l.is_empty())
                            .unwrap_or(key.as_str())
                            .to_string(),
                        key,
                        field_type: f["type"].as_str().unwrap_or("text").to_string(),
                        required: f["required"].as_bool().unwrap_or(false),
                        placeholder: f["placeholder"].as_str().unwrap_or_default().to_string(),
                        advanced: f["advanced"].as_bool().unwrap_or(false),
                        options: f["options"]
                            .as_array()
                            .map(|o| {
                                o.iter()
                                    .filter_map(|v| v.as_str().map(str::to_string))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        // Secrets come back with `has_value` and no `value`;
                        // the daemon never echoes a stored secret.
                        value: f["value"].as_str().unwrap_or_default().to_string(),
                        has_value: f["has_value"].as_bool().unwrap_or(false),
                    }
                })
                .filter(|f: &ChannelFieldInfo| !f.key.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch configured channel instances + the sidecar catalog in background.
pub fn spawn_fetch_channels(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            match client.get(format!("{base_url}/api/channels")).send() {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let (instances, adapters) = parse_channel_list(&body);
                    let _ = tx.send(AppEvent::ChannelListLoaded {
                        instances,
                        adapters,
                    });
                }
                // A 401 / 423 / 500 body carries no `items`, so parsing it
                // anyway would render as "No channel instances configured" —
                // an empty daemon and a rejected request must not look alike.
                Ok(resp) => {
                    let status = resp.status();
                    let payload: serde_json::Value = resp.json().unwrap_or_default();
                    let message = payload
                        .pointer("/error/message")
                        .and_then(|v| v.as_str())
                        .or_else(|| payload["message"].as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| status.to_string());
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-channel-instances-fetch-failed",
                        &[("error", &message)],
                    )));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-channel-instances-fetch-failed",
                        &[("error", &e.to_string())],
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-channels-not-available-in-process",
            )));
        }
    });
}

/// Write one `[[sidecar_channels]]` instance in background.
///
/// The adapter goes in the path and the instance name in the body — see
/// [`ConfigureRequest`], which is the only thing that decides either.
pub fn spawn_save_channel_instance(
    backend: BackendRef,
    request: ConfigureRequest,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            // The write touches secrets.env, config.toml and then restarts the
            // sidecar children, so it gets the longer timeout.
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(20));
            let url = format!("{base_url}{}", request.path());
            match client.post(&url).json(&request.body()).send() {
                Ok(resp) => {
                    let status = resp.status();
                    let payload: serde_json::Value = resp.json().unwrap_or_default();
                    if status.is_success() && payload["status"].as_str() == Some("saved") {
                        let _ = tx.send(AppEvent::ChannelInstanceSaved {
                            instance_name: request.instance_name,
                            restart_required: payload["restart_required"]
                                .as_bool()
                                .unwrap_or(false),
                            shadowed_secrets: payload["shadowed_secrets"]
                                .as_array()
                                .map(|keys| {
                                    keys.iter()
                                        .filter_map(|k| k.as_str().map(str::to_string))
                                        .collect()
                                })
                                .unwrap_or_default(),
                        });
                    } else {
                        // The daemon's own message names the offending field
                        // or the 409 conflict; it is far more useful than a
                        // status code. `ApiErrorResponse` publishes it nested
                        // and flat, so try both.
                        let message = payload
                            .pointer("/error/message")
                            .and_then(|v| v.as_str())
                            .or_else(|| payload["message"].as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| status.to_string());
                        let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                            "tui-event-channel-save-failed",
                            &[("name", &request.instance_name), ("error", &message)],
                        )));
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-channel-save-failed",
                        &[("name", &request.instance_name), ("error", &e.to_string())],
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-channels-not-available-in-process",
            )));
        }
    });
}

/// Remove one `[[sidecar_channels]]` instance in background.
///
/// Keyed by the instance name: `DELETE /api/channels/sidecar/{name}` matches
/// the `name` key of the block it deletes, so passing an adapter here would
/// delete whichever instance happens to carry the adapter's name — or 404.
pub fn spawn_delete_channel_instance(
    backend: BackendRef,
    instance_name: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(20));
            let url = format!("{base_url}/api/channels/sidecar/{instance_name}");
            match client.delete(&url).send() {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::ChannelInstanceDeleted(instance_name));
                }
                Ok(resp) => {
                    let status = resp.status();
                    let payload: serde_json::Value = resp.json().unwrap_or_default();
                    let message = payload
                        .pointer("/error/message")
                        .and_then(|v| v.as_str())
                        .or_else(|| payload["message"].as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| status.to_string());
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-channel-delete-failed",
                        &[("name", &instance_name), ("error", &message)],
                    )));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-channel-delete-failed",
                        &[("name", &instance_name), ("error", &e.to_string())],
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-channels-not-available-in-process",
            )));
        }
    });
}

/// Re-read `[[sidecar_channels]]` and restart the sidecar children.
pub fn spawn_reload_channels(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(30));
            match client
                .post(format!("{base_url}/api/channels/reload"))
                .json(&serde_json::json!({}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let payload: serde_json::Value = resp.json().unwrap_or_default();
                    let started = payload["started"].as_u64().unwrap_or(0);
                    let _ = tx.send(AppEvent::ChannelsReloaded(started));
                }
                Ok(resp) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-channels-reload-failed",
                        &[("error", &resp.status().to_string())],
                    )));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-channels-reload-failed",
                        &[("error", &e.to_string())],
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-channels-not-available-in-process",
            )));
        }
    });
}

/// Fetch workflow list in background.
pub fn spawn_fetch_workflows(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());

            if let Ok(resp) = client.get(format!("{base_url}/api/workflows")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let workflows: Vec<WorkflowInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|wf| WorkflowInfo {
                                    id: wf["id"].as_str().unwrap_or("?").to_string(),
                                    name: wf["name"].as_str().unwrap_or("?").to_string(),
                                    steps: wf["steps"].as_u64().unwrap_or(0) as usize,
                                    created: wf["created"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::WorkflowListLoaded(workflows));
                }
            }
        }
        BackendRef::InProcess(_kernel) => {
            // Workflows in in-process mode - return empty for now
            let _ = tx.send(AppEvent::WorkflowListLoaded(Vec::new()));
        }
    });
}

/// Fetch workflow runs in background.
pub fn spawn_fetch_workflow_runs(
    backend: BackendRef,
    workflow_id: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());

            if let Ok(resp) = client
                .get(format!("{base_url}/api/workflows/{workflow_id}/runs"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let runs: Vec<WorkflowRun> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|r| WorkflowRun {
                                    id: r["id"].as_str().unwrap_or("?").to_string(),
                                    state: r["state"].as_str().unwrap_or("?").to_string(),
                                    duration: r["duration"].as_str().unwrap_or("").to_string(),
                                    output_preview: r["output"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::WorkflowRunsLoaded(runs));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::WorkflowRunsLoaded(Vec::new()));
        }
    });
}

/// How long the daemon may hold the run request open before handing the run back as a background task.
///
/// Same reasoning as `WORKFLOW_RUN_WAIT_MS` in the `workflow run` command: `?wait=true` on its own ties the run's lifetime to the request, so a workflow slower than this thread's 60 s client timeout would be killed by the disconnect.
/// 45 s leaves 15 s of that budget for the response itself.
const WORKFLOW_RUN_WAIT_MS: u64 = 45_000;

/// The wait has to expire before this thread's own client does, or a slow run comes back as a disconnect instead of the 202 the screen knows how to render.
const _: () = assert!(
    WORKFLOW_RUN_WAIT_MS < 60_000,
    "spawn_run_workflow builds a 60 s client; a longer wait can never return 202"
);

/// Render one workflow-run response for the Workflows screen.
///
/// Reading `output` and nothing else meant a 202 (still running) and a 422 (the run failed) both rendered the generic "completed" line, so the screen announced success on every failure.
/// The classification is shared with `librefang workflow run` so the two surfaces cannot drift apart again.
fn workflow_run_result_message(status: reqwest::StatusCode, body: &serde_json::Value) -> String {
    match crate::commands::automation::classify_workflow_run(status, body) {
        WorkflowRunOutcome::Completed { output, .. } => output.to_string(),
        WorkflowRunOutcome::Accepted { run_id } => {
            crate::i18n::t_args("tui-event-workflow-still-running", &[("id", run_id)])
        }
        WorkflowRunOutcome::Failed { error } => crate::i18n::t_args(
            "tui-event-workflow-run-failed",
            &[("status", &status.to_string()), ("detail", error)],
        ),
    }
}

/// Run a workflow in background.
pub fn spawn_run_workflow(
    backend: BackendRef,
    workflow_id: String,
    input: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(60));

            match client
                .post(format!(
                    "{base_url}/api/workflows/{workflow_id}/run?wait=true&timeout_ms={WORKFLOW_RUN_WAIT_MS}"
                ))
                .json(&serde_json::json!({"input": input}))
                .send()
            {
                Ok(resp) => {
                    let status = resp.status();
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let _ = tx.send(AppEvent::WorkflowRunResult(
                        workflow_run_result_message(status, &body),
                    ));
                }
                // The request never left, so there is no run status to report.
                // `spawn_create_workflow` already established `-` as the
                // status placeholder for that case, and the same key carries
                // it here rather than a hardcoded English line that no locale
                // can translate.
                Err(e) => {
                    let _ = tx.send(AppEvent::WorkflowRunResult(crate::i18n::t_args(
                        "tui-event-workflow-run-failed",
                        &[("status", "-"), ("detail", &transport_detail(&e))],
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::WorkflowRunResult(crate::i18n::t(
                "tui-event-workflow-exec-not-available-in-process",
            )));
        }
    });
}

/// Why the workflow creator's raw `steps` field could not become a request body.
///
/// Kept separate from its rendered message so the parser stays a pure
/// function the unit tests can exercise without initialising the locale
/// bundles.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StepsJsonError {
    /// The operator advanced past the steps field without typing anything.
    Empty,
    /// The text is not JSON at all; carries serde's position-bearing message.
    NotJson(String),
    /// Valid JSON, but a scalar or an object where the API wants an array.
    NotArray,
}

impl StepsJsonError {
    fn message(&self) -> String {
        match self {
            StepsJsonError::Empty => crate::i18n::t("tui-event-workflow-steps-empty"),
            StepsJsonError::NotJson(detail) => {
                crate::i18n::t_args("tui-event-workflow-steps-invalid", &[("error", detail)])
            }
            StepsJsonError::NotArray => crate::i18n::t("tui-event-workflow-steps-not-array"),
        }
    }
}

/// Turn the creator's free-text `steps` field into the array
/// `POST /api/workflows` expects.
///
/// The wizard collects the steps as a raw JSON string, and that string used
/// to be forwarded as a JSON *string*. `create_workflow` reads
/// `req["steps"].as_array()`, so every submission was rejected with
/// `Missing 'steps' array` and the TUI could not create a workflow at all.
/// Parsing here also turns a typo into a message naming the position of the
/// mistake instead of a bare HTTP failure.
pub(crate) fn parse_workflow_steps_json(raw: &str) -> Result<serde_json::Value, StepsJsonError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(StepsJsonError::Empty);
    }
    let value: serde_json::Value =
        serde_json::from_str(trimmed).map_err(|e| StepsJsonError::NotJson(e.to_string()))?;
    if !value.is_array() {
        return Err(StepsJsonError::NotArray);
    }
    Ok(value)
}

/// Create a workflow in background.
pub fn spawn_create_workflow(
    backend: BackendRef,
    name: String,
    description: String,
    steps_json: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let steps = match parse_workflow_steps_json(&steps_json) {
                Ok(steps) => steps,
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(e.message()));
                    return;
                }
            };
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(10));

            match client
                .post(format!("{base_url}/api/workflows"))
                .json(&serde_json::json!({
                    "name": name,
                    "description": description,
                    "steps": steps,
                }))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    // `create_workflow` answers with `workflow_id`; reading
                    // `id` meant every success reported the placeholder.
                    let id = body["workflow_id"]
                        .as_str()
                        .or_else(|| body["id"].as_str())
                        .unwrap_or("created")
                        .to_string();
                    let _ = tx.send(AppEvent::WorkflowCreated(id));
                }
                Ok(resp) => {
                    // A 400 from the step parser used to be reported as a
                    // successful creation, leaving the operator hunting for a
                    // workflow that was never registered.
                    let status = resp.status().to_string();
                    let detail = resp.text().unwrap_or_default();
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-workflow-create-failed",
                        &[("status", &status), ("detail", &detail)],
                    )));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-workflow-create-failed",
                        &[("status", "-"), ("detail", &e.to_string())],
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-workflow-create-not-available-in-process",
            )));
        }
    });
}

/// Fetch triggers in background.
pub fn spawn_fetch_triggers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());

            if let Ok(resp) = client.get(format!("{base_url}/api/triggers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let triggers: Vec<TriggerInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|tr| TriggerInfo {
                                    id: tr["id"].as_str().unwrap_or("?").to_string(),
                                    agent_id: tr["agent_id"].as_str().unwrap_or("?").to_string(),
                                    pattern: tr["pattern"].as_str().unwrap_or("?").to_string(),
                                    fires: tr["fires"].as_u64().unwrap_or(0),
                                    enabled: tr["enabled"].as_bool().unwrap_or(true),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::TriggerListLoaded(triggers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::TriggerListLoaded(Vec::new()));
        }
    });
}

/// Create a trigger in background.
pub fn spawn_create_trigger(
    backend: BackendRef,
    agent_id: String,
    pattern_type: String,
    pattern_param: String,
    prompt: String,
    max_fires: u64,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(10));

            match client
                .post(format!("{base_url}/api/triggers"))
                .json(&serde_json::json!({
                    "agent_id": agent_id,
                    "pattern_type": pattern_type,
                    "pattern_param": pattern_param,
                    "prompt": prompt,
                    "max_fires": max_fires,
                }))
                .send()
            {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let id = body["id"].as_str().unwrap_or("created").to_string();
                    let _ = tx.send(AppEvent::TriggerCreated(id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Create trigger: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-trigger-create-not-available-in-process",
            )));
        }
    });
}

/// Delete a trigger in background.
pub fn spawn_delete_trigger(backend: BackendRef, trigger_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());

            let outcome = daemon_response(
                client
                    .delete(format!("{base_url}/api/triggers/{trigger_id}"))
                    .send(),
                || {
                    crate::i18n::t_args(
                        "tui-event-trigger-delete-failed",
                        &[("trigger_id", &trigger_id)],
                    )
                },
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::TriggerDeleted(trigger_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-trigger-delete-not-available-in-process",
            )));
        }
    });
}

/// Kill an agent in background (for detail view action).
pub fn spawn_kill_agent(backend: BackendRef, agent_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());

            let outcome = daemon_response(
                client
                    .delete(format!("{base_url}/api/agents/{agent_id}"))
                    .send(),
                || crate::i18n::t_args("tui-event-agent-kill-failed", &[("agent_id", &agent_id)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::AgentKilled { id: agent_id });
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::AgentKillError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            // Try to parse as UUID-based AgentId
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = AgentId(uuid);
                match kernel.kill_agent(aid) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::AgentKilled { id: agent_id });
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::AgentKillError(format!("{e}")));
                    }
                }
            } else {
                let _ = tx.send(AppEvent::AgentKillError(crate::i18n::t_args(
                    "tui-event-agent-invalid-id",
                    &[("agent_id", &agent_id)],
                )));
            }
        }
    });
}

/// Fetch skill assignment for an agent.
pub fn spawn_fetch_agent_skills(backend: BackendRef, agent_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/agents/{agent_id}/skills"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let assigned: Vec<String> = body["assigned"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let available: Vec<String> = body["available"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AgentSkillsLoaded {
                        assigned,
                        available,
                    });
                    return;
                }
            }
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-skills-fetch-failed",
            )));
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = librefang_types::agent::AgentId(uuid);
                let assigned = kernel
                    .agent_registry_ref()
                    .get(aid)
                    .map(|e| e.manifest.skills.clone())
                    .unwrap_or_default();
                let available = kernel
                    .skill_registry_ref()
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .skill_names();
                let _ = tx.send(AppEvent::AgentSkillsLoaded {
                    assigned,
                    available,
                });
            }
        }
    });
}

/// Fetch MCP server assignment for an agent.
pub fn spawn_fetch_agent_mcp_servers(
    backend: BackendRef,
    agent_id: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/agents/{agent_id}/mcp_servers"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let assigned: Vec<String> = body["assigned"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let available: Vec<String> = body["available"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AgentMcpServersLoaded {
                        assigned,
                        available,
                    });
                    return;
                }
            }
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-mcp-fetch-failed",
            )));
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = librefang_types::agent::AgentId(uuid);
                let assigned = kernel
                    .agent_registry_ref()
                    .get(aid)
                    .map(|e| e.manifest.mcp_servers.clone())
                    .unwrap_or_default();
                let mut available = Vec::new();
                if let Ok(mcp_tools) = kernel.tools_ref().lock() {
                    let configured_servers: Vec<String> = kernel
                        .effective_servers_ref()
                        .read()
                        .map(|servers| servers.iter().map(|s| s.name.clone()).collect())
                        .unwrap_or_default();
                    let mut seen = std::collections::HashSet::new();
                    for tool in mcp_tools.iter() {
                        if let Some(server) = librefang_runtime::mcp::resolve_mcp_server_from_known(
                            &tool.name,
                            configured_servers.iter().map(String::as_str),
                        ) {
                            if seen.insert(server.to_string()) {
                                available.push(server.to_string());
                            }
                        }
                    }
                }
                let _ = tx.send(AppEvent::AgentMcpServersLoaded {
                    assigned,
                    available,
                });
            }
        }
    });
}

/// Update an agent's skills.
pub fn spawn_update_agent_skills(
    backend: BackendRef,
    agent_id: String,
    skills: Vec<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .put(format!("{base_url}/api/agents/{agent_id}/skills"))
                    .json(&serde_json::json!({"skills": skills}))
                    .send(),
                || crate::i18n::t("tui-event-skills-update-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::AgentSkillsUpdated(agent_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = librefang_types::agent::AgentId(uuid);
                match kernel.set_agent_skills(aid, skills) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::AgentSkillsUpdated(agent_id));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                            "tui-event-skills-update-error",
                            &[("error", &e.to_string())],
                        )));
                    }
                }
            }
        }
    });
}

/// Update an agent's MCP servers.
pub fn spawn_update_agent_mcp_servers(
    backend: BackendRef,
    agent_id: String,
    servers: Vec<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .put(format!("{base_url}/api/agents/{agent_id}/mcp_servers"))
                    .json(&serde_json::json!({"mcp_servers": servers}))
                    .send(),
                || crate::i18n::t("tui-event-mcp-update-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::AgentMcpServersUpdated(agent_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = librefang_types::agent::AgentId(uuid);
                match kernel.set_agent_mcp_servers(aid, servers) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::AgentMcpServersUpdated(agent_id));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                            "tui-event-mcp-update-error",
                            &[("error", &e.to_string())],
                        )));
                    }
                }
            }
        }
    });
}

/// Fetch the channel allowlist for an agent (#7742).
///
/// `GET /api/agents/{id}/channels` had no client anywhere in the tree — not here, not in the
/// dashboard — so assigning a channel to a running agent meant hand-editing `agent.toml`.
/// The in-process branch reads the same two sources the HTTP handler does: the manifest for
/// `assigned`, and `sidecar_channels` for the catalogue of `channel_type` strings to offer.
pub fn spawn_fetch_agent_channels(
    backend: BackendRef,
    agent_id: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/agents/{agent_id}/channels"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let read = |key: &str| -> Vec<String> {
                        body[key]
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|v| v.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default()
                    };
                    let _ = tx.send(AppEvent::AgentChannelsLoaded {
                        assigned: read("assigned"),
                        available: read("available"),
                    });
                    return;
                }
            }
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-channels-fetch-failed",
            )));
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = librefang_types::agent::AgentId(uuid);
                let assigned = kernel
                    .agent_registry_ref()
                    .get(aid)
                    .map(|e| e.manifest.channels.clone())
                    .unwrap_or_default();
                let mut available: Vec<String> = kernel
                    .config_ref()
                    .sidecar_channels
                    .iter()
                    .map(|sc| sc.channel_type.clone().unwrap_or_else(|| sc.name.clone()))
                    .collect();
                // A channel already on the manifest but no longer configured must still be
                // offered, or opening the editor and saving would silently drop it.
                for name in &assigned {
                    if !available.contains(name) {
                        available.push(name.clone());
                    }
                }
                let _ = tx.send(AppEvent::AgentChannelsLoaded {
                    assigned,
                    available,
                });
            }
        }
    });
}

/// Update an agent's channel allowlist.
///
/// An empty list is a legitimate value, not a no-op: `AgentManifest::channels` treats empty as
/// "every channel", so clearing the selection widens access rather than revoking it.
pub fn spawn_update_agent_channels(
    backend: BackendRef,
    agent_id: String,
    channels: Vec<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .put(format!("{base_url}/api/agents/{agent_id}/channels"))
                    .json(&serde_json::json!({"channels": channels}))
                    .send(),
                || crate::i18n::t("tui-event-channels-update-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::AgentChannelsUpdated(agent_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = librefang_types::agent::AgentId(uuid);
                match kernel.set_agent_channels(aid, channels) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::AgentChannelsUpdated(agent_id));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                            "tui-event-channels-update-error",
                            &[("error", &e.to_string())],
                        )));
                    }
                }
            }
        }
    });
}

// ── New screen spawn functions ───────────────────────────────────────────────

/// Build a blocking reqwest client for daemon calls.
///
/// When `api_key` is `Some`, attaches an `Authorization: Bearer <key>` default
/// header so every request through this client is authenticated.
fn make_daemon_client(api_key: Option<&str>) -> reqwest::blocking::Client {
    make_daemon_client_with_timeout(api_key, Duration::from_secs(5))
}

/// Build a blocking reqwest client for daemon calls with a custom timeout.
///
/// When `api_key` is `Some`, attaches an `Authorization: Bearer <key>` default
/// header so every request through this client is authenticated.
fn make_daemon_client_with_timeout(
    api_key: Option<&str>,
    timeout: Duration,
) -> reqwest::blocking::Client {
    let mut builder = crate::http_client::client_builder().timeout(timeout);
    if let Some(key) = api_key {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
        builder = builder.default_headers(headers);
    }
    builder
        .build()
        .unwrap_or_else(|_| crate::http_client::new_client())
}

/// The reason a non-success daemon response gives, as a line for the operator.
///
/// The daemon answers a rejected request with a JSON body carrying the reason, and that reason is the whole diagnosis — `YAML parse error at line 3` for a broken marketplace skill, `skill not found on registry` for a slug that no longer exists.
/// A screen that discards it and prints its own generic line spends the operator a debugging round trip: the message reads like the daemon fell over, so they go looking for a fault in their own installation when the daemon already named a fault in someone else's artefact.
///
/// When the body carries no usable reason, the status line is the only thing actually known, so that is what gets reported rather than an invented cause.
fn daemon_error_detail(status: reqwest::StatusCode, body: Option<&serde_json::Value>) -> String {
    body.and_then(daemon_error_reason)
        .map(flatten_to_one_line)
        .filter(|reason| !reason.is_empty())
        .unwrap_or_else(|| {
            crate::i18n::t_args(
                "tui-event-daemon-http-status",
                &[("status", &status_line(status))],
            )
        })
}

/// Pull the reason out of whichever error envelope the daemon used.
///
/// Two shapes are in service and both have to be read.
/// `librefang-api`'s standard `ApiErrorResponse` nests the reason as `error.message` and mirrors it at the top level as `message` (`crates/librefang-api/src/types.rs`) — that is what every route behind backups, sessions, memory KV, model overrides, provider keys, hands and `DELETE /api/agents/{id}` returns, so it covers most of this module's call sites.
/// The ad-hoc `Json(json!({"error": reason}))` tuples used by ClawHub install and the agent-config routes put a bare string at `error` instead.
/// Reading only the bare string leaves the majority of endpoints reporting nothing but a status code, which is the whole defect this helper exists to remove; `spawn_restore_backup` in this module already read both shapes.
fn daemon_error_reason(body: &serde_json::Value) -> Option<&str> {
    let error = body.get("error");
    error
        .and_then(|v| v.as_str())
        .or_else(|| {
            error
                .and_then(|v| v.get("message"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| body.get("message").and_then(|v| v.as_str()))
}

/// `409 Conflict` rather than a bare `409`.
///
/// The reason phrase costs nothing and is half of what a status code tells the operator, and the module's other status-reporting handler (`spawn_create_workflow`) already prints it.
fn status_line(status: reqwest::StatusCode) -> String {
    match status.canonical_reason() {
        Some(reason) => format!("{} {reason}", status.as_str()),
        None => status.as_str().to_string(),
    }
}

/// Flatten a daemon reason onto the single line a status area can show.
///
/// The bytes are not always the daemon's own: a rejected ClawHub install echoes the registry's `SkillError` verbatim, so a third party chooses them.
/// A newline breaks the one-line status area and an escape byte reaches the terminal through ratatui's cell buffer, so control characters collapse to spaces and runs of whitespace fold together.
fn flatten_to_one_line(text: &str) -> String {
    let uncontrolled: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    uncontrolled
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A transport failure, with the cause `reqwest`'s own `Display` leaves out.
///
/// `reqwest::Error`'s `Display` prints the kind and the URL and stops — `error sending request for url (http://127.0.0.1:4545/api/backups)` — keeping the part that decides the operator's next step (`Connection refused`: the daemon is not running; `operation timed out`: it is running but wedged) on the `source()` chain.
/// Without walking the chain every transport failure renders identically, which is the same loss of information as the generic line this helper replaced.
fn transport_detail(error: &reqwest::Error) -> String {
    let mut line = error.to_string();
    let mut next = std::error::Error::source(error);
    while let Some(cause) = next {
        let text = cause.to_string();
        if !text.is_empty() && !line.contains(&text) {
            line.push_str(": ");
            line.push_str(&text);
        }
        next = cause.source();
    }
    flatten_to_one_line(&line)
}

/// Join an operation's generic summary line to the reason behind it.
fn with_detail(summary: String, detail: String) -> String {
    crate::i18n::t_args(
        "tui-event-daemon-failure-detail",
        &[("summary", &summary), ("detail", &detail)],
    )
}

/// Collapse a daemon call into either its response or the line to show the operator.
///
/// Every caller used to write `Ok(resp) if resp.status().is_success() => … , _ => <generic line>`, which threw away two different things at once: the reason the daemon put in the body, and the distinction between *the daemon rejected this* and *the request never reached the daemon* — two problems with two different next steps, reported identically.
///
/// `summary` is called only on failure, so the localized generic line is built only when it is going to be shown.
fn daemon_response(
    outcome: Result<reqwest::blocking::Response, reqwest::Error>,
    summary: impl FnOnce() -> String,
) -> Result<reqwest::blocking::Response, String> {
    match outcome {
        Ok(resp) if resp.status().is_success() => Ok(resp),
        Ok(resp) => {
            let status = resp.status();
            let body = resp.json::<serde_json::Value>().ok();
            Err(with_detail(
                summary(),
                daemon_error_detail(status, body.as_ref()),
            ))
        }
        // The request never left, so there is no daemon verdict to report; the
        // transport error is the actionable part.
        Err(e) => Err(with_detail(summary(), transport_detail(&e))),
    }
}

/// Fetch sessions list.
pub fn spawn_fetch_sessions(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/sessions")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let sessions: Vec<SessionInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|s| SessionInfo {
                                    id: s["id"].as_str().unwrap_or("").to_string(),
                                    agent_name: s["agent_name"].as_str().unwrap_or("").to_string(),
                                    agent_id: s["agent_id"].as_str().unwrap_or("").to_string(),
                                    message_count: s["message_count"].as_u64().unwrap_or(0),
                                    created: s["created"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SessionsLoaded(sessions));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SessionsLoaded(Vec::new()));
        }
    });
}

/// Delete a session.
pub fn spawn_delete_session(backend: BackendRef, session_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .delete(format!("{base_url}/api/sessions/{session_id}"))
                    .send(),
                || {
                    crate::i18n::t_args(
                        "tui-event-session-delete-failed",
                        &[("session_id", &session_id)],
                    )
                },
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::SessionDeleted(session_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-session-management-not-available-in-process",
            )));
        }
    });
}

/// Fetch agents for memory screen agent selector.
/// Fetch the memory configuration for the terminal's config panel.
///
/// Reads `effective_extraction_model` and `extraction_model_source` rather
/// than the raw `extraction_model`: unset means "inherit the kernel default",
/// so the raw field answers "nobody chose one" instead of naming the model
/// that runs after every reply.
///
/// Daemon-only. The in-process backend has no HTTP surface to ask, and the
/// panel says so rather than showing a blank as if it were configuration.
pub fn spawn_fetch_memory_config(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        if let BackendRef::Daemon { base_url, api_key } = backend {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/memory/config")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let pm = &body["proactive_memory"];
                    let view = crate::tui::screens::memory::MemoryConfigView {
                        embedding_provider: body["embedding_provider"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        embedding_model: body["embedding_model"].as_str().unwrap_or("").to_string(),
                        auto_memorize: pm["auto_memorize"].as_bool().unwrap_or(false),
                        auto_retrieve: pm["auto_retrieve"].as_bool().unwrap_or(false),
                        effective_extraction_model: pm["effective_extraction_model"]
                            .as_str()
                            .unwrap_or("")
                            .to_string(),
                        extraction_model_inherited: pm["extraction_model_source"].as_str()
                            == Some("inherited_default"),
                    };
                    let _ = tx.send(AppEvent::MemoryConfigLoaded(view));
                }
            }
        }
    });
}

pub fn spawn_fetch_memory_agents(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let agents: Vec<AgentEntry> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|a| AgentEntry {
                                    id: a["id"].as_str().unwrap_or("").to_string(),
                                    name: a["name"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::MemoryAgentsLoaded(agents));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let agents: Vec<AgentEntry> = kernel
                .agent_registry_ref()
                .list()
                .iter()
                .map(|e| AgentEntry {
                    id: format!("{}", e.id),
                    name: e.name.clone(),
                })
                .collect();
            let _ = tx.send(AppEvent::MemoryAgentsLoaded(agents));
        }
    });
}

/// Fetch KV pairs for an agent.
pub fn spawn_fetch_memory_kv(backend: BackendRef, agent_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/memory/agents/{agent_id}/kv"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let pairs: Vec<KvPair> = if let Some(obj) = body.as_object() {
                        obj.iter()
                            .map(|(k, v)| KvPair {
                                key: k.clone(),
                                value: v.as_str().unwrap_or(&v.to_string()).to_string(),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let _ = tx.send(AppEvent::MemoryKvLoaded(pairs));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::MemoryKvLoaded(Vec::new()));
        }
    });
}

/// Save a KV pair.
pub fn spawn_save_memory_kv(
    backend: BackendRef,
    agent_id: String,
    key: String,
    value: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .put(format!("{base_url}/api/memory/agents/{agent_id}/kv/{key}"))
                    .json(&serde_json::json!({"value": value}))
                    .send(),
                || crate::i18n::t("tui-event-kv-save-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::MemoryKvSaved { key });
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-kv-not-available-in-process",
            )));
        }
    });
}

/// Delete a KV pair.
pub fn spawn_delete_memory_kv(
    backend: BackendRef,
    agent_id: String,
    key: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .delete(format!("{base_url}/api/memory/agents/{agent_id}/kv/{key}"))
                    .send(),
                || crate::i18n::t("tui-event-kv-delete-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::MemoryKvDeleted(key));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-kv-not-available-in-process",
            )));
        }
    });
}

/// Fetch installed skills.
pub fn spawn_fetch_skills(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/skills")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let skills: Vec<SkillInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|s| SkillInfo {
                                    name: s["name"].as_str().unwrap_or("").to_string(),
                                    runtime: s["runtime"].as_str().unwrap_or("").to_string(),
                                    source: s["source"].as_str().unwrap_or("").to_string(),
                                    description: s["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SkillsLoaded(skills));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SkillsLoaded(Vec::new()));
        }
    });
}

/// Search ClawHub marketplace.
pub fn spawn_search_clawhub(backend: BackendRef, query: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let encoded: String = query
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                        c.to_string()
                    } else {
                        format!("%{:02X}", c as u32)
                    }
                })
                .collect();
            let url = format!("{base_url}/api/clawhub/search?q={encoded}");
            if let Ok(resp) = client.get(&url).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let results = parse_clawhub_results(&body);
                    let _ = tx.send(AppEvent::ClawHubLoaded(results));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::ClawHubLoaded(Vec::new()));
        }
    });
}

/// Browse ClawHub marketplace.
pub fn spawn_browse_clawhub(backend: BackendRef, sort: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let url = format!("{base_url}/api/clawhub/browse?sort={sort}");
            if let Ok(resp) = client.get(&url).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let results = parse_clawhub_results(&body);
                    let _ = tx.send(AppEvent::ClawHubLoaded(results));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::ClawHubLoaded(Vec::new()));
        }
    });
}

fn parse_clawhub_results(body: &serde_json::Value) -> Vec<ClawHubResult> {
    // API returns {"items": [...]} wrapper, fall back to bare array for compat
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array());

    items
        .map(|arr| {
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            arr.iter()
                .map(|r| ClawHubResult {
                    name: r["name"].as_str().unwrap_or("").to_string(),
                    slug: r["slug"].as_str().unwrap_or("").to_string(),
                    description: r["description"].as_str().unwrap_or("").to_string(),
                    downloads: r["downloads"].as_u64().unwrap_or(0),
                    runtime: r["runtime"].as_str().unwrap_or("").to_string(),
                })
                // The index serves the same skill under more than one display
                // casing ("Prd" and "prd"), and the install call addresses a
                // skill by slug — so the duplicates are one skill, not two.
                // Left as-is the browse list showed it twice and a single
                // install press lit the pending state on every copy.
                //
                // Keyed on the lowercased slug, and first-wins: the index is
                // returned in relevance order, so the first spelling of a slug
                // is the one the server ranked highest.
                .filter(|entry| seen.insert(entry.slug.to_lowercase()))
                .collect()
        })
        .unwrap_or_default()
}

/// Install a skill from ClawHub.
pub fn spawn_install_skill(backend: BackendRef, slug: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/clawhub/install"))
                    .json(&serde_json::json!({"slug": slug}))
                    .send(),
                || crate::i18n::t_args("tui-event-skill-install-failed", &[("slug", &slug)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::SkillInstalled(slug));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-skill-install-not-available-in-process",
            )));
        }
    });
}

/// Uninstall a skill.
pub fn spawn_uninstall_skill(backend: BackendRef, name: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/skills/uninstall"))
                    .json(&serde_json::json!({"name": name}))
                    .send(),
                || crate::i18n::t_args("tui-event-skill-uninstall-failed", &[("name", &name)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::SkillUninstalled(name));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-skill-uninstall-not-available-in-process",
            )));
        }
    });
}

/// Fetch MCP servers.
pub fn spawn_fetch_mcp_servers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/mcp/servers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let servers: Vec<McpServerInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|s| McpServerInfo {
                                    name: s["name"].as_str().unwrap_or("").to_string(),
                                    connected: s["connected"].as_bool().unwrap_or(false),
                                    tool_count: s["tool_count"].as_u64().unwrap_or(0) as usize,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::McpServersLoaded(servers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::McpServersLoaded(Vec::new()));
        }
    });
}

/// Fetch provider auth status for templates screen.
/// Reject a template name that could escape the templates directory or the URL path.
/// Mirrors `validate_template_name` in the API so a name the daemon would refuse fails here too instead of producing a confusing 404.
/// Names only ever originate from a directory listing, so this is belt-and-braces.
fn is_safe_template_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn workspace_agents_dir() -> std::path::PathBuf {
    librefang_kernel::config::librefang_home()
        .join("workspaces")
        .join("agents")
}

/// Operator-authored agent types, one flat `{name}.toml` each — the same directory
/// `POST`/`PUT /api/templates` writes (#7740).
fn agent_types_dir() -> std::path::PathBuf {
    librefang_kernel::config::librefang_home().join("agent-types")
}

/// Resolve one agent type's manifest path, agent-types first.
///
/// Mirrors the precedence `GET /api/templates/{name}` uses, so the in-process backend
/// and the daemon backend spawn from the same document for the same row.
fn local_agent_type_path(name: &str) -> Option<std::path::PathBuf> {
    let own = agent_types_dir().join(format!("{name}.toml"));
    if own.is_file() {
        return Some(own);
    }
    let workspace = workspace_agents_dir().join(name).join("agent.toml");
    workspace.is_file().then_some(workspace)
}

/// Read the agent types on disk.
/// The in-process backend has no HTTP surface to ask, so it reads the same directory `GET /api/templates` serves.
fn local_agent_templates() -> Vec<TemplateInfo> {
    // Same two sources, and the same precedence, as `GET /api/templates`.
    let mut out = Vec::new();
    collect_agent_type_files(&agent_types_dir(), &mut out);
    collect_workspace_agent_manifests(&workspace_agents_dir(), &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup_by(|a, b| a.name == b.name);
    out
}

/// Read `agent-types/{name}.toml` — the documents the write verbs own.
fn collect_agent_type_files(dir: &std::path::Path, out: &mut Vec<TemplateInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !is_safe_template_name(name) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        push_template_info(name.to_string(), &content, &path, out);
    }
}

/// Read `workspaces/agents/{name}/agent.toml` — every live agent is spawnable-from too.
fn collect_workspace_agent_manifests(dir: &std::path::Path, out: &mut Vec<TemplateInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_safe_template_name(&name) {
            continue;
        }
        let manifest_path = entry.path().join("agent.toml");
        let Ok(content) = std::fs::read_to_string(&manifest_path) else {
            continue;
        };
        push_template_info(name, &content, &manifest_path, out);
    }
}

fn push_template_info(
    name: String,
    content: &str,
    path: &std::path::Path,
    out: &mut Vec<TemplateInfo>,
) {
    match toml::from_str::<librefang_types::agent::AgentManifest>(content) {
        Ok(manifest) => out.push(TemplateInfo {
            name,
            description: manifest.description,
            category: templates::MANIFEST_CATEGORY.to_string(),
            provider: manifest.model.provider,
            model: manifest.model.model,
            source: TemplateSource::Manifest,
        }),
        // Naming the file turns "my agent type vanished" into a one-line diagnosis instead of a silent absence.
        Err(e) => tracing::warn!(
            "skipping agent template {}: invalid manifest: {e}",
            path.display()
        ),
    }
}

fn parse_api_templates(body: &serde_json::Value) -> Vec<TemplateInfo> {
    body["templates"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let name = t["name"].as_str()?.to_string();
                    Some(TemplateInfo {
                        name,
                        description: t["description"].as_str().unwrap_or_default().to_string(),
                        category: templates::MANIFEST_CATEGORY.to_string(),
                        provider: t["provider"].as_str().unwrap_or("default").to_string(),
                        model: t["model"].as_str().unwrap_or("default").to_string(),
                        source: TemplateSource::Manifest,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch the operator-created agent types (#7760).
///
/// The templates screen used to render a compiled-in list and nothing else, so `GET /api/templates` was never called and every agent type an operator created was invisible.
pub fn spawn_fetch_agent_templates(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let templates = match backend {
            BackendRef::Daemon { base_url, api_key } => {
                let client = make_daemon_client(api_key.as_deref());
                client
                    .get(format!("{base_url}/api/templates"))
                    .send()
                    .ok()
                    .and_then(|resp| resp.json::<serde_json::Value>().ok())
                    .map(|body| parse_api_templates(&body))
                    .unwrap_or_default()
            }
            BackendRef::InProcess(_) => local_agent_templates(),
        };
        let _ = tx.send(AppEvent::AgentTemplatesLoaded(templates));
    });
}

/// Fetch one agent type's `agent.toml` verbatim, for spawning.
///
/// The screen used to string-format a manifest from the row's name and description and pin a fixed tool list onto it, so every agent type spawned from there got shell plus filesystem write plus network regardless of what its real manifest declared (#7760).
/// Reading the real file is the fix.
pub fn spawn_fetch_template_toml(backend: BackendRef, name: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let toml = if !is_safe_template_name(&name) {
            None
        } else {
            match backend {
                BackendRef::Daemon { base_url, api_key } => {
                    let client = make_daemon_client(api_key.as_deref());
                    client
                        .get(format!("{base_url}/api/templates/{name}/toml"))
                        .send()
                        .ok()
                        .filter(|resp| resp.status().is_success())
                        .and_then(|resp| resp.text().ok())
                }
                BackendRef::InProcess(_) => {
                    local_agent_type_path(&name).and_then(|p| std::fs::read_to_string(p).ok())
                }
            }
        };
        let _ = tx.send(AppEvent::TemplateTomlLoaded { name, toml });
    });
}

pub fn spawn_fetch_template_providers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/providers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    // API returns { "providers": [...], "total": N }
                    let arr = body["providers"].as_array();
                    let providers: Vec<ProviderAuth> = arr
                        .map(|arr| {
                            arr.iter()
                                .map(|p| {
                                    let auth = p["auth_status"].as_str().unwrap_or("missing");
                                    ProviderAuth {
                                        name: p["id"].as_str().unwrap_or("").to_string(),
                                        configured: auth == "configured" || auth == "not_required",
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::TemplateProvidersLoaded(providers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::TemplateProvidersLoaded(Vec::new()));
        }
    });
}

/// Fetch security status.
pub fn spawn_fetch_security(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/security")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let features: Vec<SecurityFeature> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|f| {
                                    use super::screens::security::SecuritySection;
                                    let section = match f["section"].as_str().unwrap_or("core") {
                                        "configurable" => SecuritySection::Configurable,
                                        "monitoring" => SecuritySection::Monitoring,
                                        _ => SecuritySection::Core,
                                    };
                                    SecurityFeature {
                                        name: f["name"].as_str().unwrap_or("").to_string(),
                                        active: f["active"].as_bool().unwrap_or(true),
                                        description: f["description"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        section,
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if !features.is_empty() {
                        let _ = tx.send(AppEvent::SecurityLoaded(features));
                    }
                }
            }
        }
        BackendRef::InProcess(_) => {
            // Use builtin defaults (already loaded in SecurityState::new())
        }
    });
}

/// Verify audit chain.
pub fn spawn_verify_chain(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            match client.get(format!("{base_url}/api/audit/verify")).send() {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let valid = body["valid"].as_bool().unwrap_or(false);
                    let message = body["message"]
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            crate::i18n::t("tui-event-security-verification-complete")
                        });
                    let _ = tx.send(AppEvent::SecurityChainVerified { valid, message });
                    let _ = tx.send(AppEvent::AuditChainVerified(valid));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::SecurityChainVerified {
                        valid: false,
                        message: format!("{e}"),
                    });
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SecurityChainVerified {
                valid: true,
                message: crate::i18n::t("tui-event-security-chain-not-applicable"),
            });
        }
    });
}

/// Fetch audit entries (for dedicated audit screen).
pub fn spawn_fetch_audit(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/audit/recent?n=200"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries: Vec<AuditEntry> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|e| AuditEntry {
                                    timestamp: e["timestamp"].as_str().unwrap_or("").to_string(),
                                    action: e["action"].as_str().unwrap_or("").to_string(),
                                    agent: e["agent"].as_str().unwrap_or("").to_string(),
                                    detail: e["detail"].as_str().unwrap_or("").to_string(),
                                    tip_hash: e["tip_hash"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AuditEntriesLoaded(entries));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::AuditEntriesLoaded(Vec::new()));
        }
    });
}

/// Fetch usage summary.
pub fn spawn_fetch_usage(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            // Summary
            if let Ok(resp) = client.get(format!("{base_url}/api/usage/summary")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let _ = tx.send(AppEvent::UsageSummaryLoaded(UsageSummary {
                        total_input_tokens: body["total_input_tokens"].as_u64().unwrap_or(0),
                        total_output_tokens: body["total_output_tokens"].as_u64().unwrap_or(0),
                        total_cost_usd: body["total_cost_usd"].as_f64().unwrap_or(0.0),
                        total_calls: body["total_calls"].as_u64().unwrap_or(0),
                    }));
                }
            }
            // By model
            if let Ok(resp) = client.get(format!("{base_url}/api/usage/by-model")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let models: Vec<ModelUsage> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|m| ModelUsage {
                                    model_id: m["model_id"].as_str().unwrap_or("").to_string(),
                                    input_tokens: m["input_tokens"].as_u64().unwrap_or(0),
                                    output_tokens: m["output_tokens"].as_u64().unwrap_or(0),
                                    cost_usd: m["cost_usd"].as_f64().unwrap_or(0.0),
                                    calls: m["calls"].as_u64().unwrap_or(0),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::UsageByModelLoaded(models));
                }
            }
            // By agent
            if let Ok(resp) = client.get(format!("{base_url}/api/usage")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let agents: Vec<AgentUsage> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|a| AgentUsage {
                                    agent_name: a["agent_name"].as_str().unwrap_or("").to_string(),
                                    agent_id: a["agent_id"].as_str().unwrap_or("").to_string(),
                                    total_tokens: a["total_tokens"].as_u64().unwrap_or(0),
                                    cost_usd: a["cost_usd"].as_f64().unwrap_or(0.0),
                                    tool_calls: a["tool_calls"].as_u64().unwrap_or(0),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::UsageByAgentLoaded(agents));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::UsageSummaryLoaded(UsageSummary::default()));
            let _ = tx.send(AppEvent::UsageByModelLoaded(Vec::new()));
            let _ = tx.send(AppEvent::UsageByAgentLoaded(Vec::new()));
        }
    });
}

/// Fetch settings providers.
pub fn spawn_fetch_providers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/providers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    // API returns { "providers": [...], "total": N }
                    let arr = body["providers"].as_array();
                    let providers: Vec<ProviderInfo> = arr
                        .map(|arr| {
                            arr.iter()
                                .map(|p| {
                                    let auth = p["auth_status"].as_str().unwrap_or("missing");
                                    let key_required = p["key_required"].as_bool().unwrap_or(true);
                                    let configured = auth == "configured" || auth == "not_required";
                                    let is_local =
                                        p["is_local"].as_bool().unwrap_or(false) || !key_required;
                                    ProviderInfo {
                                        name: p["id"].as_str().unwrap_or("").to_string(),
                                        configured,
                                        env_var: p["api_key_env"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        is_local,
                                        reachable: if is_local {
                                            p["reachable"].as_bool()
                                        } else {
                                            None
                                        },
                                        latency_ms: if is_local {
                                            p["latency_ms"].as_u64()
                                        } else {
                                            None
                                        },
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SettingsProvidersLoaded(providers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SettingsProvidersLoaded(Vec::new()));
        }
    });
}

/// Fetch settings models.
///
/// `GET /api/models` answers with `{ "models": [...], "total": n, "available": n }`.
/// Reading the body as a bare array — which this did — yields `None` on every
/// call, so the Settings > Models list rendered empty against a healthy daemon.
/// The chat model picker below already reads `body["models"]`; this is the same
/// shape.
/// The cost keys are the API's own (`input_cost_per_m` / `output_cost_per_m`);
/// the `cost_input` / `cost_output` names read here appear nowhere in the
/// response, so the prices column was pinned to `$0.00/$0.00`.
pub fn spawn_fetch_models(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/models")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let models: Vec<ModelInfo> = body["models"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|m| ModelInfo {
                                    id: m["id"].as_str().unwrap_or("").to_string(),
                                    provider: m["provider"].as_str().unwrap_or("").to_string(),
                                    tier: m["tier"].as_str().unwrap_or("").to_string(),
                                    context_window: m["context_window"].as_u64().unwrap_or(0),
                                    cost_input: m["input_cost_per_m"].as_f64().unwrap_or(0.0),
                                    cost_output: m["output_cost_per_m"].as_f64().unwrap_or(0.0),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SettingsModelsLoaded(models));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SettingsModelsLoaded(Vec::new()));
        }
    });
}

/// Fetch the model catalogue for the Models screen, with each entry's effective
/// and catalog-declared capacity limits side by side (refs #7774).
///
/// `context_window` / `max_output_tokens` on the row are the values in force
/// after the operator override; `limits_catalog` carries what the registry or a
/// discovery probe declared. The screen needs both — the difference is what
/// tells an operator a model has been corrected, and what a reset restores.
pub fn spawn_fetch_model_catalog(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            match client.get(format!("{base_url}/api/models")).send() {
                Ok(resp) => {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        let mut models: Vec<ModelRow> = body["models"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .map(|m| ModelRow {
                                        id: m["id"].as_str().unwrap_or("").to_string(),
                                        provider: m["provider"].as_str().unwrap_or("").to_string(),
                                        tier: m["tier"].as_str().unwrap_or("").to_string(),
                                        context_window_effective: m["context_window"]
                                            .as_u64()
                                            .unwrap_or(0),
                                        context_window_catalog: m["limits_catalog"]
                                            ["context_window"]
                                            .as_u64()
                                            .unwrap_or(0),
                                        max_output_tokens_effective: m["max_output_tokens"]
                                            .as_u64()
                                            .unwrap_or(0),
                                        max_output_tokens_catalog: m["limits_catalog"]
                                            ["max_output_tokens"]
                                            .as_u64()
                                            .unwrap_or(0),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        // The catalogue arrives in whatever order the providers
                        // were merged in; a stable sort keeps the cursor from
                        // landing on a different model after a refresh.
                        models.sort_by(|a, b| {
                            (a.provider.as_str(), a.id.as_str())
                                .cmp(&(b.provider.as_str(), b.id.as_str()))
                        });
                        let _ = tx.send(AppEvent::ModelCatalogLoaded(models));
                    }
                }
                Err(_) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                        "tui-event-models-load-failed",
                    )));
                    let _ = tx.send(AppEvent::ModelCatalogLoaded(Vec::new()));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-models-not-available-in-process",
            )));
            let _ = tx.send(AppEvent::ModelCatalogLoaded(Vec::new()));
        }
    });
}

/// Read the stored override document for one model, so a write can merge into
/// it instead of replacing it.
///
/// `PUT /api/models/overrides/{id}` takes a whole `ModelOverrides` document and
/// clears every field the body omits, so submitting only the two capacity
/// limits would silently drop the temperature, top-p and reasoning-effort
/// settings stored under the same key.
fn read_model_overrides(
    client: &reqwest::blocking::Client,
    base_url: &str,
    key: &str,
) -> serde_json::Value {
    client
        .get(format!("{base_url}/api/models/overrides/{key}"))
        .send()
        .ok()
        .and_then(|resp| resp.json::<serde_json::Value>().ok())
        .filter(|doc| doc.is_object())
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Persist one model's operator capacity limits (refs #7774).
///
/// `None` for a limit removes that field, which lets the catalog answer again.
/// Everything else already stored under the key is carried across untouched.
pub fn spawn_save_model_limits(
    backend: BackendRef,
    key: String,
    context_window: Option<u64>,
    max_output_tokens: Option<u64>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let mut doc = read_model_overrides(&client, &base_url, &key);
            set_or_clear(&mut doc, "context_window", context_window);
            set_or_clear(&mut doc, "max_output_tokens", max_output_tokens);
            let outcome = daemon_response(
                client
                    .put(format!("{base_url}/api/models/overrides/{key}"))
                    .json(&doc)
                    .send(),
                || crate::i18n::t_args("tui-event-model-limits-save-failed", &[("model", &key)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::ModelLimitsSaved(key));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-models-not-available-in-process",
            )));
        }
    });
}

/// Drop one model's capacity-limit overrides so the catalog answers again.
///
/// Only the two limit fields are removed: the inference parameters under the
/// same key are a different concern and this screen does not own them. When
/// nothing is left, the whole entry is deleted so the file does not accumulate
/// empty documents.
pub fn spawn_reset_model_limits(backend: BackendRef, key: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let mut doc = read_model_overrides(&client, &base_url, &key);
            set_or_clear(&mut doc, "context_window", None);
            set_or_clear(&mut doc, "max_output_tokens", None);
            let empty = doc.as_object().map(|o| o.is_empty()).unwrap_or(true);
            let outcome = if empty {
                client
                    .delete(format!("{base_url}/api/models/overrides/{key}"))
                    .send()
            } else {
                client
                    .put(format!("{base_url}/api/models/overrides/{key}"))
                    .json(&doc)
                    .send()
            };
            let outcome = daemon_response(outcome, || {
                crate::i18n::t_args("tui-event-model-limits-reset-failed", &[("model", &key)])
            });
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::ModelLimitsReset(key));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-models-not-available-in-process",
            )));
        }
    });
}

/// Write `value` into `doc`, or remove the key entirely when it is `None`.
///
/// Removing rather than writing `null` matters: the field is
/// `skip_serializing_if = "Option::is_none"` on the way out, and leaving a
/// `null` behind would round-trip as a set-but-empty override.
fn set_or_clear(doc: &mut serde_json::Value, field: &str, value: Option<u64>) {
    let Some(obj) = doc.as_object_mut() else {
        return;
    };
    match value {
        Some(v) => {
            obj.insert(field.to_string(), serde_json::json!(v));
        }
        None => {
            obj.remove(field);
        }
    }
}

/// Fetch settings tools.
pub fn spawn_fetch_tools(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/tools")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let tools: Vec<ToolInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|t| ToolInfo {
                                    name: t["name"].as_str().unwrap_or("").to_string(),
                                    description: t["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SettingsToolsLoaded(tools));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SettingsToolsLoaded(Vec::new()));
        }
    });
}

/// Save a provider API key.
pub fn spawn_save_provider_key(
    backend: BackendRef,
    name: String,
    api_key: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon {
            base_url,
            api_key: daemon_api_key,
        } => {
            let client = make_daemon_client(daemon_api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/providers/{name}/key"))
                    .json(&serde_json::json!({"key": api_key}))
                    .send(),
                || crate::i18n::t_args("tui-event-provider-save-key-failed", &[("name", &name)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::ProviderKeySaved(name));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-provider-key-management-not-available-in-process",
            )));
        }
    });
}

/// Delete a provider API key.
pub fn spawn_delete_provider_key(backend: BackendRef, name: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .delete(format!("{base_url}/api/providers/{name}/key"))
                    .send(),
                || crate::i18n::t_args("tui-event-provider-delete-key-failed", &[("name", &name)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::ProviderKeyDeleted(name));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-provider-key-management-not-available-in-process",
            )));
        }
    });
}

/// Fetch the backup archives the daemon holds.
pub fn spawn_fetch_backups(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome =
                daemon_response(client.get(format!("{base_url}/api/backups")).send(), || {
                    crate::i18n::t("tui-event-backups-list-failed")
                });
            match outcome {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let _ = tx.send(AppEvent::BackupsLoaded(parse_backup_list(&body)));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-backups-need-daemon",
            )));
        }
    });
}

/// Read `GET /api/backups` into the rows the settings screen draws.
///
/// `created_at` and `components` are both `null` when the archive's
/// `manifest.json` could not be read, so the file's own `modified_at` is the
/// fallback timestamp and the component list stays empty — which the restore
/// form reads as "restore everything", matching the endpoint's own default.
fn parse_backup_list(body: &serde_json::Value) -> Vec<BackupInfo> {
    body["backups"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .map(|b| BackupInfo {
                    filename: b["filename"].as_str().unwrap_or_default().to_string(),
                    size_bytes: b["size_bytes"].as_u64().unwrap_or(0),
                    created_at: b["created_at"]
                        .as_str()
                        .or_else(|| b["modified_at"].as_str())
                        .unwrap_or_default()
                        .to_string(),
                    components: b["components"]
                        .as_array()
                        .map(|c| {
                            c.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Create a new backup archive.
pub fn spawn_create_backup(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            // A backup walks the whole home directory, so it outlives the
            // default client timeout on any non-trivial install.
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(300));
            let outcome =
                daemon_response(client.post(format!("{base_url}/api/backup")).send(), || {
                    crate::i18n::t("tui-event-backup-create-failed")
                });
            match outcome {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let filename = body["filename"].as_str().unwrap_or_default().to_string();
                    let _ = tx.send(AppEvent::BackupCreated(filename));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-backups-need-daemon",
            )));
        }
    });
}

/// Delete one backup archive.
pub fn spawn_delete_backup(backend: BackendRef, filename: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .delete(format!("{base_url}/api/backups/{filename}"))
                    .send(),
                || {
                    crate::i18n::t_args(
                        "tui-event-backup-delete-failed",
                        &[("filename", &filename)],
                    )
                },
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::BackupDeleted(filename));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-backups-need-daemon",
            )));
        }
    });
}

/// Restore one backup archive.
///
/// `body` is built by `settings::restore_request_body`, which is what decides
/// whether `components` is present at all — the endpoint reads an absent field
/// as "everything" and rejects `[]`, so the shape must not be reassembled here.
pub fn spawn_restore_backup(
    backend: BackendRef,
    body: serde_json::Value,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let filename = body["filename"].as_str().unwrap_or_default().to_string();
        match backend {
            BackendRef::Daemon { base_url, api_key } => {
                // Restoring decompresses and writes the whole archive, so it
                // gets the same generous timeout the create side does.
                let client =
                    make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(300));
                // The daemon's own message carries why (an unknown component
                // name, a missing manifest), and `daemon_response` is what
                // reads it out of either error envelope.
                let outcome = daemon_response(
                    client
                        .post(format!("{base_url}/api/restore"))
                        .json(&body)
                        .send(),
                    || {
                        crate::i18n::t_args(
                            "tui-event-backup-restore-failed",
                            &[("filename", &filename)],
                        )
                    },
                );
                match outcome {
                    Ok(resp) => {
                        let payload: serde_json::Value = resp.json().unwrap_or_default();
                        let _ = tx.send(AppEvent::BackupRestored {
                            filename,
                            restored_files: payload["restored_files"].as_u64().unwrap_or(0),
                            errors: payload["errors"].as_array().map_or(0, |e| e.len()),
                        });
                    }
                    Err(message) => {
                        let _ = tx.send(AppEvent::FetchError(message));
                    }
                }
            }
            BackendRef::InProcess(_) => {
                let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                    "tui-event-backups-need-daemon",
                )));
            }
        }
    });
}

/// Test a provider connection.
pub fn spawn_test_provider(backend: BackendRef, name: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client =
                make_daemon_client_with_timeout(api_key.as_deref(), Duration::from_secs(15));
            let start = std::time::Instant::now();
            match client
                .post(format!("{base_url}/api/providers/{name}/test"))
                .send()
            {
                Ok(resp) => {
                    let latency = start.elapsed().as_millis() as u64;
                    let success = resp.status().is_success();
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let message = body["message"]
                        .as_str()
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            if success {
                                crate::i18n::t("tui-event-provider-connection-ok")
                            } else {
                                crate::i18n::t("tui-event-provider-test-failed")
                            }
                        });
                    let _ = tx.send(AppEvent::ProviderTestResult(TestResult {
                        provider: name,
                        success,
                        latency_ms: latency,
                        message,
                    }));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::ProviderTestResult(TestResult {
                        provider: name,
                        success: false,
                        latency_ms: 0,
                        message: format!("{e}"),
                    }));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::ProviderTestResult(TestResult {
                provider: name,
                success: false,
                latency_ms: 0,
                message: crate::i18n::t("tui-event-provider-test-not-available-in-process"),
            }));
        }
    });
}

/// Fetch user groups (#7745).
///
/// `GET /api/groups` is Admin-or-above; the write verbs are Owner-only and the
/// screen does not offer them, so a Viewer-scoped TUI degrades to an empty list
/// rather than a wall of permission errors.
pub fn spawn_fetch_groups(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/groups")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let groups: Vec<GroupInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|g| GroupInfo {
                                    name: g["name"].as_str().unwrap_or("").to_string(),
                                    description: g["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    member_count: g["member_count"].as_u64().unwrap_or(0),
                                    roles: g["roles"]
                                        .as_array()
                                        .map(|r| {
                                            r.iter()
                                                .filter_map(|v| v.as_str())
                                                .collect::<Vec<_>>()
                                                .join(",")
                                        })
                                        .unwrap_or_default(),
                                    has_unregistered_members: g["unknown_members"]
                                        .as_array()
                                        .map(|u| !u.is_empty())
                                        .unwrap_or(false),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::GroupsLoaded(groups));
                }
            }
        }
        // Groups live in `config.toml`, which the in-process backend has no
        // HTTP surface for; the daemon path is the only one that can answer.
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::GroupsLoaded(Vec::new()));
        }
    });
}

/// Fetch peers.
pub fn spawn_fetch_peers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/peers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let peers: Vec<PeerInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|p| PeerInfo {
                                    node_id: p["node_id"].as_str().unwrap_or("").to_string(),
                                    node_name: p["node_name"].as_str().unwrap_or("").to_string(),
                                    address: p["address"].as_str().unwrap_or("").to_string(),
                                    state: p["state"].as_str().unwrap_or("").to_string(),
                                    agent_count: p["agent_count"].as_u64().unwrap_or(0),
                                    protocol_version: p["protocol_version"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::PeersLoaded(peers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::PeersLoaded(Vec::new()));
        }
    });
}

/// Fetch log entries (uses audit endpoint, polled frequently).
pub fn spawn_fetch_logs(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/audit/recent?n=200"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries: Vec<LogEntry> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|e| {
                                    let action = e["action"].as_str().unwrap_or("").to_string();
                                    let detail = e["detail"].as_str().unwrap_or("").to_string();
                                    let level =
                                        super::screens::logs::classify_level(&action, &detail);
                                    LogEntry {
                                        timestamp: e["timestamp"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        level,
                                        action,
                                        detail,
                                        agent: e["agent"].as_str().unwrap_or("").to_string(),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::LogsLoaded(entries));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::LogsLoaded(Vec::new()));
        }
    });
}

// ── Hands events ────────────────────────────────────────────────────────────

/// Fetch hand definitions (marketplace).
pub fn spawn_fetch_hands(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/hands")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let hands: Vec<HandInfo> = body["hands"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|h| HandInfo {
                                    id: h["id"].as_str().unwrap_or("").to_string(),
                                    name: h["name"].as_str().unwrap_or("").to_string(),
                                    description: h["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    category: h["category"].as_str().unwrap_or("").to_string(),
                                    icon: h["icon"].as_str().unwrap_or("").to_string(),
                                    requirements_met: h["requirements_met"]
                                        .as_bool()
                                        .unwrap_or(false),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::HandsLoaded(hands));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let defs = kernel.hand_registry_ref().list_definitions();
            let hands: Vec<HandInfo> = defs
                .iter()
                .map(|d| {
                    let reqs_met = kernel
                        .hand_registry_ref()
                        .check_requirements(&d.id)
                        .map(|r| r.iter().all(|(_, ok)| *ok))
                        .unwrap_or(false);
                    HandInfo {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        description: d.description.clone(),
                        category: d.category.to_string(),
                        icon: d.icon.clone(),
                        requirements_met: reqs_met,
                    }
                })
                .collect();
            let _ = tx.send(AppEvent::HandsLoaded(hands));
        }
    });
}

/// Fetch active hand instances.
pub fn spawn_fetch_active_hands(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/hands/active")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let instances: Vec<HandInstanceInfo> = body["instances"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|i| HandInstanceInfo {
                                    instance_id: i["instance_id"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    hand_id: i["hand_id"].as_str().unwrap_or("").to_string(),
                                    status: i["status"].as_str().unwrap_or("").to_string(),
                                    agent_name: i["agent_name"].as_str().unwrap_or("").to_string(),
                                    agent_id: i["agent_id"].as_str().unwrap_or("").to_string(),
                                    activated_at: i["activated_at"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::ActiveHandsLoaded(instances));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let instances: Vec<HandInstanceInfo> = kernel
                .hand_registry_ref()
                .list_instances()
                .iter()
                .map(|i| HandInstanceInfo {
                    instance_id: i.instance_id.to_string(),
                    hand_id: i.hand_id.clone(),
                    status: i.status.to_string(),
                    agent_name: i.agent_name().to_string(),
                    agent_id: i.agent_id().map(|a| a.to_string()).unwrap_or_default(),
                    activated_at: i.activated_at.to_rfc3339(),
                })
                .collect();
            let _ = tx.send(AppEvent::ActiveHandsLoaded(instances));
        }
    });
}

/// Activate a hand.
pub fn spawn_activate_hand(backend: BackendRef, hand_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/hands/{hand_id}/activate"))
                    .json(&serde_json::json!({}))
                    .send(),
                || crate::i18n::t("tui-event-hand-activation-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::HandActivated(hand_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            match kernel.activate_hand(&hand_id, std::collections::HashMap::new()) {
                Ok(_) => {
                    let _ = tx.send(AppEvent::HandActivated(hand_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-hand-activation-failed-error",
                        &[("error", &e.to_string())],
                    )));
                }
            }
        }
    });
}

/// Deactivate a hand instance.
pub fn spawn_deactivate_hand(backend: BackendRef, instance_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .delete(format!("{base_url}/api/hands/instances/{instance_id}"))
                    .send(),
                || {
                    crate::i18n::t_args(
                        "tui-event-hand-deactivate-failed",
                        &[("instance_id", &instance_id)],
                    )
                },
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::HandDeactivated(instance_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => match uuid::Uuid::parse_str(&instance_id) {
            Ok(uuid) => match kernel.deactivate_hand(uuid) {
                Ok(()) => {
                    let _ = tx.send(AppEvent::HandDeactivated(instance_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-hand-deactivate-failed-error",
                        &[("error", &e.to_string())],
                    )));
                }
            },
            Err(e) => {
                let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                    "tui-event-hand-invalid-instance-id",
                    &[("error", &e.to_string())],
                )));
            }
        },
    });
}

/// Pause a hand instance.
pub fn spawn_pause_hand(backend: BackendRef, instance_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!(
                        "{base_url}/api/hands/instances/{instance_id}/pause"
                    ))
                    .send(),
                || {
                    crate::i18n::t_args(
                        "tui-event-hand-pause-failed",
                        &[("instance_id", &instance_id)],
                    )
                },
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::HandPaused(instance_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => match uuid::Uuid::parse_str(&instance_id) {
            Ok(uuid) => match kernel.pause_hand(uuid) {
                Ok(()) => {
                    let _ = tx.send(AppEvent::HandPaused(instance_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-hand-pause-failed-error",
                        &[("error", &e.to_string())],
                    )));
                }
            },
            Err(e) => {
                let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                    "tui-event-hand-invalid-instance-id",
                    &[("error", &e.to_string())],
                )));
            }
        },
    });
}

/// Resume a hand instance.
pub fn spawn_resume_hand(backend: BackendRef, instance_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!(
                        "{base_url}/api/hands/instances/{instance_id}/resume"
                    ))
                    .send(),
                || {
                    crate::i18n::t_args(
                        "tui-event-hand-resume-failed",
                        &[("instance_id", &instance_id)],
                    )
                },
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::HandResumed(instance_id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(kernel) => match uuid::Uuid::parse_str(&instance_id) {
            Ok(uuid) => match kernel.resume_hand(uuid) {
                Ok(()) => {
                    let _ = tx.send(AppEvent::HandResumed(instance_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                        "tui-event-hand-resume-failed-error",
                        &[("error", &e.to_string())],
                    )));
                }
            },
            Err(e) => {
                let _ = tx.send(AppEvent::FetchError(crate::i18n::t_args(
                    "tui-event-hand-invalid-instance-id",
                    &[("error", &e.to_string())],
                )));
            }
        },
    });
}

// ── Extension spawn functions ───────────────────────────────────────────────

/// Fetch all extensions (available + installed).
pub fn spawn_fetch_extensions(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/mcp/catalog")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let extensions: Vec<ExtensionInfo> = body["entries"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|e| {
                                    let id = e["id"].as_str().unwrap_or("").to_string();
                                    let installed = e["installed"].as_bool().unwrap_or(false);
                                    ExtensionInfo {
                                        id: id.clone(),
                                        name: e["name"].as_str().unwrap_or("").to_string(),
                                        description: e["description"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        icon: e["icon"].as_str().unwrap_or("").to_string(),
                                        category: e["category"].as_str().unwrap_or("").to_string(),
                                        installed,
                                        status: if installed {
                                            "installed".to_string()
                                        } else {
                                            "available".to_string()
                                        },
                                        tags: e["tags"]
                                            .as_array()
                                            .map(|t| {
                                                t.iter()
                                                    .filter_map(|v| v.as_str().map(String::from))
                                                    .collect()
                                            })
                                            .unwrap_or_default(),
                                        has_oauth: e["has_oauth"].as_bool().unwrap_or(false),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::ExtensionsLoaded(extensions));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let installed_ids: std::collections::HashSet<String> = kernel
                .config_ref()
                .mcp_servers
                .iter()
                .filter_map(|s| s.template_id.clone())
                .collect();
            let catalog = kernel.mcp_catalog_load();
            let extensions: Vec<ExtensionInfo> = catalog
                .list()
                .iter()
                .map(|t| {
                    let installed = installed_ids.contains(&t.id);
                    ExtensionInfo {
                        id: t.id.clone(),
                        name: t.name.clone(),
                        description: t.description.clone(),
                        icon: t.icon.clone(),
                        category: t.category.to_string(),
                        installed,
                        status: if installed {
                            "installed".to_string()
                        } else {
                            "available".to_string()
                        },
                        tags: t.tags.clone(),
                        has_oauth: t.oauth.is_some(),
                    }
                })
                .collect();
            let _ = tx.send(AppEvent::ExtensionsLoaded(extensions));
        }
    });
}

/// Fetch extension health data.
pub fn spawn_fetch_extension_health(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            if let Ok(resp) = client.get(format!("{base_url}/api/mcp/health")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries: Vec<ExtensionHealthInfo> = body["health"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|h| ExtensionHealthInfo {
                                    id: h["id"].as_str().unwrap_or("").to_string(),
                                    status: h["status"].as_str().unwrap_or("").to_string(),
                                    tool_count: h["tool_count"].as_u64().unwrap_or(0) as usize,
                                    last_ok: h["last_ok"].as_str().unwrap_or("").to_string(),
                                    last_error: h["last_error"].as_str().unwrap_or("").to_string(),
                                    consecutive_failures: h["consecutive_failures"]
                                        .as_u64()
                                        .unwrap_or(0)
                                        as u32,
                                    reconnecting: h["reconnecting"].as_bool().unwrap_or(false),
                                    connected_since: h["connected_since"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::ExtensionHealthLoaded(entries));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let health = kernel.health().all_health();
            let entries: Vec<ExtensionHealthInfo> = health
                .iter()
                .map(|h| ExtensionHealthInfo {
                    id: h.id.clone(),
                    status: h.status.to_string(),
                    tool_count: h.tool_count,
                    last_ok: h.last_ok.map(|t| t.to_rfc3339()).unwrap_or_default(),
                    last_error: h.last_error.clone().unwrap_or_default(),
                    consecutive_failures: h.consecutive_failures,
                    reconnecting: h.reconnecting,
                    connected_since: h
                        .connected_since
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_default(),
                })
                .collect();
            let _ = tx.send(AppEvent::ExtensionHealthLoaded(entries));
        }
    });
}

/// Install an extension.
pub fn spawn_install_extension(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/mcp/servers"))
                    .json(&serde_json::json!({"template_id": id}))
                    .send(),
                || crate::i18n::t_args("tui-event-extension-install-failed", &[("id", &id)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::ExtensionInstalled(id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-extension-install-not-supported",
            )));
        }
    });
}

/// Remove an extension.
///
/// Routes through `/api/extensions/uninstall` rather than
/// `DELETE /api/mcp/servers/{name}` because the UI list carries the
/// catalog `entry.id` (template_id), and that can diverge from the
/// configured server name (user renamed it, or the catalog entry id
/// doesn't match the final server name). The extensions endpoint
/// resolves either form; the MCP endpoint only accepts the exact name.
pub fn spawn_remove_extension(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/extensions/uninstall"))
                    .json(&serde_json::json!({ "name": id }))
                    .send(),
                || crate::i18n::t_args("tui-event-extension-remove-failed", &[("id", &id)]),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::ExtensionRemoved(id));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-extension-remove-not-supported",
            )));
        }
    });
}

/// Reconnect an extension's MCP server.
pub fn spawn_reconnect_extension(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/mcp/servers/{id}/reconnect"))
                    .send(),
                || crate::i18n::t_args("tui-event-extension-reconnect-failed", &[("id", &id)]),
            );
            match outcome {
                Ok(resp) => {
                    let tool_count = resp
                        .json::<serde_json::Value>()
                        .ok()
                        .and_then(|b| b["tool_count"].as_u64())
                        .unwrap_or(0) as usize;
                    let _ = tx.send(AppEvent::ExtensionReconnected(id, tool_count));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::FetchError(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(crate::i18n::t(
                "tui-event-extension-reconnect-not-supported",
            )));
        }
    });
}

/// Fetch comms topology + events.
pub fn spawn_fetch_comms(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    use super::screens::comms::{CommsEdge, CommsEventItem, CommsNode};

    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            // Fetch topology
            if let Ok(resp) = client.get(format!("{base_url}/api/comms/topology")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let nodes: Vec<CommsNode> = body["nodes"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|n| CommsNode {
                                    id: n["id"].as_str().unwrap_or("").to_string(),
                                    name: n["name"].as_str().unwrap_or("").to_string(),
                                    state: n["state"].as_str().unwrap_or("").to_string(),
                                    model: n["model"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let edges: Vec<CommsEdge> = body["edges"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|e| CommsEdge {
                                    from: e["from"].as_str().unwrap_or("").to_string(),
                                    to: e["to"].as_str().unwrap_or("").to_string(),
                                    kind: e["kind"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::CommsTopologyLoaded { nodes, edges });
                }
            }
            // Fetch events
            if let Ok(resp) = client
                .get(format!("{base_url}/api/comms/events?limit=100"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let events: Vec<CommsEventItem> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|e| CommsEventItem {
                                    id: e["id"].as_str().unwrap_or("").to_string(),
                                    timestamp: e["timestamp"].as_str().unwrap_or("").to_string(),
                                    kind: e["kind"].as_str().unwrap_or("").to_string(),
                                    source_name: e["source_name"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    target_name: e["target_name"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    detail: e["detail"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::CommsEventsLoaded(events));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::CommsTopologyLoaded {
                nodes: Vec::new(),
                edges: Vec::new(),
            });
            let _ = tx.send(AppEvent::CommsEventsLoaded(Vec::new()));
        }
    });
}

/// Send a message between agents via comms endpoint.
pub fn spawn_comms_send(
    backend: BackendRef,
    from: String,
    to: String,
    msg: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let body = serde_json::json!({
                "from_agent_id": from,
                "to_agent_id": to,
                "message": msg,
            });
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/comms/send"))
                    .json(&body)
                    .send(),
                || crate::i18n::t("tui-event-comms-send-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::CommsSendResult(crate::i18n::t(
                        "tui-event-comms-message-sent",
                    )));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::CommsSendResult(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::CommsSendResult(crate::i18n::t(
                "tui-event-comms-send-not-supported-in-process",
            )));
        }
    });
}

/// Post a task via comms endpoint.
pub fn spawn_comms_task(
    backend: BackendRef,
    title: String,
    desc: String,
    assign: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon { base_url, api_key } => {
            let client = make_daemon_client(api_key.as_deref());
            let mut body = serde_json::json!({
                "title": title,
                "description": desc,
            });
            if !assign.is_empty() {
                body["assigned_to"] = serde_json::Value::String(assign);
            }
            let outcome = daemon_response(
                client
                    .post(format!("{base_url}/api/comms/task"))
                    .json(&body)
                    .send(),
                || crate::i18n::t("tui-event-comms-post-failed"),
            );
            match outcome {
                Ok(_) => {
                    let _ = tx.send(AppEvent::CommsTaskResult(crate::i18n::t(
                        "tui-event-comms-task-posted",
                    )));
                }
                Err(message) => {
                    let _ = tx.send(AppEvent::CommsTaskResult(message));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::CommsTaskResult(crate::i18n::t(
                "tui-event-comms-post-not-supported-in-process",
            )));
        }
    });
}

/// Fetch the model label for a daemon agent (used when entering chat).
/// Sends `ChatModelLabelLoaded` so the event loop can update `chat.model_label`
/// without blocking the render/input thread.
pub fn spawn_fetch_agent_model_label(
    base_url: String,
    agent_id: String,
    api_key: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let client = make_daemon_client(api_key.as_deref());
        if let Ok(resp) = client
            .get(format!("{base_url}/api/agents/{agent_id}"))
            .send()
        {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                let provider = body["model_provider"].as_str().unwrap_or("?");
                let model = body["model_name"].as_str().unwrap_or("?");
                let label = format!("{provider}/{model}");
                let _ = tx.send(AppEvent::ChatModelLabelLoaded { agent_id, label });
            }
        }
    });
}

/// Fetch the model list from the daemon for the chat model picker.
/// Sends `ChatModelsForPicker` so the event loop can open the picker
/// without blocking the render/input thread.
pub fn spawn_fetch_models_for_picker(
    base_url: String,
    api_key: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let client = make_daemon_client(api_key.as_deref());
        if let Ok(resp) = client.get(format!("{base_url}/api/models")).send() {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                let models: Vec<super::screens::chat::ModelEntry> = body["models"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter(|m| m["available"].as_bool().unwrap_or(false))
                            .map(|m| super::screens::chat::ModelEntry {
                                id: m["id"].as_str().unwrap_or("").to_string(),
                                display_name: m["display_name"].as_str().unwrap_or("").to_string(),
                                provider: m["provider"].as_str().unwrap_or("").to_string(),
                                tier: m["tier"].as_str().unwrap_or("Balanced").to_string(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = tx.send(AppEvent::ChatModelsForPicker(models));
            }
        }
    });
}

/// Fetch the agent list from the daemon for the /agents chat command.
/// Sends `ChatAgentListLoaded` so the event loop can push the reply
/// without blocking the render/input thread.
pub fn spawn_fetch_agents_for_chat(
    base_url: String,
    api_key: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let client = make_daemon_client(api_key.as_deref());
        if let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() {
            if let Ok(body) = resp.json::<serde_json::Value>() {
                let arr = if let Some(arr) = body.as_array() {
                    arr.clone()
                } else if let Some(items) = body.get("items").and_then(|v| v.as_array()) {
                    items.clone()
                } else {
                    Vec::new()
                };
                let lines: Vec<String> = arr
                    .iter()
                    .map(|a| {
                        format!(
                            "{} [{}] {}",
                            a["name"].as_str().unwrap_or("?"),
                            a["state"].as_str().unwrap_or("?"),
                            a["model_name"].as_str().unwrap_or("?"),
                        )
                    })
                    .collect();
                let _ = tx.send(AppEvent::ChatAgentListLoaded(lines));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    /// `GET /api/channels` mixes configured instances and catalog adapters in
    /// one array, and `name` means a different thing in each. Getting the
    /// split wrong is the #8055 / #8063 identifier mix-up, so it is pinned
    /// here rather than left to the caller.
    #[test]
    fn the_channel_listing_splits_instances_from_catalog_adapters() {
        let body = serde_json::json!({
            "items": [
                {
                    "name": "telegram-support",
                    "channel_type": "telegram",
                    "configured": true,
                    "agent": "support-bot",
                    "supervised": true,
                    "connected": true,
                    "messages_received": 12,
                    "messages_sent": 7,
                    "last_error": "circuit break at 03:12",
                    "fields": [
                        {
                            "key": "TELEGRAM_BOT_TOKEN",
                            "label": "Bot Token",
                            "type": "secret",
                            "required": true,
                            "has_value": true
                        },
                        {
                            "key": "ALLOWED_USERS",
                            "label": "Allowed users",
                            "type": "list",
                            "advanced": true,
                            "value": "1,2"
                        }
                    ]
                },
                {
                    "name": "ntfy",
                    "display_name": "ntfy",
                    "configured": false,
                    "fields": [],
                    "schema_error": "librefang-sdk not installed"
                },
                {
                    "name": "feishu",
                    "display_name": "Feishu / Lark",
                    "configured": false,
                    "fields": [{
                        "key": "FEISHU_REGION",
                        "label": "Region",
                        "type": "select",
                        "options": ["cn", "intl"]
                    }]
                }
            ],
            "total": 3
        });

        let (instances, adapters) = parse_channel_list(&body);

        assert_eq!(instances.len(), 1);
        let instance = &instances[0];
        assert_eq!(instance.name, "telegram-support");
        assert_eq!(
            instance.adapter, "telegram",
            "the adapter comes from channel_type, never from the instance name"
        );
        assert_eq!(instance.agent.as_deref(), Some("support-bot"));
        assert_eq!(instance.messages_received, 12);
        assert_eq!(instance.messages_sent, 7);
        assert_eq!(
            instance.health(),
            crate::tui::screens::channels::InstanceHealth::Degraded,
            "a connected instance with a sticky error is degraded, not dead"
        );
        // The secret is flagged as set but never carries a value.
        assert!(instance.fields[0].has_value);
        assert!(instance.fields[0].value.is_empty());
        assert_eq!(instance.fields[1].value, "1,2");
        assert!(instance.fields[1].advanced);

        // Catalog rows are sorted, so the picker order does not depend on the
        // daemon's declaration order.
        let names: Vec<&str> = adapters.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, vec!["feishu", "ntfy"]);
        assert_eq!(adapters[0].display_name, "Feishu / Lark");
        assert_eq!(adapters[0].fields[0].options, vec!["cn", "intl"]);
        assert_eq!(
            adapters[1].schema_error.as_deref(),
            Some("librefang-sdk not installed")
        );
    }

    /// A configured entry may omit `channel_type`, and the daemon then reads
    /// its name as the type — the TUI has to agree or the configure path for
    /// that instance would be empty.
    #[test]
    fn a_configured_row_without_a_channel_type_uses_its_name_as_the_adapter() {
        let body = serde_json::json!({
            "items": [{ "name": "ntfy", "configured": true, "fields": [] }]
        });
        let (instances, adapters) = parse_channel_list(&body);
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].adapter, "ntfy");
        assert!(adapters.is_empty());
    }

    /// A row the daemon could not name is unusable in either direction: it
    /// cannot be deleted and it cannot be configured. Dropping it beats
    /// rendering a blank line the cursor can land on.
    #[test]
    fn nameless_and_shapeless_rows_are_dropped() {
        let body = serde_json::json!({
            "items": [
                { "configured": true, "fields": [] },
                { "name": "", "configured": false },
                {
                    "name": "telegram",
                    "configured": true,
                    "fields": [{ "label": "no key here", "type": "text" }]
                }
            ]
        });
        let (instances, adapters) = parse_channel_list(&body);
        assert_eq!(instances.len(), 1, "only the named row survives");
        assert_eq!(instances[0].name, "telegram");
        assert!(
            instances[0].fields.is_empty(),
            "a schema field with no key cannot be sent, so it is not offered"
        );
        assert!(adapters.is_empty());
        // Absent liveness keys read as "not supervised" rather than panicking.
        assert!(!instances[0].supervised);
    }

    /// An empty or malformed body must not panic the render loop.
    #[test]
    fn a_missing_items_array_yields_two_empty_lists() {
        let (instances, adapters) = parse_channel_list(&serde_json::json!({}));
        assert!(instances.is_empty());
        assert!(adapters.is_empty());
    }

    /// `GET /api/backups` nests its rows under `backups`, and the settings
    /// screen reads `created_at` and `components` straight out of each one.
    #[test]
    fn backup_rows_parse_out_of_the_listing_envelope() {
        let body = serde_json::json!({
            "backups": [{
                "filename": "librefang-backup-20260101-000000.zip",
                "path": "/home/u/.librefang/backups/librefang-backup-20260101-000000.zip",
                "size_bytes": 8192,
                "modified_at": "2026-01-02T00:00:00Z",
                "components": ["config", "skills"],
                "librefang_version": "2026.8.19",
                "created_at": "2026-01-01T00:00:00Z"
            }],
            "total": 1
        });
        let rows = parse_backup_list(&body);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].filename, "librefang-backup-20260101-000000.zip");
        assert_eq!(rows[0].size_bytes, 8192);
        assert_eq!(
            rows[0].created_at, "2026-01-01T00:00:00Z",
            "the manifest timestamp must win over the file's mtime"
        );
        assert_eq!(rows[0].components, vec!["config", "skills"]);
    }

    /// An archive whose `manifest.json` could not be read reports `created_at`
    /// and `components` as `null`. Falling back to the file's own mtime keeps
    /// the row dated, and the empty component list is what makes the restore
    /// form omit `components` — the endpoint's own "restore everything".
    #[test]
    fn a_manifestless_row_falls_back_to_the_file_mtime() {
        let body = serde_json::json!({
            "backups": [{
                "filename": "hand-rolled.zip",
                "size_bytes": 10,
                "modified_at": "2026-03-04T05:06:07Z",
                "components": null,
                "librefang_version": null,
                "created_at": null
            }],
            "total": 1
        });
        let rows = parse_backup_list(&body);
        assert_eq!(rows[0].created_at, "2026-03-04T05:06:07Z");
        assert!(rows[0].components.is_empty());
    }

    #[test]
    fn an_empty_listing_parses_to_no_rows() {
        let rows = parse_backup_list(&serde_json::json!({"backups": [], "total": 0}));
        assert!(rows.is_empty());
    }

    /// The workflow creator's raw `steps` field must reach the API as a JSON
    /// array. It used to be forwarded as a JSON string, which
    /// `create_workflow` rejects with `Missing 'steps' array` — the wizard
    /// could not create anything.
    #[test]
    fn workflow_steps_json_parses_into_an_array() {
        let steps = parse_workflow_steps_json(
            r#"[{"name":"draft","agent_name":"writer","prompt":"{{input}}","session_mode":"new"}]"#,
        )
        .expect("a JSON array must parse");
        let array = steps.as_array().expect("must stay an array");
        assert_eq!(array.len(), 1);
        assert_eq!(array[0]["agent_name"], "writer");
        assert_eq!(
            array[0]["session_mode"], "new",
            "the per-step session override must survive the parse untouched"
        );
    }

    /// The Workflows screen used to read `output` and nothing else, so the generic "completed" line was printed for a 202 that had not finished and for a 422 that had failed.
    /// A failure must never render as a completion.
    #[test]
    fn a_failed_run_is_not_rendered_as_completed() {
        let message = workflow_run_result_message(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            &serde_json::json!({"error": "workflow_failed", "detail": "step 'draft' timed out"}),
        );

        assert!(
            message.contains("step 'draft' timed out"),
            "the run's reason must reach the screen: {message}"
        );
        assert_ne!(
            message,
            crate::i18n::t("tui-event-workflow-completed"),
            "a failed run must not render as a completed one"
        );
    }

    #[test]
    fn an_accepted_run_names_the_run_id_to_poll() {
        let message = workflow_run_result_message(
            reqwest::StatusCode::ACCEPTED,
            &serde_json::json!({"run_id": "3f1c-run", "status": "running"}),
        );

        assert!(
            message.contains("3f1c-run"),
            "a still-running launch must name the run so it stays traceable: {message}"
        );
        assert_ne!(
            message,
            crate::i18n::t("tui-event-workflow-completed"),
            "a run that has not finished must not claim to have completed"
        );
    }

    #[test]
    fn a_finished_run_shows_its_output_verbatim() {
        let message = workflow_run_result_message(
            reqwest::StatusCode::OK,
            &serde_json::json!({"run_id": "3f1c-run", "output": "the summary"}),
        );

        assert_eq!(message, "the summary");
    }

    #[test]
    fn workflow_steps_json_tolerates_surrounding_whitespace() {
        assert!(parse_workflow_steps_json("  [ ]  \n").is_ok());
    }

    #[test]
    fn workflow_steps_json_rejects_the_shapes_the_api_would_reject() {
        assert_eq!(
            parse_workflow_steps_json("   "),
            Err(StepsJsonError::Empty),
            "an untouched field must be named as empty, not sent as a doomed request"
        );
        assert_eq!(
            parse_workflow_steps_json(r#"{"name":"draft"}"#),
            Err(StepsJsonError::NotArray),
            "a bare step object is the likeliest typo and must be caught here"
        );
        assert!(
            matches!(
                parse_workflow_steps_json("[{name: draft}]"),
                Err(StepsJsonError::NotJson(_))
            ),
            "malformed JSON must carry serde's message rather than a bare failure"
        );
    }

    /// The TUI's sweep loops must survive the call that spawned them.
    ///
    /// The original bug had two halves: `spawn_approval_sweep_task` was called from the runtime-free TUI main thread (panic), and the only runtime in sight — the throwaway one in `spawn_kernel_boot` — was dropped when the boot thread returned, which would have aborted the loop anyway.
    /// This pins the second half: a task spawned under the guard keeps running after the guard is dropped.
    #[test]
    fn tui_runtime_tasks_outlive_the_spawning_scope() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_task = Arc::clone(&flag);

        {
            let _guard = tui_runtime().enter();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                flag_task.store(true, Ordering::SeqCst);
            });
        } // guard dropped here — a throwaway runtime would abort the task

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !flag.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            flag.load(Ordering::SeqCst),
            "task spawned on the TUI runtime was aborted after its spawning scope ended"
        );
    }

    /// Tasks detached *during* a `block_on_tui` call must survive its return.
    ///
    /// This is the `/new` data-loss bug in miniature.
    /// `reset_session` is async and internally does `Handle::try_current()` + `handle.spawn(...)` to write the session summary off the return path.
    /// When the TUI drove it with a throwaway `Runtime::new()`, that runtime dropped as soon as `block_on` returned and took the not-yet-finished summary write with it — no panic, no warning, just a missing summary.
    #[test]
    fn block_on_tui_detached_tasks_survive_the_call() {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_task = Arc::clone(&flag);

        block_on_tui(async move {
            // Same shape as save_session_summary: detach and return immediately.
            tokio::runtime::Handle::current().spawn(async move {
                tokio::time::sleep(Duration::from_millis(80)).await;
                flag_task.store(true, Ordering::SeqCst);
            });
        });

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !flag.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            flag.load(Ordering::SeqCst),
            "task detached during block_on_tui was aborted when the call returned"
        );
    }

    /// The ClawHub index serves the same skill under more than one display
    /// casing. Install addresses a skill by slug, so those rows are one skill.
    #[test]
    fn the_index_yields_one_row_per_slug_whatever_its_casing() {
        let body = serde_json::json!({
            "items": [
                {"name": "Prd", "slug": "Prd", "description": "first", "downloads": 9, "runtime": "python"},
                {"name": "prd", "slug": "prd", "description": "second", "downloads": 4, "runtime": "python"},
                {"name": "other", "slug": "other", "description": "third", "downloads": 1, "runtime": "node"}
            ]
        });

        let results = parse_clawhub_results(&body);

        assert_eq!(
            results.len(),
            2,
            "`Prd` and `prd` are one skill; the list showed it twice"
        );
        // First-wins: the index comes back in relevance order, so the row the
        // server ranked highest is the one kept.
        assert_eq!(results[0].slug, "Prd");
        assert_eq!(results[0].description, "first");
        assert_eq!(results[1].slug, "other");
    }

    /// Deduping must not reach across slugs — two genuinely different skills
    /// that merely share a display name are still two rows.
    #[test]
    fn distinct_slugs_survive_even_when_their_names_collide() {
        let body = serde_json::json!({
            "items": [
                {"name": "deploy", "slug": "deploy-k8s", "description": "a", "downloads": 2, "runtime": "python"},
                {"name": "deploy", "slug": "deploy-nomad", "description": "b", "downloads": 3, "runtime": "python"}
            ]
        });

        let results = parse_clawhub_results(&body);

        assert_eq!(results.len(), 2);
    }

    /// The bare-array shape (no `items` envelope) goes through the same path.
    #[test]
    fn the_bare_array_shape_is_deduped_too() {
        let body = serde_json::json!([
            {"name": "Prd", "slug": "PRD", "description": "a", "downloads": 1, "runtime": "python"},
            {"name": "prd", "slug": "prd", "description": "b", "downloads": 1, "runtime": "python"}
        ]);

        assert_eq!(parse_clawhub_results(&body).len(), 1);
    }

    /// A rejected ClawHub install comes back as a 4xx whose body names the real
    /// fault, and that name is the whole diagnosis. Discarding it costs the
    /// operator a debugging round trip against their own installation for a
    /// fault the daemon already located in someone else's skill.
    #[test]
    fn a_rejected_install_reports_the_reason_the_daemon_gave() {
        let detail = daemon_error_detail(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            Some(&serde_json::json!({"error": "YAML parse error at line 3"})),
        );

        assert_eq!(detail, "YAML parse error at line 3");
    }

    /// Most of this module's endpoints answer with `ApiErrorResponse`, which
    /// nests the reason at `error.message` instead of putting a bare string at
    /// `error`. Reading only the bare-string shape left every backup, session,
    /// memory-KV, model-override, provider-key and hand failure reporting a
    /// status code and nothing else.
    #[test]
    fn the_standard_api_error_envelope_is_read() {
        let detail = daemon_error_detail(
            reqwest::StatusCode::CONFLICT,
            Some(&serde_json::json!({
                "error": {
                    "code": "delete_confirmation_required",
                    "message": "Re-issue with confirm=true to proceed.",
                    "request_id": "abc",
                },
                "message": "Re-issue with confirm=true to proceed.",
                "code": "delete_confirmation_required",
            })),
        );

        assert_eq!(detail, "Re-issue with confirm=true to proceed.");
    }

    /// The flat `message` mirror is the same envelope seen by a client that
    /// only got the compatibility half, so it counts as the daemon's verdict
    /// too.
    #[test]
    fn the_flat_message_mirror_is_read() {
        let detail = daemon_error_detail(
            reqwest::StatusCode::FORBIDDEN,
            Some(&serde_json::json!({"message": "Only the owner may write this key"})),
        );

        assert_eq!(detail, "Only the owner may write this key");
    }

    /// With no reason in any of the shapes the daemon uses, the status line is
    /// the only thing actually known, so it has to be reported rather than
    /// replaced by an invented cause — and it carries the reason phrase, which
    /// is free and is half the diagnosis.
    #[test]
    fn a_body_without_a_reason_falls_back_to_the_status() {
        let detail = daemon_error_detail(
            reqwest::StatusCode::BAD_GATEWAY,
            Some(&serde_json::json!({"upstream": "clawhub", "retryable": true})),
        );

        assert!(
            detail.contains("502"),
            "the status is all that is known and must survive: {detail}"
        );
        assert!(
            detail.contains("Bad Gateway"),
            "the reason phrase is free and belongs on the line: {detail}"
        );
        assert!(
            !detail.contains("clawhub"),
            "only the error envelope is the daemon's verdict; nothing else may be read as one: {detail}"
        );
    }

    /// A reason is not always the daemon's own text — a rejected ClawHub
    /// install echoes the registry's `SkillError` — so a multi-line or
    /// control-character body must not reach the terminal buffer intact.
    #[test]
    fn a_multiline_reason_is_flattened_onto_one_line() {
        let detail = daemon_error_detail(
            reqwest::StatusCode::UNPROCESSABLE_ENTITY,
            Some(&serde_json::json!({"error": "YAML parse error\n\tat line 3\u{1b}[31m"})),
        );

        assert_eq!(detail, "YAML parse error at line 3 [31m");
    }

    /// An empty or blank `error` is worse than no field at all — it renders as
    /// a failure line that trails off — so it takes the status fallback too.
    #[test]
    fn a_blank_error_field_falls_back_to_the_status() {
        assert!(daemon_error_detail(
            reqwest::StatusCode::BAD_REQUEST,
            Some(&serde_json::json!({"error": "   "})),
        )
        .contains("400"));
    }

    /// `error` is not always a string or an object: a proxy or a
    /// non-LibreFang service in front of the daemon can put a number, a null
    /// or a list there. Nothing may be `Display`ed out of the wrong shape and
    /// nothing may panic — the status is what is actually known, so it is what
    /// gets reported.
    #[test]
    fn a_non_string_error_field_falls_back_to_the_status() {
        for body in [
            serde_json::json!({"error": 42}),
            serde_json::json!({"error": null}),
            serde_json::json!({"error": true}),
            serde_json::json!({"error": ["a", "b"]}),
            serde_json::json!({"error": {"code": "nope"}}),
            serde_json::json!({"error": {"message": 7}}),
        ] {
            let detail = daemon_error_detail(reqwest::StatusCode::SERVICE_UNAVAILABLE, Some(&body));

            assert_eq!(
                detail, "HTTP 503 Service Unavailable",
                "a non-string reason must not be rendered: {body}"
            );
        }
    }

    /// The bare `error` string wins over the nested mirror when a route sends
    /// both, so a route that deliberately narrows the message is not overruled
    /// by the envelope's generic half.
    #[test]
    fn the_bare_error_string_wins_over_the_nested_message() {
        let detail = daemon_error_detail(
            reqwest::StatusCode::FORBIDDEN,
            Some(&serde_json::json!({
                "error": "Security scan blocked this skill",
                "message": "Forbidden",
            })),
        );

        assert_eq!(detail, "Security scan blocked this skill");
    }

    /// A body that is not JSON at all (an HTML error page from a proxy, an
    /// empty 404) parses to nothing, and the status still has to reach the
    /// screen.
    #[test]
    fn an_unparseable_body_falls_back_to_the_status() {
        assert!(daemon_error_detail(reqwest::StatusCode::NOT_FOUND, None).contains("404"));
    }

    /// The failure line has to keep both halves: which operation failed, and
    /// why. Either alone leaves the operator guessing.
    #[test]
    fn the_failure_line_names_the_operation_and_the_reason() {
        let message = with_detail(
            crate::i18n::t_args("tui-event-skill-install-failed", &[("slug", "prd")]),
            daemon_error_detail(
                reqwest::StatusCode::NOT_FOUND,
                Some(&serde_json::json!({"error": "skill not found on registry"})),
            ),
        );

        assert!(message.contains("prd"), "which install failed: {message}");
        assert!(
            message.contains("skill not found on registry"),
            "why it failed: {message}"
        );
    }
}
