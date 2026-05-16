use super::*;

/// Regression: when the per-user gate returns `NeedsApproval`, the
/// `DeferredToolExecution.force_human` flag MUST be set so the
/// kernel's `submit_tool_approval` can disable the hand-agent
/// auto-approve carve-out. (B3 of PR #3205 review.)
#[tokio::test]
async fn tool_runner_rbac_force_human_propagates_to_deferred() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(std::sync::Mutex::new(None));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ForceHumanCapturingKernel {
        approval_requests: Arc::clone(&approval_requests),
        last_force_human: Arc::clone(&last),
        user_gate_override: Some(librefang_types::user_policy::UserToolGate::NeedsApproval {
            reason: "user policy escalated".to_string(),
        }),
    });

    let workspace = tempfile::tempdir().expect("tempdir");
    let _ = execute_tool(
        "tu-1",
        "file_write",
        &serde_json::json!({"path": "scratch.txt", "content": "hi"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("bob"),
        Some("telegram"),
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
    assert_eq!(
        *last.lock().unwrap(),
        Some(true),
        "force_human must be true when user policy escalated"
    );
}

/// Sanity: when the user gate is `Allow` and only the global
/// `require_approval` list pulls the call into approval, `force_human`
/// stays false — hand-agent auto-approval keeps working in the
/// non-RBAC path.
#[tokio::test]
async fn tool_runner_rbac_force_human_stays_false_for_global_require_approval() {
    let approval_requests = Arc::new(AtomicUsize::new(0));
    let last = Arc::new(std::sync::Mutex::new(None));
    let kernel: Arc<dyn KernelHandle> = Arc::new(ForceHumanCapturingKernel {
        approval_requests: Arc::clone(&approval_requests),
        last_force_human: Arc::clone(&last),
        user_gate_override: Some(librefang_types::user_policy::UserToolGate::Allow),
    });

    let _ = execute_tool(
        "tu-1",
        "shell_exec",
        &serde_json::json!({"command": "echo ok"}),
        Some(&kernel),
        None,
        Some("agent-1"),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("alice"),
        Some("telegram"),
        None,
        None,
        None,
        None,
        None,
    )
    .await;

    assert_eq!(approval_requests.load(Ordering::SeqCst), 1);
    assert_eq!(*last.lock().unwrap(), Some(false));
}

#[test]
fn test_builtin_tool_definitions() {
    let tools = builtin_tool_definitions();
    assert!(
        tools.len() >= 40,
        "Expected at least 40 tools, got {}",
        tools.len()
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    // Original 12
    assert!(names.contains(&"file_read"));
    assert!(names.contains(&"shell_exec"));
    assert!(names.contains(&"agent_send"));
    assert!(names.contains(&"agent_spawn"));
    assert!(names.contains(&"agent_list"));
    assert!(names.contains(&"agent_kill"));
    assert!(names.contains(&"memory_store"));
    assert!(names.contains(&"memory_recall"));
    assert!(names.contains(&"memory_list"));
    // 7 collaboration tools
    assert!(names.contains(&"agent_find"));
    assert!(names.contains(&"task_post"));
    assert!(names.contains(&"task_claim"));
    assert!(names.contains(&"task_complete"));
    assert!(names.contains(&"task_list"));
    assert!(names.contains(&"task_status"));
    assert!(names.contains(&"event_publish"));
    // 5 new Phase 3 tools
    assert!(names.contains(&"schedule_create"));
    assert!(names.contains(&"schedule_list"));
    assert!(names.contains(&"schedule_delete"));
    assert!(names.contains(&"image_analyze"));
    assert!(names.contains(&"location_get"));
    assert!(names.contains(&"system_time"));
    // 6 browser tools
    assert!(names.contains(&"browser_navigate"));
    assert!(names.contains(&"browser_click"));
    assert!(names.contains(&"browser_type"));
    assert!(names.contains(&"browser_screenshot"));
    assert!(names.contains(&"browser_read_page"));
    assert!(names.contains(&"browser_close"));
    assert!(names.contains(&"browser_scroll"));
    assert!(names.contains(&"browser_wait"));
    assert!(names.contains(&"browser_run_js"));
    assert!(names.contains(&"browser_back"));
    // 3 media/image generation tools
    assert!(names.contains(&"media_describe"));
    assert!(names.contains(&"media_transcribe"));
    assert!(names.contains(&"image_generate"));
    // 3 video/music generation tools
    assert!(names.contains(&"video_generate"));
    assert!(names.contains(&"video_status"));
    assert!(names.contains(&"music_generate"));
    // 3 cron tools
    assert!(names.contains(&"cron_create"));
    assert!(names.contains(&"cron_list"));
    assert!(names.contains(&"cron_cancel"));
    // 1 channel send tool
    assert!(names.contains(&"channel_send"));
    // 4 hand tools
    assert!(names.contains(&"hand_list"));
    assert!(names.contains(&"hand_activate"));
    assert!(names.contains(&"hand_status"));
    assert!(names.contains(&"hand_deactivate"));
    // 3 voice/docker tools
    assert!(names.contains(&"text_to_speech"));
    assert!(names.contains(&"speech_to_text"));
    assert!(names.contains(&"docker_exec"));
    // Goal tracking tool
    assert!(names.contains(&"goal_update"));
    // Workflow execution tool
    assert!(names.contains(&"workflow_run"));
    // Canvas tool
    assert!(names.contains(&"canvas_present"));
}

#[test]
fn test_collaboration_tool_schemas() {
    let tools = builtin_tool_definitions();
    let collab_tools = [
        "agent_find",
        "task_post",
        "task_claim",
        "task_complete",
        "task_list",
        "task_status",
        "event_publish",
    ];
    for name in &collab_tools {
        let tool = tools
            .iter()
            .find(|t| t.name == *name)
            .unwrap_or_else(|| panic!("Tool '{}' not found", name));
        // Verify each has a valid JSON schema
        assert!(
            tool.input_schema.is_object(),
            "Tool '{}' schema should be an object",
            name
        );
        assert_eq!(
            tool.input_schema["type"], "object",
            "Tool '{}' should have type=object",
            name
        );
    }
}

#[tokio::test]
async fn test_file_read_missing() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": "nonexistent_99999/file.txt"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(
        result.is_error,
        "Expected error but got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_file_read_path_traversal_blocked() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": "../../etc/passwd"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("traversal"));
}

#[tokio::test]
async fn test_file_write_path_traversal_blocked() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let result = execute_tool(
        "test-id",
        "file_write",
        &serde_json::json!({"path": "../../../tmp/evil.txt", "content": "pwned"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("traversal"));
}

#[tokio::test]
async fn test_file_list_path_traversal_blocked() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let result = execute_tool(
        "test-id",
        "file_list",
        &serde_json::json!({"path": "/foo/../../etc"}),
        None,
        None,
        None,
        None,
        None, // allowed_skills
        None,
        None,
        None,
        None,
        Some(workspace.path()),
        None, // media_engine
        None, // media_drivers
        None, // exec_policy
        None, // tts_engine
        None, // docker_config
        None, // process_manager
        None, // process_registry
        None, // sender_id
        None, // channel
        None, // checkpoint_manager
        None, // interrupt
        None, // session_id
        None, // dangerous_command_checker
        None, // available_tools
    )
    .await;
    assert!(result.is_error);
    assert!(result.content.contains("traversal"));
}

// ── Named-workspace read-side support ────────────────────────────────
//
// Mock kernel that surfaces a configurable list of named workspaces
// (paired with their access modes) via `named_workspace_prefixes`.
// `readonly_workspace_prefixes` is derived from that list so the existing
// file_write denial path stays consistent.

pub(super) struct NamedWsKernel {
    pub(super) named: Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)>,
    /// Optional channel-bridge download dir surfaced via
    /// `KernelHandle::channel_file_download_dir` (#4434 regression test
    /// hook). `None` matches the default trait behaviour.
    pub(super) download_dir: Option<std::path::PathBuf>,
    /// Whether `ToolPolicy::deduplicate_file_reads()` should return `true`
    /// (#4971 regression test hook). The stub trait default is `false`.
    pub(super) dedup_enabled: bool,
}

// ---- BEGIN role-trait impls (split from former `impl KernelHandle for NamedWsKernel`, #3746) ----

#[async_trait::async_trait]
impl AgentControl for NamedWsKernel {
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

impl MemoryAccess for NamedWsKernel {
    fn memory_store(
        &self,
        _key: &str,
        _value: serde_json::Value,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_recall(
        &self,
        _key: &str,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Option<serde_json::Value>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }

    fn memory_list(
        &self,
        _agent_id: Option<&str>,
        _peer_id: Option<&str>,
    ) -> Result<Vec<String>, librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

impl WikiAccess for NamedWsKernel {}

#[async_trait::async_trait]
impl TaskQueue for NamedWsKernel {
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
impl EventBus for NamedWsKernel {
    async fn publish_event(
        &self,
        _event_type: &str,
        _payload: serde_json::Value,
    ) -> Result<(), librefang_kernel_handle::KernelOpError> {
        Err("not used".into())
    }
}

#[async_trait::async_trait]
impl KnowledgeGraph for NamedWsKernel {
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

impl ToolPolicy for NamedWsKernel {
    fn named_workspace_prefixes(
        &self,
        _agent_id: &str,
    ) -> Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)> {
        self.named.clone()
    }

    fn readonly_workspace_prefixes(&self, _agent_id: &str) -> Vec<std::path::PathBuf> {
        self.named
            .iter()
            .filter(|(_, m)| *m == librefang_types::agent::WorkspaceMode::ReadOnly)
            .map(|(p, _)| p.clone())
            .collect()
    }
    fn channel_file_download_dir(&self) -> Option<std::path::PathBuf> {
        self.download_dir.clone()
    }
    fn deduplicate_file_reads(&self) -> bool {
        self.dedup_enabled
    }
}

// No-op role-trait impls (#3746) — mock relies on default bodies.
impl CronControl for NamedWsKernel {}
impl HandsControl for NamedWsKernel {}
impl ApprovalGate for NamedWsKernel {}
impl A2ARegistry for NamedWsKernel {}
impl ChannelSender for NamedWsKernel {}
impl PromptStore for NamedWsKernel {}
impl WorkflowRunner for NamedWsKernel {}
impl GoalControl for NamedWsKernel {}
impl librefang_kernel_handle::CatalogQuery for NamedWsKernel {}
impl ApiAuth for NamedWsKernel {
    fn auth_snapshot(&self) -> ApiAuthSnapshot {
        ApiAuthSnapshot::default()
    }
}
impl SessionWriter for NamedWsKernel {
    fn inject_attachment_blocks(
        &self,
        _agent_id: librefang_types::agent::AgentId,
        _blocks: Vec<librefang_types::message::ContentBlock>,
    ) {
    }
}
impl AcpFsBridge for NamedWsKernel {}
impl AcpTerminalBridge for NamedWsKernel {}

// ---- END role-trait impls (#3746) ----

pub(super) fn make_named_ws_kernel(
    named: Vec<(std::path::PathBuf, librefang_types::agent::WorkspaceMode)>,
) -> Arc<dyn KernelHandle> {
    Arc::new(NamedWsKernel {
        named,
        download_dir: None,
        dedup_enabled: false,
    })
}

fn make_download_dir_kernel(download_dir: std::path::PathBuf) -> Arc<dyn KernelHandle> {
    Arc::new(NamedWsKernel {
        named: vec![],
        download_dir: Some(download_dir),
        dedup_enabled: false,
    })
}

#[tokio::test]
async fn test_file_read_allows_named_workspace_path() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();
    let target = shared_canon.join("note.txt");
    std::fs::write(&target, "hello shared").unwrap();

    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadWrite)]);

    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": target.to_str().unwrap()}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000001"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(!result.is_error, "got error: {}", result.content);
    assert_eq!(result.content, "hello shared");
}

#[tokio::test]
async fn test_file_list_allows_named_workspace_path() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();
    std::fs::write(shared_canon.join("a.txt"), "a").unwrap();
    std::fs::write(shared_canon.join("b.txt"), "b").unwrap();

    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

    let result = execute_tool(
        "test-id",
        "file_list",
        &serde_json::json!({"path": shared_canon.to_str().unwrap()}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000002"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(!result.is_error, "got error: {}", result.content);
    assert!(result.content.contains("a.txt"));
    assert!(result.content.contains("b.txt"));
}

/// #4434: channel bridges save attachments to a shared download dir
/// (default `/tmp/librefang_uploads`) which lives outside any agent's
/// `workspace_root`. The runtime must widen `file_read`'s sandbox
/// accept-list with `KernelHandle::channel_file_download_dir()` so
/// agents can open the very files the bridge tells them about.
#[tokio::test]
async fn test_file_read_allows_channel_download_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    let target = download_canon.join("attachment.txt");
    std::fs::write(&target, "from-telegram").unwrap();

    let kernel = make_download_dir_kernel(download_canon.clone());

    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": target.to_str().unwrap()}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000010"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(!result.is_error, "got error: {}", result.content);
    assert_eq!(result.content, "from-telegram");
}

/// Companion to the file_read test: file_list must also see into the
/// channel download dir so an agent can enumerate inbox attachments.
#[tokio::test]
async fn test_file_list_allows_channel_download_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    std::fs::write(download_canon.join("one.pdf"), "1").unwrap();
    std::fs::write(download_canon.join("two.pdf"), "2").unwrap();

    let kernel = make_download_dir_kernel(download_canon.clone());

    let result = execute_tool(
        "test-id",
        "file_list",
        &serde_json::json!({"path": download_canon.to_str().unwrap()}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000011"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(!result.is_error, "got error: {}", result.content);
    assert!(result.content.contains("one.pdf"));
    assert!(result.content.contains("two.pdf"));
}

/// #4981: media read tools (`image_analyze`, `media_describe`,
/// `media_transcribe`, `speech_to_text`) must also see into the
/// channel-bridge staging dir. The kernel writes inbound voice
/// notes and images there (e.g.
/// `/var/folders/.../T/librefang_uploads/<uuid>.oga`) and hands
/// the path to the agent — the agent's first tool call against
/// that exact path must not be rejected by the sandbox.
///
/// Tested via `image_analyze` because it has no `MediaEngine`
/// dependency. The dispatcher arm for the other three media
/// read tools (`media_describe`, `media_transcribe`,
/// `speech_to_text`) widens the allowlist with the same single
/// line — by inspection they share the security envelope this
/// test locks in.
#[tokio::test]
async fn test_image_analyze_allows_channel_download_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    // Minimal valid PNG (1x1, fully transparent) so `tokio::fs::read`
    // succeeds and `detect_image_format` returns "png".
    let png_bytes: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    let target = download_canon.join("inbound.png");
    std::fs::write(&target, png_bytes).unwrap();

    let kernel = make_download_dir_kernel(download_canon.clone());

    let result = execute_tool(
        "test-id",
        "image_analyze",
        &serde_json::json!({"path": target.to_str().unwrap()}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000020"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(
        !result.is_error,
        "media read should accept staging-dir path, got error: {}",
        result.content
    );
    assert!(
        result.content.contains("\"format\": \"png\""),
        "expected format=png in result, got: {}",
        result.content
    );
}

/// #4981 negative: a path that is OUTSIDE the workspace AND OUTSIDE
/// the channel staging dir must still be rejected by the media read
/// tools. This confirms the allowlist is scoped to the actual
/// staging-dir path, not its parent (e.g. `/var/folders/.../T/`).
#[tokio::test]
async fn test_image_analyze_rejects_path_outside_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let outside = tempfile::tempdir().expect("outside");
    let download_canon = download.path().canonicalize().unwrap();
    // File lives in a sibling tempdir — neither under the primary
    // workspace nor under the configured staging dir.
    let target = outside.path().canonicalize().unwrap().join("evil.png");
    std::fs::write(&target, [0x89, 0x50, 0x4E, 0x47]).unwrap();

    let kernel = make_download_dir_kernel(download_canon.clone());

    let result = execute_tool(
        "test-id",
        "image_analyze",
        &serde_json::json!({"path": target.to_str().unwrap()}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000021"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(
        result.is_error,
        "media read against path outside both workspace and staging dir must be rejected"
    );
    assert!(
        result.content.contains("Access denied")
            && result.content.contains("resolves outside workspace"),
        "expected sandbox-escape error, got: {}",
        result.content
    );
}

/// #4981 negative: a `..` traversal anchored inside the staging
/// dir must NOT escape the allowlist. `resolve_sandbox_path_ext`
/// rejects all `..` components up front, so even a path whose
/// literal prefix is the staging dir gets denied as soon as a
/// `..` component appears.
#[tokio::test]
async fn test_image_analyze_rejects_dotdot_escape_from_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();

    let kernel = make_download_dir_kernel(download_canon.clone());

    // `<staging>/..` would resolve to the parent (e.g. `/var/folders/.../T/`)
    // which we MUST NOT widen the allowlist to.
    let evil = format!("{}/../passwd", download_canon.to_str().unwrap());

    let result = execute_tool(
        "test-id",
        "image_analyze",
        &serde_json::json!({"path": evil}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000022"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(
        result.is_error,
        "`..` from inside the staging dir must be rejected"
    );
    // Either error wording satisfies the "dotdot escape rejected"
    // contract. Windows normalises `\\?\C:\…\..\passwd` into the
    // canonical UNC-extended form before `..` is examined, so the
    // sandbox-escape branch fires first there; Unix sees the literal
    // `..` component and trips `Path traversal denied`. Both are the
    // same security outcome — an attempted escape was rejected.
    let traversal_or_sandbox = result.content.contains("Path traversal denied")
        || result.content.contains("resolves outside workspace");
    assert!(
        traversal_or_sandbox,
        "expected path-traversal or sandbox-escape error, got: {}",
        result.content,
    );
}

// -----------------------------------------------------------------
// #4981 follow-up (PR #4995 review): the three remaining media read
// tools — `media_describe`, `media_transcribe`, `speech_to_text` —
// each get their own named positive / outside / dotdot test so a
// future copy-paste asymmetry in any one dispatcher arm fails CI
// with a precise test name, instead of being noticed only by
// inspection that all four arms "look identical".
//
// These tools require a real `MediaEngine` and will surface a
// provider-lookup error *after* the sandbox check (no API keys are
// set in tests). The positive case therefore asserts the negative
// invariant: the result MUST NOT carry a sandbox-rejection message.
// A future regression that drops the staging-dir widening from one
// of these arms would resurface as "Access denied: path '...'
// resolves outside workspace", which the positive assertion catches.
// -----------------------------------------------------------------

/// Bytes for a real Ogg/Opus voice note are not needed — the
/// sandbox check fires before the provider call, and the read of
/// the staged file just needs the file to exist. A tiny payload
/// keeps the tempdir cheap and avoids any accidental decode.
fn write_staged_audio(dir: &Path, name: &str) -> std::path::PathBuf {
    let p = dir.join(name);
    // "OggS" magic — harmless filler, the test never decodes it.
    std::fs::write(&p, [0x4F, 0x67, 0x67, 0x53]).unwrap();
    p
}

/// Drive `execute_tool` for one of the media read tools against
/// (primary workspace, staging dir kernel, target path) and return
/// the raw `ToolResult`. Centralising the 28-arg call keeps the
/// per-tool tests focused on the assertion that matters.
async fn run_media_read_tool(
    tool: &str,
    target_path: &str,
    primary: &Path,
    kernel: &Arc<dyn KernelHandle>,
    tool_use_id: &str,
) -> ToolResult {
    use crate::media_understanding::MediaEngine;
    use librefang_types::media::MediaConfig;

    // A real engine — `Option::None` would short-circuit with
    // "Media engine not available" BEFORE the sandbox check fires
    // and the test would no longer exercise the allowlist.
    let engine = MediaEngine::new(MediaConfig::default());

    execute_tool(
        "test-id",
        tool,
        &serde_json::json!({ "path": target_path }),
        Some(kernel),
        None,
        Some(tool_use_id),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary),
        Some(&engine),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await
}

/// Assert that a `ToolResult` did NOT come back with a sandbox
/// rejection. The media tools fail downstream (no provider keys
/// in tests) — that's expected and is NOT what these tests care
/// about. A regression in the dispatcher arm would surface as
/// "Access denied: path '...' resolves outside workspace", which
/// is what we lock out here.
fn assert_not_sandbox_reject(result: &ToolResult, tool: &str) {
    assert!(
        !(result.content.contains("Access denied")
            && result.content.contains("resolves outside workspace")),
        "{tool} rejected staging-dir path as sandbox escape — \
         allowlist widening regression. content: {}",
        result.content
    );
    assert!(
        !result.content.contains("Path traversal denied"),
        "{tool} flagged staging-dir path as traversal — \
         allowlist widening regression. content: {}",
        result.content
    );
}

// ---------- media_describe ----------

#[tokio::test]
async fn test_media_describe_allows_channel_download_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    // `media_describe` keys MIME off extension — `.png` is
    // accepted; the file body is never decoded before the
    // provider call, so the magic bytes only need to satisfy
    // `tokio::fs::read`.
    let target = download_canon.join("inbound.png");
    std::fs::write(&target, [0x89, 0x50, 0x4E, 0x47]).unwrap();

    let kernel = make_download_dir_kernel(download_canon.clone());
    let result = run_media_read_tool(
        "media_describe",
        target.to_str().unwrap(),
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000030",
    )
    .await;
    assert_not_sandbox_reject(&result, "media_describe");
}

#[tokio::test]
async fn test_media_describe_rejects_path_outside_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let outside = tempfile::tempdir().expect("outside");
    let download_canon = download.path().canonicalize().unwrap();
    let target = outside.path().canonicalize().unwrap().join("evil.png");
    std::fs::write(&target, [0x89, 0x50, 0x4E, 0x47]).unwrap();

    let kernel = make_download_dir_kernel(download_canon.clone());
    let result = run_media_read_tool(
        "media_describe",
        target.to_str().unwrap(),
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000031",
    )
    .await;
    assert!(
        result.is_error,
        "media_describe must reject path outside both workspace and staging dir"
    );
    assert!(
        result.content.contains("Access denied")
            && result.content.contains("resolves outside workspace"),
        "expected sandbox-escape error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_media_describe_rejects_dotdot_escape_from_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    let kernel = make_download_dir_kernel(download_canon.clone());

    let evil = format!("{}/../passwd", download_canon.to_str().unwrap());
    let result = run_media_read_tool(
        "media_describe",
        &evil,
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000032",
    )
    .await;
    assert!(
        result.is_error,
        "`..` from inside the staging dir must be rejected"
    );
    // Either error wording satisfies the "dotdot escape rejected"
    // contract. Windows normalises `\\?\C:\…\..\passwd` into the
    // canonical UNC-extended form before `..` is examined, so the
    // sandbox-escape branch fires first there; Unix sees the literal
    // `..` component and trips `Path traversal denied`. Both are the
    // same security outcome — an attempted escape was rejected.
    let traversal_or_sandbox = result.content.contains("Path traversal denied")
        || result.content.contains("resolves outside workspace");
    assert!(
        traversal_or_sandbox,
        "expected path-traversal or sandbox-escape error, got: {}",
        result.content,
    );
}

// ---------- media_transcribe ----------

#[tokio::test]
async fn test_media_transcribe_allows_channel_download_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    // `.oga` is the Telegram voice-note extension — the primary
    // path that motivated #4981.
    let target = write_staged_audio(&download_canon, "voice.oga");

    let kernel = make_download_dir_kernel(download_canon.clone());
    let result = run_media_read_tool(
        "media_transcribe",
        target.to_str().unwrap(),
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000033",
    )
    .await;
    assert_not_sandbox_reject(&result, "media_transcribe");
}

#[tokio::test]
async fn test_media_transcribe_rejects_path_outside_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let outside = tempfile::tempdir().expect("outside");
    let download_canon = download.path().canonicalize().unwrap();
    let target = write_staged_audio(&outside.path().canonicalize().unwrap(), "evil.oga");

    let kernel = make_download_dir_kernel(download_canon.clone());
    let result = run_media_read_tool(
        "media_transcribe",
        target.to_str().unwrap(),
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000034",
    )
    .await;
    assert!(
        result.is_error,
        "media_transcribe must reject path outside both workspace and staging dir"
    );
    assert!(
        result.content.contains("Access denied")
            && result.content.contains("resolves outside workspace"),
        "expected sandbox-escape error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_media_transcribe_rejects_dotdot_escape_from_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    let kernel = make_download_dir_kernel(download_canon.clone());

    let evil = format!("{}/../secret.oga", download_canon.to_str().unwrap());
    let result = run_media_read_tool(
        "media_transcribe",
        &evil,
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000035",
    )
    .await;
    assert!(
        result.is_error,
        "`..` from inside the staging dir must be rejected"
    );
    // Either error wording satisfies the "dotdot escape rejected"
    // contract. Windows normalises `\\?\C:\…\..\secret.oga` into the
    // canonical UNC-extended form before `..` is examined, so the
    // sandbox-escape branch fires first there; Unix sees the literal
    // `..` component and trips `Path traversal denied`. Both are the
    // same security outcome — an attempted escape was rejected.
    let traversal_or_sandbox = result.content.contains("Path traversal denied")
        || result.content.contains("resolves outside workspace");
    assert!(
        traversal_or_sandbox,
        "expected path-traversal or sandbox-escape error, got: {}",
        result.content,
    );
}

// ---------- speech_to_text ----------

#[tokio::test]
async fn test_speech_to_text_allows_channel_download_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    let target = write_staged_audio(&download_canon, "voice.mp3");

    let kernel = make_download_dir_kernel(download_canon.clone());
    let result = run_media_read_tool(
        "speech_to_text",
        target.to_str().unwrap(),
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000036",
    )
    .await;
    assert_not_sandbox_reject(&result, "speech_to_text");
}

#[tokio::test]
async fn test_speech_to_text_rejects_path_outside_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let outside = tempfile::tempdir().expect("outside");
    let download_canon = download.path().canonicalize().unwrap();
    let target = write_staged_audio(&outside.path().canonicalize().unwrap(), "evil.mp3");

    let kernel = make_download_dir_kernel(download_canon.clone());
    let result = run_media_read_tool(
        "speech_to_text",
        target.to_str().unwrap(),
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000037",
    )
    .await;
    assert!(
        result.is_error,
        "speech_to_text must reject path outside both workspace and staging dir"
    );
    assert!(
        result.content.contains("Access denied")
            && result.content.contains("resolves outside workspace"),
        "expected sandbox-escape error, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_speech_to_text_rejects_dotdot_escape_from_staging_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    let kernel = make_download_dir_kernel(download_canon.clone());

    let evil = format!("{}/../secret.mp3", download_canon.to_str().unwrap());
    let result = run_media_read_tool(
        "speech_to_text",
        &evil,
        primary.path(),
        &kernel,
        "00000000-0000-0000-0000-000000000038",
    )
    .await;
    assert!(
        result.is_error,
        "`..` from inside the staging dir must be rejected"
    );
    // Either error wording satisfies the "dotdot escape rejected"
    // contract. Windows normalises `\\?\C:\…\..\secret.mp3` into the
    // canonical UNC-extended form before `..` is examined, so the
    // sandbox-escape branch fires first there; Unix sees the literal
    // `..` component and trips `Path traversal denied`. Both are the
    // same security outcome — an attempted escape was rejected.
    let traversal_or_sandbox = result.content.contains("Path traversal denied")
        || result.content.contains("resolves outside workspace");
    assert!(
        traversal_or_sandbox,
        "expected path-traversal or sandbox-escape error, got: {}",
        result.content,
    );
}

/// Defense-in-depth: the download dir is a *read-side* allowlist only.
/// `file_write` still uses `named_ws_prefixes_writable`, so writes into
/// the bridge's directory must remain rejected.
#[tokio::test]
async fn test_file_write_rejects_channel_download_dir() {
    let primary = tempfile::tempdir().expect("primary");
    let download = tempfile::tempdir().expect("download");
    let download_canon = download.path().canonicalize().unwrap();
    let target = download_canon.join("smuggled.txt");

    let kernel = make_download_dir_kernel(download_canon.clone());

    let result = execute_tool(
        "test-id",
        "file_write",
        &serde_json::json!({
            "path": target.to_str().unwrap(),
            "content": "should-not-land",
        }),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000012"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(result.is_error, "expected write to be rejected");
    assert!(
        !target.exists(),
        "file should not have been written: {}",
        target.display()
    );
}

#[tokio::test]
async fn test_file_write_allows_rw_named_workspace_path() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();
    let target = shared_canon.join("out.txt");

    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadWrite)]);

    let result = execute_tool(
        "test-id",
        "file_write",
        &serde_json::json!({
            "path": target.to_str().unwrap(),
            "content": "wrote-it",
        }),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000003"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(!result.is_error, "got error: {}", result.content);
    let written = std::fs::read_to_string(&target).unwrap();
    assert_eq!(written, "wrote-it");
}

#[tokio::test]
async fn test_file_write_denies_readonly_named_workspace_path() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();
    let target = shared_canon.join("out.txt");

    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

    let result = execute_tool(
        "test-id",
        "file_write",
        &serde_json::json!({
            "path": target.to_str().unwrap(),
            "content": "should-not-write",
        }),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000004"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("read-only"),
        "expected read-only denial, got: {}",
        result.content
    );
    assert!(!target.exists(), "file should not have been written");
}

#[tokio::test]
async fn test_file_read_outside_all_workspaces_still_blocked() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let other = tempfile::tempdir().expect("other");
    let shared_canon = shared.path().canonicalize().unwrap();
    let other_path = other.path().canonicalize().unwrap().join("nope.txt");
    std::fs::write(&other_path, "secret").unwrap();

    let kernel = make_named_ws_kernel(vec![(shared_canon, WorkspaceMode::ReadWrite)]);

    let result = execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": other_path.to_str().unwrap()}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000005"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(result.is_error);
    assert!(
        result.content.contains("outside the agent's workspace"),
        "expected sandbox denial, got: {}",
        result.content
    );
}

#[tokio::test]
async fn test_apply_patch_allows_rw_named_workspace_path() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();
    let target = shared_canon.join("added.txt");

    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadWrite)]);

    let patch = format!(
        "*** Begin Patch\n*** Add File: {}\n+hello-from-patch\n*** End Patch\n",
        target.to_str().unwrap()
    );

    let result = execute_tool(
        "test-id",
        "apply_patch",
        &serde_json::json!({"patch": patch}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000006"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(!result.is_error, "got error: {}", result.content);
    let written = std::fs::read_to_string(&target).unwrap();
    assert_eq!(written, "hello-from-patch");
}

#[tokio::test]
async fn test_apply_patch_denies_readonly_named_workspace_path() {
    use librefang_types::agent::WorkspaceMode;

    let primary = tempfile::tempdir().expect("primary");
    let shared = tempfile::tempdir().expect("shared");
    let shared_canon = shared.path().canonicalize().unwrap();
    let target = shared_canon.join("added.txt");

    let kernel = make_named_ws_kernel(vec![(shared_canon.clone(), WorkspaceMode::ReadOnly)]);

    let patch = format!(
        "*** Begin Patch\n*** Add File: {}\n+should-not-write\n*** End Patch\n",
        target.to_str().unwrap()
    );

    let result = execute_tool(
        "test-id",
        "apply_patch",
        &serde_json::json!({"patch": patch}),
        Some(&kernel),
        None,
        Some("00000000-0000-0000-0000-000000000007"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(primary.path()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .await;
    assert!(result.is_error, "expected denial, got: {}", result.content);
    assert!(!target.exists(), "file should not have been written");
}

// ── Bug #3822: shell_exec must respect named workspace read-only mode ────
