//! Persistent JSON-over-stdio subprocess transport.
//!
//! A small, dependency-light bridge for talking to a long-lived child process
//! over a newline-delimited JSON request/reply protocol. It owns the parts that
//! every LibreFang sidecar bridge was re-implementing: spawning the child,
//! reading replies on a background task, matching them to waiters by id,
//! bounding both the write and the reply-line size, forwarding stderr to the
//! log, and reaping the child on drop.
//!
//! It is deliberately protocol-light: the caller supplies a JSON request
//! object, the transport injects an `id`, writes `{"id": N, …caller fields}`,
//! and resolves the call with the matching reply. The reply convention is the
//! one shared across the bridges — `{"id": N, "ok": <value>}` or
//! `{"id": N, "error": "<msg>"}`.
//!
//! # Why a separate crate
//!
//! Both `librefang-channels` and `librefang-runtime` need this, so it lives
//! below both in the dependency graph and pulls in no `librefang-*` crate.
//!
//! # Stdio pitfalls it handles for you
//!
//! - Reply lines are read with an explicit byte cap (a buggy child that streams
//!   without a newline cannot grow memory without bound).
//! - The write is timeout-bounded, not just the reply wait — a child that stops
//!   reading its stdin (full pipe) can't wedge the caller past the deadline.

use serde_json::Value;
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, warn};

/// Default cap on a single newline-delimited reply line (16 MiB).
pub const DEFAULT_MAX_REPLY_LINE_BYTES: usize = 16 * 1024 * 1024;

/// Why a [`SubprocessTransport::request`] did not yield a successful reply.
#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    /// The child has exited (or never spawned); no further requests can succeed.
    #[error("subprocess transport is dead")]
    Dead,
    /// The write or the reply wait exceeded the configured timeout.
    #[error("request timed out after {0:?}")]
    Timeout(Duration),
    /// The child replied with `{"error": "<msg>"}`.
    #[error("subprocess returned an error: {0}")]
    Remote(String),
    /// The request was not a JSON object, so an `id` could not be attached.
    #[error("request must be a JSON object")]
    BadRequest,
}

/// How to launch and frame a [`SubprocessTransport`].
#[derive(Clone)]
pub struct TransportConfig {
    /// Executable to launch (resolved via `PATH`).
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Per-request wall-clock budget, applied to the write and the reply wait.
    pub request_timeout: Duration,
    /// Cap on a single reply line; over-cap drops the transport (fall back).
    pub max_reply_line_bytes: usize,
    /// Short label for logs and the `subprocess_transport_exited` metric, e.g.
    /// `"context_engine"`.
    pub label: String,
}

impl TransportConfig {
    /// Config with the default 16 MiB reply-line cap.
    pub fn new(
        command: impl Into<String>,
        args: Vec<String>,
        request_timeout: Duration,
        label: impl Into<String>,
    ) -> Self {
        Self {
            command: command.into(),
            args,
            request_timeout,
            max_reply_line_bytes: DEFAULT_MAX_REPLY_LINE_BYTES,
            label: label.into(),
        }
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

/// A live connection to a long-lived child process.
///
/// Cheap to clone-free share behind an `Arc`. Dropping it kills and reaps the
/// child (the handle is held with `kill_on_drop`).
pub struct SubprocessTransport {
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    next_id: AtomicU64,
    alive: Arc<AtomicBool>,
    timeout: Duration,
    label: String,
    // Retained so the process lives as long as the transport; `kill_on_drop`
    // reaps it on drop. A `std::sync::Mutex` purely to keep `Self: Sync` (we
    // never lock it).
    _child: std::sync::Mutex<tokio::process::Child>,
}

impl SubprocessTransport {
    /// Spawn the child and start the background reader + stderr drain.
    pub fn spawn(cfg: TransportConfig) -> std::io::Result<Self> {
        let mut child = Command::new(&cfg.command)
            .args(&cfg.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("subprocess stdin unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| std::io::Error::other("subprocess stdout unavailable"))?;
        let stderr = child.stderr.take();

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let alive = Arc::new(AtomicBool::new(true));
        let max_line = cfg.max_reply_line_bytes;
        let label = cfg.label.clone();

        // Reader task: match replies to waiters by id. Ends on EOF, a read
        // error, or an over-cap line — all mean the transport is no longer
        // trustworthy, so it marks itself dead and drains every waiter.
        {
            let pending = Arc::clone(&pending);
            let alive = Arc::clone(&alive);
            let label = label.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(stdout);
                let mut buf: Vec<u8> = Vec::new();
                loop {
                    let line = match read_capped_line(&mut reader, &mut buf, max_line).await {
                        Ok(Line::Data(line)) => line,
                        Ok(Line::Eof) => break,
                        Ok(Line::TooLong) => {
                            warn!(
                                label = %label,
                                cap = max_line,
                                "subprocess transport: reply line exceeded cap; \
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
                        warn!(label = %label, "subprocess transport: non-JSON reply dropped");
                        continue;
                    };
                    let Some(id) = reply.get("id").and_then(Value::as_u64) else {
                        warn!(label = %label, "subprocess transport: reply without id dropped");
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
                // Drop every pending sender (don't send an error string): a
                // closed channel resolves the waiter as `TransportError::Dead`,
                // keeping "process died" distinct from a `{"error": …}` reply
                // (`TransportError::Remote`).
                pending.lock().await.clear();
                // Operator-actionable: a dead transport otherwise looks like
                // normal operation to whoever falls back to a built-in path.
                metrics::counter!("subprocess_transport_exited", "label" => label.clone())
                    .increment(1);
                warn!(
                    label = %label,
                    "subprocess transport process exited; callers now fall back \
                     until the transport is recreated"
                );
            });
        }

        if let Some(stderr) = stderr {
            let label = label.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    debug!(target: "subprocess_transport", label = %label, "{line}");
                }
            });
        }

        debug!(label = %label, command = %cfg.command, "subprocess transport spawned");
        Ok(Self {
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            alive,
            timeout: cfg.request_timeout,
            label,
            _child: std::sync::Mutex::new(child),
        })
    }

    /// `true` until the child exits (or a fatal protocol error drops it).
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Send one request and await its reply.
    ///
    /// `request` must be a JSON object; an `id` is injected before sending.
    /// Returns the `ok` payload, or a [`TransportError`] (the caller decides how
    /// to recover — typically a fall back to an in-process path).
    pub async fn request(&self, request: Value) -> Result<Value, TransportError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(TransportError::Dead);
        }
        let Value::Object(mut map) = request else {
            return Err(TransportError::BadRequest);
        };
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        map.insert("id".to_string(), Value::from(id));
        let mut line =
            serde_json::to_string(&Value::Object(map)).map_err(|_| TransportError::BadRequest)?;
        line.push('\n');

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        {
            let mut w = self.stdin.lock().await;
            // Bound the write itself: a child that stops reading its stdin fills
            // the pipe buffer and `write_all` would otherwise block forever
            // while holding the stdin lock. On timeout the future (and the
            // guard) drops, freeing the lock. A flush error is a write failure.
            let write = async {
                w.write_all(line.as_bytes()).await?;
                w.flush().await
            };
            if !matches!(tokio::time::timeout(self.timeout, write).await, Ok(Ok(()))) {
                warn!(label = %self.label, "subprocess transport: write timed out or failed");
                self.alive.store(false, Ordering::SeqCst);
                self.pending.lock().await.remove(&id);
                return Err(TransportError::Dead);
            }
        }

        match tokio::time::timeout(self.timeout, rx).await {
            Ok(Ok(Ok(value))) => Ok(value),
            Ok(Ok(Err(msg))) => Err(TransportError::Remote(msg)),
            // Channel dropped (process died) or timed out.
            other => {
                self.pending.lock().await.remove(&id);
                if other.is_err() {
                    Err(TransportError::Timeout(self.timeout))
                } else {
                    Err(TransportError::Dead)
                }
            }
        }
    }
}

/// A [`SubprocessTransport`] that re-spawns the child after it dies.
///
/// Construction is lazy and never fails: the child is spawned on the first
/// [`request`](Self::request) and re-spawned on the first request after a
/// crash, so a consumer can hold one unconditionally and let calls fall back
/// while the child is down — no "dead until the daemon restarts" cliff. A
/// `respawn_cooldown` rate-limits attempts so a persistently-broken command
/// can't spawn-storm.
///
/// `request` is the only contention point's gate: the brief liveness check /
/// (re)spawn is serialized, but the request itself runs on a cloned handle
/// outside that lock, so the underlying transport's id-matched concurrency is
/// preserved.
pub struct SupervisedTransport {
    cfg: TransportConfig,
    respawn_cooldown: Duration,
    current: Mutex<Option<Arc<SubprocessTransport>>>,
    last_attempt: std::sync::Mutex<Option<std::time::Instant>>,
}

impl SupervisedTransport {
    /// Wrap `cfg` with a 5s respawn cooldown. Nothing is spawned until the
    /// first [`request`](Self::request).
    pub fn new(cfg: TransportConfig) -> Self {
        Self::with_cooldown(cfg, Duration::from_secs(5))
    }

    /// As [`new`](Self::new) but with an explicit minimum interval between
    /// (re)spawn attempts.
    pub fn with_cooldown(cfg: TransportConfig, respawn_cooldown: Duration) -> Self {
        Self {
            cfg,
            respawn_cooldown,
            current: Mutex::new(None),
            last_attempt: std::sync::Mutex::new(None),
        }
    }

    /// Send a request, (re)spawning the child first if it is absent or dead.
    pub async fn request(&self, request: Value) -> Result<Value, TransportError> {
        let transport = self.ensure_live().await?;
        transport.request(request).await
    }

    /// Return the live transport, spawning (or re-spawning) if needed.
    ///
    /// Review-followup D: this holds `self.current` (a `tokio::Mutex`)
    /// across `SubprocessTransport::spawn`. Spawn is a synchronous
    /// syscall and is sub-millisecond in the common case, but it can
    /// stretch into the seconds on slow filesystems (NFS, Windows AV
    /// hooks, fork() under memory pressure). Concurrent `request`
    /// callers serialise behind this lock while the (re)spawn is in
    /// flight — acceptable because we *want* exactly one spawn per
    /// crash event, not N parallel ones, and the cooldown gate above
    /// already short-circuits the racers that wake up during the
    /// guard window. Switching to a double-checked pattern would let
    /// concurrent callers race past the cooldown check and is left as
    /// future work if a slow-spawn profile shows up in practice.
    async fn ensure_live(&self) -> Result<Arc<SubprocessTransport>, TransportError> {
        let mut current = self.current.lock().await;
        if let Some(t) = current.as_ref() {
            if t.is_alive() {
                return Ok(Arc::clone(t));
            }
        }
        // Absent or dead. Respect the cooldown so a broken command can't
        // spawn-storm on every call.
        {
            let mut last = self.last_attempt.lock().unwrap();
            if let Some(at) = *last {
                if at.elapsed() < self.respawn_cooldown {
                    return Err(TransportError::Dead);
                }
            }
            *last = Some(std::time::Instant::now());
        }
        match SubprocessTransport::spawn(self.cfg.clone()) {
            Ok(t) => {
                let arc = Arc::new(t);
                *current = Some(Arc::clone(&arc));
                Ok(arc)
            }
            Err(e) => {
                *current = None;
                warn!(label = %self.cfg.label, error = %e,
                    "subprocess transport (re)spawn failed");
                Err(TransportError::Dead)
            }
        }
    }
}

/// Outcome of a [`read_capped_line`] call.
pub enum Line {
    /// A `\n`-terminated line (terminator stripped), decoded lossily as UTF-8.
    ///
    /// **Includes partial lines at EOF.** If the stream closes after
    /// emitting bytes but before a `\n`, those bytes are returned as
    /// `Data` (review-followup E). For JSON-over-stdio consumers the
    /// downstream `serde_json::from_str` will reject the truncated
    /// payload, so this never lands as silent corruption — but callers
    /// that care about the distinction (e.g. a strict line-protocol
    /// parser) need to handle it explicitly.
    Data(String),
    /// The stream reached EOF with no pending bytes — a clean shutdown
    /// of an idle protocol. Distinct from "EOF with bytes pending",
    /// which surfaces as `Data` (see above).
    Eof,
    /// The line exceeded the cap before a `\n` was seen; the caller
    /// should treat the stream as untrustworthy and stop reading it.
    TooLong,
}

/// Read one `\n`-terminated line, capping accumulation at `max`.
///
/// The bound is `AsyncBufRead` rather than the looser `AsyncRead` so
/// the per-byte read loop is served from the reader's in-memory buffer
/// — calling this on a raw `AsyncRead` (e.g. unbuffered `ChildStdout`)
/// would issue one syscall per byte and is a footgun at 4–16 MiB
/// message sizes. All in-tree callers wrap their stdout in
/// `BufReader`; the tighter bound makes that requirement
/// compile-time-checked rather than implicit
/// (review-followup B). Bounds memory without the unbounded-line risk
/// of `AsyncBufReadExt::lines()` / `read_until`.
pub async fn read_capped_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<Line> {
    buf.clear();
    let mut byte = [0u8; 1];
    loop {
        if reader.read(&mut byte).await? == 0 {
            return Ok(if buf.is_empty() {
                Line::Eof
            } else {
                // Partial-line-at-EOF — see `Line::Data` docs.
                Line::Data(String::from_utf8_lossy(buf).into_owned())
            });
        }
        if byte[0] == b'\n' {
            return Ok(Line::Data(String::from_utf8_lossy(buf).into_owned()));
        }
        if buf.len() >= max {
            return Ok(Line::TooLong);
        }
        buf.push(byte[0]);
    }
}

/// Write `line` to `stdin` and flush, bounded by `timeout`.
///
/// Bounds the write itself, not just a later reply wait: a child that stops
/// reading its stdin fills the pipe buffer and an unbounded `write_all` would
/// block forever. On timeout this returns a `TimedOut` error so the caller can
/// treat the transport as dead. `line` should already include any trailing
/// `\n`.
pub async fn write_line_timeout(
    stdin: &mut ChildStdin,
    line: &[u8],
    timeout: Duration,
) -> std::io::Result<()> {
    let write = async {
        stdin.write_all(line).await?;
        stdin.flush().await
    };
    match tokio::time::timeout(timeout, write).await {
        Ok(result) => result,
        Err(_) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "subprocess stdin write timed out",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn python() -> Option<&'static str> {
        ["python3", "python"].into_iter().find(|cmd| {
            std::process::Command::new(cmd)
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        })
    }

    fn spawn_py(
        py: &str,
        dir: &std::path::Path,
        body: &str,
        timeout_secs: u64,
    ) -> SubprocessTransport {
        let script = dir.join("s.py");
        std::fs::write(&script, body).unwrap();
        SubprocessTransport::spawn(TransportConfig::new(
            py,
            vec![script.to_str().unwrap().to_string()],
            Duration::from_secs(timeout_secs),
            "test",
        ))
        .unwrap()
    }

    // A responder that echoes the request's `method` back inside `ok`. Uses
    // readline (not `for line in sys.stdin`, which read-ahead-buffers) and
    // flushes after every reply (a long-lived process's stdout is block-buffered
    // when piped).
    const ECHO: &str = r#"
import sys, json
while True:
    line = sys.stdin.readline()
    if not line:
        break
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    sys.stdout.write(json.dumps({"id": req["id"], "ok": {"echo": req.get("method")}}) + "\n")
    sys.stdout.flush()
"#;

    #[tokio::test]
    async fn request_roundtrips_and_matches_by_id() {
        let Some(py) = python() else {
            eprintln!("skipping: no python3");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let t = spawn_py(py, dir.path(), ECHO, 5);
        let r = t.request(json!({"method": "ping"})).await.unwrap();
        assert_eq!(r, json!({"echo": "ping"}));
        // A second concurrent-ish call gets its own id matched correctly.
        let r2 = t.request(json!({"method": "pong"})).await.unwrap();
        assert_eq!(r2, json!({"echo": "pong"}));
    }

    #[tokio::test]
    async fn spawn_failure_surfaces_as_io_error() {
        let err = SubprocessTransport::spawn(TransportConfig::new(
            "/nonexistent/transport-binary",
            vec![],
            Duration::from_secs(5),
            "test",
        ));
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn error_reply_maps_to_remote() {
        let Some(py) = python() else {
            return;
        };
        let body = r#"
import sys, json
while True:
    line = sys.stdin.readline()
    if not line:
        break
    line = line.strip()
    if not line:
        continue
    rid = json.loads(line)["id"]
    sys.stdout.write(json.dumps({"id": rid, "error": "boom"}) + "\n")
    sys.stdout.flush()
"#;
        let dir = tempfile::tempdir().unwrap();
        let t = spawn_py(py, dir.path(), body, 5);
        let err = t.request(json!({"method": "x"})).await.unwrap_err();
        assert!(matches!(err, TransportError::Remote(m) if m == "boom"));
    }

    #[tokio::test]
    async fn dead_child_requests_fail_not_hang() {
        let Some(py) = python() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let t = spawn_py(py, dir.path(), "import sys\nsys.exit(0)\n", 5);
        // The child exits immediately; the request must error promptly rather
        // than hang for the full timeout.
        let err = t.request(json!({"method": "x"})).await.unwrap_err();
        assert!(matches!(
            err,
            TransportError::Dead | TransportError::Timeout(_)
        ));
    }

    #[tokio::test]
    async fn non_object_request_is_rejected() {
        let Some(py) = python() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let t = spawn_py(py, dir.path(), ECHO, 5);
        let err = t.request(json!("not-an-object")).await.unwrap_err();
        assert!(matches!(err, TransportError::BadRequest));
    }

    #[tokio::test]
    async fn supervised_respawns_after_child_exits() {
        let Some(py) = python() else {
            return;
        };
        // Handles exactly one request, then exits — so the second call only
        // succeeds if the supervisor re-spawned a fresh child.
        let body = r#"
import sys, json
line = sys.stdin.readline()
req = json.loads(line)
sys.stdout.write(json.dumps({"id": req["id"], "ok": {"n": 1}}) + "\n")
sys.stdout.flush()
"#;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("once.py");
        std::fs::write(&script, body).unwrap();
        let cfg = TransportConfig::new(
            py,
            vec![script.to_str().unwrap().to_string()],
            Duration::from_secs(5),
            "test",
        );
        // Zero cooldown so the re-spawn isn't rate-limited within the test.
        let t = SupervisedTransport::with_cooldown(cfg, Duration::ZERO);

        assert_eq!(
            t.request(json!({"method": "x"})).await.unwrap(),
            json!({"n": 1})
        );
        // The child exited after replying; let the reader observe EOF.
        tokio::time::sleep(Duration::from_millis(200)).await;
        // Second call must re-spawn and succeed again.
        assert_eq!(
            t.request(json!({"method": "x"})).await.unwrap(),
            json!({"n": 1})
        );
    }

    #[tokio::test]
    async fn supervised_cooldown_blocks_respawn_storm() {
        // A command that never exists: the first call attempts a spawn (fails),
        // and a second call within the cooldown returns Dead without a second
        // spawn attempt.
        let t = SupervisedTransport::with_cooldown(
            TransportConfig::new(
                "/nonexistent/transport-binary",
                vec![],
                Duration::from_secs(5),
                "test",
            ),
            Duration::from_secs(3600),
        );
        assert!(matches!(
            t.request(json!({"method": "x"})).await.unwrap_err(),
            TransportError::Dead
        ));
        assert!(matches!(
            t.request(json!({"method": "x"})).await.unwrap_err(),
            TransportError::Dead
        ));
    }

    /// Review-followup F: strengthen the cooldown test by exercising the
    /// actual gating behaviour rather than just "broken-stays-broken".
    /// Uses a real, exits-after-one-call sidecar so the cooldown is the
    /// only thing standing between the second call and a fresh spawn.
    ///
    /// Sequence:
    /// 1. First call succeeds, child exits.
    /// 2. Wait for the reader to observe EOF (alive → false).
    /// 3. Second call *within* the cooldown must return Dead — proving
    ///    the cooldown short-circuits the re-spawn attempt.
    /// 4. Wait past the cooldown.
    /// 5. Third call must succeed — proving the cooldown is a window,
    ///    not a permanent latch.
    #[tokio::test]
    async fn supervised_cooldown_window_short_circuits_then_releases() {
        let Some(py) = python() else {
            eprintln!("skipping: no python3");
            return;
        };
        let body = r#"
import sys, json
line = sys.stdin.readline()
req = json.loads(line)
sys.stdout.write(json.dumps({"id": req["id"], "ok": {"n": 1}}) + "\n")
sys.stdout.flush()
"#;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("once.py");
        std::fs::write(&script, body).unwrap();
        let cfg = TransportConfig::new(
            py,
            vec![script.to_str().unwrap().to_string()],
            Duration::from_secs(5),
            "test",
        );
        let cooldown = Duration::from_millis(400);
        let t = SupervisedTransport::with_cooldown(cfg, cooldown);

        // 1. First call: child handles + exits.
        assert_eq!(
            t.request(json!({"method": "x"})).await.unwrap(),
            json!({"n": 1})
        );
        // 2. Let the reader see EOF.
        tokio::time::sleep(Duration::from_millis(150)).await;

        // 3. Second call lands inside the cooldown window: Dead, not a
        //    fresh spawn. The cooldown started ticking on the *first*
        //    spawn attempt (during step 1), so we're still inside it.
        let err = t.request(json!({"method": "x"})).await.unwrap_err();
        assert!(
            matches!(err, TransportError::Dead),
            "in-cooldown request must short-circuit to Dead; got {err:?}"
        );

        // 4. Wait past the cooldown.
        tokio::time::sleep(cooldown + Duration::from_millis(150)).await;

        // 5. Third call must succeed — cooldown released, fresh spawn allowed.
        assert_eq!(
            t.request(json!({"method": "x"})).await.unwrap(),
            json!({"n": 1}),
            "post-cooldown request must succeed via a fresh spawn"
        );
    }
}
