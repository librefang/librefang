//! Out-of-process context engine.
//!
//! `SidecarContextEngine` implements [`ContextEngine`](super::ContextEngine) by
//! delegating the async, non-LLM lifecycle hooks — `bootstrap`, `ingest`,
//! `assemble`, `after_turn` — to a long-lived subprocess over a
//! newline-delimited JSON request/reply protocol, and keeping everything that
//! must stay in Rust (LLM-bearing `compact`, the cheap synchronous hooks,
//! metrics) on a wrapped built-in engine.
//!
//! The subprocess plumbing — spawn, the background reply reader, id-matching,
//! the reply-line cap, the write timeout, stderr draining, child reaping, and
//! lazy auto-respawn after a crash — lives in
//! [`librefang_subprocess::SupervisedTransport`] (which wraps
//! [`librefang_subprocess::SubprocessTransport`]). This module is just the
//! context-engine policy on top of it.
//!
//! # Why this split
//!
//! Context **policy** (what to recall, how to trim/reorder the window, what to
//! do after a turn) is high-churn and a natural fit for a hot-swappable
//! external implementation. The **mechanism** it needs — the LLM driver and
//! token streaming used by compaction — is substrate that stays in Rust:
//! `compact` takes an `Arc<dyn LlmDriver>` that cannot cross a process
//! boundary, so it is delegated to the inner engine.
//!
//! # Robustness
//!
//! The context engine is on the per-turn critical path, so a flaky sidecar must
//! never break a turn. Every bridged call falls back to the inner engine on any
//! failure (spawn failure, write timeout, reply timeout, a `{"error": …}`
//! reply, a malformed reply, or a crashed process). A crash degrades only the
//! calls made during the respawn cooldown to the built-in engine;
//! `SupervisedTransport` re-spawns the child lazily on the next call once the
//! cooldown elapses, so a transient crash self-heals without a daemon restart.
//! See `docs/architecture/sidecar-context-engine.md`.
//!
//! # Wire protocol
//!
//! Daemon → sidecar (stdin), one JSON object per line:
//! `{"id": <u64>, "method": "<name>", "params": {…}}`.
//! Sidecar → daemon (stdout), one per line:
//! `{"id": <u64>, "ok": {…}}` or `{"id": <u64>, "error": "<msg>"}`.

use super::{AssembleResult, ContextEngine, ContextEngineConfig, IngestResult};
use crate::compactor::CompactionResult;
use crate::context_overflow::RecoveryStage;
use crate::llm_driver::LlmDriver;
use async_trait::async_trait;
use librefang_subprocess::{SupervisedTransport, TransportConfig};
use librefang_types::agent::AgentId;
use librefang_types::config::ContextEngineSidecarConfig;
use librefang_types::error::LibreFangResult;
use librefang_types::memory::MemoryFragment;
use librefang_types::message::Message;
use librefang_types::tool::ToolDefinition;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tracing::warn;

/// A context engine backed by an out-of-process implementation, with a built-in
/// engine as both the LLM-bearing path and the fallback for every bridged call.
pub struct SidecarContextEngine {
    inner: Box<dyn ContextEngine>,
    transport: SupervisedTransport,
}

impl SidecarContextEngine {
    /// Wrap `inner`, configuring an out-of-process sidecar described by `cfg`.
    ///
    /// The sidecar is spawned lazily on the first call and re-spawned after a
    /// crash (with a cooldown), so a flaky or initially-unavailable sidecar
    /// degrades to the built-in engine for individual calls rather than for the
    /// daemon's whole lifetime.
    pub fn spawn(inner: Box<dyn ContextEngine>, cfg: &ContextEngineSidecarConfig) -> Self {
        let timeout = Duration::from_secs(if cfg.request_timeout_secs == 0 {
            30
        } else {
            cfg.request_timeout_secs
        });
        let transport = SupervisedTransport::new(TransportConfig::new(
            cfg.command.clone(),
            cfg.args.clone(),
            timeout,
            "context_engine",
        ));
        Self { inner, transport }
    }

    /// Send one bridged call to the sidecar, returning its `ok` payload.
    /// `Err(())` means "fall back to the inner engine" — a sidecar failure
    /// (including one that is down and awaiting re-spawn) is never surfaced to
    /// the agent loop.
    async fn call(&self, method: &str, params: Value) -> Result<Value, ()> {
        match self
            .transport
            .request(json!({ "method": method, "params": params }))
            .await
        {
            Ok(value) => Ok(value),
            Err(e) => {
                warn!(method, error = %e, "context engine sidecar call failed; falling back");
                Err(())
            }
        }
    }
}

#[async_trait]
impl ContextEngine for SidecarContextEngine {
    async fn bootstrap(&self, config: &ContextEngineConfig) -> LibreFangResult<()> {
        // The inner engine owns the memory substrate and is the fallback for
        // every call, so it must always be bootstrapped. The sidecar gets a
        // best-effort notification with the fields it can act on.
        self.inner.bootstrap(config).await?;
        let _ = self
            .call(
                "bootstrap",
                json!({
                    "context_window_tokens": config.context_window_tokens,
                    "max_recall_results": config.max_recall_results,
                    "stable_prefix_mode": config.stable_prefix_mode,
                }),
            )
            .await;
        Ok(())
    }

    async fn ingest(
        &self,
        agent_id: AgentId,
        user_message: &str,
        peer_id: Option<&str>,
    ) -> LibreFangResult<IngestResult> {
        let params = json!({
            "agent_id": agent_id,
            "user_message": user_message,
            "peer_id": peer_id,
        });
        if let Ok(value) = self.call("ingest", params).await {
            match value
                .get("recalled_memories")
                .cloned()
                .map(serde_json::from_value::<Vec<MemoryFragment>>)
            {
                Some(Ok(recalled_memories)) => return Ok(IngestResult { recalled_memories }),
                Some(Err(e)) => warn!(error = %e,
                    "context engine sidecar: bad ingest reply; falling back"),
                None => warn!("context engine sidecar: ingest reply missing recalled_memories"),
            }
        }
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
        let params = json!({
            "agent_id": agent_id,
            "messages": &*messages,
            "system_prompt": system_prompt,
            "tools": tools,
            "context_window_tokens": context_window_tokens,
        });
        if let Ok(value) = self.call("assemble", params).await {
            // Require a well-formed `messages` array; the rewritten window is
            // the load-bearing output, so a malformed one must fall back rather
            // than silently send the model an empty/garbled context.
            match value
                .get("messages")
                .cloned()
                .map(serde_json::from_value::<Vec<Message>>)
            {
                Some(Ok(new_messages)) => {
                    // Repair the sidecar's window before it reaches the provider.
                    // The in-process engines run validate_and_repair internally,
                    // but the engine call site in run_streaming does NOT
                    // re-validate engine output — so a sloppy sidecar (e.g. a
                    // naive `messages[-N:]` window that splits a
                    // tool_use/tool_result pair, or drops the leading user turn)
                    // would otherwise hand the model a malformed sequence
                    // (Anthropic 400s on an orphan tool_result). This makes the
                    // doc's "the built-in engine still owns final ordering"
                    // claim actually true regardless of sidecar quality.
                    *messages = crate::session_repair::validate_and_repair(&new_messages);
                    let recovery = value
                        .get("recovery")
                        .cloned()
                        .and_then(|r| serde_json::from_value::<RecoveryStage>(r).ok())
                        .unwrap_or(RecoveryStage::None);
                    return Ok(AssembleResult { recovery });
                }
                Some(Err(e)) => warn!(error = %e,
                    "context engine sidecar: bad assemble reply; falling back"),
                None => warn!("context engine sidecar: assemble reply missing messages"),
            }
        }
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
        // LLM-bearing: the driver cannot cross the process boundary, so
        // compaction stays in Rust.
        self.inner
            .compact(agent_id, messages, driver, model, context_window_tokens)
            .await
    }

    async fn after_turn(&self, agent_id: AgentId, messages: &[Message]) -> LibreFangResult<()> {
        let params = json!({ "agent_id": agent_id, "messages": messages });
        if self.call("after_turn", params).await.is_ok() {
            return Ok(());
        }
        self.inner.after_turn(agent_id, messages).await
    }

    fn truncate_tool_result(&self, content: &str, context_window_tokens: usize) -> String {
        // Synchronous and hot — kept in Rust.
        self.inner
            .truncate_tool_result(content, context_window_tokens)
    }

    fn should_compress(&self, current_tokens: usize, max_tokens: usize) -> bool {
        self.inner.should_compress(current_tokens, max_tokens)
    }

    fn update_model(&self, model: &str, context_length: usize) {
        self.inner.update_model(model, context_length);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_types::message::{ContentBlock, MessageContent};

    /// Minimal inner engine for tests: identity `assemble`, empty `ingest`,
    /// no-op `after_turn`/`bootstrap`. Avoids constructing a real
    /// `DefaultContextEngine` (which needs a `MemorySubstrate`).
    struct StubEngine;

    #[async_trait]
    impl ContextEngine for StubEngine {
        async fn bootstrap(&self, _config: &ContextEngineConfig) -> LibreFangResult<()> {
            Ok(())
        }
        async fn ingest(
            &self,
            _agent_id: AgentId,
            _user_message: &str,
            _peer_id: Option<&str>,
        ) -> LibreFangResult<IngestResult> {
            Ok(IngestResult {
                recalled_memories: Vec::new(),
            })
        }
        async fn assemble(
            &self,
            _agent_id: AgentId,
            _messages: &mut Vec<Message>,
            _system_prompt: &str,
            _tools: &[ToolDefinition],
            _context_window_tokens: usize,
        ) -> LibreFangResult<AssembleResult> {
            // Identity: leave the window untouched.
            Ok(AssembleResult {
                recovery: RecoveryStage::None,
            })
        }
        async fn compact(
            &self,
            _agent_id: AgentId,
            messages: &[Message],
            _driver: Arc<dyn LlmDriver>,
            _model: &str,
            _context_window_tokens: usize,
        ) -> LibreFangResult<CompactionResult> {
            Ok(CompactionResult {
                summary: String::new(),
                kept_messages: messages.to_vec(),
                compacted_count: 0,
                chunks_used: 1,
                used_fallback: true,
            })
        }
        async fn after_turn(
            &self,
            _agent_id: AgentId,
            _messages: &[Message],
        ) -> LibreFangResult<()> {
            Ok(())
        }
        fn truncate_tool_result(&self, content: &str, _context_window_tokens: usize) -> String {
            content.to_string()
        }
    }

    /// A reference sidecar that rewrites `assemble` to an empty window and
    /// echoes `ingest` with no memories. Written in Python because a long-lived
    /// shell `printf` to a pipe is block-buffered (the reply would never flush
    /// until the shell exits); `sys.stdout.flush()` makes the reply prompt.
    fn fake_sidecar_py() -> &'static str {
        // `readline()` (not `for line in sys.stdin`) because the latter
        // read-ahead-buffers and would not yield a single line until EOF.
        r#"
import sys, json
while True:
    line = sys.stdin.readline()
    if not line:
        break
    line = line.strip()
    if not line:
        continue
    try:
        req = json.loads(line)
    except Exception:
        continue
    rid = req.get("id")
    method = req.get("method")
    if method == "assemble":
        ok = {"messages": [], "recovery": "None"}
    elif method == "ingest":
        ok = {"recalled_memories": []}
    else:
        ok = {}
    sys.stdout.write(json.dumps({"id": rid, "ok": ok}) + "\n")
    sys.stdout.flush()
"#
    }

    /// Locate a Python 3 interpreter, or `None` to skip the test on runners
    /// without one (mirrors the skills crate's python-runtime tests).
    fn python3() -> Option<&'static str> {
        ["python3", "python"].into_iter().find(|cmd| {
            std::process::Command::new(cmd)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
    }

    #[tokio::test]
    async fn assemble_uses_sidecar_reply_when_well_formed() {
        let Some(py) = python3() else {
            eprintln!("skipping: no python3 on this runner");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fake.py");
        std::fs::write(&script, fake_sidecar_py()).unwrap();

        let engine = SidecarContextEngine::spawn(
            Box::new(StubEngine),
            &ContextEngineSidecarConfig {
                command: py.to_string(),
                args: vec![script.to_str().unwrap().to_string()],
                request_timeout_secs: 5,
            },
        );

        let mut messages = vec![Message::user("first"), Message::user("second")];
        let result = engine
            .assemble(AgentId(uuid::Uuid::nil()), &mut messages, "sys", &[], 1000)
            .await
            .unwrap();

        // The fake returns an empty window; the bridge must apply it verbatim
        // (proving the sidecar reply was used, not the inner stub identity).
        assert!(messages.is_empty());
        assert_eq!(result.recovery, RecoveryStage::None);
    }

    #[tokio::test]
    async fn falls_back_to_inner_when_sidecar_cannot_spawn() {
        let engine = SidecarContextEngine::spawn(
            Box::new(StubEngine),
            &ContextEngineSidecarConfig {
                command: "/nonexistent/context-engine-binary".to_string(),
                args: vec![],
                request_timeout_secs: 5,
            },
        );
        // The sidecar command does not exist, so the lazy (re)spawn fails on
        // the first call and every call falls back to the inner (stub) engine,
        // which leaves the window untouched — proving the fallback ran rather
        // than a sidecar. `Message` has no `PartialEq`, so assert on length.
        let mut messages = vec![Message::user("hi")];
        engine
            .assemble(AgentId(uuid::Uuid::nil()), &mut messages, "sys", &[], 1000)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
    }

    /// Build an engine whose sidecar runs `body` (a Python script), with the
    /// given per-request timeout. `StubEngine` is the inner/fallback engine and
    /// leaves the window untouched, so a fallback is observable as "messages
    /// unchanged".
    fn spawn_with(
        py: &str,
        dir: &std::path::Path,
        body: &str,
        timeout_secs: u64,
    ) -> SidecarContextEngine {
        let script = dir.join("s.py");
        std::fs::write(&script, body).unwrap();
        SidecarContextEngine::spawn(
            Box::new(StubEngine),
            &ContextEngineSidecarConfig {
                command: py.to_string(),
                args: vec![script.to_str().unwrap().to_string()],
                request_timeout_secs: timeout_secs,
            },
        )
    }

    /// `assemble` falls back to the inner engine (window unchanged) for every
    /// non-happy sidecar behaviour: timeout, error reply, and malformed reply.
    #[tokio::test]
    async fn assemble_falls_back_on_timeout_error_and_malformed() {
        let Some(py) = python3() else {
            eprintln!("skipping: no python3 on this runner");
            return;
        };

        // (a) Timeout: reads the request but never replies. 1s timeout keeps the
        //     test quick; the call must time out and fall back.
        let dir_t = tempfile::tempdir().unwrap();
        let slow = "import sys, time\nwhile True:\n    if not sys.stdin.readline():\n        break\n    time.sleep(30)\n";
        let engine = spawn_with(py, dir_t.path(), slow, 1);
        let mut m = vec![Message::user("hi")];
        engine
            .assemble(AgentId(uuid::Uuid::nil()), &mut m, "sys", &[], 1000)
            .await
            .unwrap();
        assert_eq!(
            m.len(),
            1,
            "timeout must fall back to inner (window unchanged)"
        );

        // (b) Error reply: sidecar returns {"id":N,"error":"boom"}.
        let dir_e = tempfile::tempdir().unwrap();
        let err = "import sys, json\nwhile True:\n    line = sys.stdin.readline()\n    if not line:\n        break\n    line = line.strip()\n    if not line:\n        continue\n    rid = json.loads(line).get(\"id\")\n    sys.stdout.write(json.dumps({\"id\": rid, \"error\": \"boom\"}) + \"\\n\")\n    sys.stdout.flush()\n";
        let engine = spawn_with(py, dir_e.path(), err, 5);
        let mut m = vec![Message::user("hi")];
        engine
            .assemble(AgentId(uuid::Uuid::nil()), &mut m, "sys", &[], 1000)
            .await
            .unwrap();
        assert_eq!(m.len(), 1, "error reply must fall back");

        // (c) Malformed reply: `messages` is a string, not an array.
        let dir_m = tempfile::tempdir().unwrap();
        let bad = "import sys, json\nwhile True:\n    line = sys.stdin.readline()\n    if not line:\n        break\n    line = line.strip()\n    if not line:\n        continue\n    rid = json.loads(line).get(\"id\")\n    sys.stdout.write(json.dumps({\"id\": rid, \"ok\": {\"messages\": \"not-an-array\"}}) + \"\\n\")\n    sys.stdout.flush()\n";
        let engine = spawn_with(py, dir_m.path(), bad, 5);
        let mut m = vec![Message::user("hi")];
        engine
            .assemble(AgentId(uuid::Uuid::nil()), &mut m, "sys", &[], 1000)
            .await
            .unwrap();
        assert_eq!(m.len(), 1, "malformed reply must fall back");
    }

    /// A sidecar that exits immediately: spawn succeeds, but the transport is
    /// dead, so calls fall back rather than hang.
    #[tokio::test]
    async fn assemble_falls_back_when_sidecar_exits_immediately() {
        let Some(py) = python3() else {
            eprintln!("skipping: no python3 on this runner");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let engine = spawn_with(py, dir.path(), "import sys\nsys.exit(0)\n", 5);
        let mut m = vec![Message::user("hi")];
        engine
            .assemble(AgentId(uuid::Uuid::nil()), &mut m, "sys", &[], 1000)
            .await
            .unwrap();
        assert_eq!(m.len(), 1, "dead sidecar must fall back to inner");
    }

    /// The sidecar window is run through validate_and_repair before it reaches
    /// the provider: an orphan tool_result (its tool_use trimmed away by the
    /// sidecar) must be dropped rather than handed to the LLM (#5849 review).
    #[tokio::test]
    async fn assemble_repairs_orphan_tool_result_from_sidecar() {
        let Some(py) = python3() else {
            eprintln!("skipping: no python3 on this runner");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        // Sidecar returns a window that is ONLY a tool_result (the matching
        // tool_use was dropped) — a malformed sequence a naive window can emit.
        let body = "import sys, json\nwhile True:\n    line = sys.stdin.readline()\n    if not line:\n        break\n    line = line.strip()\n    if not line:\n        continue\n    rid = json.loads(line).get(\"id\")\n    win = [{\"role\": \"user\", \"content\": [{\"type\": \"tool_result\", \"tool_use_id\": \"orphan\", \"content\": \"x\"}]}]\n    sys.stdout.write(json.dumps({\"id\": rid, \"ok\": {\"messages\": win, \"recovery\": \"None\"}}) + \"\\n\")\n    sys.stdout.flush()\n";
        let engine = spawn_with(py, dir.path(), body, 5);
        let mut m = vec![Message::user("first")];
        engine
            .assemble(AgentId(uuid::Uuid::nil()), &mut m, "sys", &[], 1000)
            .await
            .unwrap();
        // The orphan tool_result must not survive into the prompt.
        let has_orphan = m.iter().any(|msg| {
            if let MessageContent::Blocks(blocks) = &msg.content {
                blocks.iter().any(|b| matches!(b, ContentBlock::ToolResult { tool_use_id, .. } if tool_use_id == "orphan"))
            } else {
                false
            }
        });
        assert!(
            !has_orphan,
            "validate_and_repair must drop the orphan tool_result"
        );
    }
}
