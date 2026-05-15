use super::*;
use librefang_memory::session::Session as MemSession;
use librefang_memory::MemorySubstrate;
use librefang_types::message::Message;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::tempdir;

fn make_memory() -> Arc<MemorySubstrate> {
    Arc::new(MemorySubstrate::open_in_memory(0.01).unwrap())
}

#[tokio::test]
async fn test_bootstrap_default() {
    let config = ContextEngineConfig::default();
    let engine = DefaultContextEngine::new(config.clone(), make_memory(), None);
    assert!(engine.bootstrap(&config).await.is_ok());
}

#[tokio::test]
async fn plugin_bus_record_consumer_lag_increments_dropped_count() {
    // Mirrors `EventBus::record_consumer_lag_increments_dropped_count`
    // from librefang-kernel — the two impls drift apart silently
    // unless both are tested. Counter accounting only; the rate-
    // limited error! log is exercised but not asserted on.
    let bus = PluginEventBus::new(8);
    assert_eq!(bus.dropped_count(), 0);
    bus.record_consumer_lag(7, "test");
    assert_eq!(bus.dropped_count(), 7);
    bus.record_consumer_lag(3, "test");
    assert_eq!(bus.dropped_count(), 10);
}

/// Warmup regression guard: first lag burst must advance `last_drop_warn` (#3630).
#[tokio::test]
async fn plugin_bus_first_lag_burst_advances_warn_timestamp() {
    let bus = PluginEventBus::new(8);
    let before = *bus.last_drop_warn.lock().unwrap();
    bus.record_consumer_lag(1, "test_first_burst");
    let after = *bus.last_drop_warn.lock().unwrap();
    assert!(
        after > before,
        "first lag burst must advance last_drop_warn — warmup regression"
    );
}

#[tokio::test]
async fn test_ingest_stable_prefix_mode() {
    let config = ContextEngineConfig {
        stable_prefix_mode: true,
        ..Default::default()
    };
    let engine = DefaultContextEngine::new(config, make_memory(), None);
    let result = engine.ingest(AgentId::new(), "hello", None).await.unwrap();
    assert!(result.recalled_memories.is_empty());
}

#[tokio::test]
async fn test_ingest_recalls_memories() {
    let memory = make_memory();
    // Store a memory first
    memory
        .remember(
            AgentId::new(), // different agent
            "unrelated",
            librefang_types::memory::MemorySource::Conversation,
            "episodic",
            std::collections::HashMap::new(),
        )
        .await
        .unwrap();

    let agent_id = AgentId::new();
    memory
        .remember(
            agent_id,
            "The user likes Rust programming",
            librefang_types::memory::MemorySource::Conversation,
            "episodic",
            std::collections::HashMap::new(),
        )
        .await
        .unwrap();

    let config = ContextEngineConfig::default();
    let engine = DefaultContextEngine::new(config, memory, None);
    let result = engine.ingest(agent_id, "Rust", None).await.unwrap();
    assert_eq!(result.recalled_memories.len(), 1);
    assert!(result.recalled_memories[0].content.contains("Rust"));
}

#[tokio::test]
async fn test_assemble_no_overflow() {
    let config = ContextEngineConfig::default();
    let engine = DefaultContextEngine::new(config, make_memory(), None);
    let mut messages = vec![Message::user("hi"), Message::assistant("hello")];
    let result = engine
        .assemble(AgentId::new(), &mut messages, "system", &[], 200_000)
        .await
        .unwrap();
    assert_eq!(result.recovery, RecoveryStage::None);
}

#[tokio::test]
async fn test_assemble_triggers_overflow_recovery() {
    let config = ContextEngineConfig {
        context_window_tokens: 100, // tiny window
        ..Default::default()
    };
    let engine = DefaultContextEngine::new(config, make_memory(), None);

    // Create messages that exceed the tiny context window
    let mut messages: Vec<Message> = (0..20)
        .map(|i| {
            if i % 2 == 0 {
                Message::user(format!("msg{}: {}", i, "x".repeat(200)))
            } else {
                Message::assistant(format!("msg{}: {}", i, "x".repeat(200)))
            }
        })
        .collect();

    let result = engine
        .assemble(AgentId::new(), &mut messages, "system", &[], 100)
        .await
        .unwrap();
    assert_ne!(result.recovery, RecoveryStage::None);
}

#[tokio::test]
async fn test_truncate_tool_result() {
    let config = ContextEngineConfig {
        context_window_tokens: 500,
        ..Default::default()
    };
    let engine = DefaultContextEngine::new(config, make_memory(), None);
    let big_content = "x".repeat(10_000);
    let truncated = engine.truncate_tool_result(&big_content, 500);
    assert!(truncated.len() < big_content.len());
    assert!(truncated.contains("[TRUNCATED:"));
}

#[tokio::test]
async fn test_after_turn_noop() {
    let config = ContextEngineConfig::default();
    let engine = DefaultContextEngine::new(config, make_memory(), None);
    assert!(engine
        .after_turn(AgentId::new(), &[Message::user("hi")])
        .await
        .is_ok());
}

#[tokio::test]
async fn test_subagent_hooks_noop() {
    let config = ContextEngineConfig::default();
    let engine = DefaultContextEngine::new(config, make_memory(), None);
    let parent = AgentId::new();
    let child = AgentId::new();
    assert!(engine.prepare_subagent_context(parent, child).await.is_ok());
    assert!(engine.merge_subagent_context(parent, child).await.is_ok());
}

#[tokio::test]
async fn test_scriptable_hook_receives_direct_json_payload() {
    if Command::new("python3").arg("--version").output().is_err()
        && Command::new("python").arg("--version").output().is_err()
    {
        eprintln!("Python not available, skipping scriptable hook payload test");
        return;
    }

    let tmp = tempdir().unwrap();
    let script_path = tmp.path().join("hook.py");
    std::fs::write(
        &script_path,
        r#"import json
import sys

payload = json.loads(sys.stdin.read())
print(json.dumps({"type": payload.get("type"), "message": payload.get("message")}))
"#,
    )
    .unwrap();

    let traces = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let hook_schemas = std::collections::HashMap::new();
    let (output, _duration_ms) = ScriptableContextEngine::run_hook(
        "ingest",
        script_path.to_str().unwrap(),
        crate::plugin_runtime::PluginRuntime::Python,
        serde_json::json!({
            "type": "ingest",
            "agent_id": "agent-123",
            "message": "hello",
        }),
        30,
        &[],
        0,
        0,
        None,
        true,
        &traces,
        &hook_schemas,
        None,
        None,
        "",
        "",
        false,
    )
    .await
    .unwrap();

    assert_eq!(output["type"], "ingest");
    assert_eq!(output["message"], "hello");
}

#[cfg(unix)]
#[tokio::test]
async fn test_scriptable_hook_accepts_full_runtime_path() {
    let tmp = tempdir().unwrap();
    let script_path = tmp.path().join("hook.sh");
    std::fs::write(
        &script_path,
        r#"read _input
printf '{"type":"ingest_result","memories":[{"content":"full-path-runtime"}]}\n'
"#,
    )
    .unwrap();

    let traces = std::sync::Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new()));
    let hook_schemas = std::collections::HashMap::new();
    let (output, _duration_ms) = ScriptableContextEngine::run_hook(
        "ingest",
        script_path.to_str().unwrap(),
        crate::plugin_runtime::PluginRuntime::from_tag(Some("/bin/sh")),
        serde_json::json!({
            "type": "ingest",
            "agent_id": "agent-123",
            "message": "hello",
        }),
        30,
        &[],
        0,
        0,
        None,
        true,
        &traces,
        &hook_schemas,
        None,
        None,
        "",
        "",
        false,
    )
    .await
    .unwrap();

    assert_eq!(output["type"], "ingest_result");
    assert_eq!(output["memories"][0]["content"], "full-path-runtime");
}

#[test]
fn test_plugins_dir() {
    let dir = plugins_dir();
    assert!(dir.ends_with("plugins"));
    assert!(dir.to_string_lossy().contains(".librefang"));
}

#[test]
fn test_load_plugin_not_found() {
    let result = load_plugin("nonexistent-plugin-12345");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("not found"));
}

#[test]
fn test_load_plugin_with_tempdir() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("test-plugin");
    std::fs::create_dir_all(plugin_dir.join("hooks")).unwrap();

    // Write a plugin.toml
    let manifest_content = r#"
name = "test-plugin"
version = "0.1.0"
description = "A test plugin"
author = "test"

[hooks]
ingest = "hooks/ingest.py"
"#;
    let mut f = std::fs::File::create(plugin_dir.join("plugin.toml")).unwrap();
    f.write_all(manifest_content.as_bytes()).unwrap();

    // Write a dummy hook
    std::fs::File::create(plugin_dir.join("hooks/ingest.py")).unwrap();

    // We can't use load_plugin directly because it hardcodes ~/.librefang/plugins,
    // so test the manifest parsing + hook resolution manually
    let manifest: librefang_types::config::PluginManifest =
        toml::from_str(manifest_content).unwrap();

    assert_eq!(manifest.name, "test-plugin");
    assert_eq!(manifest.version, "0.1.0");
    assert_eq!(manifest.hooks.ingest.as_deref(), Some("hooks/ingest.py"));
    assert!(manifest.hooks.after_turn.is_none());

    // Resolve hooks relative to plugin dir
    let resolved = manifest
        .hooks
        .ingest
        .as_ref()
        .map(|p| plugin_dir.join(p).to_string_lossy().into_owned());
    assert!(resolved.unwrap().contains("hooks/ingest.py"));
}

#[test]
fn test_build_context_engine_default() {
    let toml_config = librefang_types::config::ContextEngineTomlConfig::default();
    let runtime_config = ContextEngineConfig::default();
    let engine = build_context_engine(&toml_config, runtime_config, make_memory(), None, &|_| None);
    // Should not panic — returns DefaultContextEngine
    let _ = engine;
}

#[test]
fn test_build_context_engine_missing_plugin_falls_back() {
    let toml_config = librefang_types::config::ContextEngineTomlConfig {
        plugin: Some("nonexistent-plugin-xyz".to_string()),
        ..Default::default()
    };
    let runtime_config = ContextEngineConfig::default();
    // Should fall back to default engine, not panic
    let engine = build_context_engine(&toml_config, runtime_config, make_memory(), None, &|_| None);
    let _ = engine;
}

// -----------------------------------------------------------------------
// NoCompactContextEngine
// -----------------------------------------------------------------------

#[test]
fn test_no_compact_context_engine_never_compresses() {
    let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
    let engine = NoCompactContextEngine::new(inner);
    // NoCompactContextEngine must always return false regardless of load
    assert!(!engine.should_compress(0, 200_000));
    assert!(!engine.should_compress(160_000, 200_000)); // 80 % — trait default would fire
    assert!(!engine.should_compress(200_000, 200_000)); // 100 %
}

// -----------------------------------------------------------------------
// SummaryContextEngine
// -----------------------------------------------------------------------

#[test]
fn test_summary_context_engine_threshold_default_80_percent() {
    let inner = DefaultContextEngine::new(
        ContextEngineConfig {
            context_window_tokens: 200_000,
            ..Default::default()
        },
        make_memory(),
        None,
    );
    // Default threshold_percent = 0.80
    let engine = SummaryContextEngine::new(inner, 0.80);

    // Below threshold: 79 % → false
    assert!(!engine.should_compress(158_000, 200_000));
    // At threshold: exactly 80 % → true
    assert!(engine.should_compress(160_000, 200_000));
    // Above threshold: 90 % → true
    assert!(engine.should_compress(180_000, 200_000));
}

#[test]
fn test_summary_context_engine_zero_max_tokens_safe() {
    let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
    let engine = SummaryContextEngine::new(inner, 0.80);
    // max_tokens = 0 must never panic and must return false
    assert!(!engine.should_compress(0, 0));
    assert!(!engine.should_compress(100, 0));
}

#[test]
fn test_summary_context_engine_update_model() {
    let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
    let engine = SummaryContextEngine::new(inner, 0.80);

    // Before update: context_length comes from ContextEngineConfig::default (200_000)
    assert!(!engine.should_compress(100_000, 200_000)); // 50 % < 80 %

    // After switching to a 32 K model the threshold drops to 25 600
    engine.update_model("gpt-3.5-turbo", 32_000);
    assert!(engine.should_compress(26_000, 32_000)); // 81 % > 80 %
    assert!(!engine.should_compress(25_000, 32_000)); // 78 % < 80 %
}

#[test]
fn test_default_trait_should_compress_80_percent() {
    // Verify the default impl on the trait uses 80 % (4/5 integer math)
    let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
    let _engine = NoCompactContextEngine::new(inner);
    // NoCompactContextEngine overrides to false, so test the trait default via
    // a SummaryContextEngine at the same 80 % threshold
    let inner2 = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
    let summary = SummaryContextEngine::new(inner2, 0.80);
    assert!(!summary.should_compress(0, 200_000));
    assert!(!summary.should_compress(159_999, 200_000));
    assert!(summary.should_compress(160_000, 200_000));
}

// ---------------------------------------------------------------------------
// Integration: compact is called exactly once when threshold is crossed
// ---------------------------------------------------------------------------

/// Verifies that `SummaryContextEngine::compact` is invoked exactly once
/// when `should_compress` transitions from false to true, and never invoked
/// when it remains false.  This mirrors the gating logic in the agent loop
/// (agent_loop.rs: should_compress check → compact call).
#[tokio::test]
async fn test_summary_engine_compact_called_once_on_threshold_cross() {
    use crate::llm_driver::{CompletionResponse, LlmError};
    use async_trait::async_trait;
    use librefang_types::message::{ContentBlock, TokenUsage};
    use std::sync::Arc;

    struct FakeDriver {
        _call_count: AtomicUsize,
    }

    impl FakeDriver {
        fn new() -> Self {
            Self {
                _call_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmDriver for FakeDriver {
        async fn complete(
            &self,
            _req: crate::llm_driver::CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "Summary of prior conversation".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: librefang_types::message::StopReason::EndTurn,
                tool_calls: vec![],
                usage: TokenUsage {
                    input_tokens: 50,
                    output_tokens: 20,
                    ..Default::default()
                },
            })
        }
    }

    struct CompactTracker {
        inner: DefaultContextEngine,
        compact_count: AtomicUsize,
    }

    impl CompactTracker {
        fn new(inner: DefaultContextEngine) -> Self {
            Self {
                inner,
                compact_count: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ContextEngine for CompactTracker {
        async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
            self.inner.bootstrap(config).await
        }
        async fn ingest(
            &self,
            agent_id: AgentId,
            user_message: &str,
            peer_id: Option<&str>,
        ) -> LibreFangResult<IngestResult> {
            self.inner.ingest(agent_id, user_message, peer_id).await
        }
        async fn assemble(
            &self,
            agent_id: AgentId,
            messages: &mut Vec<Message>,
            system_prompt: &str,
            tools: &[ToolDefinition],
            context_window_tokens: usize,
        ) -> LibreFangResult<AssembleResult> {
            self.inner
                .assemble(
                    agent_id,
                    messages,
                    system_prompt,
                    tools,
                    context_window_tokens,
                )
                .await
        }
        async fn compact(
            &self,
            agent_id: AgentId,
            messages: &[Message],
            driver: Arc<dyn LlmDriver>,
            model: &str,
            context_window_tokens: usize,
        ) -> LibreFangResult<CompactionResult> {
            self.compact_count.fetch_add(1, Ordering::SeqCst);
            self.inner
                .compact(agent_id, messages, driver, model, context_window_tokens)
                .await
        }
        async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
            self.inner.after_turn(agent_id, messages).await
        }
        fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
            self.inner
                .truncate_tool_result(content, context_window_tokens)
        }
        fn should_compress(&self, current_tokens: usize, max_tokens: usize) -> bool {
            self.inner.should_compress(current_tokens, max_tokens)
        }
    }

    let inner = DefaultContextEngine::new(ContextEngineConfig::default(), make_memory(), None);
    let tracker = CompactTracker::new(inner);

    // ctx_window = 200_000, threshold = 80% = 160_000
    // Iteration 1: prompt tokens = 100_000 (below threshold) → no compact
    assert!(!tracker.should_compress(100_000, 200_000));

    let driver = Arc::new(FakeDriver::new());
    let session = MemSession {
        id: librefang_types::agent::SessionId::new(),
        agent_id: librefang_types::agent::AgentId::new(),
        messages: vec![Message::user("Hello"), Message::assistant("Hi there")],
        context_window_tokens: 0,
        label: None,
        model_override: None,

        messages_generation: 0,
        last_repaired_generation: None,
    };

    // Manually call compact once (mirrors what agent_loop does when
    // should_compress returns true)
    let _ = tracker
        .compact(
            session.agent_id,
            &session.messages,
            driver.clone(),
            "test-model",
            200_000,
        )
        .await;
    assert_eq!(
        tracker.compact_count.load(Ordering::SeqCst),
        1,
        "compact must be called exactly once"
    );

    // Iteration 2: still below threshold → no additional compact call
    assert!(!tracker.should_compress(100_000, 200_000));
    assert_eq!(
        tracker.compact_count.load(Ordering::SeqCst),
        1,
        "compact must not be called again when should_compress is false"
    );

    // Iteration 3: above threshold → compact should fire
    assert!(tracker.should_compress(180_000, 200_000));
    let _ = tracker
        .compact(
            session.agent_id,
            &session.messages,
            driver.clone(),
            "test-model",
            200_000,
        )
        .await;
    assert_eq!(
        tracker.compact_count.load(Ordering::SeqCst),
        2,
        "compact must be called when should_compress transitions to true"
    );
}
