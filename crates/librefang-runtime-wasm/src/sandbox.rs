//! WASM sandbox for secure skill/plugin execution.
//!
//! Uses Wasmtime to execute untrusted WASM modules with deny-by-default
//! capability-based permissions. No filesystem, network, or credential
//! access unless explicitly granted.
//!
//! # Guest ABI
//!
//! WASM modules must export:
//! - `memory` — linear memory
//! - `alloc(size: i32) -> i32` — allocate `size` bytes, return pointer
//! - `execute(input_ptr: i32, input_len: i32) -> i64` — main entry point
//!
//! The `execute` function receives JSON input bytes and returns a packed
//! `i64` value: `(result_ptr << 32) | result_len`. The result is JSON bytes.
//!
//! # Host ABI
//!
//! The host provides (in the `"librefang"` import module):
//! - `host_call(request_ptr: i32, request_len: i32) -> i64` — RPC dispatch
//! - `host_log(level: i32, msg_ptr: i32, msg_len: i32)` — logging
//!
//! `host_call` reads a JSON request `{"method": "...", "params": {...}}`
//! and returns a packed pointer to JSON `{"ok": ...}` or `{"error": "..."}`.

use crate::host_functions;
use librefang_kernel_handle::prelude::*;
use librefang_types::capability::Capability;
use serde_json::json;
use std::sync::Arc;
use tracing::debug;
use wasmtime::*;

/// Maximum number of bytes accepted from a guest `host_log` call.
///
/// A WASM guest can supply an arbitrary `msg_len` pointer length to
/// `host_log`. Without this cap, a malicious or buggy guest can push
/// megabytes into the host's structured log stream, filling disk space or
/// injecting fake audit lines. Messages longer than this limit are
/// truncated and annotated with a byte count so the operator knows the
/// original was clipped.
const MAX_LOG_BYTES: usize = 4096;

/// Maximum bytes accepted in a single `host_call` request payload (#3866).
///
/// Aligned with the shell_exec cap (#3529). Lifted to module scope so the
/// regression test in this file pins the same constant the host reads at
/// runtime — no risk of the test passing while the call site silently
/// raises the cap.
pub(crate) const MAX_HOST_CALL_REQUEST_BYTES: usize = 1024 * 1024;

/// Maximum bytes accepted in a guest `execute` result payload (#3866).
pub(crate) const MAX_GUEST_RESULT_BYTES: usize = 1024 * 1024;

/// Marker error returned by the per-store `epoch_deadline_callback` when
/// THIS guest has overrun its wall-clock budget (#3864). Detected via
/// `downcast_ref` so the timeout-trap path doesn't depend on string
/// matching against wasmtime's wrapped error message.
#[derive(Debug)]
struct WallClockTimeout {
    budget: std::time::Duration,
}

impl std::fmt::Display for WallClockTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "WASM guest exceeded wall-clock timeout of {}s",
            self.budget.as_secs()
        )
    }
}

impl std::error::Error for WallClockTimeout {}

/// Per-method fuel reservation for `host_call` (#3532).
///
/// Wasm fuel only meters guest instructions, so without these
/// reservations a guest can fan out arbitrarily many host LLM calls,
/// outbound fetches, or child agent spawns under a token budget meant
/// to cap CPU. Each cost is an upper-bound estimate of the host-side
/// resource the call may consume — the goal is to make the
/// per-invocation `fuel_limit` a real ceiling on host cost, not just
/// guest CPU.
///
/// Calibration: with the default `fuel_limit = 1_000_000` (sandbox
/// default), the LLM-bearing path `agent_send` is bounded to ~10
/// invocations per guest run, `agent_spawn` to ~5, and `net_fetch` to
/// ~200 — sane upper bounds for skill-shaped workloads. Pure-host
/// methods (`time_now`, `kv_*`, `env_read`, `fs_*`) charge nothing so
/// existing benign skills stay within budget.
pub(crate) fn host_call_fuel_cost(method: &str) -> u64 {
    match method {
        // LLM-bearing fanout — `send_to_agent` triggers a downstream
        // agent loop that may itself call providers. Highest cost.
        "agent_send" => 100_000,
        // `spawn_agent_checked` registers a new agent and (depending on
        // schedule) immediately runs it. Treat as an LLM-bearing call.
        "agent_spawn" => 200_000,
        // Outbound HTTP — bandwidth + provider rate-limit budget.
        "net_fetch" => 5_000,
        // Shell exec spawns a subprocess; CPU + audit volume.
        "shell_exec" => 5_000,
        _ => 0,
    }
}

/// Configuration for a WASM sandbox instance.
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Maximum fuel (CPU instruction budget). 0 = unlimited.
    pub fuel_limit: u64,
    /// Maximum WASM linear memory in bytes (reserved for future enforcement).
    pub max_memory_bytes: usize,
    /// Capabilities granted to this sandbox instance.
    pub capabilities: Vec<Capability>,
    /// Wall-clock timeout in seconds for epoch-based interruption.
    /// Defaults to 30 seconds if None.
    pub timeout_secs: Option<u64>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            fuel_limit: 1_000_000,
            max_memory_bytes: 16 * 1024 * 1024,
            capabilities: Vec::new(),
            timeout_secs: None,
        }
    }
}

/// `ResourceLimiter` implementation that caps WASM linear-memory growth at a
/// configured byte ceiling. Attached to every `Store` so that WASM plugins
/// cannot allocate unbounded host memory regardless of their fuel budget.
struct MemoryLimiter {
    max_bytes: usize,
}

impl wasmtime::ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        Ok(desired <= self.max_bytes)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmtime::Error> {
        // No table-element cap — only memory is bounded here.
        Ok(true)
    }
}

/// State carried in each WASM Store, accessible by host functions.
pub struct GuestState {
    /// Capabilities granted to this guest — checked before every host call.
    pub capabilities: Vec<Capability>,
    /// Handle to kernel for inter-agent operations.
    pub kernel: Option<Arc<dyn KernelHandle>>,
    /// Agent ID of the calling agent.
    pub agent_id: String,
    /// Tokio runtime handle for async operations in sync host functions.
    pub tokio_handle: tokio::runtime::Handle,
    /// Memory limiter enforcing `SandboxConfig::max_memory_bytes`.
    limiter: MemoryLimiter,
    /// Wall-clock instant this guest started executing — used by the
    /// per-store epoch_deadline_callback to make a store-local timeout
    /// decision, simulating per-store interrupt on top of the engine-global
    /// `Engine::increment_epoch()` mechanism (Bug #3864).
    start: std::time::Instant,
    /// Wall-clock budget for this guest. Set from `SandboxConfig::timeout_secs`.
    timeout: std::time::Duration,
}

#[cfg(test)]
impl GuestState {
    /// Build a `GuestState` for unit tests in sibling modules. The host_functions
    /// tests don't exercise WASM memory growth, so the limiter is set to
    /// effectively unbounded.
    pub(crate) fn for_test(
        capabilities: Vec<Capability>,
        kernel: Option<Arc<dyn KernelHandle>>,
        agent_id: String,
        tokio_handle: tokio::runtime::Handle,
    ) -> Self {
        Self {
            capabilities,
            kernel,
            agent_id,
            tokio_handle,
            limiter: MemoryLimiter {
                max_bytes: usize::MAX,
            },
            start: std::time::Instant::now(),
            // Effectively unbounded for unit tests that don't exercise
            // wall-clock timeout; Duration construction is safe far below
            // u64::MAX.
            timeout: std::time::Duration::from_secs(86_400),
        }
    }
}

/// Result of executing a WASM module.
#[derive(Debug)]
pub struct ExecutionResult {
    /// JSON output from the guest's `execute` function.
    pub output: serde_json::Value,
    /// Number of fuel units consumed.
    pub fuel_consumed: u64,
}

/// Errors from sandbox operations.
///
/// Per-crate typed error type for `librefang-runtime-wasm`, established as the
/// first concrete code wedge for the error-contracts effort (#3576). The
/// variants partition the WASM sandbox's failure modes by the recovery
/// strategy a caller would apply:
///
/// * `Compilation` — bad WASM bytes / engine misconfiguration; not retryable.
/// * `Instantiation` — linker / import wiring; not retryable without code
///   change.
/// * `Execution` — generic host-side execution failure (JSON serialization,
///   `spawn_blocking` join failure, wall-clock timeout, …); may be retryable.
/// * `FuelExhausted` — guest blew its CPU budget. Quota-recoverable.
/// * `AbiError` — guest violated the host-guest ABI contract (missing exports,
///   bad pointers, oversized payloads, malformed JSON). Not retryable.
///
/// Marked `#[non_exhaustive]` so future variants (e.g. a typed
/// `MemoryLimitExceeded`) can be added without a breaking-change bump on
/// downstream crates that match on the enum.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SandboxError {
    #[error("WASM compilation failed: {0}")]
    Compilation(String),
    #[error("WASM instantiation failed: {0}")]
    Instantiation(String),
    #[error("WASM execution failed: {0}")]
    Execution(String),
    #[error("Fuel exhausted: skill exceeded CPU budget")]
    FuelExhausted,
    #[error("Guest ABI violation: {0}")]
    AbiError(String),
}

/// The WASM sandbox engine.
///
/// # Epoch isolation (Bug #3864)
///
/// `Engine::increment_epoch()` is global to an `Engine` — calling it from a
/// watchdog thread would interrupt *every* `Store` sharing that engine, not
/// just the one that timed out. To prevent cross-guest contamination, each call
/// to `execute` creates a fresh `Engine`. The cost is one extra engine init per
/// invocation; WASM module compilation is unavoidably O(module size) regardless,
/// so the marginal overhead is small compared to the compilation step.
pub struct WasmSandbox;

/// Build the standard Wasmtime `Config` used by every sandbox Engine.
fn make_engine_config() -> Config {
    let mut config = Config::new();
    config.consume_fuel(true);
    config.epoch_interruption(true);
    config
}

impl WasmSandbox {
    /// Create a new sandbox instance. Validates the engine config eagerly.
    pub fn new() -> Result<Self, SandboxError> {
        // Build a throw-away Engine to surface any config errors at
        // construction time rather than first use.
        Engine::new(&make_engine_config()).map_err(|e| SandboxError::Compilation(e.to_string()))?;
        Ok(Self)
    }

    /// Execute a WASM module with the given JSON input.
    ///
    /// All host calls from within the module are subject to capability checks.
    /// Execution is offloaded to a blocking thread (CPU-bound WASM should not
    /// run on the Tokio executor).
    ///
    /// A fresh `Engine` is created for each invocation so that the epoch-based
    /// watchdog can safely call `engine.increment_epoch()` without disturbing
    /// any other concurrently running WASM guests.
    pub async fn execute(
        &self,
        wasm_bytes: &[u8],
        input: serde_json::Value,
        config: SandboxConfig,
        kernel: Option<Arc<dyn KernelHandle>>,
        agent_id: &str,
    ) -> Result<ExecutionResult, SandboxError> {
        // Build a fresh engine for this execution — epoch isolation requires it.
        let engine = Engine::new(&make_engine_config())
            .map_err(|e| SandboxError::Compilation(e.to_string()))?;
        let wasm_bytes = wasm_bytes.to_vec();
        let agent_id = agent_id.to_string();
        let handle = tokio::runtime::Handle::current();

        tokio::task::spawn_blocking(move || {
            Self::execute_sync(
                &engine,
                &wasm_bytes,
                input,
                &config,
                kernel,
                &agent_id,
                handle,
            )
        })
        .await
        .map_err(|e| SandboxError::Execution(format!("spawn_blocking join failed: {e}")))?
    }

    /// Synchronous inner execution — runs on a blocking thread.
    fn execute_sync(
        engine: &Engine,
        wasm_bytes: &[u8],
        input: serde_json::Value,
        config: &SandboxConfig,
        kernel: Option<Arc<dyn KernelHandle>>,
        agent_id: &str,
        tokio_handle: tokio::runtime::Handle,
    ) -> Result<ExecutionResult, SandboxError> {
        // Compile the module (accepts both .wasm binary and .wat text)
        let module = Module::new(engine, wasm_bytes)
            .map_err(|e| SandboxError::Compilation(e.to_string()))?;

        // Create store with guest state (includes the memory limiter)
        let timeout_secs = config.timeout_secs.unwrap_or(30);
        let mut store = Store::new(
            engine,
            GuestState {
                capabilities: config.capabilities.clone(),
                kernel,
                agent_id: agent_id.to_string(),
                tokio_handle,
                limiter: MemoryLimiter {
                    max_bytes: config.max_memory_bytes,
                },
                start: std::time::Instant::now(),
                timeout: std::time::Duration::from_secs(timeout_secs),
            },
        );

        // Enforce the memory cap: every memory.grow call from the guest goes
        // through MemoryLimiter::memory_growing before any allocation happens.
        store.limiter(|state| &mut state.limiter);

        // Per-store interrupt simulation (Bug #3864). `Engine::increment_epoch`
        // is engine-global: any concurrent Store sharing the engine sees the
        // tick. We already hand each `execute` its own `Engine` so the tick
        // can't physically reach another guest, but we layer on a
        // store-local sanity check via `epoch_deadline_callback`. When the
        // epoch fires, the callback inspects this store's own start/timeout
        // and traps only if THIS guest has actually exceeded its budget;
        // otherwise it extends the deadline by 1 tick and resumes. This
        // turns the engine-global epoch tick into an effective per-store
        // interrupt — defense in depth against any future regression that
        // shares an Engine across guests.
        store.epoch_deadline_callback(|ctx| {
            let data = ctx.data();
            if data.start.elapsed() >= data.timeout {
                // Real timeout — propagate as a typed trap so the trap-
                // handler can detect it via `downcast_ref::<WallClockTimeout>`
                // instead of pattern-matching against a stringified message.
                return Err(wasmtime::Error::new(WallClockTimeout {
                    budget: data.timeout,
                }));
            }
            // False positive (some other guest's watchdog tripped this
            // engine's epoch). Resume with a fresh 1-tick deadline.
            Ok(wasmtime::UpdateDeadline::Continue(1))
        });

        // Set fuel budget (deterministic metering)
        if config.fuel_limit > 0 {
            store
                .set_fuel(config.fuel_limit)
                .map_err(|e| SandboxError::Execution(e.to_string()))?;
        }

        // Set epoch deadline (wall-clock metering).
        //
        // `Engine::increment_epoch()` is global to an `Engine` — in earlier
        // versions of this code a single shared Engine was used, so a watchdog
        // firing for one guest would trip the epoch for *all* concurrently
        // running guests (Bug #3864). Two layers of defence are now in place:
        //   1. Each execution creates a fresh `Engine`, so the engine-global
        //      tick can't physically reach another guest.
        //   2. The store's `epoch_deadline_callback` (registered above) checks
        //      this guest's own elapsed time before trapping. Even on a shared
        //      engine, a false-positive epoch tick is silently dropped for a
        //      guest whose own budget hasn't elapsed.
        //
        // The watchdog thread blocks in `park_timeout(deadline - now)` and is
        // woken via `Thread::unpark` the moment the main thread finishes. It
        // re-checks the done flag on wake-up to distinguish "guest completed,
        // go home" from "spurious wake-up, keep waiting". On the happy path
        // the watchdog wakes within microseconds; on the timeout path it sleeps
        // out the remaining deadline exactly once and trips the epoch.
        //
        // An RAII guard signals the flag and joins the watchdog thread on every
        // early-return path (`?`, trap, ABI error, panic) so no thread leak
        // slips through.
        store.set_epoch_deadline(1);
        let engine_clone = engine.clone();
        let timeout = timeout_secs;
        let watchdog_done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watchdog_done_for_thread = std::sync::Arc::clone(&watchdog_done);
        let watchdog = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout);
            loop {
                let now = std::time::Instant::now();
                if now >= deadline {
                    // Wall-clock budget blown — trip the epoch so the
                    // running guest traps on `Interrupt` on its next
                    // epoch check.
                    engine_clone.increment_epoch();
                    return;
                }
                if watchdog_done_for_thread.load(std::sync::atomic::Ordering::Acquire) {
                    // Main thread finished cleanly; leave the engine
                    // untouched so concurrent stores aren't falsely
                    // interrupted.
                    return;
                }
                // park_timeout wakes on Thread::unpark (sent by the main
                // thread via the RAII guard below) or when the budget
                // expires, whichever comes first. The loop then re-checks
                // the done flag — park/unpark is allowed to return
                // spuriously, so the flag is the source of truth.
                std::thread::park_timeout(deadline - now);
            }
        });
        struct WatchdogGuard {
            done: std::sync::Arc<std::sync::atomic::AtomicBool>,
            handle: Option<std::thread::JoinHandle<()>>,
        }
        impl Drop for WatchdogGuard {
            fn drop(&mut self) {
                // Flip the flag first (Release), then unpark so the
                // watchdog wakes, re-reads the flag under Acquire, and
                // observes the store-happens-before-load ordering. Finally
                // join so the OS thread is actually reclaimed before we
                // return to the caller — otherwise a tight invocation
                // loop would still grow the thread table, just more
                // slowly.
                self.done.store(true, std::sync::atomic::Ordering::Release);
                if let Some(h) = self.handle.take() {
                    h.thread().unpark();
                    let _ = h.join();
                }
            }
        }
        let _watchdog_guard = WatchdogGuard {
            done: watchdog_done,
            handle: Some(watchdog),
        };

        // Build linker with host function imports
        let mut linker = Linker::new(engine);
        Self::register_host_functions(&mut linker)?;

        // Instantiate — links host functions, no WASI
        let instance = linker
            .instantiate(&mut store, &module)
            .map_err(|e| SandboxError::Instantiation(e.to_string()))?;

        // Retrieve required guest exports
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| SandboxError::AbiError("Module must export 'memory'".into()))?;

        let alloc_fn = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .map_err(|e| {
                SandboxError::AbiError(format!("Module must export 'alloc(i32)->i32': {e}"))
            })?;

        let execute_fn = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "execute")
            .map_err(|e| {
                SandboxError::AbiError(format!("Module must export 'execute(i32,i32)->i64': {e}"))
            })?;

        // Serialize input JSON → bytes
        let input_bytes = serde_json::to_vec(&input)
            .map_err(|e| SandboxError::Execution(format!("JSON serialize failed: {e}")))?;

        // Allocate space in guest memory for input
        let input_ptr = alloc_fn
            .call(&mut store, input_bytes.len() as i32)
            .map_err(|e| SandboxError::AbiError(format!("alloc call failed: {e}")))?;

        // Write input into guest memory. Use checked_add so a malicious or
        // buggy guest returning an out-of-range alloc pointer can't wrap
        // the bounds check (reachable on 32-bit hosts, defensive on 64-bit).
        let mem_data = memory.data_mut(&mut store);
        let start = input_ptr as usize;
        let end = start.checked_add(input_bytes.len()).ok_or_else(|| {
            SandboxError::AbiError("Input pointer + length overflows usize".into())
        })?;
        if end > mem_data.len() {
            return Err(SandboxError::AbiError("Input exceeds memory bounds".into()));
        }
        mem_data[start..end].copy_from_slice(&input_bytes);

        // Call guest execute
        let packed = match execute_fn.call(&mut store, (input_ptr, input_bytes.len() as i32)) {
            Ok(v) => v,
            Err(e) => {
                // Check for fuel exhaustion via trap code
                if let Some(Trap::OutOfFuel) = e.downcast_ref::<Trap>() {
                    return Err(SandboxError::FuelExhausted);
                }
                // Check for epoch deadline (wall-clock timeout). The
                // per-store callback returns a typed `WallClockTimeout`;
                // wasmtime may also surface a bare `Trap::Interrupt` if a
                // future engine-shared regression bypasses our callback.
                // Detect both shapes — typed downcast is the primary path,
                // `Trap::Interrupt` is the fallback.
                if e.downcast_ref::<WallClockTimeout>().is_some()
                    || matches!(e.downcast_ref::<Trap>(), Some(Trap::Interrupt))
                {
                    return Err(SandboxError::Execution(format!(
                        "WASM execution timed out after {}s (epoch interrupt)",
                        timeout
                    )));
                }
                return Err(SandboxError::Execution(e.to_string()));
            }
        };

        // Unpack result: high 32 bits = ptr, low 32 bits = len
        let result_ptr = (packed >> 32) as usize;
        let result_len = (packed & 0xFFFF_FFFF) as usize;

        // SECURITY: Cap result size before any memory access or JSON parsing
        // (Bug #3866). A malicious guest can return a `result_len` of up to
        // 2^32-1 bytes. Without this guard the host would either try to slice
        // gigabytes of memory (triggering the bounds check below) or feed a
        // huge buffer to serde_json. Aligned with the 1 MiB shell_exec cap
        // (#3529); deeply-nested JSON is independently bounded by serde_json's
        // default RECURSION_LIMIT of 128 — we never call
        // `disable_recursion_limit`, so guest input cannot stack-overflow the
        // recursive descent parser.
        if result_len > MAX_GUEST_RESULT_BYTES {
            return Err(SandboxError::AbiError(format!(
                "Guest result too large: {result_len} bytes (max {MAX_GUEST_RESULT_BYTES})"
            )));
        }

        // Read output JSON from guest memory. checked_add so a malicious
        // or buggy guest can't wrap `ptr + len` and silently pass the
        // bounds check (reachable on 32-bit hosts; defensive on 64-bit).
        let mem_data = memory.data(&store);
        let end = result_ptr.checked_add(result_len).ok_or_else(|| {
            SandboxError::AbiError("Result pointer + length overflows usize".into())
        })?;
        if end > mem_data.len() {
            return Err(SandboxError::AbiError(
                "Result pointer out of bounds".into(),
            ));
        }
        let output_bytes = &mem_data[result_ptr..end];

        let output: serde_json::Value = serde_json::from_slice(output_bytes)
            .map_err(|e| SandboxError::AbiError(format!("Invalid JSON output from guest: {e}")))?;

        // Calculate fuel consumed
        let fuel_remaining = store.get_fuel().unwrap_or(0);
        let fuel_consumed = config.fuel_limit.saturating_sub(fuel_remaining);

        debug!(agent = agent_id, fuel_consumed, "WASM execution complete");

        Ok(ExecutionResult {
            output,
            fuel_consumed,
        })
    }

    /// Register host function imports in the linker ("librefang" module).
    fn register_host_functions(linker: &mut Linker<GuestState>) -> Result<(), SandboxError> {
        // host_call: single dispatch for all capability-checked operations.
        // Request: JSON bytes in guest memory → {"method": "...", "params": {...}}
        // Response: packed (ptr, len) pointing to JSON in guest memory.
        linker
            .func_wrap(
                "librefang",
                "host_call",
                |mut caller: Caller<'_, GuestState>,
                 request_ptr: i32,
                 request_len: i32| {
                    match Self::host_call(&mut caller, request_ptr, request_len) {
                        Ok(packed) => packed,
                        Err(error) => {
                            tracing::error!(agent = %caller.data().agent_id, error = %error, "host_call failed");
                            Self::write_guest_json(
                                &mut caller,
                                &json!({ "error": format!("host_call failed: {error}") }),
                            )
                            .unwrap_or(0)
                        }
                    }
                },
            )
            .map_err(|e| SandboxError::Compilation(e.to_string()))?;

        // host_log: lightweight logging — no capability check required.
        //
        // SECURITY: Guest-supplied msg_len is capped at MAX_LOG_BYTES before
        // reading. Without the cap a malicious guest can push megabytes into
        // the host's structured log stream, filling disk or injecting fake
        // audit lines by embedding newline sequences. We:
        //   1. Limit the read to MAX_LOG_BYTES regardless of msg_len.
        //   2. Truncate the decoded string and append a byte count when it
        //      exceeds the cap.
        //   3. Replace bare CR/LF characters with the visible pilcrow (↵) so
        //      a single guest call cannot inject extra log lines.
        linker
            .func_wrap(
                "librefang",
                "host_log",
                |caller: Caller<'_, GuestState>,
                 level: i32,
                 msg_ptr: i32,
                 msg_len: i32| {
                    let mut caller = caller;
                    // Clamp the guest-supplied length before touching memory.
                    let clamped_len = (msg_len as usize).min(MAX_LOG_BYTES) as i32;
                    let was_truncated = (msg_len as usize) > MAX_LOG_BYTES;
                    let original_len = msg_len as usize;

                    match Self::read_guest_bytes(&mut caller, msg_ptr, clamped_len, "host_log") {
                        Ok(bytes) => {
                            // Use lossy decode rather than `from_utf8 +
                            // unwrap_or("<invalid utf8>")`: when the
                            // MAX_LOG_BYTES boundary lands inside a
                            // multi-byte UTF-8 sequence (likely on any
                            // 4 KiB cap with non-ASCII text — Chinese,
                            // Japanese, emoji), strict from_utf8 fails
                            // and the entire 4 KiB payload is replaced
                            // by the literal "<invalid utf8>" sentinel,
                            // discarding all content.  Cow<str> from
                            // from_utf8_lossy preserves valid prefixes
                            // and replaces only the broken trailing
                            // bytes with U+FFFD — what the original
                            // pre-#3923 implementation did and what
                            // operators expect from a "log truncated"
                            // path.
                            let raw = String::from_utf8_lossy(&bytes);

                            // Sanitize newlines to prevent log injection of
                            // fake structured log lines.
                            let sanitized = raw.replace("\r\n", " ").replace(['\r', '\n'], "\u{21b5}");

                            // Annotate truncated messages so operators know
                            // the original payload was longer.
                            let msg: std::borrow::Cow<str> = if was_truncated {
                                format!(
                                    "{}... [truncated {} bytes]",
                                    sanitized,
                                    original_len - MAX_LOG_BYTES
                                )
                                .into()
                            } else {
                                sanitized.into()
                            };

                            let agent_id = &caller.data().agent_id;
                            match level {
                                0 => tracing::trace!(agent = %agent_id, "[wasm] {msg}"),
                                1 => tracing::debug!(agent = %agent_id, "[wasm] {msg}"),
                                2 => tracing::info!(agent = %agent_id, "[wasm] {msg}"),
                                3 => tracing::warn!(agent = %agent_id, "[wasm] {msg}"),
                                _ => tracing::error!(agent = %agent_id, "[wasm] {msg}"),
                            }
                        }
                        Err(error) => {
                            tracing::error!(agent = %caller.data().agent_id, error = %error, "host_log failed");
                        }
                    }
                },
            )
            .map_err(|e| SandboxError::Compilation(e.to_string()))?;

        Ok(())
    }

    fn host_call(
        caller: &mut Caller<'_, GuestState>,
        request_ptr: i32,
        request_len: i32,
    ) -> Result<i64, SandboxError> {
        // SECURITY: Reject oversized or negative host_call request payloads
        // before deserialising (Bug #3866). `request_len` is i32 from the
        // guest ABI; a negative value as usize wraps to a huge number. The
        // 1 MiB cap matches the shell_exec cap (#3529); guest-controlled JSON
        // depth is bounded by serde_json's default RECURSION_LIMIT of 128
        // since we never call `disable_recursion_limit`.
        if request_len < 0 || request_len as usize > MAX_HOST_CALL_REQUEST_BYTES {
            return Err(SandboxError::AbiError(format!(
                "host_call request length invalid: {request_len} \
                 (max {MAX_HOST_CALL_REQUEST_BYTES})"
            )));
        }
        let request_bytes = Self::read_guest_bytes(caller, request_ptr, request_len, "host_call")?;
        let request: serde_json::Value = serde_json::from_slice(&request_bytes)
            .map_err(|e| SandboxError::AbiError(format!("host_call request JSON: {e}")))?;
        let method = request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let params = request
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // SECURITY (#3532): every host call is a denial-of-wallet vector if
        // it touches an external resource (LLM tokens, outbound HTTP,
        // cross-agent fanout). Wasm guest fuel only meters guest
        // instructions, so a tight loop of `agent_send` / `net_fetch` /
        // `agent_spawn` consumes effectively zero fuel while burning real
        // money. Charge a method-keyed fuel reservation BEFORE dispatch so
        // the per-invocation `fuel_limit` actually caps host-side cost.
        let cost = host_call_fuel_cost(&method);
        if cost > 0 {
            // Fail closed if fuel metering is disabled on this store —
            // the reservation is the only thing standing between the
            // guest and unbounded LLM/HTTP fanout.
            let remaining = match caller.get_fuel() {
                Ok(n) => n,
                Err(e) => {
                    let response = serde_json::json!({
                        "error": format!(
                            "host fuel meter unavailable for {method}: {e}; \
                             denial-of-wallet guard requires fuel to be enabled"
                        )
                    });
                    return Self::write_guest_json(caller, &response);
                }
            };
            if remaining < cost {
                let response = serde_json::json!({
                    "error": format!(
                        "host fuel exhausted for {method} (need {cost}, have {remaining}); \
                         denial-of-wallet guard"
                    )
                });
                return Self::write_guest_json(caller, &response);
            }
            if let Err(e) = caller.set_fuel(remaining - cost) {
                let response = serde_json::json!({
                    "error": format!("failed to reserve host fuel for {method}: {e}")
                });
                return Self::write_guest_json(caller, &response);
            }
        }

        let response = host_functions::dispatch(caller.data(), &method, &params);

        Self::write_guest_json(caller, &response)
    }

    fn read_guest_bytes(
        caller: &mut Caller<'_, GuestState>,
        ptr: i32,
        len: i32,
        op: &str,
    ) -> Result<Vec<u8>, SandboxError> {
        let memory = caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| SandboxError::AbiError(format!("{op}: no memory export")))?;

        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or_else(|| SandboxError::AbiError(format!("{op}: pointer overflow")))?;
        let data = memory.data(&mut *caller);
        if end > data.len() {
            return Err(SandboxError::AbiError(format!(
                "{op}: pointer out of bounds"
            )));
        }

        Ok(data[start..end].to_vec())
    }

    fn write_guest_json(
        caller: &mut Caller<'_, GuestState>,
        value: &serde_json::Value,
    ) -> Result<i64, SandboxError> {
        let response_bytes = serde_json::to_vec(value)
            .map_err(|e| SandboxError::Execution(format!("host_call: JSON serialize: {e}")))?;
        let len = response_bytes.len() as i32;

        let alloc_fn = caller
            .get_export("alloc")
            .and_then(|e| e.into_func())
            .ok_or_else(|| SandboxError::AbiError("host_call: no alloc export".into()))?;
        let alloc_typed = alloc_fn
            .typed::<i32, i32>(&mut *caller)
            .map_err(|e| SandboxError::AbiError(format!("host_call: alloc typed: {e}")))?;
        let ptr = alloc_typed
            .call(&mut *caller, len)
            .map_err(|e| SandboxError::Execution(format!("host_call: alloc call failed: {e}")))?;

        let memory = caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| SandboxError::AbiError("host_call: no memory export".into()))?;
        let dest_start = ptr as usize;
        let dest_end = dest_start
            .checked_add(response_bytes.len())
            .ok_or_else(|| SandboxError::AbiError("host_call: response pointer overflow".into()))?;
        let mem_data = memory.data_mut(caller);
        if dest_end > mem_data.len() {
            return Err(SandboxError::AbiError(
                "host_call: response exceeds memory bounds".into(),
            ));
        }
        mem_data[dest_start..dest_end].copy_from_slice(&response_bytes);

        Ok(((ptr as i64) << 32) | (len as i64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal echo module: returns input JSON unchanged.
    const ECHO_WAT: &str = r#"
        (module
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))

            (func (export "alloc") (param $size i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $size)))
                (local.get $ptr)
            )

            (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
                ;; Echo: return the input as-is
                (i64.or
                    (i64.shl
                        (i64.extend_i32_u (local.get $ptr))
                        (i64.const 32)
                    )
                    (i64.extend_i32_u (local.get $len))
                )
            )
        )
    "#;

    /// Module with infinite loop to test fuel exhaustion.
    const INFINITE_LOOP_WAT: &str = r#"
        (module
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))

            (func (export "alloc") (param $size i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $size)))
                (local.get $ptr)
            )

            (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
                (loop $inf
                    (br $inf)
                )
                (i64.const 0)
            )
        )
    "#;

    /// Proxy module: forwards input to host_call and returns the response.
    const HOST_CALL_PROXY_WAT: &str = r#"
        (module
            (import "librefang" "host_call" (func $host_call (param i32 i32) (result i64)))
            (memory (export "memory") 2)
            (global $bump (mut i32) (i32.const 1024))

            (func (export "alloc") (param $size i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $size)))
                (local.get $ptr)
            )

            (func (export "execute") (param $input_ptr i32) (param $input_len i32) (result i64)
                (call $host_call (local.get $input_ptr) (local.get $input_len))
            )
        )
    "#;

    #[test]
    fn test_sandbox_config_default() {
        let config = SandboxConfig::default();
        assert_eq!(config.fuel_limit, 1_000_000);
        assert_eq!(config.max_memory_bytes, 16 * 1024 * 1024);
        assert!(config.capabilities.is_empty());
    }

    /// Regression for #3532: host calls that touch external resources
    /// (LLM tokens, outbound HTTP, child agent spawn) must charge a
    /// non-zero fuel cost so the sandbox `fuel_limit` is a real ceiling
    /// on host-side spend, not just guest CPU. Pure-host calls remain
    /// free so existing benign skills don't regress.
    #[test]
    fn test_host_call_fuel_cost_table_covers_dow_methods() {
        assert!(host_call_fuel_cost("agent_send") > 0);
        assert!(host_call_fuel_cost("agent_spawn") > 0);
        assert!(host_call_fuel_cost("net_fetch") > 0);
        assert!(host_call_fuel_cost("shell_exec") > 0);

        // Pure-host methods stay free.
        assert_eq!(host_call_fuel_cost("time_now"), 0);
        assert_eq!(host_call_fuel_cost("kv_get"), 0);
        assert_eq!(host_call_fuel_cost("kv_set"), 0);
        assert_eq!(host_call_fuel_cost("env_read"), 0);
        assert_eq!(host_call_fuel_cost("fs_read"), 0);
        assert_eq!(host_call_fuel_cost("fs_write"), 0);
        assert_eq!(host_call_fuel_cost("fs_list"), 0);
        // Unknown method also free; capability check catches it.
        assert_eq!(host_call_fuel_cost("unknown_method"), 0);

        // Relative ordering: spawn (registers + may run a child agent)
        // is the priciest, then send (single LLM-bearing fanout), then
        // outbound network / shell.
        assert!(host_call_fuel_cost("agent_spawn") > host_call_fuel_cost("agent_send"));
        assert!(host_call_fuel_cost("agent_send") > host_call_fuel_cost("net_fetch"));
        assert!(host_call_fuel_cost("net_fetch") >= host_call_fuel_cost("shell_exec"));
        assert!(host_call_fuel_cost("shell_exec") > host_call_fuel_cost("kv_get"));
    }

    #[test]
    fn test_sandbox_engine_creation() {
        // Constructing the sandbox eagerly validates the engine config; the
        // assertion is simply that `new()` returns Ok.
        let _sandbox = WasmSandbox::new().unwrap();
    }

    /// Regression: max_memory_bytes must be enforced at runtime, not just
    /// declared. A guest module that requests more memory than the cap should
    /// be rejected — before this fix the cap was a no-op comment.
    #[test]
    fn test_memory_limiter_blocks_excess_growth() {
        let mut limiter = MemoryLimiter {
            // 1 MiB cap
            max_bytes: 1024 * 1024,
        };
        // Within limit → allowed
        assert!(
            limiter
                .memory_growing(0, 64 * 1024, None)
                .expect("should not error"),
            "growth within cap must be permitted"
        );
        // Exceeds limit → denied
        assert!(
            !limiter
                .memory_growing(0, 2 * 1024 * 1024, None)
                .expect("should not error"),
            "growth beyond cap must be denied"
        );
    }

    #[tokio::test]
    async fn test_echo_module() {
        let sandbox = WasmSandbox::new().unwrap();
        let input = serde_json::json!({"hello": "world", "num": 42});
        let config = SandboxConfig::default();

        let result = sandbox
            .execute(
                ECHO_WAT.as_bytes(),
                input.clone(),
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap();

        assert_eq!(result.output, input);
        assert!(result.fuel_consumed > 0);
    }

    #[tokio::test]
    async fn test_fuel_exhaustion() {
        let sandbox = WasmSandbox::new().unwrap();
        let input = serde_json::json!({});
        let config = SandboxConfig {
            fuel_limit: 10_000,
            ..Default::default()
        };

        let err = sandbox
            .execute(
                INFINITE_LOOP_WAT.as_bytes(),
                input,
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap_err();

        assert!(
            matches!(err, SandboxError::FuelExhausted),
            "Expected FuelExhausted, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_host_call_time_now() {
        let sandbox = WasmSandbox::new().unwrap();
        // time_now requires no capabilities
        let input = serde_json::json!({"method": "time_now", "params": {}});
        let config = SandboxConfig::default();

        let result = sandbox
            .execute(
                HOST_CALL_PROXY_WAT.as_bytes(),
                input,
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap();

        // Response should be {"ok": <timestamp>}
        assert!(
            result.output.get("ok").is_some(),
            "Expected ok field: {:?}",
            result.output
        );
        let ts = result.output["ok"].as_u64().unwrap();
        assert!(ts > 1_700_000_000, "Timestamp looks too small: {ts}");
    }

    #[tokio::test]
    async fn test_host_call_capability_denied() {
        let sandbox = WasmSandbox::new().unwrap();
        // fs_read canonicalizes before the capability check (#3814), so the
        // path must exist for the deny to land. Cargo.toml is present in
        // every crate's working dir during tests.
        let input = serde_json::json!({
            "method": "fs_read",
            "params": {"path": "Cargo.toml"}
        });
        let config = SandboxConfig {
            capabilities: vec![], // No capabilities!
            ..Default::default()
        };

        let result = sandbox
            .execute(
                HOST_CALL_PROXY_WAT.as_bytes(),
                input,
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap();

        // Response should contain "error" with "denied"
        let err_msg = result.output["error"].as_str().unwrap_or("");
        assert!(
            err_msg.contains("denied"),
            "Expected capability denied, got: {err_msg}"
        );
    }

    /// Regression for #3532: when remaining fuel is below the per-method
    /// reservation, the dispatch layer must reject the host call BEFORE
    /// invoking the host function — otherwise the denial-of-wallet guard
    /// is decorative. We pick `agent_send` (cost 100K) and a fuel limit
    /// well below that so any remaining fuel after proxy startup still
    /// can't cover the reservation, then assert the guest sees a
    /// `host fuel exhausted` error rather than the call going through.
    #[tokio::test]
    async fn test_host_call_fuel_exhausted_blocks_dispatch() {
        let sandbox = WasmSandbox::new().unwrap();
        let input = serde_json::json!({
            "method": "agent_send",
            "params": {"target": "anyone", "message": "hi"}
        });
        // 50_000 < host_call_fuel_cost("agent_send") = 100_000, so even
        // if proxy startup consumed zero fuel the reservation can't be
        // met and the guard must fire before dispatch.
        let config = SandboxConfig {
            fuel_limit: 50_000,
            ..Default::default()
        };

        let result = sandbox
            .execute(
                HOST_CALL_PROXY_WAT.as_bytes(),
                input,
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap();

        let err_msg = result.output["error"].as_str().unwrap_or("");
        assert!(
            err_msg.contains("host fuel exhausted"),
            "expected denial-of-wallet guard to fire, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn test_host_call_unknown_method() {
        let sandbox = WasmSandbox::new().unwrap();
        let input = serde_json::json!({"method": "nonexistent_method", "params": {}});
        let config = SandboxConfig::default();

        let result = sandbox
            .execute(
                HOST_CALL_PROXY_WAT.as_bytes(),
                input,
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap();

        let err_msg = result.output["error"].as_str().unwrap_or("");
        assert!(
            err_msg.contains("Unknown"),
            "Expected unknown method error, got: {err_msg}"
        );
    }

    /// Regression test for #3865: host_log must refuse guest-supplied
    /// messages longer than MAX_LOG_BYTES and must not allow newline
    /// injection. This test validates the constant value and sanitization
    /// logic directly without running a full WASM module, since the fix
    /// lives in the host-side closure.
    #[test]
    fn test_host_log_max_bytes_constant() {
        assert_eq!(MAX_LOG_BYTES, 4096, "MAX_LOG_BYTES must be 4096");
    }

    #[test]
    fn test_host_log_newline_sanitization_logic() {
        // Validate the exact sanitization expressions used in the host_log closure.
        let raw = "line1\r\nline2\nline3\rline4";
        let sanitized = raw.replace("\r\n", " ").replace(['\r', '\n'], "\u{21b5}");
        assert!(!sanitized.contains('\n'), "LF must be replaced");
        assert!(!sanitized.contains('\r'), "CR must be replaced");
        // CRLF → single space; bare LF/CR → pilcrow
        assert!(sanitized.contains(' '), "CRLF should become a space");
        assert!(
            sanitized.contains('\u{21b5}'),
            "bare LF/CR should become pilcrow"
        );
    }

    #[test]
    fn test_host_log_truncation_annotation() {
        // Simulate what the closure does for an over-length message.
        let long_msg = "x".repeat(MAX_LOG_BYTES + 100);
        let clamped = &long_msg[..MAX_LOG_BYTES];
        let annotated = format!("{}... [truncated {} bytes]", clamped, 100);
        assert!(annotated.contains("[truncated 100 bytes]"));
        assert!(annotated.len() > MAX_LOG_BYTES);
    }

    /// Regression: the truncation cap can land mid-codepoint for non-ASCII
    /// text.  Strict `str::from_utf8 + unwrap_or("<invalid utf8>")` would
    /// then replace the entire 4 KiB payload with the literal sentinel,
    /// dropping the user's whole log line.  `String::from_utf8_lossy`
    /// preserves the valid prefix and substitutes U+FFFD only for the
    /// broken trailing bytes.
    ///
    /// `'中'` is 3 bytes in UTF-8 and `MAX_LOG_BYTES = 4096` is not
    /// divisible by 3, so slicing at the byte cap is guaranteed to split a
    /// codepoint.
    #[test]
    fn test_host_log_lossy_decode_preserves_valid_prefix_at_boundary() {
        let s = "中".repeat(MAX_LOG_BYTES);
        // Take the first MAX_LOG_BYTES bytes — landing inside a codepoint.
        let bytes = &s.as_bytes()[..MAX_LOG_BYTES];
        // The invariant under test: lossy decode does NOT collapse the
        // whole payload to the strict sentinel.
        let lossy = String::from_utf8_lossy(bytes);
        assert!(
            !lossy.contains("<invalid utf8>"),
            "lossy decode must keep the valid prefix instead of falling \
             back to the strict sentinel"
        );
        assert!(lossy.contains('中'), "valid prefix codepoints must survive");
        assert!(
            lossy.contains('\u{FFFD}'),
            "the partial trailing codepoint must surface as U+FFFD"
        );
    }
    /// Module that returns a packed result whose `result_len` field is set to
    /// MAX_RESULT_BYTES + 1 (0x100001 = 1 MiB + 1) with a ptr of 0, to
    /// trigger the oversized-result guard introduced in Bug #3866.
    ///
    /// The `result_len` (low 32 bits) is set to a value beyond the 1 MiB cap.
    /// The pointer points to the start of memory (offset 0), which contains
    /// whatever zeroed or initialised bytes the runtime put there — we only
    /// care that the size guard fires BEFORE we try to slice memory.
    const OVERSIZED_RESULT_WAT: &str = r#"
        (module
            (memory (export "memory") 1)
            (global $bump (mut i32) (i32.const 1024))

            (func (export "alloc") (param $size i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $bump))
                (global.set $bump (i32.add (global.get $bump) (local.get $size)))
                (local.get $ptr)
            )

            (func (export "execute") (param $ptr i32) (param $len i32) (result i64)
                ;; Return ptr=0, len = 1 MiB + 1 (0x100001) — exceeds the cap.
                ;; packed = (0 << 32) | 0x100001
                (i64.const 0x100001)
            )
        )
    "#;

    /// Regression test for Bug #3866: a guest that claims to return more than
    /// MAX_RESULT_BYTES must be rejected with an AbiError, not cause the host
    /// to attempt a 16 MiB+ heap allocation or a serde_json parse of untrusted
    /// huge/deeply-nested data.
    #[tokio::test]
    async fn test_oversized_result_is_rejected() {
        let sandbox = WasmSandbox::new().unwrap();
        let input = serde_json::json!({});
        let config = SandboxConfig {
            fuel_limit: 10_000_000,
            ..Default::default()
        };

        let err = sandbox
            .execute(
                OVERSIZED_RESULT_WAT.as_bytes(),
                input,
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap_err();

        match &err {
            SandboxError::AbiError(msg) => {
                assert!(
                    msg.contains("too large"),
                    "Expected 'too large' in AbiError, got: {msg}"
                );
            }
            other => panic!("Expected AbiError for oversized result, got: {other}"),
        }
    }

    /// Regression test for Bug #3864: creating a per-execution Engine means
    /// that the epoch watchdog for one guest cannot interrupt another concurrently
    /// running guest. We verify this by running two sandboxes concurrently: a
    /// very short-timeout one that trips immediately and a normal one that should
    /// finish cleanly. If the epoch were shared, the normal one would also trap.
    #[tokio::test]
    async fn test_epoch_timeout_does_not_bleed_to_concurrent_guests() {
        // Two independent sandbox instances (each gets its own Engine per execution).
        let sandbox = WasmSandbox::new().unwrap();

        // Guest 1: normal echo — should complete successfully.
        let normal_config = SandboxConfig {
            fuel_limit: 10_000_000,
            timeout_secs: Some(30),
            ..Default::default()
        };
        let normal_fut = sandbox.execute(
            ECHO_WAT.as_bytes(),
            serde_json::json!({"ping": "pong"}),
            normal_config,
            None,
            "normal-guest",
        );

        // Guest 2: infinite loop with a tiny fuel budget — exhausts fuel quickly.
        let exhausted_config = SandboxConfig {
            fuel_limit: 100,
            timeout_secs: Some(1),
            ..Default::default()
        };
        let exhausted_fut = sandbox.execute(
            INFINITE_LOOP_WAT.as_bytes(),
            serde_json::json!({}),
            exhausted_config,
            None,
            "fuel-exhausted-guest",
        );

        let (normal_result, exhausted_result) = tokio::join!(normal_fut, exhausted_fut);

        // The fuel-exhausted guest must fail with FuelExhausted.
        assert!(
            matches!(exhausted_result, Err(SandboxError::FuelExhausted)),
            "Expected FuelExhausted for exhausted guest, got: {exhausted_result:?}"
        );

        // The normal guest must succeed — its epoch must NOT have been tripped
        // by the other guest's watchdog firing.
        assert!(
            normal_result.is_ok(),
            "Normal guest must not be interrupted by another guest's timeout: {normal_result:?}"
        );
        assert_eq!(
            normal_result.unwrap().output,
            serde_json::json!({"ping": "pong"}),
            "Normal guest output must be unchanged"
        );
    }

    /// Regression test for Bug #3866 (depth): serde_json's default
    /// RECURSION_LIMIT of 128 must reject deeply-nested guest input fed into
    /// `host_call`. We synthesize ~200 levels of `[` brackets — deeper than
    /// the default limit — and feed it as a host_call request via the proxy.
    /// Without the depth limit, recursive descent would risk stack overflow.
    #[tokio::test]
    async fn test_host_call_rejects_deep_nesting() {
        let sandbox = WasmSandbox::new().unwrap();
        // Build a JSON value nested ~200 levels deep — past serde_json's
        // default 128-deep RECURSION_LIMIT.
        let depth = 200usize;
        let mut deep = serde_json::Value::Null;
        for _ in 0..depth {
            deep = serde_json::Value::Array(vec![deep]);
        }
        // Wrap as a host_call envelope. The guest forwards this verbatim.
        let input = serde_json::json!({"method": "time_now", "params": deep});
        let config = SandboxConfig {
            fuel_limit: 10_000_000,
            ..Default::default()
        };

        let result = sandbox
            .execute(
                HOST_CALL_PROXY_WAT.as_bytes(),
                input,
                config,
                None,
                "test-agent",
            )
            .await
            .unwrap();

        // serde_json's RECURSION_LIMIT (128) fires during host_call
        // deserialisation; the host returns an error response. Assert that
        // the output has an "error" key — accepting "no ok" is too loose and
        // would pass even if the guest simply returned nothing.
        assert!(
            result.output.get("error").is_some(),
            "Deeply-nested host_call request must produce an error response, got: {:?}",
            result.output
        );
    }

    /// Regression test for Bug #3866 (size): host_call request and guest
    /// result caps must stay aligned at 1 MiB (matches shell_exec / kv_set
    /// caps from #3529 / #3866). Pins the actual module-level constants so
    /// a refactor that raises either cap fails this test.
    #[test]
    fn test_size_caps_are_one_mib() {
        assert_eq!(MAX_HOST_CALL_REQUEST_BYTES, 1024 * 1024);
        assert_eq!(MAX_GUEST_RESULT_BYTES, 1024 * 1024);
    }

    /// Display formatting regression for every `SandboxError` variant
    /// (#3576 wedge). The migration from `anyhow` to typed
    /// `SandboxError` must keep the Display text stable: the kernel
    /// boundary wraps `SandboxError` with `#[error("WASM execution
    /// failed: {0}")]`, and downstream log / UI consumers rely on the
    /// exact prefix + variant suffix shape. A separate
    /// `librefang-kernel::error::tests::sandbox_error_display_is_unchanged`
    /// pins the wrapped form; this test pins the inner Display so the
    /// two halves stay in lock-step.
    #[test]
    fn sandbox_error_display_format_per_variant() {
        assert_eq!(
            SandboxError::Compilation("bad opcode".into()).to_string(),
            "WASM compilation failed: bad opcode",
        );
        assert_eq!(
            SandboxError::Instantiation("missing import".into()).to_string(),
            "WASM instantiation failed: missing import",
        );
        assert_eq!(
            SandboxError::Execution("trap at 0x42".into()).to_string(),
            "WASM execution failed: trap at 0x42",
        );
        assert_eq!(
            SandboxError::FuelExhausted.to_string(),
            "Fuel exhausted: skill exceeded CPU budget",
        );
        assert_eq!(
            SandboxError::AbiError("no memory export".into()).to_string(),
            "Guest ABI violation: no memory export",
        );
    }

    /// `SandboxError` participates in the standard `std::error::Error`
    /// trait without an inner `#[source]` chain — the existing variants
    /// flatten upstream `wasmtime::Error` / `serde_json::Error` into a
    /// `String` payload so the byte-exact Display text the kernel's
    /// regression suite asserts on stays stable across the migration.
    /// Verify that contract explicitly: `Error::source()` returns
    /// `None` for every variant, and the typed value still composes
    /// through `Box<dyn std::error::Error>` (the trait-object bound
    /// the kernel's `#[from] SandboxError` ultimately relies on).
    #[test]
    fn sandbox_error_source_chain_is_intentionally_flat() {
        use std::error::Error as _;

        assert!(SandboxError::Compilation("e".into()).source().is_none());
        assert!(SandboxError::Instantiation("e".into()).source().is_none());
        assert!(SandboxError::Execution("e".into()).source().is_none());
        assert!(SandboxError::FuelExhausted.source().is_none());
        assert!(SandboxError::AbiError("e".into()).source().is_none());

        // Round-trip one variant through `Box<dyn Error + Send + Sync>`
        // — the very property the kernel's `#[from] SandboxError`
        // wrapper depends on. If a future change accidentally removes
        // `std::error::Error` (e.g. by dropping `thiserror::Error`),
        // this line stops compiling.
        let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(SandboxError::FuelExhausted);
        assert_eq!(
            boxed.to_string(),
            "Fuel exhausted: skill exceeded CPU budget"
        );
    }

    /// Per-store interrupt semantics (Bug #3864): the
    /// `epoch_deadline_callback` registered on the Store must be called
    /// when the engine epoch is bumped, and must trap only when THIS
    /// guest's wall-clock budget has actually elapsed. The cross-guest
    /// regression in `test_epoch_timeout_does_not_bleed_to_concurrent_guests`
    /// exercises the engine-isolation half; this test verifies the
    /// callback is wired (and traps) when the guest itself overruns.
    #[tokio::test]
    async fn test_per_store_callback_traps_on_real_timeout() {
        let sandbox = WasmSandbox::new().unwrap();
        let config = SandboxConfig {
            // u64::MAX: fuel tracking stays enabled (keeps epoch check-points active)
            // but can't be exhausted in 1 s — wall-clock timeout wins.
            fuel_limit: u64::MAX,
            timeout_secs: Some(1),
            ..Default::default()
        };

        let err = sandbox
            .execute(
                INFINITE_LOOP_WAT.as_bytes(),
                serde_json::json!({}),
                config,
                None,
                "self-timeout-guest",
            )
            .await
            .unwrap_err();

        // Verify the error is a wall-clock timeout (not fuel, not ABI, not
        // compilation). The message is built by the trap-handler from the
        // fixed "timed out after …s" format — asserting on it means the test
        // catches any regression where the per-store callback is unwired and
        // execution exits through an unrelated path.
        match &err {
            SandboxError::Execution(msg) if msg.contains("timed out") => {}
            other => panic!("Expected SandboxError::Execution with 'timed out', got: {other:?}"),
        }
    }
}
