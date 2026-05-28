//! Runtime for LibreFang sidecar channel adapters.
//!
//! Implement [`SidecarAdapter`] for your channel, then drive it with [`run_stdio`]:
//!
//! ```ignore
//! use async_trait::async_trait;
//! use librefang_sidecar::{run_stdio, EmitFn, SendCommand, SidecarAdapter, events};
//!
//! struct MyAdapter;
//!
//! #[async_trait]
//! impl SidecarAdapter for MyAdapter {
//!     fn capabilities(&self) -> Vec<String> { vec!["typing".into()] }
//!
//!     async fn on_send(&self, cmd: SendCommand) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!         // deliver cmd.text / cmd.content to your platform
//!         Ok(())
//!     }
//!
//!     async fn produce(&self, emit: EmitFn) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!         loop {
//!             let m = my_platform_next().await?;
//!             emit(events::message_text(m.user_id, m.user_name, m.text));
//!         }
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
//!     run_stdio(MyAdapter).await
//! }
//! # fn my_platform_next() -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>>>> { unreachable!() }
//! ```
//!
//! The framework owns the stdin command-reader, the JSON envelope parsing, the `ready` re-announce handshake (bounded by `ready_max_attempts` so a pre-#5219 daemon without `ready_ack` does not flood stdout), the graceful `shutdown` path, and the discipline of keeping **stdout protocol-only** (your logs must go to stderr).
//!
//! ## Responsibility split
//!
//! - **Process restart is the daemon's job.**
//!   The supervisor in `crates/librefang-channels/src/sidecar.rs` respawns a crashed sidecar with backoff and a circuit-breaker.
//!   Your adapter must be *crash-safe* — hold no irreplaceable in-process state, and let the framework re-emit `ready` on each fresh start.
//!   Do **not** try to keep your own process alive across a fatal error.
//! - **Platform reconnect is the adapter's job.**
//!   Reconnecting a dropped WS / long-poll / SSE stream is your transport concern.
//!   [`with_backoff`] helps.
//!   It is independent of the daemon-managed process lifecycle.

use crate::protocol::{events, parse_command, Command, SendCommand};
use async_trait::async_trait;
use serde_json::Value;
use std::error::Error as StdError;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, watch, Mutex};

/// Boxed error type used across the runtime — matches what most async
/// transports already produce.
pub type DynError = Box<dyn StdError + Send + Sync>;

/// Callback an adapter's `produce` impl uses to push inbound events.
///
/// Cheap to clone and to call from multiple tasks.
/// The implementation pushes through a bounded mpsc; if the writer task has died the emit is silently dropped — same behavior as Python's `emit`, which writes to a (possibly broken) stdout.
pub type EmitFn = Arc<dyn Fn(Value) + Send + Sync>;

/// Raised by [`run`] when [`SidecarAdapter::produce`] exits with an unhandled error.
/// Cleanup ([`SidecarAdapter::on_shutdown`]) has already run by the time this is returned.
///
/// [`run_stdio`] converts this to a process exit code of 1 so the daemon supervisor sees a non-zero exit (distinguishable from a clean `shutdown` / EOF).
#[derive(Debug)]
pub struct ProducerCrashed {
    pub source: DynError,
}

impl std::fmt::Display for ProducerCrashed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "sidecar producer crashed: {}", self.source)
    }
}

impl StdError for ProducerCrashed {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&*self.source)
    }
}

/// Base trait every sidecar adapter implements.
///
/// Override [`on_send`](SidecarAdapter::on_send) (required) and, for platforms you poll, [`produce`](SidecarAdapter::produce).
/// Declare optional capabilities so LibreFang routes rich features (typing/reaction/interactive/thread/streaming/typing_events) to you instead of degrading to plain text.
///
/// ## Cancel safety / blocking work
///
/// Every async method on this trait runs inside the SDK runtime, which the daemon-side supervisor can SIGKILL once its shutdown-grace window expires.
/// `produce` is also explicitly aborted by the runtime at shutdown (via `tokio::task::JoinHandle::abort`).
/// `on_command` is awaited inline in the main read loop, so a long-running body blocks the runtime from observing subsequent `shutdown` commands until it returns — the supervisor's grace timer is the only backstop.
/// `on_shutdown` is awaited during drain and has the same SIGKILL fallback.
///
/// In all cases, an implementation that does blocking sync work (CPU-bound loop, `std::thread::sleep`, blocking syscall) without yielding will not observe cancellation and risks being killed mid-cleanup.
/// If your method body has a long sync section, sprinkle `tokio::task::yield_now().await` or move the work to `tokio::task::spawn_blocking` so the runtime can preempt it on shutdown.
///
/// ## Panic isolation
///
/// `produce`, `on_command`, and `on_shutdown` are each wrapped by the runtime in a `tokio::spawn` so a panic inside the user's body is captured (via `JoinError::is_panic()`) instead of unwinding through `run` and aborting the whole process.
/// `produce` and `on_command` panics surface as a protocol `error` event back to the daemon; `on_shutdown` panics are logged to stderr (no `emit` is available during cleanup).
#[async_trait]
pub trait SidecarAdapter: Send + Sync {
    /// Capability strings declared in the `ready` event, e.g. `["typing", "interactive"]`.
    /// Default: none — adapter degrades to plain text only.
    fn capabilities(&self) -> Vec<String> {
        Vec::new()
    }

    /// Multi-bot account id, if this adapter is one of several instances sharing a config namespace.
    fn account_id(&self) -> Option<String> {
        None
    }

    /// When `true`, the daemon posts error responses privately to the operator instead of back into the user's chat.
    /// Useful for broadcast/notification adapters.
    fn suppress_error_responses(&self) -> bool {
        false
    }

    /// Operator inbox(es) for non-conversational notifications.
    fn notification_recipients(&self) -> Vec<Value> {
        Vec::new()
    }

    /// `[(host, [[k, v], ...]), ...]` auth headers the daemon should attach when fetching media URLs the adapter exposes.
    fn header_rules(&self) -> Vec<Value> {
        Vec::new()
    }

    /// Optional protocol-version tag carried on `ready` for skew diagnostics.
    /// Logged by the daemon, never enforced.
    fn protocol_version(&self) -> Option<u32> {
        None
    }

    /// Build the `ready` event payload from the trait's declarative methods.
    /// Override for full control (rarely needed).
    fn ready_event(&self) -> Value {
        events::ready(
            self.capabilities(),
            self.account_id(),
            self.suppress_error_responses(),
            self.notification_recipients(),
            self.header_rules(),
            self.protocol_version(),
        )
    }

    /// Deliver an outbound message to the platform. **Required.**
    async fn on_send(&self, cmd: SendCommand) -> Result<(), DynError>;

    /// Dispatch any inbound command.
    /// Default routes [`Command::Send`] to [`on_send`](SidecarAdapter::on_send); other variants are no-ops unless you override this.
    /// `ReadyAck` / `Shutdown` are handled by the framework and never reach here.
    async fn on_command(&self, cmd: Command) -> Result<(), DynError> {
        if let Command::Send(s) = cmd {
            self.on_send(s).await?;
        }
        Ok(())
    }

    /// Pull inbound platform events and push each one via `emit`.
    /// Default: nothing — for command/webhook-only adapters.
    ///
    /// See the trait-level "Cancel safety" note: the runtime explicitly aborts this future at shutdown, and a non-yielding body will be SIGKILLed by the supervisor's grace timer.
    async fn produce(&self, _emit: EmitFn) -> Result<(), DynError> {
        Ok(())
    }

    /// Cleanup on graceful shutdown.
    async fn on_shutdown(&self) -> Result<(), DynError> {
        Ok(())
    }
}

/// Retry `op` with exponential backoff until it returns `Ok`.
///
/// For **platform** reconnection only — process restart is the daemon's job (see module docs).
/// Propagates cancellation by dropping.
/// Logs each failure to stderr via `eprintln!` to keep stdout protocol-clean.
pub async fn with_backoff<F, Fut, T>(
    mut op: F,
    initial: Duration,
    maximum: Duration,
    factor: f64,
) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, DynError>>,
{
    // Refuse the inputs that turn this loop into a CPU spin or a converging delay.
    // factor < 1.0 makes the delay shrink toward zero on every failure (silent rate-limit / IP-ban escalation against the remote).
    // factor <= 0.0 + non-finite values produce a zero or NaN delay that becomes a tight retry spin.
    // initial == ZERO is a degenerate seed because `0.0 * factor == 0.0`, so the multiplicative growth never escapes zero — every failure becomes a tight retry spin even when `factor >= 1.0` would otherwise grow the delay correctly.
    // initial > maximum is a configuration typo — the loop would honor `initial` on iteration 0 then clamp later, masking the typo.
    assert!(
        factor.is_finite() && factor >= 1.0,
        "with_backoff: factor must be finite and >= 1.0 (got {factor})"
    );
    assert!(
        initial > Duration::ZERO,
        "with_backoff: initial must be > Duration::ZERO (got {initial:?}) — a zero seed means delay * factor stays zero forever, turning the retry loop into a tight CPU spin against a failing remote"
    );
    assert!(
        initial <= maximum,
        "with_backoff: initial ({initial:?}) must be <= maximum ({maximum:?})"
    );
    let mut delay = initial;
    loop {
        match op().await {
            Ok(v) => return v,
            Err(e) => {
                eprintln!(
                    "[librefang-sidecar] operation failed; backing off {}ms: {}",
                    delay.as_millis(),
                    e
                );
                tokio::time::sleep(delay).await;
                let next = (delay.as_secs_f64() * factor).min(maximum.as_secs_f64());
                delay = Duration::from_secs_f64(next);
            }
        }
    }
}

/// Best-effort stringification of a `Box<dyn Any + Send + 'static>` produced by `tokio::task::JoinError::into_panic`.
/// Covers the two common payload types `panic!` emits — `&'static str` and `String` — and falls back to an opaque label so the message slot is never empty.
fn panic_payload_message(payload: Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        return (*s).to_string();
    }
    if let Some(s) = payload.downcast_ref::<String>() {
        return s.clone();
    }
    "(non-string panic payload)".to_string()
}

/// Convert a `JoinError` into a one-line panic message, prefixed with `label`.
/// Returns `None` for the non-panic cases (clean exit or cancellation) so the caller can branch on whether something blew up.
/// Three sites — produce / on_command / on_shutdown — all want the same `"<label> panicked: <msg>"` shape; centralising it here keeps the format consistent.
fn format_join_panic(je: tokio::task::JoinError, label: &str) -> Option<String> {
    if je.is_panic() {
        Some(format!(
            "{label} panicked: {}",
            panic_payload_message(je.into_panic())
        ))
    } else {
        None
    }
}

/// Drive an adapter against a `mpsc::Receiver<String>` line source and a `mpsc::Sender<Value>` emit sink.
/// Returns when LibreFang sends `shutdown` or `line_rx` reaches EOF (the sender side dropped).
///
/// `ready_max_attempts` bounds the un-acked `ready` re-announce (`0` = re-announce forever).
/// After the cap the loop stops re-announcing but the run continues — a pre-#5219 daemon that never sends `ready_ack` still gets the first `ready` and the adapter keeps serving without flooding stdout.
pub async fn run<A: SidecarAdapter + 'static>(
    adapter: Arc<A>,
    mut line_rx: mpsc::Receiver<String>,
    emit_tx: mpsc::Sender<Value>,
    ready_interval: Duration,
    ready_max_attempts: u32,
) -> Result<(), ProducerCrashed> {
    // Stop signal — set on Shutdown or stdin EOF. Watch so multiple
    // tasks can observe (ready loop, producer).
    let (stop_tx, mut stop_rx) = watch::channel(false);

    // ReadyAck signal — set when the daemon sends ready_ack.
    let (acked_tx, acked_rx) = watch::channel(false);

    // Shared producer error, if any — surfaced as ProducerCrashed.
    let producer_err: Arc<Mutex<Option<DynError>>> = Arc::new(Mutex::new(None));
    // Signal fired when the producer task exits (with or without error).
    // The main loop selects on it so a producer crash terminates the
    // run promptly instead of waiting for the next stdin line.
    let (producer_done_tx, mut producer_done_rx) = watch::channel(false);

    // Emit callback handed to the producer. Sync; pushes through emit_tx via try_send.
    // A full or closed channel still drops the event (the supervisor's stdout pipe is back-pressured or broken), but we count drops in an Arc<AtomicU64> and log the 1st and every 100th drop to stderr — same rate-limited pattern the daemon-side supervisor uses (`crates/librefang-channels/src/sidecar.rs` overflow=drop_newest path).
    // A silent drop is the worst outcome; a noisy log lets the operator see "the daemon isn't draining my stdout" instead of "my messages just disappear".
    let emit: EmitFn = {
        let tx = emit_tx.clone();
        let dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
        Arc::new(move |v: Value| {
            if tx.try_send(v).is_err() {
                let n = dropped.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if n == 1 || n % 100 == 0 {
                    eprintln!(
                        "[librefang-sidecar] producer emit dropped — channel full or writer task gone ({n} dropped total). Sustained drops indicate the supervisor is not draining sidecar stdout."
                    );
                }
            }
        })
    };

    // Ready re-announce task. Emits ready every `ready_interval` until acked, stop fires, or max_attempts is reached.
    // Every emit_tx.send(...) is itself wrapped in select! with stop_rx so a wedged writer (e.g. supervisor stopped reading sidecar stdout) cannot park this task on send and prevent it from observing the stop signal during shutdown.
    let ready_handle = {
        let adapter = adapter.clone();
        let emit_tx = emit_tx.clone();
        let mut acked_rx = acked_rx.clone();
        let mut stop_rx = stop_rx.clone();
        tokio::spawn(async move {
            let mut attempts: u32 = 0;
            loop {
                if *acked_rx.borrow() || *stop_rx.borrow() {
                    return;
                }
                tokio::select! {
                    _ = emit_tx.send(adapter.ready_event()) => {}
                    _ = stop_rx.changed() => return,
                }
                attempts += 1;
                if ready_max_attempts != 0 && attempts >= ready_max_attempts {
                    // Stop re-announcing but keep the adapter alive — a pre-#5219 daemon without ready_ack still got the first ready.
                    // Producer + reader loop continue.
                    return;
                }
                tokio::select! {
                    _ = tokio::time::sleep(ready_interval) => {}
                    _ = acked_rx.changed() => {}
                    _ = stop_rx.changed() => {}
                }
            }
        })
    };

    // Producer task — adapter's inbound stream.
    // Only an ERROR or PANIC from `produce` terminates the run (via `producer_done_tx`); a clean `Ok(())` return is a legitimate "emit-once-and-exit" adapter and must not kill the command loop.
    // produce() is spawned as its own child task so a panic inside the user's `produce` impl surfaces as `JoinError::is_panic()` instead of silently aborting the future and leaving the main loop blocked forever waiting for a wake-up that never comes.
    // The inner JoinHandle is polled by-reference (`&mut inner`) inside the select so the stop-arm can call `inner.abort()` afterward — dropping a JoinHandle in tokio detaches the task (leaving the user's produce running past shutdown holding Arc<A> and emit-sender clones), abort actually cancels it.
    let producer_handle = {
        let adapter = adapter.clone();
        let emit = emit.clone();
        let producer_err = producer_err.clone();
        let producer_done_tx = producer_done_tx.clone();
        let mut stop_rx = stop_rx.clone();
        tokio::spawn(async move {
            let inner_adapter = adapter.clone();
            let inner_emit = emit.clone();
            let mut inner = tokio::spawn(async move { inner_adapter.produce(inner_emit).await });
            tokio::select! {
                // `biased` so that an inner produce completion (Ok / Err / panic) ALWAYS wins over a stop signal arriving in the same scheduler tick.
                // Without it, tokio::select! is pseudo-random across ready branches and a producer panic racing with shutdown can be swallowed by the stop arm's `let _ = inner.await;` discard, hiding the crash from the supervisor.
                biased;
                join_result = &mut inner => {
                    let to_record: Option<DynError> = match join_result {
                        Ok(Ok(())) => None,
                        Ok(Err(e)) => Some(e),
                        // Cancellation only fires if `inner` was externally aborted; today only the stop-arm below does that, which exits the select via a different branch.
                        // format_join_panic returns None on JoinError::Cancelled and Some(msg) on JoinError::is_panic, so this single arm covers both panic-record and cancel-ignore.
                        Err(je) => format_join_panic(je, "produce").map(|s| s.into()),
                    };
                    if let Some(e) = to_record {
                        *producer_err.lock().await = Some(e);
                        let _ = producer_done_tx.send(true);
                    }
                }
                _ = stop_rx.changed() => {
                    // Stop fired; cancel the inner produce future to prevent task leak.
                    inner.abort();
                    let _ = inner.await;
                }
            }
        })
    };

    // Main loop — read commands until stop, EOF, or producer crash.
    loop {
        tokio::select! {
            line = line_rx.recv() => {
                match line {
                    None => {
                        // EOF on stdin — treat as clean shutdown.
                        break;
                    }
                    Some(line) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        match parse_command(trimmed) {
                            Ok(Command::Shutdown) => break,
                            Ok(Command::ReadyAck) => {
                                let _ = acked_tx.send(true);
                            }
                            Ok(cmd) => {
                                // on_command is dispatched via a child tokio::spawn so a panic inside the user's adapter (e.g. `unwrap()` on a refreshed-token JSON missing a field) surfaces as `JoinError::is_panic()` instead of unwinding through `run` and aborting the whole process — panic-capture parity with the producer task and on_shutdown.
                                // NOTE: this only equalises panic isolation; the dispatch itself remains serialized (the loop awaits the spawn handle inline) and a long-running on_command still blocks subsequent line_rx polling. If a future change wants cancel-on-stop here, wrap the await in tokio::select! with stop_rx and store an abort handle.
                                let cmd_adapter = adapter.clone();
                                let outcome =
                                    tokio::spawn(async move { cmd_adapter.on_command(cmd).await })
                                        .await;
                                let to_report: Option<String> = match outcome {
                                    Ok(Ok(())) => None,
                                    Ok(Err(e)) => Some(format!("{e}")),
                                    // Cancellation unreachable today (spawn has no abort handle exposed); format_join_panic returns None in that case so the arm safely degrades to no-op.
                                    Err(je) => format_join_panic(je, "on_command"),
                                };
                                if let Some(msg) = to_report {
                                    // emit wrapped in select! with stop_rx so a wedged writer does not stall the loop and prevent subsequent shutdown commands from being read.
                                    // `biased` so a ready emit ALWAYS wins over a stop signal arriving in the same tick — without it the supervisor can lose the error event that explains why the loop is exiting, leaving a panic / parse-error invisible. Same fix-class as the producer-side biased at line 326.
                                    let err_event = events::error(msg);
                                    tokio::select! {
                                        biased;
                                        _ = emit_tx.send(err_event) => {}
                                        _ = stop_rx.changed() => break,
                                    }
                                }
                            }
                            Err(e) => {
                                let err_event = events::error(format!("protocol parse error: {e}"));
                                tokio::select! {
                                    biased;
                                    _ = emit_tx.send(err_event) => {}
                                    _ = stop_rx.changed() => break,
                                }
                            }
                        }
                    }
                }
            }
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() { break; }
            }
            _ = producer_done_rx.changed() => {
                if *producer_done_rx.borrow() {
                    // Producer exited — either clean (Ok) or with an
                    // error stashed in producer_err. Stop the run
                    // either way; the post-loop block surfaces the
                    // error if present.
                    break;
                }
            }
        }
    }

    // Drain: signal stop, await background tasks, run on_shutdown once.
    let _ = stop_tx.send(true);
    let _ = ready_handle.await;
    let _ = producer_handle.await;
    // on_shutdown is wrapped in tokio::spawn for the same reason as produce / on_command: a panic in the user's cleanup must not unwind through `run` and abort the whole process with no exit-code distinguishing a clean shutdown from a crashed one.
    // No protocol error event is emitted because the trait method takes only `&self` (no `emit` parameter) and the spawn closure deliberately does not capture `emit_tx` — we keep cleanup diagnostics on stderr so the supervisor's stderr ingest still sees the panic message in its main log, and the writer task stays single-purpose (drains user-facing events only, not infrastructure logs).
    let shutdown_adapter = adapter.clone();
    let shutdown_outcome = tokio::spawn(async move { shutdown_adapter.on_shutdown().await }).await;
    match shutdown_outcome {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            eprintln!("[librefang-sidecar] on_shutdown returned error: {e}");
        }
        // Cancellation unreachable today (same contract as produce / on_command — no abort handle exposed); format_join_panic returns None then so the arm is a no-op.
        Err(je) => {
            if let Some(msg) = format_join_panic(je, "on_shutdown") {
                eprintln!("[librefang-sidecar] {msg}");
            }
        }
    }

    if let Some(e) = producer_err.lock().await.take() {
        return Err(ProducerCrashed { source: e });
    }
    Ok(())
}

/// Production entry point: wire the adapter to real `stdin`/`stdout` and drive `run`.
///
/// Spawns a reader task that converts `stdin` lines into the `line_rx` mpsc; spawns a writer task that drains `emit_tx` and writes one newline-delimited JSON line per event to `stdout`.
/// Both tasks shut down when the underlying I/O completes or the channels close.
///
/// Returns when LibreFang sends `shutdown`, stdin reaches EOF, or [`SidecarAdapter::produce`] errors out (the last case yields [`ProducerCrashed`]).
pub async fn run_stdio<A: SidecarAdapter + 'static>(adapter: A) -> Result<(), DynError> {
    run_stdio_with(adapter, Duration::from_secs(2), 5).await
}

/// [`run_stdio`] with explicit tunables for the `ready` re-announce loop.
/// `ready_max_attempts = 0` re-announces forever.
pub async fn run_stdio_with<A: SidecarAdapter + 'static>(
    adapter: A,
    ready_interval: Duration,
    ready_max_attempts: u32,
) -> Result<(), DynError> {
    let adapter = Arc::new(adapter);

    let (line_tx, line_rx) = mpsc::channel::<String>(256);
    let (emit_tx, mut emit_rx) = mpsc::channel::<Value>(256);

    // Reader: stdin → line_tx.
    let reader_handle = tokio::spawn(async move {
        let stdin = tokio::io::stdin();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line_tx.send(line).await.is_err() {
                        // Receiver dropped — main loop returned.
                        return;
                    }
                }
                Ok(None) | Err(_) => {
                    // EOF or read error — drop line_tx so the main
                    // loop sees None and exits cleanly.
                    return;
                }
            }
        }
    });

    // Writer: emit_rx → stdout (one JSON object per line).
    let writer_handle = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(v) = emit_rx.recv().await {
            let mut line = match serde_json::to_string(&v) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("[librefang-sidecar] failed to serialize event: {e}");
                    continue;
                }
            };
            line.push('\n');
            if let Err(e) = stdout.write_all(line.as_bytes()).await {
                eprintln!("[librefang-sidecar] stdout write failed: {e}");
                return;
            }
            if let Err(e) = stdout.flush().await {
                eprintln!("[librefang-sidecar] stdout flush failed: {e}");
                return;
            }
        }
    });

    let result = run(
        adapter,
        line_rx,
        emit_tx,
        ready_interval,
        ready_max_attempts,
    )
    .await;

    // Abort the reader first.
    // After `run` returns we no longer care about further stdin lines, and a daemon that keeps its write half of the sidecar's stdin open (e.g. during a graceful drain or when the supervisor is itself stuck) leaves `next_line().await` parked indefinitely.
    // Without abort, `reader_handle.await` would block until the OS finally closes the pipe — turning a clean shutdown into a wedged process the supervisor must SIGKILL.
    reader_handle.abort();
    let _ = reader_handle.await;
    // The writer task drains naturally once emit_tx is dropped (which happened when `run` returned by stack-unwind), so a plain await is correct here.
    let _ = writer_handle.await;

    match result {
        Ok(()) => Ok(()),
        Err(e) => Err(Box::new(e) as DynError),
    }
}

/// One-stop adapter `main` helper: handle `--describe` and otherwise build and drive the adapter via `run_stdio`.
///
/// The daemon's discovery contract (`crates/librefang-api/src/routes/sidecar_describe.rs`) is to spawn the adapter binary with `--describe`, expect a single JSON object on stdout, and then exit; any other run path drives the JSON-RPC protocol on stdio normally.
/// Python's equivalent is `librefang.sidecar.runtime.run_stdio_main(AdapterClass)`, which takes the adapter *class* so `--describe` can serve the schema without instantiating; this Rust version takes a `FnOnce() -> Result<A, DynError>` builder for the same reason — the closure runs only after the `--describe` branch decides not to serve discovery, so adapters whose constructor reads required env vars (bot tokens, credentials) still discover cleanly at boot before the operator has configured anything.
/// Returning `Result` from the builder means a missing-env bootstrap failure becomes a structured error message instead of a `panic!` + stack trace, which is what `MyAdapter::new()` -> `expect("BOT_TOKEN must be set")` would otherwise produce.
///
/// ```ignore
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
///     run_stdio_main(MyAdapter::schema, || Ok(MyAdapter::new())).await
/// }
/// ```
///
/// `schema_fn` is called lazily (only when `--describe` is present) so the schema-construction work is skipped on the normal run path; it returns a `Schema` infallibly, so the schema itself should be pure (no env-var lookups or disk reads — push those into `build_fn`, which returns `Result`).
/// When `--describe` is present in argv this emits the schema JSON to stdout and returns `Ok(())` without constructing the adapter, and a failure to flush stdout is intentionally swallowed (matches Python `describe_main` which returns 0 unconditionally after writing — a flush error here from a broken read half would otherwise let the dashboard read a successful schema yet treat the adapter as broken via the non-zero exit code).
pub async fn run_stdio_main<A, S, B>(schema_fn: S, build_fn: B) -> Result<(), DynError>
where
    A: SidecarAdapter + 'static,
    S: FnOnce() -> crate::protocol::Schema,
    B: FnOnce() -> Result<A, DynError>,
{
    if std::env::args().any(|a| a == "--describe") {
        let schema = schema_fn();
        let body = serde_json::to_string(&schema)?;
        let mut stdout = tokio::io::stdout();
        stdout.write_all(body.as_bytes()).await?;
        let _ = stdout.flush().await;
        return Ok(());
    }
    let adapter = build_fn()?;
    run_stdio(adapter).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{ChannelUser, Content};
    use std::sync::atomic::{AtomicU32, Ordering};

    struct TestAdapter {
        sends: AtomicU32,
    }

    #[async_trait]
    impl SidecarAdapter for TestAdapter {
        fn capabilities(&self) -> Vec<String> {
            vec!["typing".into()]
        }
        async fn on_send(&self, _cmd: SendCommand) -> Result<(), DynError> {
            self.sends.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn shutdown_terminates_run_cleanly() {
        let (line_tx, line_rx) = mpsc::channel::<String>(8);
        let (emit_tx, _emit_rx) = mpsc::channel::<Value>(8);
        let adapter = Arc::new(TestAdapter {
            sends: AtomicU32::new(0),
        });

        line_tx
            .send(r#"{"method":"shutdown"}"#.into())
            .await
            .unwrap();

        let result = run(adapter, line_rx, emit_tx, Duration::from_millis(50), 3).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn send_command_routes_to_on_send() {
        let (line_tx, line_rx) = mpsc::channel::<String>(8);
        let (emit_tx, _emit_rx) = mpsc::channel::<Value>(8);
        let adapter = Arc::new(TestAdapter {
            sends: AtomicU32::new(0),
        });

        // Two send commands followed by shutdown.
        let send = serde_json::to_string(&serde_json::json!({
            "method": "send",
            "params": {
                "channel_id": "c1", "text": "hi",
                "user": {"platform_id": "c1", "display_name": "B", "librefang_user": null},
            }
        }))
        .unwrap();
        line_tx.send(send.clone()).await.unwrap();
        line_tx.send(send).await.unwrap();
        line_tx
            .send(r#"{"method":"shutdown"}"#.into())
            .await
            .unwrap();

        let adapter_ref = adapter.clone();
        run(adapter, line_rx, emit_tx, Duration::from_millis(50), 3)
            .await
            .unwrap();
        assert_eq!(adapter_ref.sends.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn ready_is_emitted_until_acked() {
        let (line_tx, line_rx) = mpsc::channel::<String>(8);
        let (emit_tx, mut emit_rx) = mpsc::channel::<Value>(8);
        let adapter = Arc::new(TestAdapter {
            sends: AtomicU32::new(0),
        });

        // Run the driver with a tight ready interval. ACK after the
        // first ready, then shutdown.
        let driver = tokio::spawn({
            let adapter = adapter.clone();
            async move { run(adapter, line_rx, emit_tx, Duration::from_millis(20), 0).await }
        });

        // Drain the first ready event.
        let first = emit_rx.recv().await.expect("first ready");
        assert_eq!(first["method"], "ready");
        assert_eq!(
            first["params"]["capabilities"],
            serde_json::json!(["typing"])
        );

        line_tx
            .send(r#"{"method":"ready_ack"}"#.into())
            .await
            .unwrap();
        line_tx
            .send(r#"{"method":"shutdown"}"#.into())
            .await
            .unwrap();

        // The driver should exit cleanly.
        driver.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn malformed_line_produces_error_event_and_continues() {
        let (line_tx, line_rx) = mpsc::channel::<String>(8);
        let (emit_tx, mut emit_rx) = mpsc::channel::<Value>(8);
        let adapter = Arc::new(TestAdapter {
            sends: AtomicU32::new(0),
        });

        line_tx.send("{not json".into()).await.unwrap();
        line_tx
            .send(r#"{"method":"shutdown"}"#.into())
            .await
            .unwrap();

        let driver = tokio::spawn(async move {
            run(adapter, line_rx, emit_tx, Duration::from_millis(50), 3).await
        });

        // The first emitted event should be either the periodic ready
        // (we're using a 50ms interval, so it's likely first) or the
        // error from the malformed line — drain until we see the
        // error event.
        let mut saw_error = false;
        while let Some(v) = emit_rx.recv().await {
            if v["method"] == "error" {
                saw_error = true;
                break;
            }
        }
        assert!(saw_error, "expected an error event for malformed line");
        driver.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn channel_user_typed_emit_round_trips_through_send() {
        // Smoke check: ChannelUser → Send → JSON → parse → ChannelUser.
        let original = SendCommand {
            channel_id: "c1".into(),
            text: "hi".into(),
            content: Some(Content::text("hi")),
            thread_id: Some("t1".into()),
            user: ChannelUser {
                platform_id: "c1".into(),
                display_name: "Bob".into(),
                librefang_user: None,
            },
        };
        let wire = serde_json::to_string(&serde_json::json!({
            "method": "send",
            "params": original,
        }))
        .unwrap();
        let parsed = parse_command(&wire).unwrap();
        match parsed {
            Command::Send(s) => assert_eq!(s, original),
            _ => panic!("expected Send"),
        }
    }
}
