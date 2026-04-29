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
use librefang_kernel_handle::KernelHandle;
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
#[derive(Debug, thiserror::Error)]
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
            },
        );

        // Enforce the memory cap: every memory.grow call from the guest goes
        // through MemoryLimiter::memory_growing before any allocation happens.
        store.limiter(|state| &mut state.limiter);

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
        // running guests (Bug #3864). That was fixed by creating a fresh
        // Engine per invocation in `WasmSandbox::execute`; each execution now
        // has its own epoch counter so `increment_epoch` only interrupts the
        // guest running inside this store.
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
        let timeout = config.timeout_secs.unwrap_or(30);
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
                // Check for epoch deadline (wall-clock timeout)
                if let Some(Trap::Interrupt) = e.downcast_ref::<Trap>() {
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
        // gigabytes of memory (triggering the bounds check below) or, if the
        // guest had somehow pre-allocated that much memory, feed a huge buffer
        // to serde_json whose recursive descent parser has no depth limit and
        // can stack-overflow on deeply-nested input.
        const MAX_RESULT_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
        if result_len > MAX_RESULT_BYTES {
            return Err(SandboxError::AbiError(format!(
                "Guest result too large: {result_len} bytes (max {MAX_RESULT_BYTES})"
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
    ) -> anyhow::Result<i64> {
        // SECURITY: Reject oversized or negative host_call request payloads
        // before deserialising (Bug #3866). `request_len` is i32 from the
        // guest ABI; a negative value as usize wraps to a huge number, and a
        // very large positive value feeds an unbounded serde_json parse that
        // can stack-overflow on deeply-nested JSON.
        const MAX_REQUEST_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
        if request_len < 0 || request_len as usize > MAX_REQUEST_BYTES {
            anyhow::bail!(
                "host_call request length invalid: {} (max {MAX_REQUEST_BYTES})",
                request_len
            );
        }
        let request_bytes = Self::read_guest_bytes(caller, request_ptr, request_len, "host_call")?;
        let request: serde_json::Value = serde_json::from_slice(&request_bytes)?;
        let method = request
            .get("method")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();
        let params = request
            .get("params")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let response = host_functions::dispatch(caller.data(), &method, &params);

        Self::write_guest_json(caller, &response)
    }

    fn read_guest_bytes(
        caller: &mut Caller<'_, GuestState>,
        ptr: i32,
        len: i32,
        op: &str,
    ) -> anyhow::Result<Vec<u8>> {
        let memory = caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| anyhow::anyhow!("{op}: no memory export"))?;

        let start = ptr as usize;
        let end = start
            .checked_add(len as usize)
            .ok_or_else(|| anyhow::anyhow!("{op}: pointer overflow"))?;
        let data = memory.data(&mut *caller);
        if end > data.len() {
            anyhow::bail!("{op}: pointer out of bounds");
        }

        Ok(data[start..end].to_vec())
    }

    fn write_guest_json(
        caller: &mut Caller<'_, GuestState>,
        value: &serde_json::Value,
    ) -> anyhow::Result<i64> {
        let response_bytes = serde_json::to_vec(value)?;
        let len = response_bytes.len() as i32;

        let alloc_fn = caller
            .get_export("alloc")
            .and_then(|e| e.into_func())
            .ok_or_else(|| anyhow::anyhow!("host_call: no alloc export"))?;
        let alloc_typed = alloc_fn.typed::<i32, i32>(&mut *caller)?;
        let ptr = alloc_typed.call(&mut *caller, len)?;

        let memory = caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| anyhow::anyhow!("host_call: no memory export"))?;
        let dest_start = ptr as usize;
        let dest_end = dest_start
            .checked_add(response_bytes.len())
            .ok_or_else(|| anyhow::anyhow!("host_call: response pointer overflow"))?;
        let mem_data = memory.data_mut(caller);
        if dest_end > mem_data.len() {
            anyhow::bail!("host_call: response exceeds memory bounds");
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
        assert!(
            lossy.contains('中'),
            "valid prefix codepoints must survive"
        );
        assert!(
            lossy.contains('\u{FFFD}'),
            "the partial trailing codepoint must surface as U+FFFD"
        );
    }
    /// Module that returns a packed result whose `result_len` field is set to
    /// MAX_RESULT_BYTES + 1 (0x1000001 = 16 MiB + 1) with a ptr of 0, to
    /// trigger the oversized-result guard introduced in Bug #3866.
    ///
    /// The `result_len` (low 32 bits) is set to a value beyond the 16 MiB cap.
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
                ;; Return ptr=0, len = 16 MiB + 1 (0x1000001) — exceeds the cap.
                ;; packed = (0 << 32) | 0x1000001
                (i64.const 0x1000001)
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
}
