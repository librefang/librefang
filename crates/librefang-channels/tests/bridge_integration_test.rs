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
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use librefang_types::agent::AgentId;
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
}

impl MockAdapter {
    /// Create a new mock adapter. Returns (adapter, sender) — use the sender
    /// to inject test messages into the adapter's stream.
    fn new(name: &str, channel_type: ChannelType) -> (Arc<Self>, mpsc::Sender<ChannelMessage>) {
        let (tx, rx) = mpsc::channel(256);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);

        let adapter = Arc::new(Self {
            name: name.to_string(),
            channel_type,
            rx: Mutex::new(Some(rx)),
            sent: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx,
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
/// configured operator user, mirroring how `TelegramAdapter` exposes its
/// `allowed_users`. Optionally carries an `account_id` so the bridge's
/// approval scoping (#4985) can resolve the right router channel key for
/// multi-bot configurations.
struct NotifyingAdapter {
    name: String,
    recipients: Vec<ChannelUser>,
    sent: Arc<Mutex<Vec<(String, String)>>>,
    account_id: Option<String>,
}

impl NotifyingAdapter {
    fn new(name: &str, recipients: Vec<ChannelUser>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            recipients,
            sent: Arc::new(Mutex::new(Vec::new())),
            account_id: None,
        })
    }

    fn with_account(name: &str, account_id: &str, recipients: Vec<ChannelUser>) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            recipients,
            sent: Arc::new(Mutex::new(Vec::new())),
            account_id: Some(account_id.to_string()),
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
        ChannelType::Telegram
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
