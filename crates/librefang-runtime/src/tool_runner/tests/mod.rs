//! Inline unit tests for tool_runner — moved out of mod.rs in #3710 so the
//! parent module stays focused on dispatch + child-module wiring.

use super::agent::{build_agent_manifest_toml, tools_to_parent_capabilities};
use super::channel::parse_poll_options;
use super::image::{detect_image_format, extract_image_dimensions, format_file_size};
use super::media::{audio_mime_from_ext, SUPPORTED_AUDIO_EXTS_DOC};
use super::schedule::parse_schedule_to_cron;
use super::*;
use librefang_skills::registry::SkillRegistry;
use librefang_types::taint::TaintSink;
use librefang_types::tool::{ToolDefinition, ToolResult};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// ── audio_mime_from_ext ──────────────────────────────────────────────────

#[test]
fn audio_mime_from_ext_maps_known_audio_types() {
    assert_eq!(audio_mime_from_ext("mp3"), Some("audio/mpeg"));
    assert_eq!(audio_mime_from_ext("wav"), Some("audio/wav"));
    assert_eq!(audio_mime_from_ext("ogg"), Some("audio/ogg"));
    assert_eq!(audio_mime_from_ext("flac"), Some("audio/flac"));
    assert_eq!(audio_mime_from_ext("m4a"), Some("audio/mp4"));
    assert_eq!(audio_mime_from_ext("webm"), Some("audio/webm"));
    // `.oga` is a distinct MIME on purpose — see fn doc-comment.
    assert_eq!(audio_mime_from_ext("oga"), Some("audio/oga"));
    assert_ne!(audio_mime_from_ext("oga"), audio_mime_from_ext("ogg"));
}

#[test]
fn audio_mime_from_ext_returns_none_for_unsupported() {
    assert_eq!(audio_mime_from_ext(""), None);
    assert_eq!(audio_mime_from_ext("txt"), None);
    assert_eq!(audio_mime_from_ext("opus"), None);
    // Caller is expected to lowercase before invoking.
    assert_eq!(audio_mime_from_ext("OGA"), None);
}

#[test]
fn supported_audio_exts_doc_lists_every_implemented_extension() {
    let exts: Vec<&str> = SUPPORTED_AUDIO_EXTS_DOC
        .split(", ")
        .map(|s| s.trim())
        .collect();
    assert!(!exts.is_empty(), "const must list at least one extension");
    for ext in &exts {
        assert!(
            audio_mime_from_ext(ext).is_some(),
            "SUPPORTED_AUDIO_EXTS_DOC lists '{ext}' but audio_mime_from_ext does not map it"
        );
    }
}

// ── check_taint_outbound_text ────────────────────────────────────────

#[test]
fn test_taint_outbound_text_blocks_key_value_pairs() {
    let sink = TaintSink::agent_message();
    for body in [
        "here is my api_key=sk-123",
        "x-api-key: abcdef",
        "{\"token\":\"mytoken\"}",
        "{\"authorization\": \"Bearer sk-live-secret\"}",
        "{\"proxy-authorization\": \"Basic Zm9vOmJhcg==\"}",
        "api_key = sk-123",
        "'password': 'hunter2'",
        "Authorization: Bearer abc",
        "some text bearer=abc",
    ] {
        assert!(
            check_taint_outbound_text(body, &sink).is_some(),
            "outbound taint check must reject {body:?}"
        );
    }
}

#[test]
fn test_taint_outbound_text_blocks_well_known_prefixes() {
    let sink = TaintSink::agent_message();
    for tok in [
        "sk-12345678901234567890123456789012",
        "ghp_1234567890123456789012345678901234567890",
        "xoxb-0000-0000-xxxxxxxxxxxx",
        "AKIAIOSFODNN7EXAMPLE",
        "AIzaSyDummyGoogleKeyLooksLikeThis00",
    ] {
        assert!(
            check_taint_outbound_text(tok, &sink).is_some(),
            "outbound taint check must reject well-known prefix {tok:?}"
        );
    }
}

#[test]
fn test_taint_outbound_text_blocks_long_opaque_tokens() {
    let sink = TaintSink::agent_message();
    // 40-char mixed-case base64-ish payload with no whitespace or
    // prose: smells like a raw bearer token.
    let payload = "AbCdEf0123456789AbCdEf0123456789AbCdEf01";
    assert!(
        check_taint_outbound_text(payload, &sink).is_some(),
        "outbound taint check must reject long opaque token"
    );
    // Same length but with punctuation — also looks tokenish.
    let payload_punct = "abcdef0123456789-abcdef0123456789-abcdef";
    assert!(
        check_taint_outbound_text(payload_punct, &sink).is_some(),
        "outbound taint check must reject punctuated token"
    );
}

#[test]
fn test_taint_outbound_text_allows_git_sha() {
    // 40-char lowercase hex commit SHA — legitimate inter-agent
    // payload, must not be blocked.
    let sink = TaintSink::agent_message();
    let sha = "18060f6401234567890abcdef0123456789abcde";
    assert!(
        check_taint_outbound_text(sha, &sink).is_none(),
        "git commit SHA must not be treated as a secret"
    );
}

#[test]
fn test_taint_outbound_text_allows_sha256_hex() {
    // 64-char lowercase hex sha256 digest — also legitimate.
    let sink = TaintSink::agent_message();
    let digest = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    assert!(
        check_taint_outbound_text(digest, &sink).is_none(),
        "sha256 hex digest must not be treated as a secret"
    );
}

#[test]
fn test_taint_outbound_text_allows_uuid_hex() {
    // 32-char UUID-without-dashes (hex) — allowed.
    let sink = TaintSink::agent_message();
    let uuid = "550e8400e29b41d4a716446655440000";
    assert!(
        check_taint_outbound_text(uuid, &sink).is_none(),
        "undashed UUID must not be treated as a secret"
    );
}

#[test]
fn test_taint_outbound_header_blocks_authorization_bearer() {
    // Regression for the header-name-bypass bug: a Bearer token
    // with a space between scheme and value defeats every
    // content-based heuristic, so we must trip on the header name.
    let sink = TaintSink::net_fetch();
    assert!(
        check_taint_outbound_header("Authorization", "Bearer sk-x", &sink).is_some(),
        "Authorization: Bearer <anything> must be blocked"
    );
    assert!(
        check_taint_outbound_header("authorization", "Token abc", &sink).is_some(),
        "lowercased authorization header must also be blocked"
    );
    assert!(
        check_taint_outbound_header("Proxy-Authorization", "Basic Zm9vOmJhcg==", &sink).is_some(),
        "Proxy-Authorization header must be blocked"
    );
    assert!(
        check_taint_outbound_header("X-Api-Key", "hunter2", &sink).is_some(),
        "X-Api-Key header must be blocked"
    );
}

#[test]
fn test_taint_outbound_header_allows_benign_headers() {
    let sink = TaintSink::net_fetch();
    assert!(
        check_taint_outbound_header("Accept", "application/json", &sink).is_none(),
        "benign Accept header must pass"
    );
    assert!(
        check_taint_outbound_header("User-Agent", "librefang/1.0", &sink).is_none(),
        "benign User-Agent header must pass"
    );
}

#[test]
fn test_taint_outbound_text_allows_prose() {
    let sink = TaintSink::agent_message();
    for benign in [
        "Please summarise this article about encryption.",
        "Could you check whether our token economy works?",
        "The passwd file lives at /etc/passwd on Linux — explain it.",
        "Write a haiku about secret gardens.",
        "",
    ] {
        assert!(
            check_taint_outbound_text(benign, &sink).is_none(),
            "outbound taint check must allow prose: {benign:?}"
        );
    }
}

#[test]
fn test_taint_outbound_text_allows_short_identifiers() {
    // A 16-char id is below the 32-char opaque-token threshold and
    // doesn't match any key=value shape, so it should pass even
    // though it looks alphanumeric.
    let sink = TaintSink::agent_message();
    let id = "req_0123456789ab";
    assert!(check_taint_outbound_text(id, &sink).is_none());
}

// ── tool_a2a_send / tool_channel_send taint integration ─────────────
//
// Regression: prior to this patch the taint sink was only enforced
// on agent_send and web_fetch. tool_a2a_send and tool_channel_send
// were exfiltration sinks with NO check at all.

#[tokio::test]
async fn test_tool_a2a_send_blocks_secret_in_message() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::new(AtomicUsize::new(0)),
        user_gate_override: None,
    });
    let input = serde_json::json!({
        "agent_url": "https://example.com/a2a",
        "message": "leaking api_key=sk-abcdefghijklmnop now",
    });
    let err = tool_a2a_send(&input, Some(&kernel))
        .await
        .expect_err("a2a_send must reject tainted message");
    assert!(
        err.contains("taint") || err.contains("violation"),
        "expected taint violation, got: {err}"
    );
}

#[tokio::test]
async fn test_tool_channel_send_blocks_secret_in_text_message() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::new(AtomicUsize::new(0)),
        user_gate_override: None,
    });
    let input = serde_json::json!({
        "channel": "telegram",
        "recipient": "@user",
        "message": "here is the api_key=sk-abcdefghijklmnop",
    });
    let err = tool_channel_send(&input, Some(&kernel), None, Some("test_user_id"), None, &[])
        .await
        .expect_err("channel_send must reject tainted message");
    assert!(
        err.contains("taint") || err.contains("violation"),
        "expected taint violation, got: {err}"
    );
}

#[tokio::test]
async fn test_tool_channel_send_blocks_secret_in_image_caption() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::new(AtomicUsize::new(0)),
        user_gate_override: None,
    });
    let input = serde_json::json!({
        "channel": "telegram",
        "recipient": "@user",
        "image_url": "https://example.com/cat.png",
        "message": "see attached. token=sk-abcdefghijklmnop",
    });
    let err = tool_channel_send(&input, Some(&kernel), None, Some("test_user_id"), None, &[])
        .await
        .expect_err("image caption must be sink-checked");
    assert!(
        err.contains("taint") || err.contains("violation"),
        "expected taint violation, got: {err}"
    );
}

#[tokio::test]
async fn test_tool_channel_send_blocks_secret_in_poll_question() {
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::new(AtomicUsize::new(0)),
        user_gate_override: None,
    });
    let input = serde_json::json!({
        "channel": "telegram",
        "recipient": "@user",
        "poll_question": "guess my api_key=sk-abcdefghijklmnop",
        "poll_options": ["yes", "no"],
    });
    let err = tool_channel_send(&input, Some(&kernel), None, Some("test_user_id"), None, &[])
        .await
        .expect_err("poll question must be sink-checked");
    assert!(
        err.contains("taint") || err.contains("violation"),
        "expected taint violation, got: {err}"
    );
}

#[tokio::test]
async fn test_tool_channel_send_auto_fills_recipient_from_sender_id() {
    // Test that channel_send uses sender_id when recipient is omitted
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::new(AtomicUsize::new(0)),
        user_gate_override: None,
    });
    let input = serde_json::json!({
        "channel": "telegram",
        // recipient intentionally omitted
        "message": "Hello from auto-reply!",
    });
    // This should NOT error with "Missing recipient" because sender_id is provided
    // It will error with "Channel send not available" because the mock kernel
    // doesn't implement channel_send, but that's expected
    let result = tool_channel_send(
        &input,
        Some(&kernel),
        None,
        Some("12345_telegram"),
        None,
        &[],
    )
    .await;
    // The error should NOT be about missing recipient
    let err_msg = result.unwrap_err();
    assert!(
        !err_msg.contains("Missing 'recipient'"),
        "Expected auto-fill to work, but got: {err_msg}"
    );
}

#[tokio::test]
async fn test_tool_channel_send_requires_recipient_without_sender_id() {
    // Test that channel_send still requires recipient when sender_id is None
    let kernel: Arc<dyn KernelHandle> = Arc::new(ApprovalKernel {
        approval_requests: Arc::new(AtomicUsize::new(0)),
        user_gate_override: None,
    });
    let input = serde_json::json!({
        "channel": "telegram",
        // recipient intentionally omitted
        "message": "Hello!",
    });
    let err = tool_channel_send(&input, Some(&kernel), None, None, None, &[])
        .await
        .expect_err("channel_send must require recipient without sender_id");
    assert!(
        err.contains("Missing 'recipient'"),
        "Expected missing recipient error, got: {err}"
    );
}

// ── channel_send mirror tests ────────────────────────────────────────────

/// A minimal kernel for mirror tests.
///
/// - `send_channel_message` always succeeds (returns Ok).
/// - `resolve_channel_owner` returns the configured `owner_id`.
/// - `append_to_session` records calls into `appended`.
/// - `fail_append` makes `append_to_session` simulate a save failure (warn path).
struct MirrorKernel {
    owner_id: Option<librefang_types::agent::AgentId>,
    appended: Arc<std::sync::Mutex<Vec<librefang_types::message::Message>>>,
    fail_append: bool,
}

#[async_trait::async_trait]
impl AgentControl for MirrorKernel {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn send_to_agent(
        &self,
        _agent_id: &str,
        _message: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }
    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}
impl MemoryAccess for MirrorKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    fn memory_list(
        &self,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}
impl WikiAccess for MirrorKernel {}
#[async_trait::async_trait]
impl KnowledgeGraph for MirrorKernel {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Ok(vec![])
    }
}
#[async_trait::async_trait]
impl TaskQueue for MirrorKernel {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_claim(
        &self,
        _agent_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }
    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
    async fn task_list(
        &self,
        _status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(vec![])
    }
    async fn task_delete(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
    async fn task_retry(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
    async fn task_get(
        &self,
        _task_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Ok(None)
    }
    async fn task_update_status(
        &self,
        _task_id: &str,
        _new_status: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Ok(false)
    }
}
impl ApprovalGate for MirrorKernel {}
impl CronControl for MirrorKernel {}
impl HandsControl for MirrorKernel {}
impl A2ARegistry for MirrorKernel {}
impl PromptStore for MirrorKernel {}
impl WorkflowRunner for MirrorKernel {}
impl GoalControl for MirrorKernel {}
impl ToolPolicy for MirrorKernel {}
impl librefang_kernel_handle::CatalogQuery for MirrorKernel {}
impl ApiAuth for MirrorKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}
impl AcpFsBridge for MirrorKernel {}
impl AcpTerminalBridge for MirrorKernel {}

#[async_trait::async_trait]
impl EventBus for MirrorKernel {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl ChannelSender for MirrorKernel {
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        _message: &str,
        _thread_id: Option<&str>,
        _account_id: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Ok(format!("sent to {recipient} on {channel}"))
    }

    fn resolve_channel_owner(
        &self,
        _channel: &str,
        _chat_id: &str,
    ) -> Option<librefang_types::agent::AgentId> {
        self.owner_id
    }
}

impl SessionWriter for MirrorKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }

    fn append_to_session(
        &self,
        _session_id: librefang_types::agent::SessionId,
        _agent_id: librefang_types::agent::AgentId,
        message: librefang_types::message::Message,
    ) {
        if self.fail_append {
            // Simulate a save failure — caller should not see this error.
            tracing::warn!("MirrorKernel: simulated append_to_session failure");
            return;
        }
        self.appended.lock().unwrap().push(message);
    }
}

// `multi_thread` is required so that the `block_in_place` call inside
// `append_to_session` does not panic (block_in_place requires a
// multi-threaded runtime). This test exercises the mock-only path;
// the real block_in_place coverage lives in `channel_send_mirror_test.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn test_channel_send_mirrors_to_channel_owner_session() {
    use librefang_types::agent::AgentId;
    use librefang_types::message::Role;

    let owner = AgentId(uuid::Uuid::parse_str("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee").unwrap());
    let appended = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(MirrorKernel {
        owner_id: Some(owner),
        appended: Arc::clone(&appended),
        fail_append: false,
    });

    let input = serde_json::json!({
        "channel": "telegram",
        "recipient": "99999",
        "message": "Hello from cron agent",
    });

    let result = tool_channel_send(
        &input,
        Some(&kernel),
        None,
        Some("99999"),
        Some("caller-agent-id"),
        &[],
    )
    .await;

    assert!(result.is_ok(), "send should succeed: {:?}", result);

    let msgs = appended.lock().unwrap();
    assert_eq!(msgs.len(), 1, "exactly one message should be mirrored");
    assert_eq!(
        msgs[0].role,
        Role::User,
        "mirrored message must use user role"
    );

    let content = msgs[0].content.text_content();
    assert_eq!(
        content, r#"{"mirror_from":"caller-agent-id","body":"Hello from cron agent"}"#,
        "mirror text must be a JSON envelope with mirror_from and body fields"
    );
}

// `multi_thread` is required so that the `block_in_place` call inside
// `append_to_session` does not panic. This test exercises the mock-only
// path; the real block_in_place coverage lives in `channel_send_mirror_test.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn test_channel_send_mirrors_when_caller_is_channel_owner() {
    // Decision 1: mirror unconditionally, even when caller == owner.
    use librefang_types::agent::AgentId;
    use librefang_types::message::Role;

    let owner = AgentId(uuid::Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap());
    let appended = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(MirrorKernel {
        owner_id: Some(owner),
        appended: Arc::clone(&appended),
        fail_append: false,
    });

    let input = serde_json::json!({
        "channel": "telegram",
        "recipient": "42",
        "message": "Self-mirror test",
    });

    // caller_agent_id could be the same agent as the channel owner
    let result = tool_channel_send(
        &input,
        Some(&kernel),
        None,
        Some("42"),
        Some("same-agent"),
        &[],
    )
    .await;

    assert!(result.is_ok(), "send should succeed: {:?}", result);
    let msgs = appended.lock().unwrap();
    assert_eq!(msgs.len(), 1, "mirror must land even when caller == owner");
    assert_eq!(msgs[0].role, Role::User);
}

// `multi_thread` is required so that the `block_in_place` call inside
// `append_to_session` does not panic. This test exercises the mock-only
// path; the real block_in_place coverage lives in `channel_send_mirror_test.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn test_channel_send_succeeds_even_when_mirror_fails() {
    // Decision 3: mirror failure must not fail the tool call.
    let owner = librefang_types::agent::AgentId(
        uuid::Uuid::parse_str("ffffffff-ffff-ffff-ffff-ffffffffffff").unwrap(),
    );
    let appended = Arc::new(std::sync::Mutex::new(Vec::new()));
    let kernel: Arc<dyn KernelHandle> = Arc::new(MirrorKernel {
        owner_id: Some(owner),
        appended: Arc::clone(&appended),
        fail_append: true, // simulates a save error
    });

    let input = serde_json::json!({
        "channel": "telegram",
        "recipient": "77",
        "message": "Mirror failure test",
    });

    let result = tool_channel_send(
        &input,
        Some(&kernel),
        None,
        Some("77"),
        Some("caller-id"),
        &[],
    )
    .await;

    // Platform send must still succeed even though append failed.
    assert!(
        result.is_ok(),
        "tool call must succeed despite mirror failure"
    );
    // fail_append returns without pushing — confirm nothing was appended.
    let msgs = appended.lock().unwrap();
    assert!(msgs.is_empty(), "no message appended on simulated failure");
}

// ── end channel_send mirror tests ────────────────────────────────────────

struct ApprovalKernel {
    approval_requests: Arc<AtomicUsize>,
    /// RBAC M3 — overrides what `resolve_user_tool_decision` returns
    /// for every call. `None` keeps the default-impl behaviour
    /// (`UserToolGate::Allow`) so pre-RBAC tests are unaffected.
    user_gate_override: Option<librefang_types::user_policy::UserToolGate>,
}

/// Captures the `DeferredToolExecution.force_human` flag so tests
/// can assert that the user-gate escalation propagates through.
struct ForceHumanCapturingKernel {
    approval_requests: Arc<AtomicUsize>,
    last_force_human: Arc<std::sync::Mutex<Option<bool>>>,
    user_gate_override: Option<librefang_types::user_policy::UserToolGate>,
}

// ---- BEGIN role-trait impls (split from former `impl KernelHandle for ApprovalKernel`, #3746) ----

#[async_trait::async_trait]
impl AgentControl for ApprovalKernel {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn send_to_agent(
        &self,
        _agent_id: &str,
        _message: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for ApprovalKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_list(
        &self,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

impl WikiAccess for ApprovalKernel {}

#[async_trait::async_trait]
impl TaskQueue for ApprovalKernel {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_claim(
        &self,
        _agent_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_list(
        &self,
        _status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_delete(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_retry(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_get(
        &self,
        _task_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_update_status(
        &self,
        _task_id: &str,
        _new_status: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl EventBus for ApprovalKernel {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl KnowledgeGraph for ApprovalKernel {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl ApprovalGate for ApprovalKernel {
    fn requires_approval(&self, tool_name: &str) -> bool {
        tool_name == "shell_exec"
    }

    async fn request_approval(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        _action_summary: &str,
        _session_id: Option<&str>,
    ) -> Result<librefang_types::approval::ApprovalDecision, librefang_kernel_handle::KernelOpError>
    {
        self.approval_requests.fetch_add(1, Ordering::SeqCst);
        Ok(librefang_types::approval::ApprovalDecision::Denied)
    }

    async fn submit_tool_approval(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        _action_summary: &str,
        _deferred: librefang_types::tool::DeferredToolExecution,
        _session_id: Option<&str>,
    ) -> Result<librefang_types::tool::ToolApprovalSubmission, librefang_kernel_handle::KernelOpError>
    {
        self.approval_requests.fetch_add(1, Ordering::SeqCst);
        Ok(librefang_types::tool::ToolApprovalSubmission::Pending {
            request_id: uuid::Uuid::new_v4(),
        })
    }

    fn resolve_user_tool_decision(
        &self,
        _tool_name: &str,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> librefang_types::user_policy::UserToolGate {
        self.user_gate_override
            .clone()
            .unwrap_or(librefang_types::user_policy::UserToolGate::Allow)
    }
}

// No-op role-trait impls (#3746) — mock relies on default bodies.
impl CronControl for ApprovalKernel {}
impl HandsControl for ApprovalKernel {}
impl A2ARegistry for ApprovalKernel {}
impl ChannelSender for ApprovalKernel {}
impl PromptStore for ApprovalKernel {}
impl WorkflowRunner for ApprovalKernel {}
impl GoalControl for ApprovalKernel {}
impl ToolPolicy for ApprovalKernel {}
impl librefang_kernel_handle::CatalogQuery for ApprovalKernel {}
impl ApiAuth for ApprovalKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}
impl SessionWriter for ApprovalKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl AcpFsBridge for ApprovalKernel {}
impl AcpTerminalBridge for ApprovalKernel {}

// ---- END role-trait impls (#3746) ----

// ---- BEGIN role-trait impls (split from former `impl KernelHandle for ForceHumanCapturingKernel`, #3746) ----

#[async_trait::async_trait]
impl AgentControl for ForceHumanCapturingKernel {
    async fn spawn_agent(
        &self,
        _manifest_toml: &str,
        _parent_id: Option<&str>,
    ) -> Result<(String, String), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn send_to_agent(
        &self,
        _agent_id: &str,
        _message: &str,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn list_agents(&self) -> Vec<AgentInfo> {
        vec![]
    }

    fn kill_agent(&self, _agent_id: &str) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
        vec![]
    }
}

impl MemoryAccess for ForceHumanCapturingKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_list(
        &self,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

impl WikiAccess for ForceHumanCapturingKernel {}

#[async_trait::async_trait]
impl TaskQueue for ForceHumanCapturingKernel {
    async fn task_post(
        &self,
        _title: &str,
        _description: &str,
        _assigned_to: Option<&str>,
        _created_by: Option<&str>,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_claim(
        &self,
        _agent_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_complete(
        &self,
        _agent_id: &str,
        _task_id: &str,
        _result: &str,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_list(
        &self,
        _status: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_delete(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_retry(
        &self,
        _task_id: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_get(
        &self,
        _task_id: &str,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn task_update_status(
        &self,
        _task_id: &str,
        _new_status: &str,
    ) -> Result<bool, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl EventBus for ForceHumanCapturingKernel {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl KnowledgeGraph for ForceHumanCapturingKernel {
    async fn knowledge_add_entity(
        &self,
        _entity: &librefang_types::memory::Entity,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_add_relation(
        &self,
        _relation: &librefang_types::memory::Relation,
    ) -> Result<String, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    async fn knowledge_query(
        &self,
        _pattern: librefang_types::memory::GraphPattern,
    ) -> Result<Vec<librefang_types::memory::GraphMatch>, librefang_kernel_handle::KernelOpError>
    {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl ApprovalGate for ForceHumanCapturingKernel {
    fn requires_approval(&self, tool_name: &str) -> bool {
        tool_name == "shell_exec"
    }

    async fn submit_tool_approval(
        &self,
        _agent_id: &str,
        _tool_name: &str,
        _action_summary: &str,
        deferred: librefang_types::tool::DeferredToolExecution,
        _session_id: Option<&str>,
    ) -> Result<librefang_types::tool::ToolApprovalSubmission, librefang_kernel_handle::KernelOpError>
    {
        self.approval_requests.fetch_add(1, Ordering::SeqCst);
        *self.last_force_human.lock().unwrap() = Some(deferred.force_human);
        Ok(librefang_types::tool::ToolApprovalSubmission::Pending {
            request_id: uuid::Uuid::new_v4(),
        })
    }

    fn resolve_user_tool_decision(
        &self,
        _tool_name: &str,
        _sender_id: Option<&str>,
        _channel: Option<&str>,
    ) -> librefang_types::user_policy::UserToolGate {
        self.user_gate_override
            .clone()
            .unwrap_or(librefang_types::user_policy::UserToolGate::Allow)
    }
}

// No-op role-trait impls (#3746) — mock relies on default bodies.
impl CronControl for ForceHumanCapturingKernel {}
impl HandsControl for ForceHumanCapturingKernel {}
impl A2ARegistry for ForceHumanCapturingKernel {}
impl ChannelSender for ForceHumanCapturingKernel {}
impl PromptStore for ForceHumanCapturingKernel {}
impl WorkflowRunner for ForceHumanCapturingKernel {}
impl GoalControl for ForceHumanCapturingKernel {}
impl ToolPolicy for ForceHumanCapturingKernel {}
impl librefang_kernel_handle::CatalogQuery for ForceHumanCapturingKernel {}
impl ApiAuth for ForceHumanCapturingKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}
impl SessionWriter for ForceHumanCapturingKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl AcpFsBridge for ForceHumanCapturingKernel {}
impl AcpTerminalBridge for ForceHumanCapturingKernel {}

// ---- END role-trait impls (#3746) ----

mod policy;
mod shell;
mod skills;
mod workspace;
