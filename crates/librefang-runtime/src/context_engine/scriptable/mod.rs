//! Scriptable context engine: hook-driven implementation that runs plugin
//! scripts for `bootstrap`, `ingest`, `assemble`, `compact`, `after_turn`,
//! `prepare_subagent`, `merge_subagent`, and `truncate_tool_result`.
//!
//! Includes the hook telemetry surface (`HookTrace`, `HookStats`,
//! `HookMetrics`), the per-hook circuit breaker / rate limiter, and the
//! plugin-loading helpers that resolve plugin paths and parse manifests.

use super::*;

mod engine;

/// One recorded hook invocation — input, output, timing, and outcome.
///
/// Stored in a bounded ring buffer inside `ScriptableContextEngine` and
/// surfaced via `GET /api/context-engine/traces` for debugging.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HookTrace {
    /// Unique identifier for this hook invocation. 16 hex chars (8 random bytes).
    /// Stable across retries — generated once before the retry loop.
    pub trace_id: String,
    /// Shared ID for all hook calls within the same agent turn.
    /// Empty string when not available (e.g. bootstrap, which runs outside a turn).
    pub correlation_id: String,
    /// Hook name (`"ingest"`, `"assemble"`, …).
    pub hook: String,
    /// ISO-8601 timestamp of when the hook started.
    pub started_at: String,
    /// Wall-clock duration in milliseconds.
    pub elapsed_ms: u64,
    /// Whether the hook succeeded.
    pub success: bool,
    /// Error message, if the hook failed.
    pub error: Option<String>,
    /// JSON input sent to the hook script (may be truncated for large payloads).
    pub input_preview: serde_json::Value,
    /// JSON output returned by the hook script (None on failure).
    pub output_preview: Option<serde_json::Value>,
    /// Arbitrary metadata from the hook's `"annotations"` response field.
    /// Stored for observability — surfaced in trace history queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
}

/// Maximum number of traces kept in the ring buffer.
pub(super) const TRACE_BUFFER_CAPACITY: usize = 100;

// ---------------------------------------------------------------------------
// Hook invocation metrics
// ---------------------------------------------------------------------------

/// Per-hook invocation counters.  Stored inside `ScriptableContextEngine` behind
/// an `Arc<Mutex<…>>` so callers can read them without holding the engine lock.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct HookStats {
    /// Total invocations (includes failures).
    pub calls: u64,
    /// Successful invocations.
    pub successes: u64,
    /// Failed invocations (timeout, crash, bad JSON, …).
    pub failures: u64,
    /// Cumulative wall-clock time of all invocations in milliseconds.
    pub total_ms: u64,
}

/// Snapshot of all hook stats for a `ScriptableContextEngine`.
#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct HookMetrics {
    pub ingest: HookStats,
    pub after_turn: HookStats,
    pub bootstrap: HookStats,
    pub assemble: HookStats,
    pub compact: HookStats,
    pub prepare_subagent: HookStats,
    pub merge_subagent: HookStats,
}

// ---------------------------------------------------------------------------
// Circuit breaker
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct CircuitBreakerState {
    consecutive_failures: u32,
    /// When the circuit tripped (entered open state). `None` = closed.
    opened_at: Option<std::time::Instant>,
    /// Set to `true` when cooldown has elapsed and one probe call is allowed.
    half_open: bool,
}

impl CircuitBreakerState {
    fn new() -> Self {
        Self {
            consecutive_failures: 0,
            opened_at: None,
            half_open: false,
        }
    }

    /// Returns `true` when the hook should be skipped (circuit open + not half-open).
    fn is_open(&mut self, max_failures: u32, reset_secs: u64) -> bool {
        if self.consecutive_failures < max_failures {
            return false; // circuit closed
        }
        match self.opened_at {
            None => {
                // Restored from persistent storage without a timestamp (opened_at was NULL).
                // The failure count already meets the threshold, so latch the circuit now
                // so that the full cooldown period is enforced from this moment.
                self.opened_at = Some(std::time::Instant::now());
                true
            }
            Some(t) => {
                if t.elapsed().as_secs() >= reset_secs {
                    // Cooldown elapsed → allow one half-open probe
                    if !self.half_open {
                        self.half_open = true;
                        self.opened_at = None; // reset timer so next trip re-latches
                    }
                    false // allow the probe call through
                } else {
                    true // still in cooldown
                }
            }
        }
    }

    fn record_success(&mut self) {
        self.consecutive_failures = 0;
        self.opened_at = None;
        self.half_open = false;
    }

    fn record_failure(&mut self, max_failures: u32) {
        self.consecutive_failures += 1;
        self.half_open = false; // probe failed → close half-open window
                                // (Re-)latch the circuit when threshold is reached
        if self.consecutive_failures >= max_failures {
            self.opened_at = Some(std::time::Instant::now());
        }
    }
}

// ---------------------------------------------------------------------------
// Per-hook sliding-window rate limiter
// ---------------------------------------------------------------------------

/// Sliding-window call counter for one hook.
#[derive(Default)]
struct HookRateLimiter {
    /// Ring of timestamps (as `std::time::Instant`) for recent calls.
    calls: std::collections::VecDeque<std::time::Instant>,
}

impl HookRateLimiter {
    /// Record a call and return whether the call is allowed.
    ///
    /// Evicts entries older than 60 seconds, then checks the count against
    /// `max_per_minute`.  Returns `true` if the call may proceed, `false` if
    /// the rate limit is exceeded.
    fn check_and_record(&mut self, max_per_minute: u32) -> bool {
        if max_per_minute == 0 {
            return true; // unlimited
        }
        let now = std::time::Instant::now();
        let window = std::time::Duration::from_secs(60);
        // Evict calls older than the window.
        while self
            .calls
            .front()
            .is_some_and(|t| now.duration_since(*t) > window)
        {
            self.calls.pop_front();
        }
        if self.calls.len() >= max_per_minute as usize {
            return false; // rate limit exceeded
        }
        self.calls.push_back(now);
        true
    }
}

// ---------------------------------------------------------------------------
// Scriptable context engine — wraps DefaultContextEngine + Python script hooks
// ---------------------------------------------------------------------------

/// Context engine that delegates to a [`DefaultContextEngine`] for heavy
/// operations (assemble, compact) and optionally invokes scripts for
/// light lifecycle hooks (ingest, after_turn).
///
/// Hook scripts are language-agnostic — they speak JSON over stdin/stdout.
/// The `runtime` field on the hooks config picks the launcher (`python`
/// stays the default; `native`, `v`, `node`, `deno`, `go` are also
/// supported). See [`crate::plugin_runtime`] for the full protocol.
///
/// ```toml
/// [context_engine.hooks]
/// ingest = "~/.librefang/plugins/my_recall.py"
/// after_turn = "~/.librefang/plugins/my_indexer.py"
/// runtime = "python"  # or "v", "node", "go", "native", ...
/// ```
///
/// **ingest hook** receives:
/// ```json
/// {"type": "ingest", "agent_id": "...", "message": "..."}
/// ```
/// Returns:
/// ```json
/// {"type": "ingest_result", "memories": [{"content": "remembered fact"}]}
/// ```
///
/// **after_turn hook** receives:
/// ```json
/// {"type": "after_turn", "agent_id": "...", "messages": [...]}
/// ```
/// Returns:
/// ```json
/// {"type": "ok"}
/// ```
pub struct ScriptableContextEngine {
    inner: DefaultContextEngine,
    ingest_script: Option<String>,
    after_turn_script: Option<String>,
    bootstrap_script: Option<String>,
    assemble_script: Option<String>,
    compact_script: Option<String>,
    prepare_subagent_script: Option<String>,
    merge_subagent_script: Option<String>,
    runtime: crate::plugin_runtime::PluginRuntime,
    /// Per-invocation timeout for all hooks. Bootstrap uses 2× this.
    hook_timeout_secs: u64,
    /// Plugin-declared env vars (from `[env]` in plugin.toml), passed to every hook.
    plugin_env: Vec<(String, String)>,
    /// Live invocation counters. Shared so callers can snapshot without &mut self.
    metrics: std::sync::Arc<std::sync::Mutex<HookMetrics>>,
    /// What to do when a hook fails after all retries are exhausted.
    on_hook_failure: librefang_types::config::HookFailurePolicy,
    /// How many times to retry a failing hook before applying `on_hook_failure`.
    max_retries: u32,
    /// Milliseconds to wait between retries.
    retry_delay_ms: u64,
    /// Optional substring filter for the `ingest` hook.
    ingest_filter: Option<String>,
    /// Restrict hooks to specific agent ID substrings (empty = all agents).
    agent_id_filter: Vec<String>,
    /// Per-hook JSON Schema definitions for input/output validation.
    hook_schemas: std::collections::HashMap<String, librefang_types::config::HookSchema>,
    /// Bounded ring buffer of recent hook invocations for debugging.
    traces: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HookTrace>>>,
    /// Memory limit (MiB) forwarded to HookConfig.
    max_memory_mb: Option<u64>,
    /// Whether hook subprocesses are allowed network access.
    allow_network: bool,
    /// Hook protocol version declared by this plugin (stored for future compatibility checks).
    #[allow(dead_code)]
    hook_protocol_version: u32,
    /// Optional TTL-based cache for the `ingest` hook (seconds). `None` = disabled.
    ingest_cache_ttl_secs: Option<u64>,
    /// In-memory cache: maps SHA-256(input_json) → (cached_output, expires_at).
    ingest_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, (serde_json::Value, std::time::Instant)>,
        >,
    >,
    /// Whether to use persistent subprocesses (process pool) for hooks.
    persistent_subprocess: bool,
    /// Shared pool of persistent hook subprocesses (used when `persistent_subprocess = true`).
    process_pool: std::sync::Arc<crate::plugin_runtime::HookProcessPool>,
    /// TTL-based cache for `assemble` hook results.
    assemble_cache_ttl_secs: Option<u64>,
    assemble_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, (serde_json::Value, std::time::Instant)>,
        >,
    >,
    /// TTL-based cache for `compact` hook results.
    compact_cache_ttl_secs: Option<u64>,
    compact_cache: std::sync::Arc<
        std::sync::Mutex<
            std::collections::HashMap<String, (serde_json::Value, std::time::Instant)>,
        >,
    >,
    /// Compiled regex filter for the `ingest` hook (from `ingest_regex` config).
    ingest_regex: Option<regex_lite::Regex>,
    /// Path to the per-plugin shared state JSON file (when `enable_shared_state = true`).
    shared_state_path: Option<std::path::PathBuf>,
    /// Circuit breaker states per hook name.
    circuit_breakers:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, CircuitBreakerState>>>,
    /// Circuit breaker config (None = disabled).
    circuit_breaker_cfg: Option<librefang_types::config::CircuitBreakerConfig>,
    /// Semaphore bounding concurrent `after_turn` background tasks.
    after_turn_sem: std::sync::Arc<tokio::sync::Semaphore>,
    /// Whether to pre-warm subprocesses on engine init.
    prewarm_subprocesses: bool,
    /// Per-agent hook call counters: agent_id → HookStats.
    per_agent_metrics:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, HookStats>>>,
    /// OTel OTLP endpoint for this plugin (advisory; logged if set).
    #[allow(dead_code)]
    otel_endpoint: Option<String>,
    /// Canonical plugin name — used as the `plugin` column when writing to trace_store.
    plugin_name: String,
    /// Persistent SQLite trace store (None if it could not be opened at construction time).
    trace_store: Option<std::sync::Arc<crate::trace_store::TraceStore>>,
    /// Tracks all spawned after_turn background tasks for graceful shutdown.
    after_turn_tasks: std::sync::Arc<tokio::sync::Mutex<tokio::task::JoinSet<()>>>,
    /// Memory substrate for after_turn hook memory injection.
    memory_substrate: std::sync::Arc<librefang_memory::MemorySubstrate>,
    /// Overrides applied by the bootstrap hook at startup.
    bootstrap_applied_overrides: std::sync::Arc<std::sync::Mutex<BootstrapOverrides>>,
    /// Per-hook sliding-window rate limiters.
    rate_limiters:
        std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, HookRateLimiter>>>,
    /// Script to invoke when an event is received from the event bus.
    on_event_script: Option<String>,
    /// Optional shared event bus. When set, events emitted by this plugin's
    /// hooks are published to all subscribers.
    event_bus: Option<std::sync::Arc<PluginEventBus>>,
    /// Config schema declared in `[config]` of plugin.toml.
    ///
    /// Used to build the resolved config JSON file passed to hook subprocesses
    /// via `LIBREFANG_PLUGIN_CONFIG`.
    plugin_config_schema:
        std::collections::HashMap<String, librefang_types::config::PluginConfigField>,
}

impl ScriptableContextEngine {
    /// Create a scriptable context engine from config.
    ///
    /// Also validates that every declared hook script file actually exists.
    /// Missing scripts are logged as warnings at construction time (not fatal)
    /// so the engine degrades gracefully rather than refusing to start.
    pub fn new(
        inner: DefaultContextEngine,
        hooks: &librefang_types::config::ContextEngineHooks,
    ) -> Self {
        // Warn at construction time for any declared script that cannot be found.
        let all_declared: &[(&str, &Option<String>)] = &[
            ("ingest", &hooks.ingest),
            ("after_turn", &hooks.after_turn),
            ("bootstrap", &hooks.bootstrap),
            ("assemble", &hooks.assemble),
            ("compact", &hooks.compact),
            ("prepare_subagent", &hooks.prepare_subagent),
            ("merge_subagent", &hooks.merge_subagent),
            ("on_event", &hooks.on_event),
        ];
        for (name, path_opt) in all_declared {
            if let Some(path) = path_opt {
                let resolved = Self::resolve_script_path(path);
                if !std::path::Path::new(&resolved).exists() {
                    warn!(
                        hook = *name,
                        path = resolved.as_str(),
                        "Hook script declared in plugin.toml does not exist; \
                         hook will be skipped at runtime"
                    );
                }
            }
        }

        const CURRENT_PROTOCOL: u32 = 1;
        let proto = hooks.hook_protocol_version.unwrap_or(1);
        if proto > CURRENT_PROTOCOL {
            warn!(
                declared = proto,
                current = CURRENT_PROTOCOL,
                "Plugin declares hook_protocol_version {proto} but runtime only supports \
                 version {CURRENT_PROTOCOL}. The plugin may use unsupported features."
            );
        }

        let memory_substrate = std::sync::Arc::clone(inner.memory_substrate());
        Self {
            inner,
            ingest_script: hooks.ingest.clone(),
            after_turn_script: hooks.after_turn.clone(),
            bootstrap_script: hooks.bootstrap.clone(),
            assemble_script: hooks.assemble.clone(),
            compact_script: hooks.compact.clone(),
            prepare_subagent_script: hooks.prepare_subagent.clone(),
            merge_subagent_script: hooks.merge_subagent.clone(),
            runtime: crate::plugin_runtime::PluginRuntime::from_tag(hooks.runtime.as_deref()),
            hook_timeout_secs: hooks.hook_timeout_secs.unwrap_or(30),
            plugin_env: Vec::new(), // populated via with_plugin_env()
            metrics: std::sync::Arc::new(std::sync::Mutex::new(HookMetrics::default())),
            on_hook_failure: hooks.on_hook_failure.clone(),
            max_retries: hooks.max_retries,
            retry_delay_ms: hooks.retry_delay_ms,
            ingest_filter: hooks.ingest_filter.clone(),
            agent_id_filter: hooks.only_for_agent_ids.clone(),
            hook_schemas: hooks.hook_schemas.clone(),
            traces: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::VecDeque::with_capacity(TRACE_BUFFER_CAPACITY),
            )),
            max_memory_mb: hooks.max_memory_mb,
            allow_network: hooks.allow_network,
            hook_protocol_version: proto,
            ingest_cache_ttl_secs: hooks.hook_cache_ttl_secs,
            ingest_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            persistent_subprocess: hooks.persistent_subprocess,
            process_pool: std::sync::Arc::new(crate::plugin_runtime::HookProcessPool::new()),
            assemble_cache_ttl_secs: hooks.assemble_cache_ttl_secs,
            assemble_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            compact_cache_ttl_secs: hooks.compact_cache_ttl_secs,
            compact_cache: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            ingest_regex: hooks.ingest_regex.as_deref().and_then(
                |pat| match regex_lite::Regex::new(pat) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        warn!(pattern = pat, error = %e, "Invalid ingest_regex — ignored");
                        None
                    }
                },
            ),
            // When enable_shared_state is true, set a placeholder path; the
            // actual plugin-scoped path is filled in by `with_plugin_name()`.
            shared_state_path: if hooks.enable_shared_state {
                Some(std::path::PathBuf::from(".state.json"))
            } else {
                None
            },
            circuit_breakers: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            circuit_breaker_cfg: hooks.circuit_breaker.clone(),
            after_turn_sem: std::sync::Arc::new(tokio::sync::Semaphore::new(
                hooks.after_turn_queue_depth.max(1) as usize,
            )),
            prewarm_subprocesses: hooks.prewarm_subprocesses,
            per_agent_metrics: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            otel_endpoint: hooks.otel_endpoint.clone(),
            plugin_name: String::new(), // filled in by with_plugin_name()
            trace_store: None,          // filled in by with_plugin_name()
            after_turn_tasks: std::sync::Arc::new(tokio::sync::Mutex::new(
                tokio::task::JoinSet::new(),
            )),
            memory_substrate,
            bootstrap_applied_overrides: std::sync::Arc::new(std::sync::Mutex::new(
                BootstrapOverrides::default(),
            )),
            rate_limiters: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            on_event_script: hooks.on_event.clone(),
            event_bus: None,
            plugin_config_schema: std::collections::HashMap::new(), // populated via with_plugin_config()
        }
    }

    /// Set the plugin name to resolve the per-plugin shared state file path.
    ///
    /// Call after `new()` when the plugin name is known. If `enable_shared_state`
    /// was `false`, `shared_state_path` is `None` and this is a no-op.
    pub fn with_plugin_name(mut self, name: &str) -> Self {
        self.plugin_name = name.to_string();

        if self.shared_state_path.is_some() {
            // Replace the placeholder with the actual plugin-scoped path.
            self.shared_state_path = Some(
                crate::plugin_manager::plugins_dir()
                    .join(name)
                    .join(".state.json"),
            );
        }

        // Open the persistent trace store. Failure is non-fatal — traces will
        // still land in the in-memory ring buffer even if SQLite is unavailable.
        self.trace_store = crate::plugin_manager::open_trace_store()
            .map(std::sync::Arc::new)
            .map_err(|e| {
                warn!(plugin = name, error = %e, "Could not open hook trace store; SQLite persistence disabled");
            })
            .ok();

        // Restore circuit breaker state from SQLite so tripped circuits survive daemon restarts.
        if let Some(ref store) = self.trace_store {
            if let Ok(saved) = store.load_circuit_states() {
                if let Ok(mut guard) = self.circuit_breakers.lock() {
                    for (key, (failures, opened_at)) in saved {
                        guard.entry(key).or_insert_with(|| {
                            let opened_instant = opened_at.as_deref().and_then(|s| {
                                chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| {
                                    // Convert persisted UTC timestamp to a std::time::Instant
                                    // approximation: compute how many seconds ago it opened.
                                    let elapsed_secs = chrono::Utc::now()
                                        .signed_duration_since(dt.with_timezone(&chrono::Utc))
                                        .num_seconds()
                                        .max(0)
                                        as u64;
                                    std::time::Instant::now()
                                        .checked_sub(std::time::Duration::from_secs(elapsed_secs))
                                        .unwrap_or_else(std::time::Instant::now)
                                })
                            });
                            CircuitBreakerState {
                                consecutive_failures: failures,
                                opened_at: opened_instant,
                                half_open: false,
                            }
                        });
                    }
                }
            }
        }

        self
    }

    /// Set plugin-level env vars from `[env]` in plugin.toml.
    pub fn with_plugin_env(mut self, env: Vec<(String, String)>) -> Self {
        self.plugin_env = env;
        self
    }

    /// Set the config schema declared in `[config]` of plugin.toml.
    ///
    /// Before each hook invocation the resolved config (defaults only for now)
    /// is written to a temporary JSON file and the path is exposed to the
    /// subprocess as `LIBREFANG_PLUGIN_CONFIG`.
    pub fn with_plugin_config(
        mut self,
        schema: std::collections::HashMap<String, librefang_types::config::PluginConfigField>,
    ) -> Self {
        self.plugin_config_schema = schema;
        self
    }

    /// Write the resolved plugin config (defaults merged with user overrides) to a
    /// temporary JSON file.
    ///
    /// Returns the file path, or `None` if the schema is empty.
    ///
    /// The file is written to the system temp directory as
    /// `librefang-plugin-config-<plugin_name>.json` and is overwritten on each
    /// hook invocation (no temp dir cleanup needed — the OS handles it).
    fn write_plugin_config_file(
        plugin_name: &str,
        config_schema: &std::collections::HashMap<
            String,
            librefang_types::config::PluginConfigField,
        >,
        user_overrides: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Option<std::path::PathBuf> {
        if config_schema.is_empty() {
            return None;
        }
        let mut resolved: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        // Start with defaults from the schema.
        for (key, field) in config_schema {
            if let Some(ref default_val) = field.default {
                resolved.insert(key.clone(), default_val.clone());
            }
        }
        // Apply user overrides (only for keys declared in the schema).
        for (key, val) in user_overrides {
            if config_schema.contains_key(key.as_str()) {
                resolved.insert(key.clone(), val.clone());
            }
        }
        let json = serde_json::to_string_pretty(&resolved).ok()?;
        let path = std::env::temp_dir().join(format!("librefang-plugin-config-{plugin_name}.json"));
        std::fs::write(&path, json).ok()?;
        Some(path)
    }

    /// Attach a shared event bus to this engine.
    ///
    /// Attach an event bus so this engine both emits events (from `after_turn` output)
    /// and receives events for its `on_event` hook.
    ///
    /// Starts a background subscription task on the bus: when any plugin on the same
    /// bus emits an event, this engine's `on_event` script (if configured) is invoked.
    pub fn with_event_bus(mut self, bus: std::sync::Arc<PluginEventBus>) -> Self {
        self.event_bus = Some(bus.clone());

        // Start listener only when there is an on_event script to invoke.
        if self.on_event_script.is_some() {
            // Build a lightweight clone of the fields needed inside the task.
            // Using Arc clones keeps it cheap; the spawned task holds them for its lifetime.
            let plugin_name = self.plugin_name.clone();
            let on_event_script = self.on_event_script.clone().unwrap();
            let runtime = self.runtime.clone();
            let hook_timeout_secs = self.hook_timeout_secs;
            let plugin_env = self.plugin_env.clone();
            let bootstrap_overrides = self.bootstrap_applied_overrides.clone();
            let traces = self.traces.clone();
            let hook_schemas = self.hook_schemas.clone();
            let shared_state_path = self.shared_state_path.clone();
            let trace_store = self.trace_store.clone();
            let max_memory_mb = self.max_memory_mb;
            let allow_network = self.allow_network;
            let output_schema_strict = self.inner.config.output_schema_strict;

            let mut rx = bus.subscribe();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(event) => {
                            // Skip events emitted by this same plugin to avoid infinite loops.
                            if event.source_plugin == plugin_name {
                                continue;
                            }

                            let effective_env = {
                                let guard = bootstrap_overrides
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner());
                                let mut env = plugin_env.clone();
                                for (k, v) in &guard.env_overrides {
                                    if !env.iter().any(|(ek, _)| ek == k) {
                                        env.push((k.clone(), v.clone()));
                                    }
                                }
                                env
                            };
                            let effective_allow_network = {
                                let guard = bootstrap_overrides
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner());
                                guard.allow_network.unwrap_or(allow_network)
                            };
                            let input = serde_json::json!({"event": event});
                            let plugin_name_c = plugin_name.clone();
                            let script = on_event_script.clone();
                            let traces_c = traces.clone();
                            let schemas_c = hook_schemas.clone();
                            let state_c = shared_state_path.clone();
                            let store_c = trace_store.clone();
                            let runtime = runtime.clone();
                            tokio::spawn(async move {
                                let _ = ScriptableContextEngine::run_hook(
                                    "on_event",
                                    &script,
                                    runtime,
                                    input,
                                    hook_timeout_secs,
                                    &effective_env,
                                    0, // on_event is best-effort, no retries
                                    0,
                                    max_memory_mb,
                                    effective_allow_network,
                                    &traces_c,
                                    &schemas_c,
                                    state_c.as_deref(),
                                    store_c.as_ref(),
                                    &plugin_name_c,
                                    &generate_trace_id(),
                                    output_schema_strict,
                                )
                                .await;
                            });
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            // Route through the bus's lag counter so plugin
                            // on_event misses are observable in
                            // `dropped_count()` and emit a rate-limited
                            // error! log (#3630). The previous per-listener
                            // warn! was silent in aggregate metrics.
                            bus.record_consumer_lag(n, "plugin.on_event");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        self
    }

    /// Return a snapshot of all hook invocation metrics.
    pub fn metrics(&self) -> HookMetrics {
        self.metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Return recent hook invocation traces (up to `TRACE_BUFFER_CAPACITY`).
    pub fn traces_snapshot(&self) -> Vec<HookTrace> {
        self.traces
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .cloned()
            .collect()
    }

    /// Push a trace record into the in-memory ring buffer and the SQLite store.
    ///
    /// The ring buffer provides fast in-process access; the SQLite store persists
    /// traces across daemon restarts for post-mortem analysis.  Both writes are
    /// best-effort — errors are silently swallowed so a telemetry failure never
    /// propagates to the caller.
    fn push_trace(
        traces: &std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HookTrace>>>,
        trace: HookTrace,
        trace_store: Option<&std::sync::Arc<crate::trace_store::TraceStore>>,
        plugin_name: &str,
    ) {
        // Persist to SQLite first (borrows trace by ref).
        if let Some(store) = trace_store {
            store.insert(plugin_name, &trace);
        }
        // Then push into the bounded in-memory ring buffer.
        if let Ok(mut buf) = traces.lock() {
            if buf.len() >= TRACE_BUFFER_CAPACITY {
                buf.pop_front();
            }
            buf.push_back(trace);
        }
    }

    /// Validate a JSON value against a subset of JSON Schema.
    ///
    /// Checks:
    /// - `required`: all listed keys are present (objects only)
    /// - `type`: value matches the declared JSON type
    /// - `enum`: value is one of the listed options
    /// - `minimum` / `maximum`: numeric range (numbers only)
    /// - `minLength` / `maxLength`: string length (strings only)
    /// - `properties`: recursively validate each declared property
    ///
    /// Returns a list of human-readable violation messages (empty = valid).
    /// The caller decides whether to warn or error based on `output_schema_strict`.
    fn validate_schema(
        schema: &serde_json::Value,
        value: &serde_json::Value,
        context: &str,
    ) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();

        // --- type check ---
        if let Some(expected_type) = schema.get("type").and_then(|t| t.as_str()) {
            let actual_matches = match expected_type {
                "object" => value.is_object(),
                "array" => value.is_array(),
                "string" => value.is_string(),
                "number" => value.is_number(),
                "integer" => value.is_i64() || value.is_u64(),
                "boolean" => value.is_boolean(),
                "null" => value.is_null(),
                _ => true, // unknown type — don't reject
            };
            if !actual_matches {
                errors.push(format!(
                    "[{context}] type mismatch: expected={expected_type}, actual={}",
                    value.to_string().chars().take(80).collect::<String>()
                ));
            }
        }

        // --- enum check ---
        if let Some(variants) = schema.get("enum").and_then(|e| e.as_array()) {
            if !variants.contains(value) {
                errors.push(format!(
                    "[{context}] value not in enum: {}",
                    value.to_string().chars().take(80).collect::<String>()
                ));
            }
        }

        // --- numeric range ---
        if let Some(n) = value.as_f64() {
            if let Some(min) = schema.get("minimum").and_then(|v| v.as_f64()) {
                if n < min {
                    errors.push(format!(
                        "[{context}] below minimum: value={n}, minimum={min}"
                    ));
                }
            }
            if let Some(max) = schema.get("maximum").and_then(|v| v.as_f64()) {
                if n > max {
                    errors.push(format!(
                        "[{context}] above maximum: value={n}, maximum={max}"
                    ));
                }
            }
        }

        // --- string length ---
        if let Some(s) = value.as_str() {
            if let Some(min_len) = schema.get("minLength").and_then(|v| v.as_u64()) {
                if (s.len() as u64) < min_len {
                    errors.push(format!(
                        "[{context}] string too short: len={}, min_len={min_len}",
                        s.len()
                    ));
                }
            }
            if let Some(max_len) = schema.get("maxLength").and_then(|v| v.as_u64()) {
                if (s.len() as u64) > max_len {
                    errors.push(format!(
                        "[{context}] string too long: len={}, max_len={max_len}",
                        s.len()
                    ));
                }
            }
        }

        // --- required fields ---
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            if let Some(obj) = value.as_object() {
                for field in required {
                    if let Some(field_str) = field.as_str() {
                        if !obj.contains_key(field_str) {
                            errors.push(format!("[{context}] required field missing: {field_str}"));
                        }
                    }
                }
            }
        }

        // --- properties: recursive per-property validation ---
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            if let Some(obj) = value.as_object() {
                for (key, prop_schema) in props {
                    if let Some(prop_value) = obj.get(key) {
                        errors.extend(Self::validate_schema(
                            prop_schema,
                            prop_value,
                            &format!("{context}.{key}"),
                        ));
                    }
                }
            }
        }

        errors
    }

    fn circuit_is_open(&self, hook: &str, agent_id: Option<&AgentId>) -> bool {
        let Some(ref cfg) = self.circuit_breaker_cfg else {
            return false;
        };
        let key = match agent_id {
            Some(id) => format!("{}:{}", id.0, hook),
            None => hook.to_string(),
        };
        let mut guard = self.circuit_breakers.lock().unwrap();
        guard
            .entry(key)
            .or_insert_with(CircuitBreakerState::new)
            .is_open(cfg.max_failures, cfg.reset_secs)
    }

    fn circuit_record(&self, hook: &str, agent_id: Option<&AgentId>, success: bool) {
        let Some(ref cfg) = self.circuit_breaker_cfg else {
            return;
        };
        let key = match agent_id {
            Some(id) => format!("{}:{}", id.0, hook),
            None => hook.to_string(),
        };
        let (failures, opened_at_rfc3339, just_reset) = {
            let mut guard = self.circuit_breakers.lock().unwrap();
            let state = guard
                .entry(key.clone())
                .or_insert_with(CircuitBreakerState::new);
            if success {
                state.record_success();
                // Reset — signal deletion from SQLite.
                (0u32, None::<String>, true)
            } else {
                state.record_failure(cfg.max_failures);
                if state.consecutive_failures == cfg.max_failures {
                    warn!(
                        hook,
                        cooldown_secs = cfg.reset_secs,
                        "Hook circuit breaker opened"
                    );
                }
                // Compute RFC-3339 opened_at from the stored Instant if available.
                let opened_str = state.opened_at.map(|instant| {
                    let elapsed = instant.elapsed();
                    (chrono::Utc::now() - chrono::Duration::from_std(elapsed).unwrap_or_default())
                        .to_rfc3339()
                });
                (state.consecutive_failures, opened_str, false)
            }
        };

        // Persist to SQLite if trace store is available.
        if let Some(ref store) = self.trace_store {
            if just_reset {
                let _ = store.delete_circuit_state(&key);
            } else {
                let _ = store.save_circuit_state(&key, failures, opened_at_rfc3339.as_deref());
            }
        }
    }

    fn record_per_agent(&self, agent_id: &AgentId, elapsed_ms: u64, success: bool) {
        if let Ok(mut map) = self.per_agent_metrics.lock() {
            let stats = map.entry(agent_id.0.to_string()).or_default();
            stats.calls += 1;
            stats.total_ms += elapsed_ms;
            if success {
                stats.successes += 1;
            } else {
                stats.failures += 1;
            }
        }
    }

    pub fn per_agent_metrics_snapshot(&self) -> std::collections::HashMap<String, HookStats> {
        self.per_agent_metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub async fn prewarm(&self) {
        if !self.prewarm_subprocesses || !self.persistent_subprocess {
            return;
        }
        let runtime = self.runtime.clone();
        let hooks: &[(&str, &Option<String>)] = &[
            ("ingest", &self.ingest_script),
            ("after_turn", &self.after_turn_script),
            ("bootstrap", &self.bootstrap_script),
            ("assemble", &self.assemble_script),
            ("compact", &self.compact_script),
            ("on_event", &self.on_event_script),
        ];
        for (name, script_opt) in hooks {
            if let Some(ref script) = script_opt {
                let resolved = Self::resolve_script_path(script);
                if std::path::Path::new(&resolved).exists() {
                    let runtime = runtime.clone();
                    match self
                        .process_pool
                        .prewarm(&resolved, runtime.clone(), &self.plugin_env)
                        .await
                    {
                        Ok(()) => debug!(hook = name, "Pre-warmed hook subprocess"),
                        Err(e) => warn!(hook = name, error = %e, "Pre-warm failed"),
                    }
                }
            }
        }
    }

    /// Evict all persistent hook subprocesses for this plugin.
    ///
    /// Forces fresh subprocess spawns on the next hook call — useful after
    /// a plugin hot-reload so the new script version is picked up immediately
    /// rather than waiting for the old process to die naturally.
    pub async fn evict_hook_processes(&self) {
        if !self.persistent_subprocess {
            return;
        }
        let hooks: &[&Option<String>] = &[
            &self.ingest_script,
            &self.after_turn_script,
            &self.bootstrap_script,
            &self.assemble_script,
            &self.compact_script,
            &self.on_event_script,
        ];
        for script in hooks.iter().filter_map(|opt| opt.as_deref()) {
            let resolved = Self::resolve_script_path(script);
            self.process_pool.evict(&resolved).await;
        }
    }

    /// Wait for all in-flight after_turn background tasks to complete.
    ///
    /// Call this during daemon shutdown after stopping the agent loop so that
    /// no after_turn work is silently dropped. Times out after `timeout_secs`.
    pub async fn wait_for_after_turn_tasks(&self, timeout_secs: u64) {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
        // Lock once and drain; this is called during shutdown so no new tasks will
        // be spawned. Holding the async Mutex across join_next().await is safe here.
        let mut tasks = self.after_turn_tasks.lock().await;
        loop {
            if tasks.is_empty() {
                break;
            }
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                _ = tasks.join_next() => {}
                _ = tokio::time::sleep(remaining) => { break; }
            }
        }
    }

    /// Returns true when the agent_id passes the configured agent_id_filter.
    fn agent_passes_filter(&self, agent_id: &AgentId) -> bool {
        if self.agent_id_filter.is_empty() {
            return true;
        }
        let id_str = agent_id.0.to_string();
        self.agent_id_filter
            .iter()
            .any(|f| id_str.contains(f.as_str()))
    }

    /// Record the outcome of one hook invocation into the named slot.
    fn record_hook(
        metrics: &std::sync::Arc<std::sync::Mutex<HookMetrics>>,
        slot: &str,
        elapsed_ms: u64,
        ok: bool,
    ) {
        if let Ok(mut m) = metrics.lock() {
            let stats = match slot {
                "ingest" => &mut m.ingest,
                "after_turn" => &mut m.after_turn,
                "bootstrap" => &mut m.bootstrap,
                "assemble" => &mut m.assemble,
                "compact" => &mut m.compact,
                "prepare_subagent" => &mut m.prepare_subagent,
                "merge_subagent" => &mut m.merge_subagent,
                _ => return,
            };
            stats.calls += 1;
            stats.total_ms += elapsed_ms;
            if ok {
                stats.successes += 1;
            } else {
                stats.failures += 1;
            }
        }
    }

    /// Process the JSON output returned by an after_turn hook.
    ///
    /// Recognised fields:
    /// - `"memories"`: inject new memories for the agent
    /// - `"log"`:      emit the value as an info-level log line
    /// - `"annotations"`: arbitrary metadata (logged at debug level)
    ///
    /// Unknown fields are silently ignored so future hook versions stay
    /// backwards-compatible with older runtimes.
    fn process_after_turn_output(
        output: &serde_json::Value,
        agent_id: &str,
        memory_substrate: Option<&std::sync::Arc<librefang_memory::MemorySubstrate>>,
        plugin_name: &str,
        event_bus: Option<&std::sync::Arc<PluginEventBus>>,
    ) {
        // "log" field — emit as info log from the plugin's perspective.
        if let Some(msg) = output.get("log").and_then(|v| v.as_str()) {
            let trimmed = msg.chars().take(512).collect::<String>();
            tracing::info!(
                agent_id,
                plugin_log = trimmed.as_str(),
                "after_turn hook log"
            );
        }

        // "annotations" field — debug-level dump for observability.
        if let Some(ann) = output.get("annotations") {
            tracing::debug!(
                agent_id,
                annotations = ann
                    .to_string()
                    .chars()
                    .take(1024)
                    .collect::<String>()
                    .as_str(),
                "after_turn hook annotations"
            );
        }

        // "memories" field — store each entry in the memory substrate.
        if let Some(mems) = output.get("memories").and_then(|v| v.as_array()) {
            if let Some(substrate) = memory_substrate {
                for mem in mems {
                    let content = match mem.get("content").and_then(|v| v.as_str()) {
                        Some(c) if !c.is_empty() => c.to_string(),
                        _ => continue,
                    };
                    let tags: Vec<String> = mem
                        .get("tags")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|t| t.as_str().map(String::from))
                                .take(16)
                                .collect()
                        })
                        .unwrap_or_default();

                    // Fire-and-forget: memory injection is best-effort and must not block.
                    let substrate = std::sync::Arc::clone(substrate);
                    let agent_id_owned = agent_id.to_string();
                    tokio::spawn(async move {
                        use librefang_types::memory::Memory as _;
                        let parsed_id = uuid::Uuid::parse_str(&agent_id_owned)
                            .map(librefang_types::agent::AgentId)
                            .unwrap_or_else(|_| librefang_types::agent::AgentId::new());
                        let scope = if tags.is_empty() {
                            "hook".to_string()
                        } else {
                            tags.join(",")
                        };
                        if let Err(e) = substrate
                            .remember(
                                parsed_id,
                                &content,
                                librefang_types::memory::MemorySource::System,
                                &scope,
                                std::collections::HashMap::new(),
                            )
                            .await
                        {
                            tracing::warn!(error = %e, "after_turn hook: failed to inject memory");
                        }
                    });
                }
            }
        }

        // "events" field — publish named events to the event bus.
        if let Some(events) = output.get("events").and_then(|v| v.as_array()) {
            for ev in events {
                if let (Some(name), payload) = (
                    ev.get("name").and_then(|v| v.as_str()),
                    ev.get("payload")
                        .cloned()
                        .unwrap_or(serde_json::Value::Null),
                ) {
                    let event = PluginEvent {
                        name: name.to_string(),
                        payload,
                        source_plugin: plugin_name.to_string(),
                    };
                    if let Some(bus) = event_bus {
                        bus.emit(event);
                    }
                }
            }
        }
    }

    /// Dispatch a plugin event to the `on_event` hook script, if configured.
    ///
    /// The hook receives: `{"event": {"name": ..., "payload": ..., "source_plugin": ...}}`
    /// The hook's return value is ignored (fire-and-forget, spawned as background task).
    pub async fn dispatch_event(&self, event: &PluginEvent) {
        let script = match &self.on_event_script {
            Some(s) => s.clone(),
            None => return, // no on_event hook configured
        };

        let input = serde_json::json!({"event": event});
        let plugin_name = self.plugin_name.clone();
        let runtime = self.runtime.clone();
        let timeout_secs = self.hook_timeout_secs;
        let plugin_env = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let mut env = self.plugin_env.clone();
            for (k, v) in &guard.env_overrides {
                if !env.iter().any(|(ek, _)| ek == k) {
                    env.push((k.clone(), v.clone()));
                }
            }
            env
        };
        let traces = std::sync::Arc::clone(&self.traces);
        let hook_schemas = self.hook_schemas.clone();
        let shared_state_path = self.shared_state_path.clone();
        let trace_store = self.trace_store.clone();
        let max_retries = 0u32; // events are best-effort
        let retry_delay_ms = 0u64;
        let max_memory_mb = self.max_memory_mb;
        let allow_network = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard.allow_network.unwrap_or(self.allow_network)
        };
        let output_schema_strict = self.inner.config.output_schema_strict;
        let event_name = event.name.clone();
        debug!(plugin = %plugin_name, event = %event_name, "dispatching on_event hook");
        tokio::spawn(async move {
            let _ = Self::run_hook(
                "on_event",
                &script,
                runtime,
                input,
                timeout_secs,
                &plugin_env,
                max_retries,
                retry_delay_ms,
                max_memory_mb,
                allow_network,
                &traces,
                &hook_schemas,
                shared_state_path.as_deref(),
                trace_store.as_ref(),
                &plugin_name,
                &generate_trace_id(),
                output_schema_strict,
            )
            .await;
        });
    }

    /// Resolve a script path, expanding `~` to the user's home directory.
    fn resolve_script_path(path: &str) -> String {
        if let Some(rest) = path.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return format!("{}/{rest}", home.display());
            }
        }
        path.to_string()
    }

    /// Run a hook script with JSON input, return `(output, elapsed_ms)`.
    ///
    /// Retries up to `max_retries` times with `retry_delay_ms` between attempts.
    /// Records a `HookTrace` on every call (success or failure).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_hook(
        hook_name: &str,
        script_path: &str,
        runtime: crate::plugin_runtime::PluginRuntime,
        input: serde_json::Value,
        timeout_secs: u64,
        plugin_env: &[(String, String)],
        max_retries: u32,
        retry_delay_ms: u64,
        max_memory_mb: Option<u64>,
        allow_network: bool,
        traces: &std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<HookTrace>>>,
        hook_schemas: &std::collections::HashMap<String, librefang_types::config::HookSchema>,
        shared_state_path: Option<&std::path::Path>,
        trace_store: Option<&std::sync::Arc<crate::trace_store::TraceStore>>,
        plugin_name: &str,
        correlation_id: &str,
        output_schema_strict: bool,
    ) -> Result<(serde_json::Value, u64), String> {
        let resolved = Self::resolve_script_path(script_path);

        if !std::path::Path::new(&resolved).exists() {
            return Err(format!("Hook script not found: {resolved}"));
        }

        // Validate input schema if declared (always warn-only — input validation is advisory).
        if let Some(schema) = hook_schemas.get(hook_name) {
            if let Some(ref input_schema) = schema.input {
                let errs =
                    Self::validate_schema(input_schema, &input, &format!("{hook_name}/input"));
                for e in &errs {
                    warn!("{e}");
                }
            }
        }

        let config = crate::plugin_runtime::HookConfig {
            timeout_secs,
            plugin_env: plugin_env.to_vec(),
            max_memory_mb,
            allow_network,
            state_file: shared_state_path.map(|p| p.to_path_buf()),
            retry_delay_ms,
            ..Default::default()
        };

        let trace_id = generate_trace_id();
        let started_at = chrono::Utc::now().to_rfc3339();
        // Truncate large inputs for trace preview.
        let input_preview = if input.to_string().len() > 2048 {
            serde_json::json!({"_truncated": true, "type": input.get("type")})
        } else {
            input.clone()
        };

        let t = std::time::Instant::now();
        let mut last_err = String::new();
        for attempt in 0..=max_retries {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(
                    config.delay_for_attempt(attempt),
                ))
                .await;
                debug!(
                    script = resolved.as_str(),
                    attempt, max_retries, "Retrying hook after failure: {last_err}"
                );
            }
            match crate::plugin_runtime::run_hook_json(
                hook_name,
                &resolved,
                runtime.clone(),
                &input,
                &config,
            )
            .await
            {
                Ok(v) => {
                    let elapsed_ms = t.elapsed().as_millis() as u64;
                    // Validate output schema if declared.
                    if let Some(schema) = hook_schemas.get(hook_name) {
                        if let Some(ref output_schema) = schema.output {
                            let errs = Self::validate_schema(
                                output_schema,
                                &v,
                                &format!("{hook_name}/output"),
                            );
                            if !errs.is_empty() {
                                if output_schema_strict {
                                    let err_msg = format!(
                                        "hook {hook_name} output failed schema validation: {}",
                                        errs.join("; ")
                                    );
                                    // Record the failure trace before surfacing the error so
                                    // the trace store is never missing an entry for this call.
                                    Self::push_trace(
                                        traces,
                                        HookTrace {
                                            trace_id: trace_id.clone(),
                                            correlation_id: correlation_id.to_string(),
                                            hook: hook_name.to_string(),
                                            started_at: started_at.clone(),
                                            elapsed_ms: t.elapsed().as_millis() as u64,
                                            success: false,
                                            error: Some(err_msg.clone()),
                                            input_preview: input_preview.clone(),
                                            output_preview: None,
                                            annotations: None,
                                        },
                                        trace_store,
                                        plugin_name,
                                    );
                                    return Err(err_msg);
                                }
                                for e in &errs {
                                    warn!("{e}");
                                }
                            }
                        }
                    }
                    Self::push_trace(
                        traces,
                        HookTrace {
                            trace_id: trace_id.clone(),
                            correlation_id: correlation_id.to_string(),
                            hook: hook_name.to_string(),
                            started_at: started_at.clone(),
                            elapsed_ms,
                            success: true,
                            error: None,
                            input_preview: input_preview.clone(),
                            output_preview: Some(v.clone()),
                            annotations: v.get("annotations").cloned(),
                        },
                        trace_store,
                        plugin_name,
                    );
                    return Ok((v, elapsed_ms));
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        let elapsed_ms = t.elapsed().as_millis() as u64;
        let err_msg = format!("Hook script failed after {max_retries} retries: {last_err}");
        Self::push_trace(
            traces,
            HookTrace {
                trace_id: trace_id.clone(),
                correlation_id: correlation_id.to_string(),
                hook: hook_name.to_string(),
                started_at,
                elapsed_ms,
                success: false,
                error: Some(err_msg.clone()),
                input_preview,
                output_preview: None,
                annotations: None,
            },
            trace_store,
            plugin_name,
        );
        Err(err_msg)
    }

    /// Dispatch a hook call to either the persistent process pool or a fresh subprocess.
    ///
    /// When `self.persistent_subprocess` is `true`, the call is routed through
    /// `self.process_pool` (JSON-lines, long-lived process). Otherwise a fresh
    /// subprocess is spawned via `Self::run_hook`. Either way the return is
    /// `Ok((output, elapsed_ms))` or `Err(message)`.
    async fn call_hook_dispatch(
        &self,
        hook_name: &str,
        script_path: &str,
        input: serde_json::Value,
        timeout_secs: u64,
        agent_id: Option<&AgentId>,
    ) -> Result<(serde_json::Value, u64), String> {
        // Circuit breaker: reject immediately when open
        if self.circuit_is_open(hook_name, agent_id) {
            return Err(format!(
                "circuit-open: '{hook_name}' suspended after repeated failures"
            ));
        }
        // Rate limiting check.
        let max_rpm = self.inner.config.max_hook_calls_per_minute;
        if max_rpm > 0 {
            let mut limiters = self.rate_limiters.lock().unwrap_or_else(|e| e.into_inner());
            // Key by "{agent_id}:{hook_name}" so one agent cannot exhaust the
            // rate limit for all other agents sharing the same plugin.
            let rl_key = format!(
                "{}:{}",
                agent_id.map(|id| id.0.to_string()).unwrap_or_default(),
                hook_name
            );
            let limiter = limiters.entry(rl_key).or_default();
            if !limiter.check_and_record(max_rpm) {
                warn!(
                    hook = hook_name,
                    max_rpm, "hook rate limit exceeded — skipping call"
                );
                // Return a neutral result: empty object (passthrough for callers).
                return Ok((serde_json::Value::Object(serde_json::Map::new()), 0));
            }
        }
        let correlation_id = generate_trace_id();
        let agent_id_str = agent_id.map(|id| id.0.to_string());
        let result = self
            .call_hook_dispatch_raw(
                hook_name,
                script_path,
                input,
                timeout_secs,
                &correlation_id,
                agent_id_str.as_deref(),
            )
            .await;
        // Update circuit breaker.
        // Schema validation is performed inside call_hook_dispatch_raw (persistent path)
        // and run_hook (non-persistent path) so that Err propagates here correctly.
        match &result {
            Ok(_) => self.circuit_record(hook_name, agent_id, true),
            Err(_) => self.circuit_record(hook_name, agent_id, false),
        }
        result
    }

    async fn call_hook_dispatch_raw(
        &self,
        hook_name: &str,
        script_path: &str,
        input: serde_json::Value,
        timeout_secs: u64,
        correlation_id: &str,
        agent_id: Option<&str>,
    ) -> Result<(serde_json::Value, u64), String> {
        // Compute effective env and network permission, merging bootstrap overrides.
        let (mut effective_env, effective_allow_network) = {
            let guard = self
                .bootstrap_applied_overrides
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let mut env = self.plugin_env.clone();
            for (k, v) in &guard.env_overrides {
                if !env.iter().any(|(ek, _)| ek == k) {
                    env.push((k.clone(), v.clone()));
                }
            }
            let allow_net = guard.allow_network.unwrap_or(self.allow_network);
            (env, allow_net)
        };

        // Write the resolved plugin config JSON file and expose its path via
        // LIBREFANG_PLUGIN_CONFIG so hook scripts can read typed settings.
        let empty_overrides = std::collections::HashMap::new();
        if let Some(config_path) = Self::write_plugin_config_file(
            &self.plugin_name,
            &self.plugin_config_schema,
            &empty_overrides,
        ) {
            effective_env.push((
                "LIBREFANG_PLUGIN_CONFIG".to_string(),
                config_path.to_string_lossy().into_owned(),
            ));
        }

        // Scope state file to this agent when agent_id is known.
        let effective_state_path = self
            .shared_state_path
            .as_deref()
            .map(|p| agent_scoped_state_path(p, agent_id));

        if self.persistent_subprocess {
            let config = crate::plugin_runtime::HookConfig {
                timeout_secs,
                plugin_env: effective_env.clone(),
                max_memory_mb: self.max_memory_mb,
                allow_network: effective_allow_network,
                state_file: effective_state_path.clone(),
                ..Default::default()
            };
            let trace_id = generate_trace_id();
            let input_preview = if input.to_string().len() > 2048 {
                serde_json::json!({"_truncated": true, "type": input.get("type")})
            } else {
                input.clone()
            };
            let started_at = chrono::Utc::now().to_rfc3339();
            let t = std::time::Instant::now();
            let call_result = self
                .process_pool
                .call(script_path, self.runtime.clone(), &input, &config)
                .await;
            let elapsed_ms = t.elapsed().as_millis() as u64;
            match call_result {
                Ok(output) => {
                    // Validate output schema before recording a success trace so that
                    // schema violations are reflected in both the trace and the circuit
                    // breaker (the Err propagates to call_hook_dispatch which calls
                    // circuit_record(false)).  Mirrors the identical logic in run_hook().
                    if let Some(schema) = self.hook_schemas.get(hook_name) {
                        if let Some(ref output_schema) = schema.output {
                            let errs = Self::validate_schema(
                                output_schema,
                                &output,
                                &format!("{hook_name}/output"),
                            );
                            if !errs.is_empty() {
                                if self.inner.config.output_schema_strict {
                                    let err_msg = format!(
                                        "hook {hook_name} output failed schema validation: {}",
                                        errs.join("; ")
                                    );
                                    Self::push_trace(
                                        &self.traces,
                                        HookTrace {
                                            trace_id: trace_id.clone(),
                                            correlation_id: correlation_id.to_string(),
                                            hook: hook_name.to_string(),
                                            started_at,
                                            elapsed_ms,
                                            success: false,
                                            error: Some(err_msg.clone()),
                                            input_preview,
                                            output_preview: None,
                                            annotations: None,
                                        },
                                        self.trace_store.as_ref(),
                                        &self.plugin_name,
                                    );
                                    return Err(err_msg);
                                }
                                for e in &errs {
                                    warn!("{e}");
                                }
                            }
                        }
                    }
                    Self::push_trace(
                        &self.traces,
                        HookTrace {
                            trace_id: trace_id.clone(),
                            correlation_id: correlation_id.to_string(),
                            hook: hook_name.to_string(),
                            started_at,
                            elapsed_ms,
                            success: true,
                            error: None,
                            input_preview,
                            output_preview: Some(output.clone()),
                            annotations: output.get("annotations").cloned(),
                        },
                        self.trace_store.as_ref(),
                        &self.plugin_name,
                    );
                    Ok((output, elapsed_ms))
                }
                Err(e) => {
                    let err_msg = e.to_string();
                    Self::push_trace(
                        &self.traces,
                        HookTrace {
                            trace_id: trace_id.clone(),
                            correlation_id: correlation_id.to_string(),
                            hook: hook_name.to_string(),
                            started_at,
                            elapsed_ms,
                            success: false,
                            error: Some(err_msg.clone()),
                            input_preview,
                            output_preview: None,
                            annotations: None,
                        },
                        self.trace_store.as_ref(),
                        &self.plugin_name,
                    );
                    Err(err_msg)
                }
            }
        } else {
            Self::run_hook(
                hook_name,
                script_path,
                self.runtime.clone(),
                input,
                timeout_secs,
                &effective_env,
                self.max_retries,
                self.retry_delay_ms,
                self.max_memory_mb,
                effective_allow_network,
                &self.traces,
                &self.hook_schemas,
                effective_state_path.as_deref(),
                self.trace_store.as_ref(),
                &self.plugin_name,
                correlation_id,
                self.inner.config.output_schema_strict,
            )
            .await
        }
    }

    /// Apply the configured failure policy to a hook error.
    ///
    /// Returns `Ok(None)` when the policy is Warn or Skip (continue with
    /// fallback), or `Err(…)` when the policy is Abort.
    fn apply_failure_policy(&self, hook: &str, err: &str) -> LibreFangResult<()> {
        use librefang_types::config::HookFailurePolicy;
        match self.on_hook_failure {
            HookFailurePolicy::Warn => {
                warn!(
                    hook,
                    error = err,
                    "Hook failed (warn policy — using fallback)"
                );
                Ok(())
            }
            HookFailurePolicy::Skip => Ok(()), // silent
            HookFailurePolicy::Abort => Err(LibreFangError::Internal(format!(
                "Hook '{hook}' failed (abort policy): {err}"
            ))),
        }
    }

    /// Compute a health snapshot for this engine layer.
    pub async fn layer_health(&self) -> EngineLayerHealth {
        // Circuit breaker: snapshot current open/closed state for each tracked key.
        // Keys are stored as "{agent_id}:{hook}" or bare "{hook}".
        // We report bare hook names; agent-scoped keys use the portion after the last ':'.
        let circuit_open: std::collections::HashMap<String, bool> = {
            let guard = self
                .circuit_breakers
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            guard
                .iter()
                .map(|(key, state)| {
                    // Extract hook name: last segment after ':' (or whole key if no ':').
                    let hook = match key.rfind(':') {
                        Some(pos) => key[pos + 1..].to_string(),
                        None => key.clone(),
                    };
                    (hook, state.opened_at.is_some())
                })
                .collect()
        };

        // Recent traces from the in-memory ring buffer (sync Mutex).
        let (recent_calls, recent_errors) = {
            let buf = self.traces.lock().unwrap_or_else(|p| p.into_inner());
            let calls = buf.len();
            let errors = buf.iter().filter(|t| t.error.is_some()).count();
            (calls, errors)
        };

        // Active hooks: count how many lifecycle script slots are populated.
        let active_hooks = [
            &self.ingest_script,
            &self.after_turn_script,
            &self.bootstrap_script,
            &self.assemble_script,
            &self.compact_script,
            &self.prepare_subagent_script,
            &self.merge_subagent_script,
            &self.on_event_script,
        ]
        .iter()
        .filter(|opt| opt.is_some())
        .count();

        EngineLayerHealth {
            plugin_name: self.plugin_name.clone(),
            circuit_open,
            active_hooks,
            recent_errors,
            recent_calls,
        }
    }

    /// Apply overrides returned by the bootstrap hook.
    ///
    /// Parses the hook output JSON into a [`BootstrapOverrides`] value and
    /// stores it in `bootstrap_applied_overrides` so subsequent hook calls
    /// pick up the overridden `plugin_env`, `ingest_filter`, and
    /// `allow_network` values.
    fn apply_bootstrap_overrides(&self, output: &serde_json::Value) {
        let overrides: BootstrapOverrides = match serde_json::from_value(output.clone()) {
            Ok(v) => v,
            Err(e) => {
                warn!(plugin = %self.plugin_name, "Failed to parse bootstrap overrides: {e}");
                return;
            }
        };

        if let Ok(mut guard) = self.bootstrap_applied_overrides.lock() {
            // Merge env overrides: only add keys not already present in the initial
            // plugin_env so that statically-configured vars take precedence.
            for (k, v) in overrides.env_overrides {
                if !self.plugin_env.iter().any(|(ek, _)| ek == &k) {
                    guard.env_overrides.insert(k, v);
                }
            }

            // Optional field overrides.
            if let Some(filter) = overrides.ingest_filter {
                guard.ingest_filter = Some(filter);
            }
            if let Some(allow) = overrides.allow_network {
                guard.allow_network = Some(allow);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin loader — resolves `plugin = "name"` to hook paths
// ---------------------------------------------------------------------------

/// Default plugin directory: `~/.librefang/plugins/`.
pub fn plugins_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".librefang")
        .join("plugins")
}

/// Load a plugin manifest from `~/.librefang/plugins/<name>/plugin.toml`.
///
/// Hook paths in the manifest are relative to the plugin directory — this
/// function resolves them to absolute paths so the script runner can find them.
/// Validate that a plugin name is a safe directory component (no path traversal).
pub(super) fn validate_plugin_name(name: &str) -> LibreFangResult<()> {
    // Strict whitelist: only ASCII alphanumeric, hyphens, and underscores.
    // Rejects spaces, null bytes, path separators, unicode, and shell specials.
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(LibreFangError::Internal(format!(
            "Invalid plugin name '{name}': must contain only ASCII letters, digits, hyphens, and underscores"
        )));
    }
    Ok(())
}

pub(super) fn load_plugin(
    plugin_name: &str,
) -> LibreFangResult<(
    librefang_types::config::PluginManifest,
    librefang_types::config::ContextEngineHooks,
)> {
    validate_plugin_name(plugin_name)?;
    let plugin_dir = plugins_dir().join(plugin_name);
    let manifest_path = plugin_dir.join("plugin.toml");

    if !manifest_path.exists() {
        return Err(LibreFangError::Internal(format!(
            "Plugin '{plugin_name}' not found at {}",
            manifest_path.display()
        )));
    }

    let content = std::fs::read_to_string(&manifest_path).map_err(|e| {
        LibreFangError::Internal(format!("Failed to read {}: {e}", manifest_path.display()))
    })?;

    let manifest: librefang_types::config::PluginManifest =
        toml::from_str(&content).map_err(|e| {
            LibreFangError::Internal(format!("Invalid plugin.toml for '{plugin_name}': {e}"))
        })?;

    // Resolve relative hook paths to absolute paths within the plugin dir
    // and verify they don't escape the plugin directory (path traversal guard).
    let canon_plugin_dir =
        std::fs::canonicalize(&plugin_dir).unwrap_or_else(|_| plugin_dir.clone());

    let resolve_and_sandbox = |rel_path: &str| -> LibreFangResult<String> {
        let abs_path = plugin_dir.join(rel_path);
        // Canonicalize to resolve any ".." components
        let canon = std::fs::canonicalize(&abs_path).map_err(|e| {
            LibreFangError::Internal(format!(
                "Cannot resolve hook path '{}': {e}",
                abs_path.display()
            ))
        })?;
        if !canon.starts_with(&canon_plugin_dir) {
            return Err(LibreFangError::Internal(format!(
                "Hook script '{}' escapes plugin directory '{}'",
                canon.display(),
                canon_plugin_dir.display()
            )));
        }
        Ok(canon.to_string_lossy().into_owned())
    };

    let resolved_hooks = librefang_types::config::ContextEngineHooks {
        ingest: manifest
            .hooks
            .ingest
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        after_turn: manifest
            .hooks
            .after_turn
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        bootstrap: manifest
            .hooks
            .bootstrap
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        assemble: manifest
            .hooks
            .assemble
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        compact: manifest
            .hooks
            .compact
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        prepare_subagent: manifest
            .hooks
            .prepare_subagent
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        merge_subagent: manifest
            .hooks
            .merge_subagent
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        // Propagate the runtime tag from the plugin manifest. `None` means
        // "use the default" which resolves to Python in PluginRuntime::from_tag.
        runtime: manifest.hooks.runtime.clone(),
        // Propagate all extended hook config fields from the manifest.
        hook_timeout_secs: manifest.hooks.hook_timeout_secs,
        max_retries: manifest.hooks.max_retries,
        retry_delay_ms: manifest.hooks.retry_delay_ms,
        ingest_filter: manifest.hooks.ingest_filter.clone(),
        on_hook_failure: manifest.hooks.on_hook_failure.clone(),
        hook_protocol_version: manifest.hooks.hook_protocol_version,
        max_memory_mb: manifest.hooks.max_memory_mb,
        allow_network: manifest.hooks.allow_network,
        only_for_agent_ids: manifest.hooks.only_for_agent_ids.clone(),
        hook_schemas: manifest.hooks.hook_schemas.clone(),
        hook_cache_ttl_secs: manifest.hooks.hook_cache_ttl_secs,
        persistent_subprocess: manifest.hooks.persistent_subprocess,
        assemble_cache_ttl_secs: manifest.hooks.assemble_cache_ttl_secs,
        compact_cache_ttl_secs: manifest.hooks.compact_cache_ttl_secs,
        priority: manifest.hooks.priority,
        ingest_regex: manifest.hooks.ingest_regex.clone(),
        env_schema: manifest.hooks.env_schema.clone(),
        enable_shared_state: manifest.hooks.enable_shared_state,
        circuit_breaker: manifest.hooks.circuit_breaker.clone(),
        after_turn_queue_depth: manifest.hooks.after_turn_queue_depth,
        prewarm_subprocesses: manifest.hooks.prewarm_subprocesses,
        allow_filesystem: manifest.hooks.allow_filesystem,
        otel_endpoint: manifest.hooks.otel_endpoint.clone(),
        on_event: manifest
            .hooks
            .on_event
            .as_ref()
            .map(|p| resolve_and_sandbox(p))
            .transpose()?,
        allowed_secrets: manifest.hooks.allowed_secrets.clone(),
    };

    debug!(
        plugin = plugin_name,
        dir = %plugin_dir.display(),
        ingest = ?resolved_hooks.ingest,
        after_turn = ?resolved_hooks.after_turn,
        bootstrap = ?resolved_hooks.bootstrap,
        assemble = ?resolved_hooks.assemble,
        compact = ?resolved_hooks.compact,
        prepare_subagent = ?resolved_hooks.prepare_subagent,
        merge_subagent = ?resolved_hooks.merge_subagent,
        "Loaded plugin manifest"
    );

    Ok((manifest, resolved_hooks))
}
