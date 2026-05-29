//! Out-of-process context engine.
//!
//! `SidecarContextEngine` implements [`ContextEngine`](super::ContextEngine) by
//! delegating the async, non-LLM lifecycle hooks — `bootstrap`, `ingest`,
//! `assemble`, `after_turn` — to a long-lived subprocess over a
//! newline-delimited JSON request/reply protocol, and keeping everything that
//! must stay in Rust (LLM-bearing `compact`, the cheap synchronous hooks,
//! metrics) on a wrapped built-in engine.
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
//! failure (spawn failure, write error, timeout, malformed reply, or a crashed
//! process). A crash degrades to the built-in engine for the rest of the
//! daemon's lifetime (restart re-spawns); this is deliberate — see the design
//! doc `docs/architecture/sidecar-context-engine.md`.
//!
//! # Wire protocol
//!
//! Daemon → sidecar (stdin), one JSON object per line:
//! `{"id": <u64>, "method": "<name>", "params": {…}}`.
//! Sidecar → daemon (stdout), one per line:
//! `{"id": <u64>, "ok": {…}}` or `{"id": <u64>, "error": "<msg>"}`.
//! stderr is free-form and forwarded to the daemon log.

use super::{AssembleResult, ContextEngine, ContextEngineConfig, IngestResult};
use crate::compactor::CompactionResult;
use crate::context_overflow::RecoveryStage;
use crate::llm_driver::LlmDriver;
use async_trait::async_trait;
use librefang_types::agent::AgentId;
use librefang_types::config::ContextEngineSidecarConfig;
use librefang_types::error::LibreFangResult;
use librefang_types::memory::MemoryFragment;
use librefang_types::message::Message;
use librefang_types::tool::ToolDefinition;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, warn};

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// Hard cap on a single newline-delimited reply line from the sidecar. The
/// sidecar is trusted operator config, but a *buggy* trusted sidecar that
/// streams without ever emitting `\n` would otherwise grow the reader task's
/// buffer without bound and OOM the daemon. On overflow we treat the transport
/// as dead so every call falls back to the in-process engine.
const MAX_REPLY_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Outcome of a bounded line read from the sidecar's stdout.
enum SidecarLine {
    Data(String),
    Eof,
    TooLong,
}

/// Read one `\n`-terminated line, capping accumulation at [`MAX_REPLY_LINE_BYTES`].
/// `reader` is a `BufReader`, so the per-byte reads are served from its buffer
/// (no syscall per byte); this bounds memory without the unbounded-line risk of
/// `AsyncBufReadExt::lines()` / `read_until`.
async fn read_capped_line<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> std::io::Result<SidecarLine> {
    buf.clear();
    let mut byte = [0u8; 1];
    loop {
        if reader.read(&mut byte).await? == 0 {
            return Ok(if buf.is_empty() {
                SidecarLine::Eof
            } else {
                SidecarLine::Data(String::from_utf8_lossy(buf).into_owned())
            });
        }
        if byte[0] == b'\n' {
            return Ok(SidecarLine::Data(String::from_utf8_lossy(buf).into_owned()));
        }
        if buf.len() >= MAX_REPLY_LINE_BYTES {
            return Ok(SidecarLine::TooLong);
        }
        buf.push(byte[0]);
    }
}

/// Live connection to the sidecar subprocess. Absent when the process could
/// not be spawned or has died — in both cases the engine serves from `inner`.
struct Transport {
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    timeout: Duration,
    // Retain the child handle so the process lives as long as the engine.
    // `kill_on_drop(true)` reaps it when the engine drops. Wrapped in a
    // `std::sync::Mutex` purely to make `Transport: Sync` (we never lock it).
    _child: std::sync::Mutex<tokio::process::Child>,
}

/// A context engine backed by an out-of-process implementation, with a built-in
/// engine as both the LLM-bearing path and the fallback for every bridged call.
pub struct SidecarContextEngine {
    inner: Box<dyn ContextEngine>,
    transport: Option<Transport>,
}

impl SidecarContextEngine {
    /// Spawn the sidecar described by `cfg`, wrapping `inner` for delegation and
    /// fallback. A spawn failure is logged and yields an engine that behaves
    /// exactly like `inner`.
    pub fn spawn(inner: Box<dyn ContextEngine>, cfg: &ContextEngineSidecarConfig) -> Self {
        let timeout = Duration::from_secs(if cfg.request_timeout_secs == 0 {
            30
        } else {
            cfg.request_timeout_secs
        });
        match Self::try_spawn(cfg, timeout) {
            Ok(transport) => {
                debug!(command = %cfg.command, "context engine sidecar spawned");
                Self {
                    inner,
                    transport: Some(transport),
                }
            }
            Err(e) => {
                warn!(error = %e, command = %cfg.command,
                    "context engine sidecar spawn failed; using built-in engine");
                Self {
                    inner,
                    transport: None,
                }
            }
        }
    }

    fn try_spawn(
        cfg: &ContextEngineSidecarConfig,
        timeout: Duration,
    ) -> std::io::Result<Transport> {
        let mut child = Command::new(&cfg.command)
            .args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Reap the subprocess when the engine (and thus this handle) drops.
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("sidecar stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("sidecar stdout unavailable"))?;
        let stderr = child.stderr.take();

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));

        // Reader task: match replies to waiters by id. On EOF / error, mark the
        // transport dead and fail every outstanding waiter so callers fall back.
        {
            let pending = Arc::clone(&pending);
            let alive = Arc::clone(&alive);
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut buf: Vec<u8> = Vec::new();
                // Loop ends on EOF, a read error, or an over-cap line — all mean
                // the transport can no longer be trusted, so we fall through to
                // marking it dead and draining waiters.
                loop {
                    let line = match read_capped_line(&mut reader, &mut buf).await {
                        Ok(SidecarLine::Data(line)) => line,
                        Ok(SidecarLine::Eof) => break,
                        Ok(SidecarLine::TooLong) => {
                            warn!(
                                cap = MAX_REPLY_LINE_BYTES,
                                "context engine sidecar: reply line exceeded cap; \
                                 dropping transport and falling back"
                            );
                            break;
                        }
                        Err(_) => break,
                    };
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                        warn!("context engine sidecar: non-JSON reply line dropped");
                        continue;
                    };
                    let Some(id) = reply.get("id").and_then(Value::as_u64) else {
                        warn!("context engine sidecar: reply without id dropped");
                        continue;
                    };
                    if let Some(tx) = pending.lock().await.remove(&id) {
                        let result = if let Some(ok) = reply.get("ok") {
                            Ok(ok.clone())
                        } else if let Some(err) = reply.get("error") {
                            Err(err
                                .as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| err.to_string()))
                        } else {
                            Err("reply has neither ok nor error".to_string())
                        };
                        let _ = tx.send(result);
                    }
                }
                alive.store(false, Ordering::SeqCst);
                // Fail any waiters still parked so their calls fall back.
                let mut map = pending.lock().await;
                for (_, tx) in map.drain() {
                    let _ = tx.send(Err("sidecar process exited".to_string()));
                }
                // Operator-actionable: silent fallback otherwise looks like
                // normal operation. WARN (not debug) + a metric so a dead
                // sidecar is visible — every call now uses the built-in engine
                // until the daemon is restarted.
                metrics::counter!("context_engine_sidecar_exited").increment(1);
                warn!(
                    "context engine sidecar process exited; all context calls now \
                     fall back to the built-in engine until the daemon is restarted"
                );
            });
        }

        // stderr → daemon log.
        if let Some(stderr) = stderr {
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(target: "context_engine_sidecar", "{line}");
                }
            });
        }

        Ok(Transport {
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            alive,
            timeout,
            _child: std::sync::Mutex::new(child),
        })
    }

    /// Send one request and await its reply. `Err(())` means "fall back to the
    /// inner engine" — the caller never surfaces a sidecar error to the loop.
    async fn call(&self, method: &str, params: Value) -> Result<Value, ()> {
        let t = self.transport.as_ref().ok_or(())?;
        if !t.alive.load(Ordering::SeqCst) {
            return Err(());
        }
        let id = t.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        t.pending.lock().await.insert(id, tx);

        let line =
            match serde_json::to_string(&json!({"id": id, "method": method, "params": params})) {
                Ok(mut s) => {
                    s.push('\n');
                    s
                }
                Err(e) => {
                    warn!(error = %e, method, "context engine sidecar: request serialize failed");
                    t.pending.lock().await.remove(&id);
                    return Err(());
                }
            };

        {
            let mut w = t.stdin.lock().await;
            // Bound the write itself, not just the reply wait: if the sidecar
            // stops reading its stdin the pipe buffer fills and `write_all`
            // blocks indefinitely, which would hang the turn past the timeout
            // and defeat the never-break-a-turn guarantee. On timeout the
            // future (and the stdin guard) drops, freeing the lock. A flush
            // error is treated as a write failure rather than swallowed.
            let write = async {
                w.write_all(line.as_bytes()).await?;
                w.flush().await
            };
            let wrote = matches!(tokio::time::timeout(t.timeout, write).await, Ok(Ok(())));
            if !wrote {
                warn!(method, "context engine sidecar: write timed out or failed");
                t.alive.store(false, Ordering::SeqCst);
                t.pending.lock().await.remove(&id);
                return Err(());
            }
        }

        match tokio::time::timeout(t.timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(msg))) => {
                warn!(method, error = %msg, "context engine sidecar returned an error");
                Err(())
            }
            // Channel dropped (process died) or timed out.
            other => {
                if other.is_err() {
                    warn!(
                        method,
                        timeout_secs = t.timeout.as_secs(),
                        "context engine sidecar call timed out"
                    );
                }
                t.pending.lock().await.remove(&id);
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
        assert!(
            engine.transport.is_none(),
            "spawn failure must yield no transport"
        );

        // The call path still works via the inner (stub) engine, which leaves
        // the window untouched — proving the fallback ran, not the sidecar
        // (there is none). `Message` has no `PartialEq`, so assert on length.
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
