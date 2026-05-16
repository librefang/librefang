use super::workspace::NamedWsKernel;
use super::*;

#[test]
fn parse_poll_options_accepts_2_to_10_strings() {
    let raw = serde_json::json!(["red", "green", "blue"]);
    let opts = parse_poll_options(Some(&raw)).expect("valid options");
    assert_eq!(opts, vec!["red", "green", "blue"]);
}

#[test]
fn parse_poll_options_rejects_non_string_entry() {
    // Regression: a previous version used filter_map(as_str) which
    // silently dropped non-string entries, letting a malformed poll
    // slip past the min-2 validation.
    let raw = serde_json::json!(["a", 42, "c"]);
    let err = parse_poll_options(Some(&raw)).expect_err("should reject number");
    assert!(
        err.contains("poll_options[1]"),
        "error mentions index: {err}"
    );
    assert!(err.contains("number"), "error mentions type: {err}");
}

#[test]
fn parse_poll_options_rejects_bool_entry() {
    let raw = serde_json::json!(["a", true]);
    let err = parse_poll_options(Some(&raw)).expect_err("should reject bool");
    assert!(err.contains("poll_options[1]"));
    assert!(err.contains("boolean"));
}

#[test]
fn parse_poll_options_rejects_null_entry() {
    let raw = serde_json::json!(["a", null, "c"]);
    let err = parse_poll_options(Some(&raw)).expect_err("should reject null");
    assert!(err.contains("poll_options[1]"));
    assert!(err.contains("null"));
}

#[test]
fn parse_poll_options_rejects_too_few() {
    let raw = serde_json::json!(["only one"]);
    let err = parse_poll_options(Some(&raw)).expect_err("should reject single option");
    assert!(err.contains("between 2 and 10"));
}

#[test]
fn parse_poll_options_rejects_too_many() {
    let raw = serde_json::json!(["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]);
    let err = parse_poll_options(Some(&raw)).expect_err("should reject 11 options");
    assert!(err.contains("between 2 and 10"));
}

#[test]
fn parse_poll_options_rejects_missing() {
    let err = parse_poll_options(None).expect_err("None should fail");
    assert!(err.contains("must be an array"));
}

#[test]
fn parse_poll_options_rejects_non_array() {
    let raw = serde_json::json!("not an array");
    let err = parse_poll_options(Some(&raw)).expect_err("string should fail");
    assert!(err.contains("must be an array"));
}

// ── skill_read_file ────────────────────────────────────────────────

fn create_skill_registry_with_file(
    dir: &std::path::Path,
    skill_name: &str,
    file_rel: &str,
    content: &str,
) -> SkillRegistry {
    let skill_dir = dir.join(skill_name);
    std::fs::create_dir_all(
        skill_dir.join(
            std::path::Path::new(file_rel)
                .parent()
                .unwrap_or(std::path::Path::new("")),
        ),
    )
    .unwrap();
    std::fs::write(skill_dir.join(file_rel), content).unwrap();
    std::fs::write(
        skill_dir.join("skill.toml"),
        format!(
            r#"[skill]
name = "{skill_name}"
version = "0.1.0"
description = "test"
"#
        ),
    )
    .unwrap();

    let mut registry = SkillRegistry::new(dir.to_path_buf());
    registry.load_all().unwrap();
    registry
}

#[tokio::test]
async fn skill_read_file_reads_companion() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry =
        create_skill_registry_with_file(dir.path(), "my-skill", "refs/guide.md", "hello world");

    let input = serde_json::json!({ "skill": "my-skill", "path": "refs/guide.md" });
    let result = tool_skill_read_file(&input, Some(&registry), None).await;
    assert_eq!(result.unwrap(), "hello world");
}

#[tokio::test]
async fn skill_read_file_rejects_traversal() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry = create_skill_registry_with_file(dir.path(), "evil", "dummy.txt", "ok");

    let input = serde_json::json!({ "skill": "evil", "path": "../../etc/passwd" });
    let result = tool_skill_read_file(&input, Some(&registry), None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn skill_read_file_rejects_unknown_skill() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry = create_skill_registry_with_file(dir.path(), "exists", "f.txt", "ok");

    let input = serde_json::json!({ "skill": "nope", "path": "f.txt" });
    let result = tool_skill_read_file(&input, Some(&registry), None).await;
    assert!(result.unwrap_err().contains("not found"));
}

#[tokio::test]
async fn skill_read_file_rejects_absolute_path() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry = create_skill_registry_with_file(dir.path(), "abs", "dummy.txt", "ok");

    // Use a platform-appropriate absolute path so the test passes on Windows too.
    let abs_path = std::env::temp_dir()
        .join("passwd")
        .to_string_lossy()
        .into_owned();
    let input = serde_json::json!({ "skill": "abs", "path": abs_path });
    let result = tool_skill_read_file(&input, Some(&registry), None).await;
    assert!(result.unwrap_err().contains("absolute paths"));
}

#[tokio::test]
async fn skill_read_file_enforces_allowlist() {
    let dir = tempfile::TempDir::new().unwrap();
    let registry = create_skill_registry_with_file(dir.path(), "secret", "data.txt", "classified");

    // Agent only allowed "other-skill", not "secret"
    let allowed = vec!["other-skill".to_string()];
    let input = serde_json::json!({ "skill": "secret", "path": "data.txt" });
    let result = tool_skill_read_file(&input, Some(&registry), Some(&allowed)).await;
    assert!(result.unwrap_err().contains("not allowed"));

    // Empty allowlist means all skills are accessible
    let empty: Vec<String> = vec![];
    let result = tool_skill_read_file(&input, Some(&registry), Some(&empty)).await;
    assert!(result.is_ok());

    // None allowlist (deferred context) also allows access
    let result = tool_skill_read_file(&input, Some(&registry), None).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn skill_read_file_truncates_without_panic() {
    let dir = tempfile::TempDir::new().unwrap();
    // Create content with multi-byte chars that exceeds 32K bytes
    let content = "é".repeat(20_000); // 2 bytes each = 40K bytes
    let registry = create_skill_registry_with_file(dir.path(), "big", "large.txt", &content);

    let input = serde_json::json!({ "skill": "big", "path": "large.txt" });
    let result = tool_skill_read_file(&input, Some(&registry), None)
        .await
        .unwrap();
    assert!(result.contains("truncated"));
    // Must not panic — the point of this test
}
// -----------------------------------------------------------------------
// notify_owner tool (§A — owner-side channel)
// -----------------------------------------------------------------------

#[test]
fn notify_owner_tool_is_registered_in_builtins() {
    let defs = builtin_tool_definitions();
    let notify = defs.iter().find(|d| d.name == "notify_owner");
    assert!(
        notify.is_some(),
        "notify_owner must appear in builtin_tool_definitions"
    );
    let schema = &notify.unwrap().input_schema;
    let required = schema["required"].as_array().expect("required array");
    let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(names.contains(&"reason"));
    assert!(names.contains(&"summary"));
}

#[test]
fn notify_owner_tool_sets_owner_notice_and_opaque_ack() {
    let input = serde_json::json!({
        "reason": "confirmation_needed",
        "summary": "Caterina has asked for confirmation of the appointment."
    });
    let r = tool_notify_owner("toolu_1", &input);
    assert!(!r.is_error, "notify_owner should not be an error: {r:?}");
    assert_eq!(r.tool_use_id, "toolu_1");
    // Owner-side payload populated with prefixed reason.
    let payload = r.owner_notice.as_deref().expect("owner_notice set");
    assert!(payload.contains("confirmation_needed"));
    assert!(payload.contains("Caterina"));
    // Opaque ack does NOT echo the summary back to the model.
    assert!(!r.content.contains("Caterina"));
    assert!(!r.content.contains("confirmation_needed"));
}

#[test]
fn notify_owner_tool_rejects_empty_args() {
    let cases = vec![
        serde_json::json!({"reason": "", "summary": "x"}),
        serde_json::json!({"reason": "x", "summary": ""}),
        serde_json::json!({"reason": "x"}),
        serde_json::json!({"summary": "x"}),
        serde_json::json!({}),
    ];
    for input in cases {
        let r = tool_notify_owner("t", &input);
        assert!(r.is_error, "expected error for input {input:?}");
        assert!(r.owner_notice.is_none());
    }
}

// ── Lazy tool loading (issue #3044) ───────────────────────────────────

#[test]
fn test_tool_meta_load_returns_schema_and_side_channel() {
    let input = serde_json::json!({"name": "file_write"});
    let r = tool_meta_load(&input, None);
    assert!(!r.is_error);
    assert!(r.content.contains("file_write"));
    assert!(r.content.contains("input_schema") || r.content.contains("content"));
    // Side-channel must carry the full ToolDefinition for the agent loop.
    let def = r
        .loaded_tool
        .expect("loaded_tool side-channel must be populated");
    assert_eq!(def.name, "file_write");
    assert!(!def.description.is_empty());
}

#[test]
fn test_tool_meta_load_rejects_unknown_name() {
    let r = tool_meta_load(&serde_json::json!({"name": "not_a_real_tool"}), None);
    assert!(r.is_error);
    assert!(r.loaded_tool.is_none());
    assert!(r.content.to_lowercase().contains("unknown"));
}

#[test]
fn test_tool_meta_load_rejects_missing_name() {
    let r = tool_meta_load(&serde_json::json!({}), None);
    assert!(r.is_error);
    assert!(r.loaded_tool.is_none());
}

#[test]
fn test_tool_meta_search_finds_by_keyword() {
    let r = tool_meta_search(&serde_json::json!({"query": "write"}), None);
    assert!(!r.is_error);
    assert!(r.content.contains("file_write") || r.content.contains("memory_store"));
    assert!(r.loaded_tool.is_none()); // search doesn't load; only load loads.
}

#[test]
fn test_tool_meta_search_respects_limit() {
    let r = tool_meta_search(&serde_json::json!({"query": "file", "limit": 2}), None);
    assert!(!r.is_error);
    // At most 2 result lines (header line + max 2 match lines).
    let match_lines = r.content.lines().filter(|l| l.contains(": ")).count();
    assert!(match_lines <= 2, "expected ≤2 matches, got {match_lines}");
}

#[test]
fn test_tool_meta_search_rejects_empty_query() {
    let r = tool_meta_search(&serde_json::json!({"query": ""}), None);
    assert!(r.is_error);
}

#[test]
fn test_always_native_tools_includes_meta_tools() {
    // The meta-tools MUST be in the always-native set — otherwise the LLM
    // can never escape eager mode when the loop trims the tool list.
    assert!(ALWAYS_NATIVE_TOOLS.contains(&"tool_load"));
    assert!(ALWAYS_NATIVE_TOOLS.contains(&"tool_search"));
}

#[test]
fn test_builtin_tool_definitions_declares_meta_tools() {
    let defs = builtin_tool_definitions();
    assert!(defs.iter().any(|t| t.name == "tool_load"));
    assert!(defs.iter().any(|t| t.name == "tool_search"));
}

#[test]
fn test_select_native_tools_trims_to_native_set() {
    let defs = builtin_tool_definitions();
    let native = select_native_tools(&defs);
    // Result is a subset of the full builtin set.
    assert!(native.len() < defs.len());
    // Every returned tool's name is in ALWAYS_NATIVE_TOOLS.
    for t in &native {
        assert!(
            ALWAYS_NATIVE_TOOLS.contains(&t.name.as_str()),
            "unexpected native tool: {}",
            t.name
        );
    }
    // Every name in ALWAYS_NATIVE_TOOLS that exists in builtins must be present.
    let builtin_names: std::collections::HashSet<&str> =
        defs.iter().map(|t| t.name.as_str()).collect();
    for want in ALWAYS_NATIVE_TOOLS {
        if builtin_names.contains(want) {
            assert!(
                native.iter().any(|t| t.name == *want),
                "native set missing expected tool: {want}"
            );
        }
    }
}

#[test]
fn test_lazy_mode_reduces_serialized_tool_payload() {
    // Quantify the savings this PR is claiming (issue #3044). The lazy
    // set serialized as JSON should be dramatically smaller than the
    // full builtin set.
    let full = builtin_tool_definitions();
    let native = select_native_tools(&full);
    let full_bytes = serde_json::to_vec(&full).unwrap().len();
    let native_bytes = serde_json::to_vec(&native).unwrap().len();
    // Expect at least a 50% reduction — in practice it's ~75%.
    assert!(
        native_bytes * 2 < full_bytes,
        "native set ({native_bytes}B) should be less than half the full set ({full_bytes}B)"
    );
}

#[test]
fn test_tool_meta_load_resolves_non_builtin_from_available_tools() {
    // Regression for PR #3047 codex review P1: a non-builtin tool
    // (MCP/skill-provided) must be loadable via tool_load as long as it
    // exists in the agent's granted `available_tools` pool. Before the
    // fix `tool_meta_load` only scanned `builtin_tool_definitions()`,
    // so dynamic tools were stripped by lazy mode and unreachable.
    let dynamic = ToolDefinition {
        name: "mcp_custom_thing".to_string(),
        description: "A dynamically-registered MCP tool".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {"x": {"type": "string"}},
            "required": ["x"],
        }),
    };
    let pool = vec![dynamic.clone()];
    let r = tool_meta_load(
        &serde_json::json!({"name": "mcp_custom_thing"}),
        Some(&pool),
    );
    assert!(!r.is_error, "expected success, got: {}", r.content);
    let loaded = r
        .loaded_tool
        .expect("loaded_tool must populate for granted non-builtin");
    assert_eq!(loaded.name, "mcp_custom_thing");
    assert_eq!(loaded.description, dynamic.description);
}

#[test]
fn test_tool_meta_load_empty_pool_is_not_builtin_fallback() {
    // `Some(&[])` must mean "granted pool is empty" — NOT "caller didn't
    // provide one, please leak the builtin catalog". Only `None` falls
    // back to builtins (for legacy execute_tool paths). This keeps the
    // semantics unambiguous for future callers.
    let empty: Vec<ToolDefinition> = Vec::new();
    let r = tool_meta_load(&serde_json::json!({"name": "file_write"}), Some(&empty));
    assert!(
        r.is_error,
        "Some(&[]) must resolve as empty pool, got content: {}",
        r.content
    );
    assert!(r.loaded_tool.is_none());
    // Sanity: None still falls back to builtin and resolves file_write.
    let r_none = tool_meta_load(&serde_json::json!({"name": "file_write"}), None);
    assert!(!r_none.is_error);
    assert_eq!(
        r_none.loaded_tool.map(|d| d.name).as_deref(),
        Some("file_write")
    );
}

// ── file_read deduplication (#4971) ───────────────────────────────

fn make_dedup_kernel(enabled: bool) -> Arc<dyn KernelHandle> {
    // `NamedWsKernel`'s `ToolPolicy::deduplicate_file_reads` reads the
    // `dedup_enabled` field below — see its impl in this module.
    Arc::new(NamedWsKernel {
        named: vec![],
        download_dir: None,
        dedup_enabled: enabled,
    })
}

async fn run_file_read_for_dedup(
    kernel: &Arc<dyn KernelHandle>,
    workspace: &Path,
    rel_path: &str,
    session_id: &str,
) -> librefang_types::tool::ToolResult {
    execute_tool(
        "test-id",
        "file_read",
        &serde_json::json!({"path": rel_path}),
        Some(kernel),
        None,
        Some("00000000-0000-0000-0000-000000000099"),
        None,
        None,
        None,
        None,
        None,
        None,
        Some(workspace),
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
        Some(session_id),
        None,
        None,
    )
    .await
}

#[tokio::test]
async fn file_read_dedup_second_unchanged_read_returns_stub() {
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::write(workspace.path().join("a.txt"), "hello world").unwrap();
    let kernel = make_dedup_kernel(true);
    // Unique session id so the global tracker doesn't bleed between tests.
    let sid = uuid::Uuid::new_v4().to_string();

    let first = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid).await;
    assert!(!first.is_error, "first read errored: {}", first.content);
    assert_eq!(first.content, "hello world");

    let second = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid).await;
    assert!(!second.is_error, "second read errored: {}", second.content);
    assert!(
        second.content.contains("already read"),
        "expected stub, got: {}",
        second.content
    );
    assert!(
        second.content.contains("turn 1"),
        "stub must reference the first turn, got: {}",
        second.content
    );
    assert!(
        !second.content.contains("hello world"),
        "stub must not leak full content, got: {}",
        second.content
    );
}

#[tokio::test]
async fn file_read_dedup_changed_content_returns_updated_header() {
    let workspace = tempfile::tempdir().expect("tempdir");
    let target = workspace.path().join("a.txt");
    std::fs::write(&target, "v1").unwrap();
    let kernel = make_dedup_kernel(true);
    let sid = uuid::Uuid::new_v4().to_string();

    let first = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid).await;
    assert_eq!(first.content, "v1");

    std::fs::write(&target, "v2-updated").unwrap();
    let second = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid).await;
    assert!(
        second.content.contains("updated since last read"),
        "expected updated header, got: {}",
        second.content
    );
    assert!(
        second.content.contains("v2-updated"),
        "full new content must follow the header, got: {}",
        second.content
    );
}

#[tokio::test]
async fn file_read_dedup_disabled_returns_full_content_each_time() {
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::write(workspace.path().join("a.txt"), "hello world").unwrap();
    let kernel = make_dedup_kernel(false);
    let sid = uuid::Uuid::new_v4().to_string();

    let first = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid).await;
    assert_eq!(first.content, "hello world");
    let second = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid).await;
    // No stub, no header — verbatim.
    assert_eq!(second.content, "hello world");
}

#[tokio::test]
async fn file_read_dedup_reset_after_compression_clears_state() {
    let workspace = tempfile::tempdir().expect("tempdir");
    std::fs::write(workspace.path().join("a.txt"), "hello").unwrap();
    let kernel = make_dedup_kernel(true);
    let sid_str = uuid::Uuid::new_v4().to_string();
    let sid = librefang_types::agent::SessionId(uuid::Uuid::parse_str(&sid_str).unwrap());

    let _ = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid_str).await;
    // Simulate the compressor's reset hook.
    crate::context_compressor::reset_post_compression_side_state(sid);
    // After reset, the next read is treated as the first read again —
    // the agent sees full content rather than a stub.
    let after = run_file_read_for_dedup(&kernel, workspace.path(), "a.txt", &sid_str).await;
    assert_eq!(after.content, "hello");
}

#[test]
fn test_tool_meta_search_scopes_to_available_tools_when_provided() {
    // Search must also prefer the agent's granted pool so results never
    // hallucinate tools the agent can't actually call.
    let only = vec![ToolDefinition {
        name: "mcp_unique_name_zzz".to_string(),
        description: "keyword_zzz".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    }];
    let r = tool_meta_search(&serde_json::json!({"query": "keyword_zzz"}), Some(&only));
    assert!(!r.is_error);
    assert!(
        r.content.contains("mcp_unique_name_zzz"),
        "expected the granted tool to appear, got: {}",
        r.content
    );
    // builtin 'file_write' must NOT show up when the pool is scoped.
    assert!(
        !r.content.contains("file_write"),
        "search leaked outside the supplied pool: {}",
        r.content
    );
}

// ── skill evolve frozen-registry gating ───────────────────────────

#[tokio::test]
async fn test_evolve_tools_rejected_when_registry_frozen() {
    // In Stable mode (registry frozen) every evolution tool must
    // refuse at the handler boundary, BEFORE touching disk. The
    // `evolution` module underneath would happily write files that
    // the frozen registry never loads — burning reviewer tokens
    // and leaving disk state the operator explicitly didn't want.
    let tmp = tempfile::tempdir().unwrap();
    let mut registry = SkillRegistry::new(tmp.path().to_path_buf());
    registry.freeze();

    let input = serde_json::json!({
        "name": "gated",
        "description": "x",
        "prompt_context": "# x",
        "tags": [],
    });
    let err = tool_skill_evolve_create(&input, Some(&registry), None)
        .await
        .expect_err("must reject under freeze");
    assert!(
        err.contains("frozen") || err.contains("Stable"),
        "error must mention Stable/frozen, got: {err}"
    );

    let err = tool_skill_evolve_delete(&serde_json::json!({ "name": "gated" }), Some(&registry))
        .await
        .expect_err("delete must reject under freeze");
    assert!(err.contains("frozen") || err.contains("Stable"));

    let err = tool_skill_evolve_write_file(
        &serde_json::json!({
            "name": "gated",
            "path": "references/x.md",
            "content": "hi",
        }),
        Some(&registry),
    )
    .await
    .expect_err("write_file must reject under freeze");
    assert!(err.contains("frozen") || err.contains("Stable"));
}

// ── read_artifact tool (#3347) ─────────────────────────────────────────────

#[tokio::test]
async fn read_artifact_round_trip() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = b"artifact payload";
    let handle = crate::artifact_store::write(
        content,
        dir.path(),
        crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    )
    .unwrap();

    let input = serde_json::json!({ "handle": handle.as_str() });
    let result = tool_read_artifact(&input, dir.path()).await.unwrap();
    assert!(result.contains("artifact payload"), "got: {result}");
    assert!(result.contains("sha256:"), "got: {result}");
}

#[tokio::test]
async fn read_artifact_with_offset_and_length() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = b"0123456789abcdef";
    let handle = crate::artifact_store::write(
        content,
        dir.path(),
        crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    )
    .unwrap();

    let input = serde_json::json!({ "handle": handle.as_str(), "offset": 4, "length": 6 });
    let result = tool_read_artifact(&input, dir.path()).await.unwrap();
    assert!(result.contains("456789"), "got: {result}");
}

#[tokio::test]
async fn read_artifact_nonexistent_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let fake = "sha256:".to_string() + &"b".repeat(64);
    let input = serde_json::json!({ "handle": fake });
    let result = tool_read_artifact(&input, dir.path()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[tokio::test]
async fn read_artifact_missing_handle_returns_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let input = serde_json::json!({});
    let result = tool_read_artifact(&input, dir.path()).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("handle"));
}

#[tokio::test]
async fn read_artifact_past_end_returns_no_more_content_message() {
    let dir = tempfile::TempDir::new().unwrap();
    let content = b"short";
    let handle = crate::artifact_store::write(
        content,
        dir.path(),
        crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES,
    )
    .unwrap();

    let input = serde_json::json!({ "handle": handle.as_str(), "offset": 9999 });
    let result = tool_read_artifact(&input, dir.path()).await.unwrap();
    assert!(result.contains("past end"), "got: {result}");
}

#[test]
fn read_artifact_registered_in_builtins() {
    let defs = builtin_tool_definitions();
    let def = defs.iter().find(|d| d.name == "read_artifact");
    assert!(
        def.is_some(),
        "read_artifact must appear in builtin_tool_definitions"
    );
    let schema = &def.unwrap().input_schema;
    let required = schema["required"].as_array().expect("required array");
    assert!(required.iter().any(|v| v.as_str() == Some("handle")));
}

#[test]
fn read_artifact_in_always_native_tools() {
    assert!(
        ALWAYS_NATIVE_TOOLS.contains(&"read_artifact"),
        "read_artifact must be in ALWAYS_NATIVE_TOOLS"
    );
}
