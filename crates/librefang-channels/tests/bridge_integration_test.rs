//! Integration tests for the BridgeManager dispatch pipeline.
//!
//! These tests create a mock channel adapter (with injectable messages)
//! and a mock kernel handle, wire them through the real BridgeManager,
//! and verify the full dispatch pipeline works end-to-end.
//!
//! No external services are contacted — all communication is in-process
//! via real tokio channels and tasks.

use async_trait::async_trait;
use futures::Stream;
use librefang_channels::bridge::{BridgeManager, ChannelBridgeHandle};
use librefang_channels::router::AgentRouter;
use librefang_channels::types::{
    AgentPhase, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
    LifecycleReaction,
};
use librefang_types::agent::AgentId;
use librefang_types::config::ChannelOverrides;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

// ---------------------------------------------------------------------------
// Test helper - condition-based polling
// ---------------------------------------------------------------------------
//
// Replace fixed `sleep(100ms)` waits with a deadline-bounded poll so the
// dispatch pipeline gets exactly as much time as it needs (and tests fail
// fast on regression rather than flaking on slow CI runners). The 2-second
// budget is well above the ~tens-of-ms the in-process pipeline actually
// needs, but tight enough that a stuck dispatch surfaces quickly.
async fn wait_until<F>(label: &str, mut cond: F)
where
    F: FnMut() -> bool,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !cond() {
        if std::time::Instant::now() >= deadline {
            panic!("wait_until timed out after 2s: {label}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

// ---------------------------------------------------------------------------
// Mock Adapter — injects test messages, captures sent responses
// ---------------------------------------------------------------------------

struct MockAdapter {
    name: String,
    channel_type: ChannelType,
    /// Receiver consumed by start() — wrapped as a Stream.
    rx: Mutex<Option<mpsc::Receiver<ChannelMessage>>>,
    /// Captures all messages sent via send().
    sent: Arc<Mutex<Vec<(String, String)>>>,
    shutdown_tx: watch::Sender<bool>,
    /// Per-instance overrides the bridge reads via `channel_overrides()`.
    /// `None` mirrors a sidecar with no command policy (allow-all fallback).
    overrides: Option<ChannelOverrides>,
}

impl MockAdapter {
    /// Create a new mock adapter. Returns (adapter, sender) — use the sender
    /// to inject test messages into the adapter's stream.
    fn new(name: &str, channel_type: ChannelType) -> (Arc<Self>, mpsc::Sender<ChannelMessage>) {
        Self::new_with_overrides(name, channel_type, None)
    }

    /// Like `new`, but the adapter carries per-instance `ChannelOverrides`
    /// (as a sidecar built from `[[sidecar_channels]]` would). The bridge
    /// prefers these over the kernel-level lookup, so command-policy gating
    /// can be exercised end-to-end.
    fn new_with_overrides(
        name: &str,
        channel_type: ChannelType,
        overrides: Option<ChannelOverrides>,
    ) -> (Arc<Self>, mpsc::Sender<ChannelMessage>) {
        let (tx, rx) = mpsc::channel(256);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);

        let adapter = Arc::new(Self {
            name: name.to_string(),
            channel_type,
            rx: Mutex::new(Some(rx)),
            sent: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx,
            overrides,
        });
        (adapter, tx)
    }

    /// Get a copy of all sent responses as (platform_id, text) pairs.
    fn get_sent(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelAdapter for MockAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let rx = self
            .rx
            .lock()
            .unwrap()
            .take()
            .expect("start() called more than once");
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let text = match content {
            ChannelContent::Text(t) => t,
            ChannelContent::Interactive { text, ref buttons } => {
                // Flatten button labels into the text for test inspection.
                let labels: Vec<String> = buttons
                    .iter()
                    .flat_map(|row| row.iter().map(|b| b.label.clone()))
                    .collect();
                if labels.is_empty() {
                    text
                } else {
                    format!("{text}\n{}", labels.join(", "))
                }
            }
            _ => return Ok(()),
        };
        self.sent
            .lock()
            .unwrap()
            .push((user.platform_id.clone(), text));
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn channel_overrides(&self) -> Option<ChannelOverrides> {
        self.overrides.clone()
    }
}

// ---------------------------------------------------------------------------
// Mock Kernel Handle — echoes messages, serves agent lists
// ---------------------------------------------------------------------------

struct MockHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
    /// Records all messages sent to agents: (agent_id, message).
    received: Arc<Mutex<Vec<(AgentId, String)>>>,
}

impl MockHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockHandle {
    async fn send_message(&self, agent_id: AgentId, message: &str) -> Result<String, String> {
        self.received
            .lock()
            .unwrap()
            .push((agent_id, message.to_string()));
        Ok(format!("Echo: {message}"))
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }
    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
        // Test mock: no event bus to forward to.
    }
}

// ---------------------------------------------------------------------------
// Helper to create a ChannelMessage
// ---------------------------------------------------------------------------

fn make_text_msg(channel: ChannelType, user_id: &str, text: &str) -> ChannelMessage {
    ChannelMessage {
        channel,
        platform_message_id: "msg1".to_string(),
        sender: ChannelUser {
            platform_id: user_id.to_string(),
            display_name: "TestUser".to_string(),
            librefang_user: None,
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    }
}

fn make_command_msg(
    channel: ChannelType,
    user_id: &str,
    cmd: &str,
    args: Vec<&str>,
) -> ChannelMessage {
    ChannelMessage {
        channel,
        platform_message_id: "msg1".to_string(),
        sender: ChannelUser {
            platform_id: user_id.to_string(),
            display_name: "TestUser".to_string(),
            librefang_user: None,
        },
        content: ChannelContent::Command {
            name: cmd.to_string(),
            args: args.into_iter().map(String::from).collect(),
        },
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test that text messages are dispatched to the correct agent and responses
/// are sent back through the adapter.
#[tokio::test]
async fn test_bridge_dispatch_text_message() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "coder".to_string())]));
    let router = Arc::new(AgentRouter::new());

    // Pre-route the user to the agent
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Inject a text message
    tx.send(make_text_msg(
        ChannelType::Telegram,
        "user1",
        "Hello agent!",
    ))
    .await
    .unwrap();

    // Wait until the async dispatch loop produces the response.
    wait_until("text message dispatch", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    // Verify: adapter received the echo response
    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1, "Expected 1 response, got {}", sent.len());
    assert_eq!(sent[0].0, "user1");
    assert_eq!(sent[0].1, "Echo: Hello agent!");

    // Verify: handle received the message
    {
        let received = handle.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, agent_id);
        assert_eq!(received[0].1, "Hello agent!");
    }

    manager.stop().await;
}

/// Test that /agents command returns the list of running agents.
#[tokio::test]
async fn test_bridge_dispatch_agents_command() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![
        (agent_id, "coder".to_string()),
        (AgentId::new(), "researcher".to_string()),
    ]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Discord);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send /agents command as ChannelContent::Command
    tx.send(make_command_msg(
        ChannelType::Discord,
        "user1",
        "agents",
        vec![],
    ))
    .await
    .unwrap();

    wait_until("agents command", || !adapter_ref.get_sent().is_empty()).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("coder"),
        "Response should list 'coder', got: {}",
        sent[0].1
    );
    assert!(
        sent[0].1.contains("researcher"),
        "Response should list 'researcher', got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test the /help command returns help text.
#[tokio::test]
async fn test_bridge_dispatch_help_command() {
    let handle = Arc::new(MockHandle::new(vec![]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Slack);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_command_msg(
        ChannelType::Slack,
        "user1",
        "help",
        vec![],
    ))
    .await
    .unwrap();

    wait_until("help command", || !adapter_ref.get_sent().is_empty()).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.contains("/agents"), "Help should mention /agents");
    assert!(sent[0].1.contains("/agent"), "Help should mention /agent");

    manager.stop().await;
}

/// Test /agent <name> command selects the agent and updates the router.
#[tokio::test]
async fn test_bridge_dispatch_agent_select_command() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "coder".to_string())]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router.clone());
    manager.start_adapter(adapter.clone()).await.unwrap();

    // User selects "coder" agent
    tx.send(make_command_msg(
        ChannelType::Telegram,
        "user42",
        "agent",
        vec!["coder"],
    ))
    .await
    .unwrap();

    wait_until("agent select command", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("Now talking to agent: coder"),
        "Expected selection confirmation, got: {}",
        sent[0].1
    );

    // Verify router was updated — user42 should now route to agent_id
    let resolved = router.resolve(&ChannelType::Telegram, "user42", None);
    assert_eq!(resolved, Some(agent_id));

    manager.stop().await;
}

/// Regression (#5931): a sidecar with `command_policy = "allowlist"` and an
/// empty `allowed_commands` must FAIL CLOSED — every command is gated, not
/// allowed. We build the overrides through the real `SidecarAdapter` so the
/// `overrides_from_sidecar_config` mapping is exercised end-to-end, then drive
/// a `/agent coder` command through the live bridge dispatch path and assert
/// it was NOT honoured (router unchanged) and was instead forwarded to the
/// agent as plain text (`/agent coder`).
#[tokio::test]
async fn test_bridge_allowlist_empty_fails_closed_gates_command_5931() {
    use librefang_channels::sidecar::SidecarAdapter;
    use librefang_channels::types::ChannelAdapter as _;

    // The real sidecar config → overrides path: allowlist + empty list.
    let sidecar_cfg: librefang_types::config::SidecarChannelConfig =
        serde_json::from_value(serde_json::json!({
            "name": "public-bot",
            "command": "true",
            "command_policy": "allowlist",
        }))
        .expect("SidecarChannelConfig from json");
    let sidecar = SidecarAdapter::new(&sidecar_cfg, std::env::temp_dir());
    let overrides = sidecar
        .channel_overrides()
        .expect("allowlist policy yields per-instance overrides");
    assert!(
        overrides.disable_commands,
        "empty allowlist must map to disable_commands (fail-closed)"
    );

    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "coder".to_string())]));
    let router = Arc::new(AgentRouter::new());
    // Pre-route the user so the forwarded text has somewhere to land.
    router.set_user_default("user42".to_string(), agent_id);

    let (adapter, tx) =
        MockAdapter::new_with_overrides("public-bot", ChannelType::Telegram, Some(overrides));
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router.clone());
    manager.start_adapter(adapter.clone()).await.unwrap();

    // A privileged command an end user must not be able to run on a public bot.
    tx.send(make_command_msg(
        ChannelType::Telegram,
        "user42",
        "agent",
        vec!["coder"],
    ))
    .await
    .unwrap();

    wait_until("gated command forwarded as text", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    // The command was GATED: it was forwarded to the agent verbatim, not
    // executed as a `/agent` switch (which would have replied with a selection
    // confirmation).
    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1, "expected exactly one reply, got {sent:?}");
    assert_eq!(
        sent[0].1, "Echo: /agent coder",
        "blocked command must be forwarded to the agent as plain text, got: {}",
        sent[0].1
    );
    assert!(
        !sent[0].1.contains("Now talking to agent"),
        "command must NOT have been honoured as an agent switch: {}",
        sent[0].1
    );

    // The agent received the raw slash text, confirming it was treated as input.
    {
        let received = handle.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].1, "/agent coder");
    }

    manager.stop().await;
}

/// Test that unrouted messages (no agent assigned) get a helpful error.
#[tokio::test]
async fn test_bridge_dispatch_no_agent_assigned() {
    let handle = Arc::new(MockHandle::new(vec![]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send message with no agent routed
    tx.send(make_text_msg(ChannelType::Telegram, "user1", "hello"))
        .await
        .unwrap();

    wait_until("no agent assigned reply", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("No agents available"),
        "Expected 'No agents available' message, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test that slash commands embedded in text (/agents, /help) are handled as commands.
#[tokio::test]
async fn test_bridge_dispatch_slash_command_in_text() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "writer".to_string())]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send "/agents" as plain text (not as a Command variant)
    tx.send(make_text_msg(ChannelType::Telegram, "user1", "/agents"))
        .await
        .unwrap();

    wait_until("slash command in text", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("writer"),
        "Should list the 'writer' agent, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test /status command returns uptime info.
#[tokio::test]
async fn test_bridge_dispatch_status_command() {
    let handle = Arc::new(MockHandle::new(vec![
        (AgentId::new(), "a".to_string()),
        (AgentId::new(), "b".to_string()),
    ]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_command_msg(
        ChannelType::Telegram,
        "user1",
        "status",
        vec![],
    ))
    .await
    .unwrap();

    wait_until("status command", || !adapter_ref.get_sent().is_empty()).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("2 agent(s) running"),
        "Expected uptime info, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test the full lifecycle: start adapter, send messages, stop adapter.
#[tokio::test]
async fn test_bridge_manager_lifecycle() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "bot".to_string())]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockAdapter::new("lifecycle-adapter", ChannelType::WebChat);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send multiple messages
    for i in 0..5 {
        tx.send(make_text_msg(
            ChannelType::WebChat,
            "user1",
            &format!("message {i}"),
        ))
        .await
        .unwrap();
    }

    wait_until("lifecycle 5 messages", || adapter_ref.get_sent().len() >= 5).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 5, "Expected 5 responses, got {}", sent.len());

    for (i, (_, text)) in sent.iter().enumerate() {
        assert_eq!(*text, format!("Echo: message {i}"));
    }

    // Stop — should complete without hanging
    manager.stop().await;
}

/// Test multiple adapters running simultaneously in the same BridgeManager.
#[tokio::test]
async fn test_bridge_multiple_adapters() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "multi".to_string())]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("tg_user".to_string(), agent_id);
    router.set_user_default("dc_user".to_string(), agent_id);

    let (tg_adapter, tg_tx) = MockAdapter::new("telegram", ChannelType::Telegram);
    let (dc_adapter, dc_tx) = MockAdapter::new("discord", ChannelType::Discord);
    let tg_ref = tg_adapter.clone();
    let dc_ref = dc_adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(tg_adapter).await.unwrap();
    manager.start_adapter(dc_adapter).await.unwrap();

    // Send to Telegram adapter
    tg_tx
        .send(make_text_msg(
            ChannelType::Telegram,
            "tg_user",
            "from telegram",
        ))
        .await
        .unwrap();

    // Send to Discord adapter
    dc_tx
        .send(make_text_msg(
            ChannelType::Discord,
            "dc_user",
            "from discord",
        ))
        .await
        .unwrap();

    wait_until("multi adapter dispatch", || {
        !tg_ref.get_sent().is_empty() && !dc_ref.get_sent().is_empty()
    })
    .await;

    let tg_sent = tg_ref.get_sent();
    assert_eq!(tg_sent.len(), 1);
    assert_eq!(tg_sent[0].1, "Echo: from telegram");

    let dc_sent = dc_ref.get_sent();
    assert_eq!(dc_sent.len(), 1);
    assert_eq!(dc_sent[0].1, "Echo: from discord");

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// Mock Streaming Adapter — supports_streaming() returns true, captures
// streamed text via send_streaming().
// ---------------------------------------------------------------------------

struct MockStreamingAdapter {
    name: String,
    channel_type: ChannelType,
    rx: Mutex<Option<mpsc::Receiver<ChannelMessage>>>,
    /// Captures text assembled from streaming deltas.
    streamed: Arc<Mutex<Vec<(String, String)>>>,
    /// Captures text sent via the non-streaming send() path.
    sent: Arc<Mutex<Vec<(String, String)>>>,
    shutdown_tx: watch::Sender<bool>,
}

impl MockStreamingAdapter {
    fn new(name: &str, channel_type: ChannelType) -> (Arc<Self>, mpsc::Sender<ChannelMessage>) {
        let (tx, rx) = mpsc::channel(256);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let adapter = Arc::new(Self {
            name: name.to_string(),
            channel_type,
            rx: Mutex::new(Some(rx)),
            streamed: Arc::new(Mutex::new(Vec::new())),
            sent: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx,
        });
        (adapter, tx)
    }

    fn get_streamed(&self) -> Vec<(String, String)> {
        self.streamed.lock().unwrap().clone()
    }

    fn get_sent(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelAdapter for MockStreamingAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let rx = self
            .rx
            .lock()
            .unwrap()
            .take()
            .expect("start() called more than once");
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let ChannelContent::Text(text) = content {
            self.sent
                .lock()
                .unwrap()
                .push((user.platform_id.clone(), text));
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    async fn send_streaming(
        &self,
        user: &ChannelUser,
        mut delta_rx: mpsc::Receiver<String>,
        _thread_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut full_text = String::new();
        while let Some(delta) = delta_rx.recv().await {
            full_text.push_str(&delta);
        }
        if !full_text.is_empty() {
            self.streamed
                .lock()
                .unwrap()
                .push((user.platform_id.clone(), full_text));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock Handle with streaming support — emits deltas one token at a time.
// ---------------------------------------------------------------------------

struct MockStreamingHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
    received: Arc<Mutex<Vec<(AgentId, String)>>>,
}

impl MockStreamingHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockStreamingHandle {
    async fn send_message(&self, agent_id: AgentId, message: &str) -> Result<String, String> {
        self.received
            .lock()
            .unwrap()
            .push((agent_id, message.to_string()));
        Ok(format!("Echo: {message}"))
    }

    async fn send_message_streaming(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> Result<mpsc::Receiver<String>, String> {
        self.received
            .lock()
            .unwrap()
            .push((agent_id, message.to_string()));
        let (tx, rx) = mpsc::channel(16);
        // Emit the response as individual word deltas.
        let words: Vec<String> = format!("Echo: {message}")
            .split(' ')
            .map(|w| format!("{w} "))
            .collect();
        tokio::spawn(async move {
            for word in words {
                if tx.send(word).await.is_err() {
                    break;
                }
            }
        });
        Ok(rx)
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }
    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
        // Test mock: no event bus to forward to.
    }
}

// ---------------------------------------------------------------------------
// Streaming Tests
// ---------------------------------------------------------------------------

/// Test that a streaming-capable adapter's `send_streaming` is called
/// instead of `send` when the handle provides streaming support.
#[tokio::test]
async fn test_bridge_streaming_adapter_uses_send_streaming() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockStreamingHandle::new(vec![(
        agent_id,
        "streamer".to_string(),
    )]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockStreamingAdapter::new("stream-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_text_msg(
        ChannelType::Telegram,
        "user1",
        "hello stream",
    ))
    .await
    .unwrap();

    wait_until("streaming adapter delivers", || {
        !adapter_ref.get_streamed().is_empty()
    })
    .await;

    // send_streaming should have been called (not send)
    let streamed = adapter_ref.get_streamed();
    assert_eq!(
        streamed.len(),
        1,
        "Expected 1 streamed response, got {}",
        streamed.len()
    );
    assert_eq!(streamed[0].0, "user1");
    assert!(
        streamed[0].1.contains("hello stream"),
        "Streamed text should contain the echo, got: {}",
        streamed[0].1
    );

    // Non-streaming send() should NOT have been called for the response
    let sent = adapter_ref.get_sent();
    assert_eq!(
        sent.len(),
        0,
        "send() should not be called when streaming succeeds, got {} calls",
        sent.len()
    );

    manager.stop().await;
}

/// Test that a non-streaming adapter falls back to `send()` even when the
/// kernel handle supports streaming.
#[tokio::test]
async fn test_bridge_non_streaming_adapter_falls_back_to_send() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockStreamingHandle::new(vec![(
        agent_id,
        "basic".to_string(),
    )]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    // Use the plain MockAdapter which does NOT support streaming
    let (adapter, tx) = MockAdapter::new("basic-adapter", ChannelType::Discord);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_text_msg(
        ChannelType::Discord,
        "user1",
        "no streaming here",
    ))
    .await
    .unwrap();

    wait_until("non-streaming fallback send", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    // Regular send() should have been called since the adapter doesn't support streaming
    let sent = adapter_ref.get_sent();
    assert_eq!(
        sent.len(),
        1,
        "Expected 1 sent response, got {}",
        sent.len()
    );
    assert_eq!(sent[0].0, "user1");
    assert!(
        sent[0].1.contains("no streaming here"),
        "Response should contain echo, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test that the default `send_streaming` implementation on `ChannelAdapter`
/// collects all deltas and sends the assembled text via `send()`.
#[tokio::test]
async fn test_default_send_streaming_collects_and_sends() {
    // The default `send_streaming` on ChannelAdapter collects all deltas
    // then calls `self.send()`. We test this using the plain MockAdapter
    // (which does NOT override send_streaming) by calling it directly.

    let (adapter, _tx) = MockAdapter::new("default-stream", ChannelType::Slack);
    let user = ChannelUser {
        platform_id: "u1".to_string(),
        display_name: "Tester".to_string(),
        librefang_user: None,
    };

    let (delta_tx, delta_rx) = mpsc::channel::<String>(16);

    // Send deltas in a background task
    tokio::spawn(async move {
        for word in &["Hello", " ", "world", "!"] {
            delta_tx.send(word.to_string()).await.unwrap();
        }
        // drop delta_tx to close the channel
    });

    // Call the default send_streaming implementation
    adapter.send_streaming(&user, delta_rx, None).await.unwrap();

    // The default impl should have called send() with the full assembled text
    let sent = adapter.get_sent();
    assert_eq!(sent.len(), 1, "Expected 1 sent message, got {}", sent.len());
    assert_eq!(sent[0].0, "u1");
    assert_eq!(sent[0].1, "Hello world!");
}

// ---------------------------------------------------------------------------
// Mock Handle that emits PROGRESS lines on the streaming-with-status path.
// ---------------------------------------------------------------------------

/// MockHandle whose `send_message_streaming_with_sender_status` synthesises
/// a delta stream containing a "🔧 tool_name" progress line followed by the
/// model's prose — mirroring what `start_stream_text_bridge_with_status`
/// would produce in production. Lets us verify that the
/// dispatch_message non-streaming-adapter branch (V2) actually surfaces
/// progress markers to adapters like Discord/Slack/Matrix.
struct MockProgressHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
}

impl MockProgressHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockProgressHandle {
    async fn send_message(&self, _agent_id: AgentId, message: &str) -> Result<String, String> {
        Ok(format!("Echo: {message}"))
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }

    async fn send_message_streaming_with_sender_status(
        &self,
        _agent_id: AgentId,
        _message: &str,
        _sender: &librefang_channels::types::SenderContext,
    ) -> Result<
        (
            mpsc::Receiver<String>,
            tokio::sync::oneshot::Receiver<Result<(), String>>,
        ),
        String,
    > {
        let (tx, rx) = mpsc::channel(16);
        let (status_tx, status_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            // Mirror what start_stream_text_bridge would inject for a real
            // ToolUseStart followed by post-tool prose.
            let _ = tx.send("\n\n🔧 `web_search`\n".to_string()).await;
            let _ = tx.send("Found 3 results.".to_string()).await;
            drop(tx);
            let _ = status_tx.send(Ok(()));
        });
        Ok((rx, status_rx))
    }
    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
        // Test mock: no event bus to forward to.
    }
}

/// Verify that a non-streaming adapter (Discord/Slack/Matrix/...) receives
/// the progress markers as part of the consolidated response message.
/// This is the V2 contract: progress is surfaced on every channel, not
/// just Telegram, via the shared dispatch_message → streaming-with-status
/// → send_response pipeline.
#[tokio::test]
async fn test_bridge_non_streaming_adapter_sees_progress_markers() {
    let agent_id = AgentId::new();
    let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockProgressHandle::new(vec![(
        agent_id,
        "tool-user".to_string(),
    )]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockAdapter::new("discord-mock", ChannelType::Discord);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_text_msg(
        ChannelType::Discord,
        "user1",
        "search for rust async",
    ))
    .await
    .unwrap();

    // Wait for the consolidated reply to land.
    wait_until("progress marker reply", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    let sent = adapter_ref.get_sent();
    assert_eq!(
        sent.len(),
        1,
        "Expected 1 consolidated reply, got {}",
        sent.len()
    );
    assert_eq!(sent[0].0, "user1");
    assert!(
        sent[0].1.contains("🔧") && sent[0].1.contains("web_search"),
        "Expected progress marker in non-streaming reply, got: {:?}",
        sent[0].1
    );
    assert!(
        sent[0].1.contains("Found 3 results."),
        "Expected post-tool prose in reply, got: {:?}",
        sent[0].1
    );

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// ToolUse lifecycle reaction on the non-streaming-adapter path (#6451)
// ---------------------------------------------------------------------------
//
// The non-streaming-adapter branch of `dispatch_message` mirrors the streaming
// path: each `\n\n🔧 <tool>\n\n` marker in the accumulated delta stream is
// surfaced as an `AgentPhase::ToolUse` reaction via `send_reaction`, so a
// non-streaming adapter that consumes reactions (the Slack sidecar's
// multi-step task display) can build a per-tool step list.
// `test_bridge_non_streaming_adapter_sees_progress_markers` above only asserts
// the marker text lands in the consolidated reply — it never exercises the
// reaction call. This test records reactions through a non-streaming adapter
// and asserts the ToolUse phase (carrying the tool name) reaches
// `send_reaction`.

/// Non-streaming adapter that records every lifecycle reaction it receives.
/// Mirrors `MockAdapter` (also non-streaming) but captures the reactions the
/// default no-op `send_reaction` would otherwise drop.
struct ReactionRecordingAdapter {
    name: String,
    channel_type: ChannelType,
    rx: Mutex<Option<mpsc::Receiver<ChannelMessage>>>,
    /// Every reaction, paired with the `message_id` it targeted.
    /// The id is what the Slack sidecar keys its receipt on (#6731), so a test that asserts "this message got no reaction" needs it recorded.
    reactions: Arc<Mutex<Vec<(String, LifecycleReaction)>>>,
    shutdown_tx: watch::Sender<bool>,
}

impl ReactionRecordingAdapter {
    fn new(name: &str, channel_type: ChannelType) -> (Arc<Self>, mpsc::Sender<ChannelMessage>) {
        let (tx, rx) = mpsc::channel(256);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        let a = Arc::new(Self {
            name: name.to_string(),
            channel_type,
            rx: Mutex::new(Some(rx)),
            reactions: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx,
        });
        (a, tx)
    }

    fn get_reactions(&self) -> Vec<LifecycleReaction> {
        self.reactions
            .lock()
            .unwrap()
            .iter()
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Phases recorded for one `message_id`, in arrival order.
    fn phases_for(&self, message_id: &str) -> Vec<AgentPhase> {
        self.reactions
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _)| id == message_id)
            .map(|(_, r)| r.phase.clone())
            .collect()
    }
}

#[async_trait]
impl ChannelAdapter for ReactionRecordingAdapter {
    fn name(&self) -> &str {
        &self.name
    }
    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }
    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let rx = self
            .rx
            .lock()
            .unwrap()
            .take()
            .expect("start() called more than once");
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
    async fn send(
        &self,
        _user: &ChannelUser,
        _content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
    async fn send_reaction(
        &self,
        _user: &ChannelUser,
        message_id: &str,
        reaction: &LifecycleReaction,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.reactions
            .lock()
            .unwrap()
            .push((message_id.to_string(), reaction.clone()));
        Ok(())
    }
    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

/// Handle whose streaming API emits a tool marker delta in the exact shape
/// `extract_tool_marker_name` recognizes — a standalone `tx.send` of
/// `\n\n🔧 <tool>\n\n`, matching the real producer
/// (`channel_bridge.rs`: `format!("\n\n🔧 {pretty}\n\n")`). `MockProgressHandle`
/// above deliberately emits a differently-shaped marker (single trailing
/// newline, backticks) that does NOT match, so it can drive the text-in-reply
/// assertion but not the reaction assertion.
struct MockToolReactionHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
}

impl MockToolReactionHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockToolReactionHandle {
    async fn send_message(&self, _agent_id: AgentId, message: &str) -> Result<String, String> {
        Ok(format!("Echo: {message}"))
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }

    async fn send_message_streaming_with_sender_status(
        &self,
        _agent_id: AgentId,
        _message: &str,
        _sender: &librefang_channels::types::SenderContext,
    ) -> Result<
        (
            mpsc::Receiver<String>,
            tokio::sync::oneshot::Receiver<Result<(), String>>,
        ),
        String,
    > {
        let (tx, rx) = mpsc::channel(16);
        let (status_tx, status_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            // Standalone marker delta in the exact shape the api bridge injects
            // on `ToolUseStart` — must be its own `tx.send` with blank lines on
            // both sides for `extract_tool_marker_name` to recognize it.
            let _ = tx.send("\n\n🔧 web_search\n\n".to_string()).await;
            let _ = tx.send("Found 3 results.".to_string()).await;
            drop(tx);
            let _ = status_tx.send(Ok(()));
        });
        Ok((rx, status_rx))
    }
    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {}
}

/// A non-streaming adapter must receive the per-tool `AgentPhase::ToolUse`
/// reaction (carrying the tool name) followed by the terminal `Done` reaction.
/// This locks the non-streaming-adapter lifecycle emission that #6451 added so
/// the Slack sidecar's step list keeps getting per-tool signals.
#[tokio::test]
async fn test_non_streaming_adapter_receives_tool_use_reaction() {
    let agent_id = AgentId::new();
    let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockToolReactionHandle::new(vec![(
        agent_id,
        "tool-user".to_string(),
    )]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = ReactionRecordingAdapter::new("slack-mock", ChannelType::Slack);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_text_msg(
        ChannelType::Slack,
        "user1",
        "search for rust async",
    ))
    .await
    .unwrap();

    // Wait until the ToolUse reaction lands.
    wait_until("tool_use reaction", || {
        adapter_ref
            .get_reactions()
            .iter()
            .any(|r| matches!(&r.phase, AgentPhase::ToolUse { .. }))
    })
    .await;

    let reactions = adapter_ref.get_reactions();
    let tool_use = reactions
        .iter()
        .find(|r| matches!(&r.phase, AgentPhase::ToolUse { .. }))
        .expect("a ToolUse reaction should have reached send_reaction");
    match &tool_use.phase {
        AgentPhase::ToolUse { tool_name } => {
            assert_eq!(
                tool_name, "web_search",
                "ToolUse reaction must carry the tool name extracted from the marker"
            );
        }
        other => panic!("expected ToolUse phase, got {other:?}"),
    }

    // The turn still terminates with a Done reaction after the tool phase.
    wait_until("done reaction", || {
        adapter_ref
            .get_reactions()
            .iter()
            .any(|r| matches!(&r.phase, AgentPhase::Done))
    })
    .await;

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// Group gating emits NO lifecycle reaction (#6731)
// ---------------------------------------------------------------------------
//
// The Slack sidecar's eyes/check receipt keys off the `Queued` lifecycle phase rather than off receiving a message, which is only correct if a message the bridge declines to answer produces no lifecycle signal at all.
// Mention-only group gating is the path #6731 was reported on: `dispatch_message` returns at the `!should_process_group_message` arm, well before the first `send_lifecycle_reaction(Queued)` call.
// This test pins that contract so a future refactor that moved the reaction above the gate would resurrect the permanently-stuck eyes on the Slack side.

/// Handle that reports `group_policy = mention_only` as a per-agent override, the way an `agent.toml` `[channel_overrides]` block does.
/// The trait default returns `None`, so without this the bridge sees no policy and processes every group message.
struct MockMentionOnlyHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
}

impl MockMentionOnlyHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockMentionOnlyHandle {
    async fn send_message(&self, _agent_id: AgentId, message: &str) -> Result<String, String> {
        Ok(format!("Echo: {message}"))
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }

    async fn agent_channel_overrides(&self, _agent_id: AgentId) -> Option<ChannelOverrides> {
        Some(ChannelOverrides {
            group_policy: Some(librefang_types::config::GroupPolicy::MentionOnly),
            ..Default::default()
        })
    }

    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {}
}

/// Build a group message with an explicit platform message id, so a test can tell one message's reactions from another's.
fn make_group_msg(
    channel: ChannelType,
    group_id: &str,
    message_id: &str,
    text: &str,
    was_mentioned: bool,
) -> ChannelMessage {
    let mut metadata = HashMap::new();
    if was_mentioned {
        metadata.insert("was_mentioned".to_string(), serde_json::json!(true));
    }
    ChannelMessage {
        channel,
        platform_message_id: message_id.to_string(),
        sender: ChannelUser {
            platform_id: group_id.to_string(),
            display_name: "GroupUser".to_string(),
            librefang_user: None,
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: true,
        thread_id: None,
        metadata,
    }
}

#[tokio::test]
async fn test_group_gating_skip_emits_no_lifecycle_reaction_6731() {
    let agent_id = AgentId::new();
    let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockMentionOnlyHandle::new(vec![(
        agent_id,
        "gated".to_string(),
    )]));
    let router = Arc::new(AgentRouter::new());
    // Group messages resolve on `sender.platform_id`, which for a group is the chat id (see `resolve_or_fallback`'s binding-keys comment).
    router.set_user_default("G01".to_string(), agent_id);

    let (adapter, tx) = ReactionRecordingAdapter::new("slack-mock", ChannelType::Slack);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Unaddressed first, then an addressed one.
    // Both are dispatched on the same serialized adapter stream, so once the addressed message has reached its terminal phase the unaddressed one has had its full turn to produce a reaction — no sleeps needed to prove the absence.
    tx.send(make_group_msg(
        ChannelType::Slack,
        "G01",
        "skip-1",
        "just chatting among ourselves",
        false,
    ))
    .await
    .unwrap();
    tx.send(make_group_msg(
        ChannelType::Slack,
        "G01",
        "pass-1",
        "hey bot, do the thing",
        true,
    ))
    .await
    .unwrap();

    // Positive control: the addressed message really did run a turn, so the harness and the override are both wired correctly.
    wait_until("done reaction for the addressed message", || {
        adapter_ref
            .phases_for("pass-1")
            .iter()
            .any(|p| matches!(p, AgentPhase::Done))
    })
    .await;

    // The gated-out message never got a lifecycle signal, so the Slack sidecar never adds an eyes it would have no way to clear.
    assert!(
        adapter_ref.phases_for("skip-1").is_empty(),
        "a mention-only-gated group message must produce no lifecycle reaction, got {:?}",
        adapter_ref.phases_for("skip-1")
    );

    manager.stop().await;
}

#[tokio::test]
async fn test_group_gating_pass_emits_queued_then_done_6731() {
    let agent_id = AgentId::new();
    let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockMentionOnlyHandle::new(vec![(
        agent_id,
        "gated".to_string(),
    )]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("G01".to_string(), agent_id);

    let (adapter, tx) = ReactionRecordingAdapter::new("slack-mock", ChannelType::Slack);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_group_msg(
        ChannelType::Slack,
        "G01",
        "pass-1",
        "hey bot, do the thing",
        true,
    ))
    .await
    .unwrap();

    wait_until("done reaction", || {
        adapter_ref
            .phases_for("pass-1")
            .iter()
            .any(|p| matches!(p, AgentPhase::Done))
    })
    .await;

    // Queued must arrive, and arrive first: it is what the Slack sidecar hangs the eyes on, and a Done without a preceding Queued would leave a turn with a check and no in-progress marker.
    let phases = adapter_ref.phases_for("pass-1");
    assert!(
        matches!(phases.first(), Some(AgentPhase::Queued)),
        "the first lifecycle phase of a dispatched turn must be Queued, got {phases:?}"
    );
    assert!(
        matches!(phases.last(), Some(AgentPhase::Done)),
        "a successful turn must end on Done, got {phases:?}"
    );

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// Mock adapter that ALWAYS fails send_streaming — used to exercise the
// buffered_text fallback branch that V2 added.
// ---------------------------------------------------------------------------

struct MockFailingStreamingAdapter {
    name: String,
    channel_type: ChannelType,
    rx: Mutex<Option<mpsc::Receiver<ChannelMessage>>>,
    sent: Arc<Mutex<Vec<(String, String)>>>,
    shutdown_tx: watch::Sender<bool>,
}

impl MockFailingStreamingAdapter {
    fn new(name: &str, channel_type: ChannelType) -> (Arc<Self>, mpsc::Sender<ChannelMessage>) {
        let (tx, rx) = mpsc::channel(256);
        let (shutdown_tx, _) = watch::channel(false);
        let a = Arc::new(Self {
            name: name.to_string(),
            channel_type,
            rx: Mutex::new(Some(rx)),
            sent: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx,
        });
        (a, tx)
    }

    fn get_sent(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelAdapter for MockFailingStreamingAdapter {
    fn name(&self) -> &str {
        &self.name
    }
    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }
    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        let rx = self.rx.lock().unwrap().take().expect("start once");
        Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let ChannelContent::Text(text) = content {
            self.sent
                .lock()
                .unwrap()
                .push((user.platform_id.clone(), text));
        }
        Ok(())
    }
    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
    fn supports_streaming(&self) -> bool {
        true
    }
    async fn send_streaming(
        &self,
        _user: &ChannelUser,
        mut delta_rx: mpsc::Receiver<String>,
        _thread_id: Option<&str>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Drain so the bridge's tee task can populate buffered_text, then fail.
        while delta_rx.recv().await.is_some() {}
        Err("simulated transport failure".into())
    }
}

// ---------------------------------------------------------------------------
// Mock handle that emits some progress/text deltas and then reports a
// terminal kernel error via the `_status` oneshot — exercises the
// "send_streaming Err + kernel Err" outcome on the Telegram-style path.
// ---------------------------------------------------------------------------

struct MockKernelErrorHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
}

impl MockKernelErrorHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockKernelErrorHandle {
    async fn send_message(&self, _agent_id: AgentId, message: &str) -> Result<String, String> {
        Ok(format!("Echo: {message}"))
    }
    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }
    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }
    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }
    async fn send_message_streaming_with_sender_status(
        &self,
        _agent_id: AgentId,
        _message: &str,
        _sender: &librefang_channels::types::SenderContext,
    ) -> Result<
        (
            mpsc::Receiver<String>,
            tokio::sync::oneshot::Receiver<Result<(), String>>,
        ),
        String,
    > {
        let (tx, rx) = mpsc::channel(16);
        let (status_tx, status_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = tx.send("\n\n🔧 `web_search`\n".to_string()).await;
            let _ = tx.send("partial answer".to_string()).await;
            drop(tx);
            // Report kernel failure AFTER the text channel drains —
            // mirrors how start_stream_text_bridge_with_status orders its
            // sends in production.
            let _ = status_tx.send(Err("rate limit hit".to_string()));
        });
        Ok((rx, status_rx))
    }
    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
        // Test mock: no event bus to forward to.
    }
}

/// Exercises the Telegram-path 4th outcome introduced in V2:
///   send_streaming Err + kernel Err
/// Expected behavior:
///   - No fallback `send()` call is made (kernel errored AND adapter
///     opts into suppress_error_responses below — but even without that,
///     buffer is consumed by drain).
///   - `record_delivery` is called with success=false.
///
/// We construct a streaming adapter whose `send_streaming` always returns
/// Err and whose handle reports a kernel error after the stream drains.
/// The bridge should detect both failures and route to the AgentPhase::Error
/// branch; the buffered fallback should NOT post anything because there is
/// no clean output to deliver.
#[tokio::test]
async fn test_bridge_streaming_adapter_kernel_and_transport_both_fail() {
    let agent_id = AgentId::new();
    let handle: Arc<dyn ChannelBridgeHandle> = Arc::new(MockKernelErrorHandle::new(vec![(
        agent_id,
        "rate-limited".to_string(),
    )]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockFailingStreamingAdapter::new("flaky-telegram", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_text_msg(ChannelType::Telegram, "user1", "go search"))
        .await
        .unwrap();

    wait_until("kernel+transport fail fallback", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    let sent = adapter_ref.get_sent();
    // The fallback path delivers buffered text via send_response (NOT
    // suppressed because Telegram is not in suppress_error_responses).
    // It must label the fallback delivery with the kernel error string so
    // metrics reflect "kernel failed" — but the user-facing text still
    // contains the partial output we accumulated.
    assert_eq!(
        sent.len(),
        1,
        "Expected exactly one fallback send() containing the buffered text, got {}",
        sent.len()
    );
    assert!(
        sent[0].1.contains("partial answer"),
        "Fallback text should include the deltas accumulated before failure, got: {:?}",
        sent[0].1
    );
    assert!(
        sent[0].1.contains("🔧"),
        "Fallback text should preserve progress markers, got: {:?}",
        sent[0].1
    );

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// Mock handle that emits text deltas + reports kernel SUCCESS via the
// status oneshot. Combined with MockFailingStreamingAdapter (always
// returns Err on send_streaming) this exercises the V3 Bug 1 fix:
// outcome 3 = send_streaming Err + kernel Ok must record_delivery as
// success=true with NO err string (the fallback send_response delivered
// the buffered text; the transport-side stream error is not relevant to
// delivery accounting).
// ---------------------------------------------------------------------------

type DeliveryLog = Arc<Mutex<Vec<(bool, Option<String>)>>>;

struct MockKernelOkHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
    /// Captures every record_delivery call so the test can assert on
    /// (success, err) pairing, which is the exact contract Bug 1 broke.
    deliveries: DeliveryLog,
}

impl MockKernelOkHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
            deliveries: Arc::new(Mutex::new(Vec::new())),
        }
    }
    fn deliveries(&self) -> Vec<(bool, Option<String>)> {
        self.deliveries.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockKernelOkHandle {
    async fn send_message(&self, _agent_id: AgentId, message: &str) -> Result<String, String> {
        Ok(format!("Echo: {message}"))
    }
    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }
    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }
    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }
    async fn record_delivery(
        &self,
        _agent_id: AgentId,
        _channel: &str,
        _recipient: &str,
        success: bool,
        error: Option<&str>,
        _thread_id: Option<&str>,
    ) {
        self.deliveries
            .lock()
            .unwrap()
            .push((success, error.map(String::from)));
    }
    async fn send_message_streaming_with_sender_status(
        &self,
        _agent_id: AgentId,
        _message: &str,
        _sender: &librefang_channels::types::SenderContext,
    ) -> Result<
        (
            mpsc::Receiver<String>,
            tokio::sync::oneshot::Receiver<Result<(), String>>,
        ),
        String,
    > {
        let (tx, rx) = mpsc::channel(16);
        let (status_tx, status_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = tx.send("clean reply text".to_string()).await;
            drop(tx);
            // Kernel succeeded — bridge.rs Bug 1 path must NOT smuggle
            // the transport-side send_streaming error into the record's
            // err field.
            let _ = status_tx.send(Ok(()));
        });
        Ok((rx, status_rx))
    }
    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
        // Test mock: no event bus to forward to.
    }
}

/// Bug 1 (review-driven fix): the Telegram-path outcome 3
///   send_streaming Err + kernel Ok
/// previously recorded delivery as (success=true, err=Some(stream_e)).
/// Success=true + err=Some is a contradictory metric — when the kernel
/// succeeded and the fallback send_response delivered the real reply,
/// the transport-side stream error is irrelevant. After the fix, err
/// must be None whenever success=true.
#[tokio::test]
async fn test_bridge_streaming_adapter_kernel_ok_transport_fail_records_clean_success() {
    let agent_id = AgentId::new();
    let handle_concrete = Arc::new(MockKernelOkHandle::new(vec![(
        agent_id,
        "happy-agent".to_string(),
    )]));
    let handle: Arc<dyn ChannelBridgeHandle> = handle_concrete.clone();
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockFailingStreamingAdapter::new("flaky-telegram-2", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_text_msg(ChannelType::Telegram, "user1", "ping"))
        .await
        .unwrap();

    wait_until("kernel-ok transport-fail fallback", || {
        !adapter_ref.get_sent().is_empty() && !handle_concrete.deliveries().is_empty()
    })
    .await;

    // Fallback send_response must have delivered the text.
    let sent = adapter_ref.get_sent();
    assert_eq!(
        sent.len(),
        1,
        "Expected fallback send to fire when send_streaming Err'd, got {}",
        sent.len()
    );
    assert!(
        sent[0].1.contains("clean reply text"),
        "Fallback should deliver the buffered text, got: {:?}",
        sent[0].1
    );

    // The metric contract: success=true MUST come with err=None.
    let deliveries = handle_concrete.deliveries();
    assert_eq!(
        deliveries.len(),
        1,
        "Expected exactly one record_delivery call, got {}",
        deliveries.len()
    );
    let (success, err) = &deliveries[0];
    assert!(
        *success,
        "Kernel succeeded — record_delivery success must be true, got {success}"
    );
    assert!(
        err.is_none(),
        "When kernel succeeded the transport stream error must NOT leak into the err field, got {err:?}"
    );

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// Approval listener (#4875)
// ---------------------------------------------------------------------------
//
// Regression coverage for `BridgeManager::start_approval_listener`: prior to
// #4875 the listener was dead code (no caller in the codebase), so approval
// requests fired by the kernel never reached channel adapters.

/// Mock kernel handle that exposes a real `tokio::broadcast` channel as its
/// event bus. The accompanying `sender` lets tests inject `Event` instances
/// as if the kernel had emitted them.
struct EventBusHandle {
    sender: tokio::sync::broadcast::Sender<Arc<librefang_types::event::Event>>,
}

impl EventBusHandle {
    fn new() -> (
        Self,
        tokio::sync::broadcast::Sender<Arc<librefang_types::event::Event>>,
    ) {
        let (sender, _) = tokio::sync::broadcast::channel(16);
        (
            Self {
                sender: sender.clone(),
            },
            sender,
        )
    }
}

#[async_trait]
impl ChannelBridgeHandle for EventBusHandle {
    async fn send_message(&self, _agent_id: AgentId, _message: &str) -> Result<String, String> {
        Err("not used by approval-listener test".to_string())
    }

    async fn find_agent_by_name(&self, _name: &str) -> Result<Option<AgentId>, String> {
        Ok(None)
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(Vec::new())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("not used by approval-listener test".to_string())
    }

    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {}

    async fn subscribe_events(
        &self,
    ) -> Option<tokio::sync::broadcast::Receiver<Arc<librefang_types::event::Event>>> {
        Some(self.sender.subscribe())
    }
}

/// Mock adapter that overrides `notification_recipients()` to expose a
/// configured operator user, mirroring how a sidecar adapter exposes its
/// `allowed_users`. Optionally carries an `account_id` so the bridge's
/// approval scoping (#4985) can resolve the right router channel key for
/// multi-bot configurations.
struct NotifyingAdapter {
    name: String,
    recipients: Vec<ChannelUser>,
    sent: Arc<Mutex<Vec<(String, String)>>>,
    account_id: Option<String>,
    channel_type: ChannelType,
    /// Every `send` fails, the way a Telegram bot that is not a member of the
    /// target chat fails. The attempt is still recorded, so a test can assert
    /// both that the adapter tried and that the listener did not count the
    /// attempt as coverage (#8228).
    fail_sends: bool,
}

impl NotifyingAdapter {
    fn new(name: &str, recipients: Vec<ChannelUser>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            recipients,
            sent: Arc::new(Mutex::new(Vec::new())),
            account_id: None,
            channel_type: ChannelType::Telegram,
            fail_sends: false,
        })
    }

    fn with_account(name: &str, account_id: &str, recipients: Vec<ChannelUser>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            recipients,
            sent: Arc::new(Mutex::new(Vec::new())),
            account_id: Some(account_id.to_string()),
            channel_type: ChannelType::Telegram,
            fail_sends: false,
        })
    }

    /// Like `with_account`, but every send fails.
    fn failing_with_account(name: &str, account_id: &str) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            recipients: Vec::new(),
            sent: Arc::new(Mutex::new(Vec::new())),
            account_id: Some(account_id.to_string()),
            channel_type: ChannelType::Telegram,
            fail_sends: true,
        })
    }

    /// Build an adapter on a non-Telegram channel type with an `account_id`
    /// override — used by the scoping regression test that pins the
    /// listener's key construction is not Telegram-specific.
    fn with_channel_and_account(
        name: &str,
        channel_type: ChannelType,
        account_id: &str,
        recipients: Vec<ChannelUser>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            recipients,
            sent: Arc::new(Mutex::new(Vec::new())),
            account_id: Some(account_id.to_string()),
            channel_type,
            fail_sends: false,
        })
    }

    fn get_sent(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelAdapter for NotifyingAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        // No inbound messages — the listener test only exercises the outbound
        // notification path. Return an immediately-closed stream so
        // `start_adapter`'s dispatch loop is well-behaved.
        let (_tx, rx) = mpsc::channel::<ChannelMessage>(1);
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        if let ChannelContent::Text(t) = content {
            self.sent
                .lock()
                .unwrap()
                .push((user.platform_id.clone(), t));
        }
        if self.fail_sends {
            return Err("simulated transport failure".into());
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    fn notification_recipients(&self) -> Vec<ChannelUser> {
        self.recipients.clone()
    }

    fn account_id(&self) -> Option<&str> {
        self.account_id.as_deref()
    }
}

/// End-to-end check: with the listener wired up, an `ApprovalRequested`
/// event flowing through the kernel's event bus reaches every channel
/// adapter's configured recipients with a formatted text notification.
#[tokio::test]
async fn test_approval_listener_delivers_to_configured_recipients() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);
    let agent_id = AgentId::new();
    // #4985: bind the channel default to the requesting agent so the scoping
    // check in the listener allows delivery through this adapter.
    let router = AgentRouter::new();
    router.set_channel_default("telegram".to_string(), agent_id);
    let router = Arc::new(router);
    let adapter = NotifyingAdapter::new(
        "telegram-mock",
        vec![ChannelUser {
            platform_id: "555".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    // Subscribers join the broadcast lazily — give the listener task a tick
    // to wire itself up before we emit. Without this, the very first send()
    // would race the spawn and silently drop.
    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    let approval = ApprovalRequestedEvent {
        request_id: "abcdef0123456789".to_string(),
        agent_id: agent_id.0.to_string(),
        tool_name: "shell_exec".to_string(),
        description: "rm -rf /tmp/foo".to_string(),
        risk_level: "high".to_string(),
        ..Default::default()
    };
    let event = Arc::new(Event::new(
        AgentId::new(),
        EventTarget::System,
        EventPayload::ApprovalRequested(approval),
    ));
    event_tx
        .send(event)
        .expect("broadcast send: listener should be subscribed");

    wait_until("approval notification delivered", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1, "expected one notification, got {sent:?}");
    let (to, text) = &sent[0];
    assert_eq!(to, "555", "notification went to wrong recipient");
    assert!(
        text.contains("abcdef01"),
        "notification should include 8-char approval id prefix, got: {text}"
    );
    assert!(
        text.contains("shell_exec"),
        "notification should name the tool, got: {text}"
    );
    assert!(
        text.contains("/approve") && text.contains("/reject"),
        "notification should include approve/reject hints, got: {text}"
    );

    manager.stop().await;
}

/// Adapter with no configured recipients (empty `allowed_users` equivalent)
/// must not crash the listener and must produce no `send()` calls — the
/// approval has nowhere to land on that channel.
#[tokio::test]
async fn test_approval_listener_skips_adapter_without_recipients() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);
    let agent_id = AgentId::new();
    let router = AgentRouter::new();
    router.set_channel_default("telegram".to_string(), agent_id);
    let router = Arc::new(router);
    let adapter = NotifyingAdapter::new("telegram-no-users", Vec::new());
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            AgentId::new(),
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "deadbeef".to_string(),
                agent_id: agent_id.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "ls".to_string(),
                risk_level: "low".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    // Give the listener task time to process the event and skip delivery.
    // 100ms is well above the in-process dispatch latency; a regression that
    // mistakenly sends to an empty recipient list would already have written
    // to `sent` by then.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        adapter_ref.get_sent().is_empty(),
        "adapter with no recipients must not receive notifications, got: {:?}",
        adapter_ref.get_sent()
    );

    manager.stop().await;
}

/// #4985 regression guard.
///
/// Before this fix, every `ApprovalRequested` event was broadcast to every
/// running adapter's notification recipients, regardless of which agent
/// triggered it — so a tool approval from agent A leaked into the bot/chat
/// of unrelated agent B. The fix scopes delivery through the router's
/// per-channel agent binding.
///
/// This test wires two adapters bound to two different agents via
/// `AgentRouter::set_channel_default` on account-qualified channel keys
/// (`telegram:bot-a` and `telegram:bot-b`), emits an approval for agent A,
/// and asserts that only adapter A's recipient received the notification.
#[tokio::test]
async fn test_approval_listener_scopes_delivery_to_requesting_agent_adapter() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_a = AgentId::new();
    let agent_b = AgentId::new();

    // Two Telegram bots in the same daemon, each bound to a different agent
    // via account-qualified channel keys — the same shape
    // channel_bridge.rs uses for multi-bot Telegram configs.
    let router = AgentRouter::new();
    router.set_channel_default("telegram:bot-a".to_string(), agent_a);
    router.set_channel_default("telegram:bot-b".to_string(), agent_b);
    let router = Arc::new(router);

    let adapter_a = NotifyingAdapter::with_account(
        "telegram",
        "bot-a",
        vec![ChannelUser {
            platform_id: "user-a".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_b = NotifyingAdapter::with_account(
        "telegram",
        "bot-b",
        vec![ChannelUser {
            platform_id: "user-b".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_a_ref = adapter_a.clone();
    let adapter_b_ref = adapter_b.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter_a.clone()).await.unwrap();
    manager.start_adapter(adapter_b.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    // Emit an approval triggered by agent A only.
    event_tx
        .send(Arc::new(Event::new(
            agent_a,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "abcdef0123456789".to_string(),
                agent_id: agent_a.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm -rf /tmp/foo".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to adapter A", || {
        !adapter_a_ref.get_sent().is_empty()
    })
    .await;

    // Give the listener some additional time to (incorrectly) deliver to
    // adapter B before asserting the negative. 100ms is well above the
    // in-process dispatch latency.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let sent_a = adapter_a_ref.get_sent();
    let sent_b = adapter_b_ref.get_sent();

    assert_eq!(
        sent_a.len(),
        1,
        "adapter bound to requesting agent should receive exactly one approval notification, got: {sent_a:?}"
    );
    assert_eq!(
        sent_a[0].0, "user-a",
        "approval should land in adapter A's configured recipient"
    );
    assert!(
        sent_b.is_empty(),
        "#4985: adapter bound to a DIFFERENT agent must NOT receive the approval notification, got: {sent_b:?}"
    );

    manager.stop().await;
}

/// #4985 follow-up: an adapter with no router binding (no
/// `channel_default` set for its channel key) is suppressed rather than
/// leaked to. Pre-fix code would have broadcast to it; the post-fix
/// listener treats "no bound agent" as "I cannot scope this safely, drop".
#[tokio::test]
async fn test_approval_listener_skips_unbound_adapter() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);
    let agent_id = AgentId::new();

    // Router has no channel_default for "telegram" — the adapter is
    // effectively unbound.
    let router = Arc::new(AgentRouter::new());

    let adapter = NotifyingAdapter::new(
        "telegram-unbound",
        vec![ChannelUser {
            platform_id: "operator".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_id,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "feedface".to_string(),
                agent_id: agent_id.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "ls".to_string(),
                risk_level: "low".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        adapter_ref.get_sent().is_empty(),
        "unbound adapter must not receive approval notifications, got: {:?}",
        adapter_ref.get_sent()
    );

    manager.stop().await;
}

/// Defense-in-depth: a malformed `agent_id` on the event drops the
/// notification rather than reverting to the pre-fix broadcast.
///
/// Note: PR #4994 follow-up raised the log level for this branch from WARN
/// to ERROR (a misconfigured `require_approval` caller emitting a non-UUID
/// silently swallowed every approval — the failure mode #4875 was about).
/// The log-level assertion is left as a comment rather than a hard check
/// because `tracing_test` is not a dependency of `librefang-channels`;
/// introducing it just to assert level emission would inflate the test
/// dep graph for no real coverage gain.
#[tokio::test]
async fn test_approval_listener_drops_malformed_agent_id() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);
    let agent_id = AgentId::new();
    let router = AgentRouter::new();
    router.set_channel_default("telegram".to_string(), agent_id);
    let router = Arc::new(router);

    let adapter = NotifyingAdapter::new(
        "telegram-mock",
        vec![ChannelUser {
            platform_id: "555".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            AgentId::new(),
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "abcdef0123456789".to_string(),
                agent_id: "not-a-uuid".to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm -rf /tmp/foo".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        adapter_ref.get_sent().is_empty(),
        "malformed agent_id must drop the notification rather than broadcast, got: {:?}",
        adapter_ref.get_sent()
    );

    manager.stop().await;
}

/// PR #4994 follow-up regression: in a mixed config (one single-bot adapter
/// + one account-qualified adapter on the same channel type), the
/// qualified adapter must NOT fall back to the bare-key binding. The bare
/// `telegram` default was set by the single-bot adapter for agent X; the
/// multi-bot adapter is not registered in `channel_defaults` and so MUST
/// receive nothing — pre-fix listener code did `account_id ?
/// qualified-lookup : bare-lookup` with `.or_else()` fallback to bare,
/// which leaked the approval to the multi-bot adapter when its requesting
/// agent happened to match the bare-key binding.
#[tokio::test]
async fn test_approval_listener_does_not_fall_back_from_qualified_to_bare_key() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    // Only the bare `telegram` key is bound. The account-qualified
    // `telegram:bot-b` key is intentionally absent.
    let router = AgentRouter::new();
    router.set_channel_default("telegram".to_string(), agent_x);
    let router = Arc::new(router);

    // Single-bot adapter: account_id = None → looked up under bare
    // `telegram` key, which IS bound to agent_x.
    let adapter_single = NotifyingAdapter::new(
        "telegram-single",
        vec![ChannelUser {
            platform_id: "user-single".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    // Multi-bot adapter: account_id = Some("bot-b") → MUST look up under
    // `telegram:bot-b` only. No fallback to bare `telegram` is allowed,
    // otherwise an approval for agent_x leaks here too.
    let adapter_multi = NotifyingAdapter::with_account(
        "telegram-multi",
        "bot-b",
        vec![ChannelUser {
            platform_id: "user-multi".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_single_ref = adapter_single.clone();
    let adapter_multi_ref = adapter_multi.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter_single.clone()).await.unwrap();
    manager.start_adapter(adapter_multi.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_x,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "deadbeef00000000".to_string(),
                agent_id: agent_x.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm -rf /tmp/foo".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to single-bot adapter", || {
        !adapter_single_ref.get_sent().is_empty()
    })
    .await;

    // 100ms grace window for an (incorrect) bare-fallback delivery before
    // asserting the negative — well above in-process dispatch latency.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let sent_single = adapter_single_ref.get_sent();
    let sent_multi = adapter_multi_ref.get_sent();

    assert_eq!(
        sent_single.len(),
        1,
        "single-bot adapter bound to agent_x via bare `telegram` key should receive the approval, got: {sent_single:?}"
    );
    assert!(
        sent_multi.is_empty(),
        "PR #4994: account-qualified adapter MUST NOT fall back to bare-key binding — the multi-bot adapter has no `telegram:bot-b` entry and must receive nothing, got: {sent_multi:?}"
    );

    manager.stop().await;
}

/// PR #4994 follow-up: the scoping mechanism is channel-type-agnostic.
/// Any adapter that overrides `account_id()` must produce a qualified key;
/// the listener must build the right key for any such adapter. This test
/// uses a mock adapter on `ChannelType::Discord` with
/// `account_id = Some("guild-1")` and asserts the qualified key
/// `discord:guild-1` is the one that gates delivery.
#[tokio::test]
async fn test_approval_listener_scopes_to_non_telegram_multibot_adapter() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_a = AgentId::new();
    let agent_b = AgentId::new();

    // Two Discord adapters bound to different agents via account-qualified
    // keys. Approval for agent A must only reach adapter A.
    let router = AgentRouter::new();
    router.set_channel_default("discord:guild-1".to_string(), agent_a);
    router.set_channel_default("discord:guild-2".to_string(), agent_b);
    let router = Arc::new(router);

    let adapter_a = NotifyingAdapter::with_channel_and_account(
        "discord-a",
        ChannelType::Discord,
        "guild-1",
        vec![ChannelUser {
            platform_id: "admin-a".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_b = NotifyingAdapter::with_channel_and_account(
        "discord-b",
        ChannelType::Discord,
        "guild-2",
        vec![ChannelUser {
            platform_id: "admin-b".to_string(),
            display_name: String::new(),
            librefang_user: None,
        }],
    );
    let adapter_a_ref = adapter_a.clone();
    let adapter_b_ref = adapter_b.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter_a.clone()).await.unwrap();
    manager.start_adapter(adapter_b.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_a,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "cafef00d12345678".to_string(),
                agent_id: agent_a.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm -rf /tmp/foo".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to discord adapter A", || {
        !adapter_a_ref.get_sent().is_empty()
    })
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let sent_a = adapter_a_ref.get_sent();
    let sent_b = adapter_b_ref.get_sent();

    assert_eq!(
        sent_a.len(),
        1,
        "discord adapter bound to agent A via `discord:guild-1` should receive the approval, got: {sent_a:?}"
    );
    assert_eq!(sent_a[0].0, "admin-a");
    assert!(
        sent_b.is_empty(),
        "discord adapter bound to agent B via `discord:guild-2` must NOT receive an approval triggered by agent A, got: {sent_b:?}"
    );

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// #5002 — binding-aware approval scoping
// ---------------------------------------------------------------------------
//
// PR #4994 / #4985 closed the cross-agent broadcast leak by gating delivery
// on `router.channel_default(<channel_key>)`. That works when an adapter has
// a `default_agent` configured, but adapters that route purely via
// `AgentBinding` (`default_agent = None` on the adapter, per-user / per-chat
// agents via bindings) have no `channel_defaults` entry — so `channel_default`
// returns `None` and the post-#4985 listener silently drops every approval
// raised by the bound agent.
//
// The fix in #5002 falls back to `AgentRouter::bound_recipients_for_agent`
// when `channel_default` does not cover the requesting agent: it walks the
// binding list, picks every binding whose `agent` resolves to the requesting
// agent on this adapter's `(channel_type, account_id)`, and delivers to each
// binding's `peer_id`. Fan-out is across ALL such bindings (multi-chat
// agents get approvals in every bound chat).
//
// Trait extension question: we deliberately did NOT add a method to
// `ChannelAdapter` — the binding store lives on `AgentRouter`, which the
// bridge already holds, and querying it directly keeps adapters
// platform-implementation-only.

/// #5002 happy path: an adapter with `default_agent = None` plus an
/// `AgentBinding` targeting agent X on chat Z delivers approvals for X to Z.
/// Pre-fix code returned `None` from `channel_default` and silently dropped.
#[tokio::test]
async fn test_approval_listener_falls_back_to_agent_binding_when_default_unset() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();
    let agent_name = "binder-x";

    // Router has NO channel_default for `telegram` — only an AgentBinding
    // routing chat `chat-z` to `binder-x`. Reproduces the #5002 repro:
    //   1. Telegram adapter with default_agent = None
    //   2. AgentBinding maps chat-z → agent X
    //   3. Agent X fires `require_approval`
    //   4. Pre-fix: nothing arrives in chat-z.
    let router = AgentRouter::new();
    router.register_agent(agent_name.to_string(), agent_x);
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: agent_name.to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            peer_id: Some("chat-z".to_string()),
            ..Default::default()
        },
    }]);
    let router = Arc::new(router);

    // Adapter has no static `notification_recipients` — the binding is the
    // only delivery target. Mirrors a Telegram bot config with empty
    // `allowed_users` but per-user `AgentBinding` routing.
    let adapter = NotifyingAdapter::new("telegram-binding-only", Vec::new());
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_x,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "5002aaaa11112222".to_string(),
                agent_id: agent_x.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm -rf /tmp/foo".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to bound chat", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    let sent = adapter_ref.get_sent();
    assert_eq!(
        sent.len(),
        1,
        "expected one notification to the bound chat, got: {sent:?}"
    );
    assert_eq!(
        sent[0].0, "chat-z",
        "approval should land in the binding's `peer_id`"
    );
    assert!(
        sent[0].1.contains("5002aaaa"),
        "notification body should include the approval id prefix, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// #5002 cross-agent guard: same setup as the happy-path test, but the
/// approval is for a DIFFERENT agent (no binding covering it). The fix must
/// NOT re-introduce the cross-agent broadcast #4985 closed — even though
/// the adapter has `default_agent = None`, an approval for an unrelated
/// agent must not be delivered.
#[tokio::test]
async fn test_approval_listener_binding_fallback_does_not_leak_cross_agent() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();
    let agent_y = AgentId::new(); // not bound on this adapter

    let router = AgentRouter::new();
    router.register_agent("binder-x".to_string(), agent_x);
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "binder-x".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            peer_id: Some("chat-z".to_string()),
            ..Default::default()
        },
    }]);
    let router = Arc::new(router);

    let adapter = NotifyingAdapter::new("telegram-binding-only", Vec::new());
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    // Approval for agent Y (which has NO binding on this adapter).
    event_tx
        .send(Arc::new(Event::new(
            agent_y,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "5002bbbb33334444".to_string(),
                agent_id: agent_y.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "ls".to_string(),
                risk_level: "low".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    // 100ms is well above in-process dispatch latency — a regression that
    // mistakenly broadcasts would already have hit `sent` by now.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        adapter_ref.get_sent().is_empty(),
        "#5002 fan-out fallback must NOT re-leak cross-agent approvals (#4985 regression), got: {:?}",
        adapter_ref.get_sent()
    );

    manager.stop().await;
}

/// #5002 multi-chat fan-out: an agent bound to two chats Z1 and Z2 on the
/// same adapter receives the approval in BOTH. Picking one arbitrarily
/// would be wrong (issue text agrees) — the operator deliberately created
/// every binding, so every binding gets the notification.
#[tokio::test]
async fn test_approval_listener_fans_out_to_all_bound_chats() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    let router = AgentRouter::new();
    router.register_agent("binder-x".to_string(), agent_x);
    router.load_bindings(&[
        librefang_types::config::AgentBinding {
            agent: "binder-x".to_string(),
            match_rule: librefang_types::config::BindingMatchRule {
                channel: Some("telegram".to_string()),
                peer_id: Some("chat-z1".to_string()),
                ..Default::default()
            },
        },
        librefang_types::config::AgentBinding {
            agent: "binder-x".to_string(),
            match_rule: librefang_types::config::BindingMatchRule {
                channel: Some("telegram".to_string()),
                peer_id: Some("chat-z2".to_string()),
                ..Default::default()
            },
        },
    ]);
    let router = Arc::new(router);

    let adapter = NotifyingAdapter::new("telegram-binding-only", Vec::new());
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_x,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "5002cccc55556666".to_string(),
                agent_id: agent_x.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm -rf /tmp/foo".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to both bound chats", || {
        adapter_ref.get_sent().len() >= 2
    })
    .await;

    // Give the listener some slack to (incorrectly) deliver a 3rd copy
    // before asserting exactly-2. A regression that double-sends would
    // show up here.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(
        sent.len(),
        2,
        "expected fan-out to both bound chats, got: {sent:?}"
    );
    let mut destinations: Vec<&str> = sent.iter().map(|(to, _)| to.as_str()).collect();
    destinations.sort();
    assert_eq!(
        destinations,
        vec!["chat-z1", "chat-z2"],
        "approval should fan out to every chat the requesting agent is bound to, got: {destinations:?}"
    );

    manager.stop().await;
}

/// #5002 unit-style coverage at the router boundary: `AgentBinding`s with
/// no `peer_id` (e.g. catch-all "every telegram message goes to agent X")
/// are NOT delivery targets — they have no chat to send to. The listener
/// must skip them, otherwise the fan-out fallback would attempt to
/// `send()` with an empty `platform_id`.
#[tokio::test]
async fn test_approval_listener_skips_binding_with_no_peer_id() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    let router = AgentRouter::new();
    router.register_agent("binder-x".to_string(), agent_x);
    // Channel-only binding — covers every chat on `telegram`, but names
    // no specific peer.
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "binder-x".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            ..Default::default()
        },
    }]);
    let router = Arc::new(router);

    let adapter = NotifyingAdapter::new("telegram-binding-only", Vec::new());
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_x,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "5002dddd77778888".to_string(),
                agent_id: agent_x.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "ls".to_string(),
                risk_level: "low".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(
        adapter_ref.get_sent().is_empty(),
        "binding without peer_id names no chat — listener must skip it rather than send to an empty platform_id, got: {:?}",
        adapter_ref.get_sent()
    );

    manager.stop().await;
}

/// #5002 account_id scoping: a binding scoped to `(channel=telegram,
/// account_id=bot-a)` must NOT fire approvals on a different bot
/// (`bot-b`). Mirrors the #4985 multi-bot leak shape but at the binding
/// layer rather than the `channel_default` layer.
#[tokio::test]
async fn test_approval_listener_binding_respects_account_id_scope() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    let router = AgentRouter::new();
    router.register_agent("binder-x".to_string(), agent_x);
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "binder-x".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            account_id: Some("bot-a".to_string()),
            peer_id: Some("chat-z".to_string()),
            ..Default::default()
        },
    }]);
    let router = Arc::new(router);

    // Two Telegram bots, both with `default_agent = None`. Only bot-a has
    // a binding to agent X.
    let adapter_a = NotifyingAdapter::with_account("telegram-a", "bot-a", Vec::new());
    let adapter_b = NotifyingAdapter::with_account("telegram-b", "bot-b", Vec::new());
    let adapter_a_ref = adapter_a.clone();
    let adapter_b_ref = adapter_b.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter_a.clone()).await.unwrap();
    manager.start_adapter(adapter_b.clone()).await.unwrap();
    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_x,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "5002eeee9999aaaa".to_string(),
                agent_id: agent_x.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to bot-a", || {
        !adapter_a_ref.get_sent().is_empty()
    })
    .await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let sent_a = adapter_a_ref.get_sent();
    let sent_b = adapter_b_ref.get_sent();
    assert_eq!(
        sent_a.len(),
        1,
        "bot-a binding should fire, got: {sent_a:?}"
    );
    assert_eq!(sent_a[0].0, "chat-z");
    assert!(
        sent_b.is_empty(),
        "bot-b has no matching binding (account_id mismatch); approval must not leak there, got: {sent_b:?}"
    );

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// #8227: "approval reached nobody" is a verdict on the whole fan-out
// ---------------------------------------------------------------------------
//
// The #5002 `WARN` sat inside `for adapter in &adapters`, so it described one
// adapter's turn while being phrased as a verdict on the approval. On a host
// running one sidecar per agent — a supported configuration — every approval
// logged N-1 lines claiming it had been dropped, while the Nth adapter
// delivered it. The guarantee #5002 wanted (a genuinely undeliverable approval
// is never silently swallowed) is preserved by evaluating the same condition
// once, after the loop.

thread_local! {
    static WARN_SINK: std::cell::RefCell<Option<Arc<Mutex<Vec<String>>>>> =
        const { std::cell::RefCell::new(None) };
}

/// Process-global subscriber that forwards `WARN`/`ERROR` events to the sink of
/// whichever thread raised them, if that thread registered one.
///
/// The obvious shape — a subscriber per test via
/// `tracing::subscriber::set_default` — is thread-local, but `tracing` caches
/// callsite interest *globally*: a sibling test dropping its guard can leave a
/// callsite cached as "never", after which the event never reaches the
/// thread-local subscriber at all. That is not hypothetical; it made these two
/// tests pass under `--test-threads=1` and time out in the parallel run.
/// Answering `enabled` from a permanent global keeps interest at "sometimes",
/// so the decision is taken per event, and the thread-local sink still keeps
/// concurrent tests from seeing each other's warnings.
struct WarnRouter;

struct WarnVisitor(String);

impl tracing::field::Visit for WarnVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.push_str(&format!(" {}={:?}", field.name(), value));
    }
}

impl tracing::Subscriber for WarnRouter {
    fn enabled(&self, meta: &tracing::Metadata<'_>) -> bool {
        *meta.level() <= tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        WARN_SINK.with(|sink| {
            if let Some(events) = sink.borrow().as_ref() {
                let mut visitor = WarnVisitor(format!("[{}]", event.metadata().level()));
                event.record(&mut visitor);
                events.lock().unwrap().push(visitor.0);
            }
        });
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// Captures `WARN`/`ERROR` events raised on this thread for as long as it lives.
///
/// `#[tokio::test]` runs a current-thread runtime, so the listener task spawned
/// by `start_approval_listener` is polled on the test's own thread and its
/// events land in this spy.
struct WarnSpy(Arc<Mutex<Vec<String>>>);

impl WarnSpy {
    fn install() -> Self {
        static INIT: std::sync::Once = std::sync::Once::new();
        INIT.call_once(|| {
            tracing::subscriber::set_global_default(WarnRouter)
                .expect("nothing else installs a global subscriber in this test binary");
        });
        let events = Arc::new(Mutex::new(Vec::new()));
        WARN_SINK.with(|sink| *sink.borrow_mut() = Some(events.clone()));
        Self(events)
    }

    fn seen(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Drop for WarnSpy {
    fn drop(&mut self) {
        WARN_SINK.with(|sink| *sink.borrow_mut() = None);
    }
}

/// Two adapters, only the second covering the requesting agent: the approval is
/// delivered, so the fan-out must produce no warning at all.
#[tokio::test]
async fn test_approval_fanout_is_silent_when_a_later_adapter_covers_the_agent() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    // No `channel_default` anywhere: both adapters route purely via bindings,
    // and only `bot-b` has one covering agent X. `bot-a` is iterated first, so
    // pre-fix it warns before `bot-b` gets its turn.
    let router = AgentRouter::new();
    router.register_agent("binder-x".to_string(), agent_x);
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "binder-x".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            account_id: Some("bot-b".to_string()),
            peer_id: Some("chat-z".to_string()),
            ..Default::default()
        },
    }]);
    let router = Arc::new(router);

    let adapter_a = NotifyingAdapter::with_account("telegram-a", "bot-a", Vec::new());
    let adapter_b = NotifyingAdapter::with_account("telegram-b", "bot-b", Vec::new());
    let adapter_b_ref = adapter_b.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter_a).await.unwrap();
    manager.start_adapter(adapter_b).await.unwrap();

    // Installed only now: adapter startup warns about the test double having
    // no webhook routes, which has nothing to do with the fan-out. Scoping the
    // spy to the listener keeps the assertion literally "zero warnings" rather
    // than a substring filter that could hide a second, differently-worded one.
    let spy = WarnSpy::install();

    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_x,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "8227aaaa00001111".to_string(),
                agent_id: agent_x.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to bot-b", || {
        !adapter_b_ref.get_sent().is_empty()
    })
    .await;
    // The uncovered adapter's turn is over by the time the covered one has
    // sent, but give the listener room in case iteration order ever changes.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let seen = spy.seen();
    assert!(
        seen.is_empty(),
        "#8227: a delivered approval must not warn about the adapters that did not cover it, got: {seen:#?}"
    );

    manager.stop().await;
}

/// No adapter covers the requesting agent: the #5002 guarantee still holds, but
/// the operator gets exactly one warning naming the approval, not one per
/// adapter.
#[tokio::test]
async fn test_approval_fanout_warns_exactly_once_when_no_adapter_covers_the_agent() {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();
    let agent_other = AgentId::new();

    // The only binding covers a different agent, so neither adapter has a
    // delivery target for agent X's approval.
    let router = AgentRouter::new();
    router.register_agent("binder-other".to_string(), agent_other);
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "binder-other".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            peer_id: Some("chat-other".to_string()),
            ..Default::default()
        },
    }]);
    let router = Arc::new(router);

    let adapter_a = NotifyingAdapter::with_account("telegram-a", "bot-a", Vec::new());
    let adapter_b = NotifyingAdapter::with_account("telegram-b", "bot-b", Vec::new());

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter_a).await.unwrap();
    manager.start_adapter(adapter_b).await.unwrap();

    // See the sibling test: the spy goes in after adapter startup so the count
    // is the fan-out's own warnings and nothing else.
    let spy = WarnSpy::install();

    manager.start_approval_listener().await;

    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(Event::new(
            agent_x,
            EventTarget::System,
            EventPayload::ApprovalRequested(ApprovalRequestedEvent {
                request_id: "8227bbbb22223333".to_string(),
                agent_id: agent_x.0.to_string(),
                tool_name: "shell_exec".to_string(),
                description: "rm".to_string(),
                risk_level: "high".to_string(),
                ..Default::default()
            }),
        )))
        .expect("broadcast send");

    wait_until("undeliverable approval warned", || !spy.seen().is_empty()).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let seen = spy.seen();
    assert_eq!(
        seen.len(),
        1,
        "#8227: an undeliverable approval must warn once for the whole fan-out, got: {seen:#?}"
    );
    assert!(
        seen[0].contains("8227bbbb22223333") && seen[0].contains(&agent_x.0.to_string()),
        "the warning must stay actionable — request id and requesting agent, got: {}",
        seen[0]
    );

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// Approval fan-out: direct-route scoping and remedy wording (#8228)
// ---------------------------------------------------------------------------

/// Build the `ApprovalRequested` event the direct-route tests below send.
fn approval_event_from_chat(
    request_id: &str,
    agent: AgentId,
    sender_id: Option<&str>,
    channel: Option<&str>,
) -> librefang_types::event::Event {
    use librefang_types::event::{ApprovalRequestedEvent, Event, EventPayload, EventTarget};

    Event::new(
        agent,
        EventTarget::System,
        EventPayload::ApprovalRequested(ApprovalRequestedEvent {
            request_id: request_id.to_string(),
            agent_id: agent.0.to_string(),
            tool_name: "shell_exec".to_string(),
            description: "rm".to_string(),
            risk_level: "high".to_string(),
            sender_id: sender_id.map(str::to_string),
            channel: channel.map(str::to_string),
            ..Default::default()
        }),
    )
}

/// Three Telegram sidecars, one per agent — the deployment #8227 was reported
/// on. An approval carrying `sender_id` + `channel` must be delivered by the
/// sidecar that routes the requesting agent and attempted by no other.
///
/// Pre-#8228 the direct-route guard tested only `src_channel == ct_str`, so all
/// three bots sent the keyboard to the same `chat_id`: the two that are not in
/// that chat each failed and logged a WARN — the N-1-warnings-per-approval
/// symptom #8227 is about, under a different message — and a sibling bot that
/// *is* in the chat delivered a duplicate approval keyboard.
#[tokio::test]
async fn test_approval_direct_route_is_scoped_to_the_adapter_routing_the_agent() {
    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_1 = AgentId::new();
    let agent_2 = AgentId::new();
    let agent_3 = AgentId::new();

    // One account-qualified `channel_default` per sidecar, which is what
    // `[[sidecar_channels]] default_agent` produces at bridge boot.
    let router = AgentRouter::new();
    router.set_channel_default("telegram:bot-1".to_string(), agent_1);
    router.set_channel_default("telegram:bot-2".to_string(), agent_2);
    router.set_channel_default("telegram:bot-3".to_string(), agent_3);
    let router = Arc::new(router);

    let adapter_1 = NotifyingAdapter::with_account("telegram-1", "bot-1", Vec::new());
    let adapter_2 = NotifyingAdapter::with_account("telegram-2", "bot-2", Vec::new());
    let adapter_3 = NotifyingAdapter::with_account("telegram-3", "bot-3", Vec::new());
    let (ref_1, ref_2, ref_3) = (adapter_1.clone(), adapter_2.clone(), adapter_3.clone());

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter_1).await.unwrap();
    manager.start_adapter(adapter_2).await.unwrap();
    manager.start_adapter(adapter_3).await.unwrap();

    let spy = WarnSpy::install();
    manager.start_approval_listener().await;
    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(approval_event_from_chat(
            "8228aaaa00001111",
            agent_1,
            Some("chat-1"),
            Some("telegram"),
        )))
        .expect("broadcast send");

    wait_until("approval delivered to bot-1", || {
        !ref_1.get_sent().is_empty()
    })
    .await;
    // The siblings' turns are over by now, but give the listener room in case
    // iteration order ever changes.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    assert_eq!(
        ref_1.get_sent().len(),
        1,
        "the sidecar routing agent-1 must deliver the approval to the originating chat"
    );
    assert_eq!(
        ref_1.get_sent()[0].0,
        "chat-1",
        "the direct route must target the originating chat"
    );
    assert!(
        ref_2.get_sent().is_empty() && ref_3.get_sent().is_empty(),
        "#8228: sidecars that do not route the requesting agent must not send to its chat, got bot-2={:?} bot-3={:?}",
        ref_2.get_sent(),
        ref_3.get_sent()
    );
    assert!(
        spy.seen().is_empty(),
        "a delivered approval must not warn, got: {:#?}",
        spy.seen()
    );

    manager.stop().await;
}

/// A binding with no `peer_id` matches every peer, so it routes inbound
/// messages to its agent while `bound_recipients_for_agent` — which requires a
/// `peer_id` to have a delivery target — returns nothing for it. Broadcast
/// routes have the same shape.
///
/// So "this adapter has a `channel_default` or a `peer_id` binding for the
/// agent" is evidence of routing but not a precondition for it, and the
/// direct route must not be made conditional on it: `ApprovalRequestedEvent`
/// documents `sender_id` as needing "no `notification_recipients` /
/// `AgentBinding` configuration". Scoping is applied only when some adapter on
/// the originating channel does show that evidence; here none does, so the
/// pre-#8228 fast path stands and the approval is delivered rather than
/// dropped with a warning.
#[tokio::test]
async fn test_approval_direct_route_survives_a_binding_without_peer_id() {
    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    let router = AgentRouter::new();
    router.register_agent("binder-x".to_string(), agent_x);
    // No `channel_default`, and the binding gates on the channel only — every
    // Telegram peer routes to agent X, and no peer is named as a target.
    router.load_bindings(&[librefang_types::config::AgentBinding {
        agent: "binder-x".to_string(),
        match_rule: librefang_types::config::BindingMatchRule {
            channel: Some("telegram".to_string()),
            ..Default::default()
        },
    }]);
    let router = Arc::new(router);

    let adapter = NotifyingAdapter::with_account("telegram-a", "bot-a", Vec::new());
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter).await.unwrap();

    let spy = WarnSpy::install();
    manager.start_approval_listener().await;
    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(approval_event_from_chat(
            "8228cccc44445555",
            agent_x,
            Some("chat-x"),
            Some("telegram"),
        )))
        .expect("broadcast send");

    wait_until("approval delivered via the direct route", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;

    assert_eq!(
        adapter_ref.get_sent()[0].0,
        "chat-x",
        "the approval must reach the originating chat"
    );
    assert!(
        spy.seen().is_empty(),
        "a delivered approval must not warn, got: {:#?}",
        spy.seen()
    );

    manager.stop().await;
}

/// Claiming coverage on an *attempted* direct send rather than a delivered one
/// lets a run where every send failed suppress the #5002 guarantee entirely.
/// The aggregate warning must still fire, and must say delivery failed instead
/// of blaming routing config that is present and correct.
#[tokio::test]
async fn test_approval_direct_route_failure_does_not_claim_coverage() {
    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    let router = AgentRouter::new();
    router.set_channel_default("telegram:bot-a".to_string(), agent_x);
    let router = Arc::new(router);

    let adapter = NotifyingAdapter::failing_with_account("telegram-a", "bot-a");
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter).await.unwrap();

    let spy = WarnSpy::install();
    manager.start_approval_listener().await;
    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    event_tx
        .send(Arc::new(approval_event_from_chat(
            "8228dddd66667777",
            agent_x,
            Some("chat-x"),
            Some("telegram"),
        )))
        .expect("broadcast send");

    wait_until("direct send attempted", || {
        !adapter_ref.get_sent().is_empty()
    })
    .await;
    wait_until("undeliverable approval warned", || {
        spy.seen()
            .iter()
            .any(|w| w.contains("Approval reached no channel"))
    })
    .await;

    let aggregate = spy
        .seen()
        .into_iter()
        .find(|w| w.contains("Approval reached no channel"))
        .expect("checked by wait_until above");
    assert!(
        aggregate.contains("delivery failed") || aggregate.contains("every send failed"),
        "#8228: a failed send must not be reported as missing routing config, got: {aggregate}"
    );
    assert!(
        !aggregate.contains("no adapter has a channel_default"),
        "routing is configured here — the operator must not be sent to channel_default, got: {aggregate}"
    );

    manager.stop().await;
}

/// `notification_recipients()` defaulting to empty is documented as correct for
/// adapters with no stable operator inbox, so an approval that reaches an
/// adapter routing the requesting agent but exposing no recipients is *not* a
/// routing misconfiguration. The pre-#8228 aggregate asserted the routing
/// remedy unconditionally and sent the operator to the one part of the config
/// that needs no change.
///
/// The warning must also name the adapter, `account_id` and channel that the
/// #5002 per-adapter WARN carried: `adapters=N` alone gives no way to tell
/// which channels were even considered.
#[tokio::test]
async fn test_approval_warning_blames_recipients_not_routing_and_names_adapters() {
    let (handle, event_tx) = EventBusHandle::new();
    let handle = Arc::new(handle);

    let agent_x = AgentId::new();

    // Routing is present and correct on both sidecars; neither exposes an
    // operator inbox.
    let router = AgentRouter::new();
    router.set_channel_default("telegram:bot-a".to_string(), agent_x);
    router.set_channel_default("telegram:bot-b".to_string(), agent_x);
    let router = Arc::new(router);

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager
        .start_adapter(NotifyingAdapter::with_account(
            "telegram-a",
            "bot-a",
            Vec::new(),
        ))
        .await
        .unwrap();
    manager
        .start_adapter(NotifyingAdapter::with_account(
            "telegram-b",
            "bot-b",
            Vec::new(),
        ))
        .await
        .unwrap();

    let spy = WarnSpy::install();
    manager.start_approval_listener().await;
    wait_until("approval listener subscribed", || {
        event_tx.receiver_count() >= 1
    })
    .await;

    // No `sender_id` / `channel`: a cron- or trigger-raised approval, which is
    // the path the aggregate's wording was wrong on.
    event_tx
        .send(Arc::new(approval_event_from_chat(
            "8228eeee88889999",
            agent_x,
            None,
            None,
        )))
        .expect("broadcast send");

    wait_until("undeliverable approval warned", || !spy.seen().is_empty()).await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let seen = spy.seen();
    assert_eq!(
        seen.len(),
        1,
        "still exactly one warning for the whole fan-out, got: {seen:#?}"
    );
    assert!(
        seen[0].contains("notification_recipients"),
        "#8228: routing is correct here — the remedy is the adapter's recipients list, got: {}",
        seen[0]
    );
    assert!(
        !seen[0].contains("no adapter has a channel_default"),
        "the operator must not be sent to channel_default config that needs no change, got: {}",
        seen[0]
    );
    for expected in ["telegram-a", "bot-a", "telegram-b", "bot-b", "telegram"] {
        assert!(
            seen[0].contains(expected),
            "#8228: the warning must name the adapters it considered — missing {expected:?} in: {}",
            seen[0]
        );
    }

    manager.stop().await;
}

// ---------------------------------------------------------------------------
// Broadcast dispatch session scope (#7140)
// ---------------------------------------------------------------------------

/// Records, per agent, the sender scope each dispatched turn carried.
///
/// `None` means the turn reached the kernel without a `SenderContext` at all,
/// which is what makes the difference visible: the kernel's session resolver
/// only derives the per-chat `SessionId::for_sender_scope` when a sender
/// context is present, and otherwise falls back to the agent's canonical
/// `entry.session_id` — the session the dashboard chat writes to, and one no
/// channel command can address.
struct ScopeRecordingHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
    #[allow(clippy::type_complexity)]
    scopes: Arc<Mutex<Vec<(AgentId, Option<(String, Option<String>)>)>>>,
}

impl ScopeRecordingHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
            scopes: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for ScopeRecordingHandle {
    async fn send_message(&self, agent_id: AgentId, message: &str) -> Result<String, String> {
        self.scopes.lock().unwrap().push((agent_id, None));
        Ok(format!("Echo: {message}"))
    }

    async fn send_message_with_sender(
        &self,
        agent_id: AgentId,
        message: &str,
        sender: &librefang_channels::types::SenderContext,
    ) -> Result<String, String> {
        self.scopes.lock().unwrap().push((
            agent_id,
            Some((sender.channel.clone(), sender.chat_id.clone())),
        ));
        Ok(format!("Echo: {message}"))
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }

    fn record_consumer_lag(&self, _n: u64, _ctx: &'static str) {
        // Test mock: no event bus to forward to.
    }
}

/// A broadcast turn must be session-scoped like every other channel turn.
///
/// Before #7140 the broadcast fan-out was the one channel dispatch that sent
/// without a `SenderContext`, so every turn landed on the agent's canonical
/// session while `/new` cleared the per-chat one: the reset acked success on
/// an empty session and the visible conversation kept its history.
#[tokio::test]
async fn broadcast_dispatch_carries_the_per_chat_session_scope() {
    let alice = AgentId::new();
    let bob = AgentId::new();
    let handle = Arc::new(ScopeRecordingHandle::new(vec![
        (alice, "alice".to_string()),
        (bob, "bob".to_string()),
    ]));
    let scopes = handle.scopes.clone();

    let router = Arc::new(AgentRouter::new());
    router.register_agent("alice".to_string(), alice);
    router.register_agent("bob".to_string(), bob);
    let mut routes = HashMap::new();
    routes.insert(
        "vip_user".to_string(),
        vec!["alice".to_string(), "bob".to_string()],
    );
    router.load_broadcast(librefang_types::config::BroadcastConfig {
        strategy: librefang_types::config::BroadcastStrategy::Sequential,
        routes,
    });

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_text_msg(
        ChannelType::Telegram,
        "vip_user",
        "Hello both",
    ))
    .await
    .unwrap();

    wait_until("broadcast dispatch", || scopes.lock().unwrap().len() == 2).await;

    let recorded = scopes.lock().unwrap().clone();
    for (agent_id, scope) in &recorded {
        assert_eq!(
            scope
                .as_ref()
                .map(|(c, chat)| (c.as_str(), chat.as_deref())),
            Some(("telegram", Some("vip_user"))),
            "broadcast target {agent_id} was dispatched without the per-chat sender scope",
        );
    }
    let mut reached: Vec<AgentId> = recorded.iter().map(|(id, _)| *id).collect();
    reached.sort_by_key(|a| a.to_string());
    let mut expected = vec![alice, bob];
    expected.sort_by_key(|a| a.to_string());
    assert_eq!(
        reached, expected,
        "both broadcast targets must receive the turn"
    );

    manager.stop().await;
}
