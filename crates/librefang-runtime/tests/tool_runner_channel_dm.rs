//! Integration tests for the `channel_dm` targeted-delivery tool (#7086),
//! dispatched through the runtime's real `execute_tool_raw`.
//!
//! The gap being closed: in a group conversation `channel_send` accepts only
//! the group as a target (the #6117 cross-chat dispatch guard), and
//! `notify_owner` — which that guard used to recommend — produces an
//! `owner_notice` in the reply envelope that no sidecar channel adapter routes
//! anywhere. An agent asked to tell one person their task had finished could
//! only broadcast it or drop it silently.
//!
//! What these tests pin down:
//!
//! - a targeted send reaches the named member and nobody else — in particular
//!   nothing is ever addressed to the group conversation as a fallback;
//! - the roster of the current conversation is the authorization set, so a
//!   platform id the daemon has never seen speak there is refused before any
//!   send;
//! - the whole path degrades into an explanatory error, never a broadcast,
//!   when the roster comes back empty (a DM, or a group whose members have
//!   never spoken);
//! - out-of-band callers with no inbound turn cannot use the tool at all,
//!   because there is no conversation whose roster could authorize anyone;
//! - the #6117 guard on `channel_send` now names a tool that can actually
//!   deliver.
//!
//! Same role-trait mock pattern as `tool_runner_agent_event.rs`: a
//! `RosterKernel` implementing the full `KernelHandle` composition, recording
//! calls on `ChannelSender` and stubbing everything else.

use async_trait::async_trait;
use librefang_kernel_handle::prelude::*;
use librefang_runtime::tool_runner::{execute_tool_raw, ToolExecContext};
use serde_json::json;
use std::path::Path;
use std::sync::{Arc, Mutex};

// --- Captured-call payloads ------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutboundMessage {
    channel: String,
    recipient: String,
    body: String,
    thread_id: Option<String>,
    account_id: Option<String>,
}

/// One roster read the tool performed.
///
/// `set` records *which* read — `"observed"` for `roster_observed_members` and `"all"` for `roster_members`.
/// It is part of the assertion rather than incidental bookkeeping: `channel_dm` authorizing against the wider read is the #7086 escalation, and a mock that answered both reads identically would let that change land green.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RosterQuery {
    channel: String,
    chat_id: String,
    set: &'static str,
}

struct Captures {
    sent: Arc<Mutex<Vec<OutboundMessage>>>,
    queries: Arc<Mutex<Vec<RosterQuery>>>,
}

// --- The mock kernel -------------------------------------------------------

struct RosterKernel {
    /// `(user_id, display_name)` pairs the daemon has **observed speaking** here.
    /// Empty models both a DM and a group nobody has spoken in.
    observed: Vec<(String, String)>,
    /// `(user_id, display_name)` pairs a platform's member list named, whom the daemon has never heard from (#7086).
    /// Present in `roster_members`, absent from `roster_observed_members` — exactly the split the real store makes with its `source` column.
    enumerated: Vec<(String, String)>,
    sent: Arc<Mutex<Vec<OutboundMessage>>>,
    queries: Arc<Mutex<Vec<RosterQuery>>>,
}

fn owned_pairs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(id, name)| ((*id).to_string(), (*name).to_string()))
        .collect()
}

impl RosterKernel {
    fn new(members: &[(&str, &str)]) -> (Self, Captures) {
        Self::with_enumerated(members, &[])
    }

    fn with_enumerated(observed: &[(&str, &str)], enumerated: &[(&str, &str)]) -> (Self, Captures) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let queries = Arc::new(Mutex::new(Vec::new()));
        let kernel = Self {
            observed: owned_pairs(observed),
            enumerated: owned_pairs(enumerated),
            sent: Arc::clone(&sent),
            queries: Arc::clone(&queries),
        };
        (kernel, Captures { sent, queries })
    }

    fn as_rows(pairs: &[(String, String)], source: &str) -> Vec<serde_json::Value> {
        pairs
            .iter()
            .map(|(user_id, display_name)| {
                json!({
                    "user_id": user_id,
                    "display_name": display_name,
                    "username": null,
                    "source": source,
                })
            })
            .collect()
    }

    fn record(&self, channel: &str, chat_id: &str, set: &'static str) {
        self.queries.lock().unwrap().push(RosterQuery {
            channel: channel.to_string(),
            chat_id: chat_id.to_string(),
            set,
        });
    }
}

#[async_trait]
impl ChannelSender for RosterKernel {
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        account_id: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        self.sent.lock().unwrap().push(OutboundMessage {
            channel: channel.to_string(),
            recipient: recipient.to_string(),
            body: message.to_string(),
            thread_id: thread_id.map(str::to_string),
            account_id: account_id.map(str::to_string),
        });
        Ok(format!("Message sent to {recipient} on {channel}"))
    }

    fn roster_members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        self.record(channel, chat_id, "all");
        let mut rows = Self::as_rows(&self.observed, "observed");
        rows.extend(Self::as_rows(&self.enumerated, "enumerated"));
        Ok(rows)
    }

    fn roster_observed_members(
        &self,
        channel: &str,
        chat_id: &str,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        self.record(channel, chat_id, "observed");
        Ok(Self::as_rows(&self.observed, "observed"))
    }
}

#[async_trait]
impl AgentControl for RosterKernel {
    async fn spawn_agent(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn send_to_agent(
        &self,
        _: &str,
        _: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn find_agents(&self, _: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for RosterKernel {
    fn memory_store(
        &self,
        _: &str,
        _: serde_json::Value,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_recall(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    fn memory_list(
        &self,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

impl WikiAccess for RosterKernel {}

#[async_trait]
impl TaskQueue for RosterKernel {
    async fn task_post(
        &self,
        _: &str,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_claim(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_complete(
        &self,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_list(
        &self,
        _: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_delete(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_retry(&self, _: &str) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_get(
        &self,
        _: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn task_update_status(
        &self,
        _: &str,
        _: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
}

#[async_trait]
impl EventBus for RosterKernel {
    async fn publish_event(
        &self,
        _: &str,
        _: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }
}

// All remaining role traits use their default impls — no behaviour needed
// for these tests, just trait coverage so `dyn KernelHandle` is satisfied.
#[async_trait]
impl KnowledgeGraph for RosterKernel {
    async fn knowledge_add_entity(
        &self,
        _: &librefang_types::memory::Entity,
        _agent_id: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_add_relation(
        &self,
        _: &librefang_types::memory::Relation,
        _agent_id: &str,
        _: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not implemented".into())
    }
    async fn knowledge_query(
        &self,
        _: librefang_types::memory::GraphPattern,
        _: Option<&str>,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not implemented".into())
    }
}
impl CronControl for RosterKernel {}
impl ApprovalGate for RosterKernel {}
impl HandsControl for RosterKernel {}
impl A2ARegistry for RosterKernel {}
impl PromptStore for RosterKernel {}
impl WorkflowRunner for RosterKernel {}
impl GoalControl for RosterKernel {}
impl ToolPolicy for RosterKernel {}
impl librefang_kernel_handle::CatalogQuery for RosterKernel {}
impl librefang_kernel_handle::ApiAuth for RosterKernel {
    fn auth_snapshot(&self) -> librefang_kernel_handle::ApiAuthSnapshot {
        librefang_kernel_handle::ApiAuthSnapshot::default()
    }
}
impl librefang_kernel_handle::SessionWriter for RosterKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _session_id: librefang_types::agent::SessionId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}

impl librefang_kernel_handle::AcpFsBridge for RosterKernel {}
impl librefang_kernel_handle::AcpTerminalBridge for RosterKernel {}

// --- Helpers ---------------------------------------------------------------

/// A turn arriving from a channel: `channel` / `chat_id` / `sender_id` are the
/// identity the tool derives its authorization set from, and none of them is a
/// tool argument.
fn turn_ctx<'a>(
    kernel: &'a Arc<dyn KernelHandle>,
    channel: Option<&'a str>,
    chat_id: Option<&'a str>,
    sender_id: Option<&'a str>,
    account_id: Option<&'a str>,
) -> ToolExecContext<'a> {
    ToolExecContext {
        kernel: Some(kernel),
        allowed_tools: None,
        available_tools: None,
        caller_agent_id: Some("agent-A"),
        skill_registry: None,
        allowed_skills: None,
        mcp_connections: None,
        web_ctx: None,
        browser_ctx: None,
        allowed_env_vars: None,
        workspace_root: None as Option<&Path>,
        media_engine: None,
        media_drivers: None,
        exec_policy: None,
        tts_engine: None,
        docker_config: None,
        process_manager: None,
        process_registry: None,
        sender_id,
        channel,
        chat_id,
        sender_account_id: account_id,
        session_id: None,
        spill_threshold_bytes: 0,
        max_artifact_bytes: 0,
        checkpoint_manager: None,
        interrupt: None,
        dangerous_command_checker: None,
        acting_principal: None,
    }
}

// --- channel_dm ------------------------------------------------------------

// The #7086 use case: in a shared group, tell one person their task is done
// without the rest of the group reading it. Exactly one message leaves, it is
// addressed to that member's platform id, and nothing at all is addressed to
// the group.
#[tokio::test]
async fn channel_dm_reaches_only_the_named_member() {
    let (kernel, caps) = RosterKernel::new(&[("U1", "Ana"), ("U2", "Bo")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(
        &kernel,
        Some("slack"),
        Some("C0DESIGN"),
        Some("U1"),
        Some("workspace-prod"),
    );
    let input = json!({"user_id": "U2", "message": "your export finished"});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(!result.is_error, "channel_dm failed: {}", result.content);

    let sent = caps.sent.lock().unwrap();
    assert_eq!(sent.len(), 1, "exactly one message must leave: {sent:?}");
    assert_eq!(
        sent[0],
        OutboundMessage {
            channel: "slack".to_string(),
            recipient: "U2".to_string(),
            body: "your export finished".to_string(),
            // The turn's thread belongs to the group; carrying it into a
            // one-to-one chat would address a thread that does not exist there.
            thread_id: None,
            // The bot account is inherited from the turn, never chosen by the
            // model, which is why no #6443 cross-account guard is needed here.
            account_id: Some("workspace-prod".to_string()),
        }
    );
    assert!(
        sent.iter().all(|m| m.recipient != "C0DESIGN"),
        "nothing may be addressed to the group: {sent:?}"
    );

    // The authorization set is the roster of the conversation the turn arrived
    // on — not one the model named.
    let queries = caps.queries.lock().unwrap();
    assert_eq!(
        *queries,
        vec![RosterQuery {
            channel: "slack".to_string(),
            chat_id: "C0DESIGN".to_string(),
            set: "observed",
        }]
    );
}

// The property that keeps #6117 closed: an arbitrary platform id is not a
// legal target, only someone the daemon has observed in this conversation.
#[tokio::test]
async fn channel_dm_refuses_a_platform_id_that_is_not_in_this_conversation() {
    let (kernel, caps) = RosterKernel::new(&[("U1", "Ana"), ("U2", "Bo")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U1"), None);
    let input = json!({"user_id": "U9", "message": "psst"});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(result.is_error, "a non-member must be refused");
    assert!(result.content.contains("U9"), "{}", result.content);
    assert!(
        result.content.contains("channel_members"),
        "the refusal must point at the tool that lists legal recipients: {}",
        result.content
    );
    assert!(
        caps.sent.lock().unwrap().is_empty(),
        "a refused recipient must produce no send at all"
    );
}

// The #7086 security boundary, and the reason bulk enumeration is not simply
// poured into the roster.
//
// `channel_members` may report everyone Slack lists in the channel. `channel_dm`
// may address only the subset that has actually spoken here. Bulk-filling the
// observational rows — the obvious way to answer "who is in this channel?" —
// would have widened the authorization set from "people this agent has
// interacted with" to "everyone the workspace lists", handing the agent a DM
// channel to strangers.
//
// This test is the negative control for that. `U7` is enumerated and has never
// spoken; the mock returns it from `roster_members` and withholds it from
// `roster_observed_members`, exactly as the store's `AND source = 'observed'`
// predicate does. Point `tool_channel_dm` at the wider read and the send
// succeeds and this fails.
#[tokio::test]
async fn channel_dm_refuses_an_enumerated_member_who_has_never_spoken() {
    let (kernel, caps) = RosterKernel::with_enumerated(&[("U1", "Ana")], &[("U7", "Never Spoken")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U1"), None);
    let input = json!({"user_id": "U7", "message": "psst"});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(
        result.is_error,
        "an enumerated-but-never-observed member must be refused: {}",
        result.content
    );
    assert!(
        caps.sent.lock().unwrap().is_empty(),
        "no send may leave the daemon for an unauthorized recipient"
    );
    assert!(
        result.content.contains("enumerated"),
        "the refusal must name the classification so the model can pick a reachable member instead: {}",
        result.content
    );

    // The authorization read itself must be the narrow one. Asserting only on
    // the refusal would still pass if the tool asked for every member and
    // filtered afterwards — a filter a later refactor can drop without any test
    // noticing.
    assert_eq!(
        *caps.queries.lock().unwrap(),
        vec![RosterQuery {
            channel: "slack".to_string(),
            chat_id: "C0DESIGN".to_string(),
            set: "observed",
        }],
        "channel_dm must authorize against roster_observed_members, never roster_members"
    );
}

// The other half of the same boundary: enumeration widens what can be
// *reported*, and `channel_members` is where that widening is supposed to show
// up. Reporting only the observed set would make bulk enumeration pointless;
// reporting it without the `source` marker would leave the model unable to tell
// who it can actually reach.
#[tokio::test]
async fn channel_members_reports_enumerated_members_and_marks_them() {
    let (kernel, _caps) =
        RosterKernel::with_enumerated(&[("U1", "Ana")], &[("U7", "Never Spoken")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U1"), None);
    let result = execute_tool_raw("t1", "channel_members", &json!({}), &ctx).await;

    assert!(
        !result.is_error,
        "channel_members failed: {}",
        result.content
    );
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["count"], 2);
    assert_eq!(parsed["observed_count"], 1);
    assert_eq!(parsed["enumerated_count"], 1);
    assert_eq!(parsed["members"][0]["user_id"], "U1");
    assert_eq!(parsed["members"][0]["source"], "observed");
    assert_eq!(parsed["members"][1]["user_id"], "U7");
    assert_eq!(parsed["members"][1]["source"], "enumerated");
    assert!(
        parsed["note"]
            .as_str()
            .unwrap_or_default()
            .contains("channel_dm"),
        "a mixed roster must say which members channel_dm will refuse: {}",
        result.content
    );
}

// Addressing the conversation id is the broadcast the caller was trying to
// avoid. It fails, and the error says which tool does that on purpose rather
// than describing it as a membership problem.
#[tokio::test]
async fn channel_dm_refuses_the_conversation_itself() {
    let (kernel, caps) = RosterKernel::new(&[("U1", "Ana")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U1"), None);
    let input = json!({"user_id": "C0DESIGN", "message": "everyone read this"});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(result.is_error, "the conversation id is not a member");
    assert!(
        result.content.contains("channel_send"),
        "{}",
        result.content
    );
    assert!(caps.sent.lock().unwrap().is_empty());
}

// The platform (or the roster) has nothing to say about this conversation: a
// DM, a group nobody has spoken in, or an adapter that stamps no sender id.
// The tool has to explain that rather than fall back to posting in the open —
// a "private" notice that silently becomes public is worse than an error.
#[tokio::test]
async fn channel_dm_degrades_to_an_error_when_the_roster_is_empty() {
    let (kernel, caps) = RosterKernel::new(&[]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, Some("slack"), Some("D0PRIVATE"), Some("U1"), None);
    let input = json!({"user_id": "U1", "message": "done"});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(result.is_error, "an empty roster authorizes nobody");
    assert!(
        result.content.contains("channel_members"),
        "{}",
        result.content
    );
    assert!(
        caps.sent.lock().unwrap().is_empty(),
        "an unresolved recipient must never fall back to a broadcast"
    );
    // The roster WAS consulted — the refusal is a real membership answer, not a
    // missing-context shortcut.
    assert_eq!(caps.queries.lock().unwrap().len(), 1);
}

// Out-of-band callers (cron, triggers, API-driven runs) carry no conversation,
// so no roster can authorize anyone. Refusing early is the point: accepting a
// caller-supplied conversation would let the model choose its own
// authorization set, which is the #6117 leak with extra steps.
#[tokio::test]
async fn channel_dm_is_unavailable_without_an_inbound_channel_turn() {
    let (kernel, caps) = RosterKernel::new(&[("U1", "Ana")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, None, None, None, None);
    let input = json!({"user_id": "U1", "message": "done"});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(result.is_error, "no turn means no authorization set");
    assert!(
        result.content.contains("channel_send"),
        "the refusal must name the tool that does work out of band: {}",
        result.content
    );
    assert!(
        caps.queries.lock().unwrap().is_empty(),
        "the kernel must not be queried at all"
    );
    assert!(caps.sent.lock().unwrap().is_empty());
}

// The WhatsApp gateway stamps `sender_channel = "whatsapp:<jid>"` (#5227). The
// suffix has to come off twice over: the roster is keyed on the bare channel
// type, and no registered adapter answers to the embedded form — so leaving it
// on would miss the roster AND fail the send.
#[tokio::test]
async fn channel_dm_strips_the_whatsapp_channel_suffix() {
    let (kernel, caps) = RosterKernel::new(&[("44777@s.whatsapp.net", "Ana")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(
        &kernel,
        Some("whatsapp:123@g.us"),
        Some("123@g.us"),
        Some("44888@s.whatsapp.net"),
        None,
    );
    let input = json!({"user_id": "44777@s.whatsapp.net", "message": "done"});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(!result.is_error, "channel_dm failed: {}", result.content);
    assert_eq!(
        *caps.queries.lock().unwrap(),
        vec![RosterQuery {
            channel: "whatsapp".to_string(),
            chat_id: "123@g.us".to_string(),
            set: "observed",
        }]
    );
    let sent = caps.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].channel, "whatsapp");
    assert_eq!(sent[0].recipient, "44777@s.whatsapp.net");
}

// An empty body would deliver a blank private message; catch it before the
// membership check so the model gets the useful error.
#[tokio::test]
async fn channel_dm_rejects_an_empty_message() {
    let (kernel, caps) = RosterKernel::new(&[("U2", "Bo")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U1"), None);
    let input = json!({"user_id": "U2", "message": "   "});
    let result = execute_tool_raw("t1", "channel_dm", &input, &ctx).await;

    assert!(result.is_error, "{}", result.content);
    assert!(caps.sent.lock().unwrap().is_empty());
}

// --- the #6117 guard's advice ---------------------------------------------

// Before #7086 the guard refused the send and then pointed the model at
// `notify_owner`, which no sidecar channel adapter routes: the model was told
// to use a path that silently delivered nothing, with no way to discover the
// failure. The refusal must now name a tool that actually delivers.
#[tokio::test]
async fn cross_chat_channel_send_now_points_at_a_path_that_delivers() {
    let (kernel, caps) = RosterKernel::new(&[("U2", "Bo")]);
    let kernel: Arc<dyn KernelHandle> = Arc::new(kernel);

    let ctx = turn_ctx(&kernel, Some("slack"), Some("C0DESIGN"), Some("U1"), None);
    let input = json!({"channel": "slack", "recipient": "U2", "message": "your export finished"});
    let result = execute_tool_raw("t1", "channel_send", &input, &ctx).await;

    assert!(
        result.is_error,
        "the #6117 cross-chat guard must still fire: {}",
        result.content
    );
    assert!(
        result.content.contains("channel_dm"),
        "the guard must name the tool that can deliver privately: {}",
        result.content
    );
    assert!(
        !result.content.contains("use notify_owner"),
        "notify_owner is not a channel delivery path: {}",
        result.content
    );
    assert!(caps.sent.lock().unwrap().is_empty());
}
