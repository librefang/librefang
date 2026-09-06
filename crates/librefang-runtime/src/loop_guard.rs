//! Tool loop detection for the agent execution loop.
//!
//! Tracks tool calls within a single agent loop execution using SHA-256
//! hashes of `(tool_name, serialized_params)`. Detects when the agent is
//! stuck calling the same tool repeatedly and provides graduated responses:
//! warn, block, or circuit-break the entire loop.
//!
//! Enhanced features beyond basic hash-counting:
//! - **Outcome-aware detection**: tracks result hashes so identical call+result
//!   pairs escalate faster than just repeated calls.
//! - **Ping-pong detection**: identifies A-B-A-B or A-B-C-A-B-C alternating
//!   patterns that evade single-hash counting.
//! - **Poll tool handling**: relaxed thresholds for tools expected to be called
//!   repeatedly (e.g. `shell_exec` status checks).
//! - **Backoff suggestions**: recommends increasing wait times for polling.
//! - **Warning bucket**: prevents spam by upgrading to Block after repeated
//!   warnings for the same call.
//! - **Statistics snapshot**: exposes internal state for debugging and API.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

/// Tools that are expected to be polled repeatedly.
const POLL_TOOLS: &[&str] = &[
    "shell_exec", // checking command output
];

/// Known command prefixes that indicate polling intent.
const POLL_COMMAND_PREFIXES: &[&str] = &[
    "docker ps",
    "docker inspect",
    "kubectl get",
    "kubectl describe",
    "kubectl wait",
    "systemctl status",
    "systemctl is-active",
    "service ",
    "supervisorctl status",
    "pgrep",
    "pidof",
    "lsof -i",
    "ss -tlnp",
    "netstat ",
    "curl -s", // health checks
    "wget -q", // health checks
];

/// Keywords in commands that strongly indicate polling intent.
const POLL_KEYWORDS: &[&str] = &[
    "status", "poll", "wait", "watch", "tail", "ps ", "jobs", "pgrep", "health", "ping", "alive",
    "ready", "check",
];

/// Maximum recent call history size for ping-pong detection.
const HISTORY_SIZE: usize = 30;

/// Shared prefix anchoring the circuit-break classifier in `agent_loop` to the producer here.
pub(crate) const CIRCUIT_BREAKER_MSG_PREFIX: &str = "Circuit breaker:";

/// Backoff schedule in milliseconds for polling tools.
const BACKOFF_SCHEDULE_MS: &[u64] = &[5000, 10000, 30000, 60000];

/// Configuration for the loop guard.
#[derive(Debug, Clone)]
pub struct LoopGuardConfig {
    /// Number of identical calls before a warning is appended.
    pub warn_threshold: u32,
    /// Number of identical calls before the call is blocked.
    pub block_threshold: u32,
    /// Total tool calls across all tools before circuit-breaking.
    pub global_circuit_breaker: u32,
    /// Multiplier for poll tool thresholds (poll tools get thresholds * this).
    pub poll_multiplier: u32,
    /// Number of identical outcome pairs before a warning.
    pub outcome_warn_threshold: u32,
    /// Number of identical outcome pairs before the next call is auto-blocked.
    pub outcome_block_threshold: u32,
    /// Minimum repeats of a ping-pong pattern before blocking.
    pub ping_pong_min_repeats: u32,
    /// Max warnings per unique tool call hash before upgrading to Block.
    pub max_warnings_per_call: u32,
}

impl Default for LoopGuardConfig {
    fn default() -> Self {
        Self {
            warn_threshold: 3,
            block_threshold: 5,
            global_circuit_breaker: 30,
            poll_multiplier: 3,
            outcome_warn_threshold: 2,
            outcome_block_threshold: 3,
            ping_pong_min_repeats: 3,
            max_warnings_per_call: 3,
        }
    }
}

/// Verdict from the loop guard on whether a tool call should proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopGuardVerdict {
    /// Proceed normally.
    Allow,
    /// Proceed, but append a warning to the tool result.
    Warn(String),
    /// Block this specific tool call (skip execution).
    Block(String),
    /// Circuit-break the entire agent loop.
    CircuitBreak(String),
}

/// Snapshot of the loop guard state (for debugging/API).
#[derive(Debug, Clone, Serialize)]
pub struct LoopGuardStats {
    /// Total tool calls made in this loop execution.
    pub total_calls: u32,
    /// Number of unique (tool_name + params) combinations seen.
    pub unique_calls: u32,
    /// Number of calls that were blocked.
    pub blocked_calls: u32,
    /// Whether a ping-pong pattern has been detected.
    pub ping_pong_detected: bool,
    /// The tool name that has been repeated the most (if any).
    pub most_repeated_tool: Option<String>,
    /// The count of the most repeated tool call.
    pub most_repeated_count: u32,
}

/// Tracks tool calls within a single agent loop to detect loops.
pub struct LoopGuard {
    config: LoopGuardConfig,
    /// Count of identical (tool_name + params) calls, keyed by SHA-256 hex hash.
    call_counts: HashMap<String, u32>,
    /// Total tool calls in this loop execution.
    total_calls: u32,
    /// Count of identical (tool_call_hash + result_hash) pairs.
    outcome_counts: HashMap<String, u32>,
    /// Call hashes that are blocked due to repeated identical outcomes.
    blocked_outcomes: HashSet<String>,
    /// Recent tool call hashes (ring buffer of last HISTORY_SIZE).
    recent_calls: Vec<String>,
    /// Warnings already emitted (to prevent spam). Key = call hash, value = count emitted.
    warnings_emitted: HashMap<String, u32>,
    /// Tracks poll counts per command hash for backoff suggestions.
    poll_counts: HashMap<String, u32>,
    /// Total calls that were blocked.
    blocked_calls: u32,
    /// Map from call hash to tool name (for stats reporting).
    hash_to_tool: HashMap<String, String>,
}

impl LoopGuard {
    /// Create a new loop guard with the given configuration.
    pub fn new(config: LoopGuardConfig) -> Self {
        Self {
            config,
            call_counts: HashMap::new(),
            total_calls: 0,
            outcome_counts: HashMap::new(),
            blocked_outcomes: HashSet::new(),
            recent_calls: Vec::with_capacity(HISTORY_SIZE),
            warnings_emitted: HashMap::new(),
            poll_counts: HashMap::new(),
            blocked_calls: 0,
            hash_to_tool: HashMap::new(),
        }
    }

    /// Check whether a tool call should proceed.
    ///
    /// Returns a verdict indicating whether to allow, warn, block, or
    /// circuit-break. The caller should act on the verdict before executing
    /// the tool.
    pub fn check(&mut self, tool_name: &str, params: &serde_json::Value) -> LoopGuardVerdict {
        self.total_calls += 1;

        // Global circuit breaker
        if self.total_calls > self.config.global_circuit_breaker {
            self.blocked_calls += 1;
            return LoopGuardVerdict::CircuitBreak(format!(
                "{CIRCUIT_BREAKER_MSG_PREFIX} exceeded {} total tool calls in this loop. \
                 The agent appears to be stuck.",
                self.config.global_circuit_breaker
            ));
        }

        let hash = Self::compute_hash(tool_name, params);
        self.hash_to_tool
            .entry(hash.clone())
            .or_insert_with(|| tool_name.to_string());

        // Track recent calls for ping-pong detection
        if self.recent_calls.len() >= HISTORY_SIZE {
            self.recent_calls.remove(0);
        }
        self.recent_calls.push(hash.clone());

        // Check if this call hash was blocked by outcome detection
        if self.blocked_outcomes.contains(&hash) {
            self.blocked_calls += 1;
            return LoopGuardVerdict::Block(format!(
                "Blocked: tool '{}' is returning identical results repeatedly. \
                 The current approach is not working — try something different.",
                tool_name
            ));
        }

        let count = self.call_counts.entry(hash.clone()).or_insert(0);
        *count += 1;
        let count_val = *count;

        // Determine effective thresholds (poll tools get relaxed thresholds)
        let is_poll = Self::is_poll_call(tool_name, params);
        let multiplier = if is_poll {
            self.config.poll_multiplier
        } else {
            1
        };
        let effective_warn = self.config.warn_threshold * multiplier;
        let effective_block = self.config.block_threshold * multiplier;

        // Check per-hash thresholds
        if count_val >= effective_block {
            self.blocked_calls += 1;
            return LoopGuardVerdict::Block(format!(
                "Blocked: tool '{}' called {} times with identical parameters. \
                 Try a different approach or different parameters.",
                tool_name, count_val
            ));
        }

        if count_val >= effective_warn {
            // Warning bucket: check if we've already warned too many times
            let warning_count = self.warnings_emitted.entry(hash.clone()).or_insert(0);
            *warning_count += 1;
            if *warning_count > self.config.max_warnings_per_call {
                // Upgrade to block after too many warnings
                self.blocked_calls += 1;
                return LoopGuardVerdict::Block(format!(
                    "Blocked: tool '{}' called {} times with identical parameters \
                     (warnings exhausted). Try a different approach.",
                    tool_name, count_val
                ));
            }
            return LoopGuardVerdict::Warn(format!(
                "Warning: tool '{}' has been called {} times with identical parameters. \
                 Consider a different approach.",
                tool_name, count_val
            ));
        }

        // Ping-pong detection (runs even if individual hash counts are low)
        if let Some(ping_pong_msg) = self.detect_ping_pong() {
            // Count how many full pattern repeats we have
            let repeats = self.count_ping_pong_repeats();
            if repeats >= self.config.ping_pong_min_repeats {
                self.blocked_calls += 1;
                return LoopGuardVerdict::Block(ping_pong_msg);
            }
            // Below min_repeats, just warn
            let warning_count = self
                .warnings_emitted
                .entry(format!("pingpong_{}", hash))
                .or_insert(0);
            *warning_count += 1;
            if *warning_count <= self.config.max_warnings_per_call {
                return LoopGuardVerdict::Warn(ping_pong_msg);
            }
        }

        LoopGuardVerdict::Allow
    }

    /// Record the outcome of a tool call. Call this AFTER tool execution.
    ///
    /// Hashes `(tool_name | params_json | result_truncated)` and tracks how
    /// many times an identical call produces an identical result. Returns a
    /// warning string if outcome repetition is detected.
    ///
    /// The hash covers the parameters as well as the result, so this only ever fires for a call repeated with identical parameters that keeps producing identical output — the case [`Self::check`] would otherwise allow to run all the way to `block_threshold`.
    /// The production caller is `agent_loop::tool_call::record_loop_guard_outcome`, which runs in the sequential phase of both the serial and the parallel dispatch path.
    pub fn record_outcome(
        &mut self,
        tool_name: &str,
        params: &serde_json::Value,
        result: &str,
    ) -> Option<String> {
        let outcome_hash = Self::compute_outcome_hash(tool_name, params, result);
        let call_hash = Self::compute_hash(tool_name, params);

        let count = self.outcome_counts.entry(outcome_hash).or_insert(0);
        *count += 1;
        let count_val = *count;

        // A declared poll repeats by design, and an unchanged answer is exactly what "not ready yet" looks like for one, so it gets the multiplier `check()` would grant it.
        // Without this the outcome path would decide a wait loop after 3 identical answers while `block_threshold * poll_multiplier` still advertises 15 — and it would win, because `check()` consults `blocked_outcomes` before it applies the multiplier.
        // The narrow predicate, not the one `check()` uses: relaxing the outcome rule for every call whose parameters happen to contain "status" or "wait" would weaken it by 3x for ordinary tools such as `task_list {"status": "pending"}`, which is the opposite of what the rule is for.
        let multiplier = if Self::is_declared_poll_call(tool_name, params) {
            self.config.poll_multiplier
        } else {
            1
        };
        let effective_block = self.config.outcome_block_threshold * multiplier;
        let effective_warn = self.config.outcome_warn_threshold * multiplier;

        if count_val >= effective_block {
            // Mark the call hash so the NEXT check() auto-blocks it
            self.blocked_outcomes.insert(call_hash);
            return Some(format!(
                "Tool '{}' is returning identical results — the approach isn't working. \
                 Another identical call will be blocked.",
                tool_name
            ));
        }

        if count_val >= effective_warn {
            return Some(format!(
                "Tool '{}' is returning identical results — the approach isn't working.",
                tool_name
            ));
        }

        None
    }

    /// Get the suggested backoff delay (in milliseconds) for a polling tool call.
    ///
    /// Returns `None` if this is not a poll call. Returns `Some(ms)` with a
    /// suggested delay from the backoff schedule, capping at the last entry.
    ///
    /// The delay is a suggestion the caller turns into advisory text on the tool result, never an in-loop sleep: stalling the agent loop would hold the session lock and the provider's prompt cache window for a minute at a time, so pacing is left to the model that decides whether the wait is still worth it.
    /// Because that suggestion is read by the model, the poll test here is the narrow `is_declared_poll_call` rather than the broader `is_poll_call` — "wait roughly 5s before checking again" is false advice on a `task_list {"status": "pending"}`.
    ///
    /// The schedule is keyed on the call alone, not on whether the answer changed: a declared poll gets the next step on every repeat, including one that finally returned something different.
    /// That is deliberate — the delay paces how often the agent asks, and a poll whose answer just changed is normally the last one anyway.
    pub fn get_poll_backoff(&mut self, tool_name: &str, params: &serde_json::Value) -> Option<u64> {
        if !Self::is_declared_poll_call(tool_name, params) {
            return None;
        }
        let hash = Self::compute_hash(tool_name, params);
        let count = self.poll_counts.entry(hash).or_insert(0);
        *count += 1;
        // count is 1-indexed; backoff starts on the second call
        if *count <= 1 {
            return None;
        }
        let idx = (*count as usize).saturating_sub(2);
        let delay = BACKOFF_SCHEDULE_MS
            .get(idx)
            .copied()
            .unwrap_or(*BACKOFF_SCHEDULE_MS.last().unwrap_or(&60000));
        Some(delay)
    }

    /// Get a snapshot of current loop guard statistics.
    pub fn stats(&self) -> LoopGuardStats {
        let unique_calls = self.call_counts.len() as u32;

        // Find the most repeated tool call
        let mut most_repeated_tool: Option<String> = None;
        let mut most_repeated_count: u32 = 0;
        for (hash, &count) in &self.call_counts {
            if count > most_repeated_count {
                most_repeated_count = count;
                most_repeated_tool = self.hash_to_tool.get(hash).cloned();
            }
        }

        LoopGuardStats {
            total_calls: self.total_calls,
            unique_calls,
            blocked_calls: self.blocked_calls,
            ping_pong_detected: self.detect_ping_pong_pure(),
            most_repeated_tool,
            most_repeated_count,
        }
    }

    /// Check if a tool call looks like a polling operation, on the broad reading.
    ///
    /// Poll tools (like `shell_exec` for status checks) are expected to be
    /// called repeatedly and get relaxed loop detection thresholds.
    ///
    /// Detection uses three strategies (no arbitrary length limits):
    /// 1. Explicit `"poll": true` parameter — callers can mark poll intent directly.
    /// 2. Known command prefix matching — e.g. `docker ps`, `kubectl get`.
    /// 3. Keyword matching — e.g. `status`, `poll`, `health`, `check`.
    ///
    /// Strategies 1 and 2 are `is_declared_poll_call`; strategy 3 is the broadening this adds on top, and it is much wider than the name suggests.
    /// The keyword scan runs over the whole serialized parameter object for *any* tool, so `task_list {"status": "pending"}` and `goal_update {"status": "in_progress"}` both read as polls.
    /// That is tolerable where the classification only relaxes a threshold — a runaway is still stopped, just later — which is why [`Self::check`] keeps using it.
    /// It is not tolerable where the classification turns into advice the model reads, or into a threshold the outcome rule depends on; those two callers use the narrow predicate instead.
    fn is_poll_call(tool_name: &str, params: &serde_json::Value) -> bool {
        // Explicit poll intent via params.
        // `false` is a veto and must be honoured before the generic scan below, which would otherwise match on the literal key name "poll".
        if let Some(poll) = params.get("poll").and_then(|v| v.as_bool()) {
            return poll;
        }

        if Self::is_declared_poll_call(tool_name, params) {
            return true;
        }

        // Generic poll detection via params keywords
        let params_str = serde_json::to_string(params)
            .unwrap_or_default()
            .to_lowercase();
        params_str.contains("status") || params_str.contains("poll") || params_str.contains("wait")
    }

    /// The narrow half of `is_poll_call`: poll intent the call itself declares, rather than poll intent inferred from a substring anywhere in its parameters.
    ///
    /// True for an explicit `"poll": true` parameter, or for a tool in `POLL_TOOLS` whose `command` matches a `POLL_COMMAND_PREFIXES` entry or contains a `POLL_KEYWORDS` entry — `docker ps`, `systemctl status`, `kubectl wait`, and the rest of the shell-shaped wait loop.
    ///
    /// This is the predicate for anything that either reaches the model or weakens the guard: [`Self::get_poll_backoff`], whose suggestion is rendered as advisory text on the tool result, and the [`Self::record_outcome`] multiplier, which decides how many identical answers a call may return before the next one is refused.
    /// Telling the author of `task_list {"status": "pending"}` to wait five seconds is simply wrong advice, and tripling that call's outcome budget weakens the rule for precisely the ordinary tools it was written for.
    fn is_declared_poll_call(tool_name: &str, params: &serde_json::Value) -> bool {
        // Explicit poll intent via params
        if let Some(poll) = params.get("poll").and_then(|v| v.as_bool()) {
            return poll;
        }

        // Known poll tools with poll-like commands (no length restriction)
        if !POLL_TOOLS.contains(&tool_name) {
            return false;
        }
        let Some(cmd) = params.get("command").and_then(|v| v.as_str()) else {
            return false;
        };
        let cmd_lower = cmd.to_lowercase();

        // Known poll command prefixes, then poll keywords anywhere in the command
        POLL_COMMAND_PREFIXES
            .iter()
            .any(|prefix| cmd_lower.starts_with(prefix))
            || POLL_KEYWORDS.iter().any(|kw| cmd_lower.contains(kw))
    }

    /// Detect ping-pong patterns (A-B-A-B or A-B-C-A-B-C) in recent call history.
    ///
    /// Checks if the last 6+ calls form a repeating pattern of length 2 or 3.
    /// Returns a warning message if a pattern is detected, `None` otherwise.
    fn detect_ping_pong(&self) -> Option<String> {
        self.detect_ping_pong_impl()
    }

    /// Pure version for stats (no &mut self needed, just reads state).
    fn detect_ping_pong_pure(&self) -> bool {
        self.detect_ping_pong_impl().is_some()
    }

    /// Shared ping-pong detection implementation.
    fn detect_ping_pong_impl(&self) -> Option<String> {
        let len = self.recent_calls.len();

        // Check for pattern of length 2 (A-B-A-B-A-B)
        // Need at least 6 entries for 3 repeats of length 2
        if len >= 6 {
            let tail = &self.recent_calls[len - 6..];
            let a = &tail[0];
            let b = &tail[1];
            if a != b && tail[2] == *a && tail[3] == *b && tail[4] == *a && tail[5] == *b {
                let tool_a = self
                    .hash_to_tool
                    .get(a)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let tool_b = self
                    .hash_to_tool
                    .get(b)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                return Some(format!(
                    "Ping-pong detected: tools '{}' and '{}' are alternating \
                     repeatedly. Break the cycle by trying a different approach.",
                    tool_a, tool_b
                ));
            }
        }

        // Check for pattern of length 3 (A-B-C-A-B-C-A-B-C)
        // Need at least 9 entries for 3 repeats of length 3
        if len >= 9 {
            let tail = &self.recent_calls[len - 9..];
            let a = &tail[0];
            let b = &tail[1];
            let c = &tail[2];
            // Ensure they're not all the same (that's just repetition, not ping-pong)
            if !(a == b && b == c)
                && tail[3] == *a
                && tail[4] == *b
                && tail[5] == *c
                && tail[6] == *a
                && tail[7] == *b
                && tail[8] == *c
            {
                let tool_a = self
                    .hash_to_tool
                    .get(a)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let tool_b = self
                    .hash_to_tool
                    .get(b)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                let tool_c = self
                    .hash_to_tool
                    .get(c)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string());
                return Some(format!(
                    "Ping-pong detected: tools '{}', '{}', '{}' are cycling \
                     repeatedly. Break the cycle by trying a different approach.",
                    tool_a, tool_b, tool_c
                ));
            }
        }

        None
    }

    /// Count how many full repeats of the detected ping-pong pattern exist
    /// in the recent call history.
    fn count_ping_pong_repeats(&self) -> u32 {
        let len = self.recent_calls.len();

        // Check pattern of length 2
        if len >= 4 {
            let a = &self.recent_calls[len - 2];
            let b = &self.recent_calls[len - 1];
            if a != b {
                let mut repeats: u32 = 0;
                let mut i = len;
                while i >= 2 {
                    i -= 2;
                    if self.recent_calls[i] == *a && self.recent_calls[i + 1] == *b {
                        repeats += 1;
                    } else {
                        break;
                    }
                }
                if repeats >= 2 {
                    return repeats;
                }
            }
        }

        // Check pattern of length 3
        if len >= 6 {
            let a = &self.recent_calls[len - 3];
            let b = &self.recent_calls[len - 2];
            let c = &self.recent_calls[len - 1];
            if !(a == b && b == c) {
                let mut repeats: u32 = 0;
                let mut i = len;
                while i >= 3 {
                    i -= 3;
                    if self.recent_calls[i] == *a
                        && self.recent_calls[i + 1] == *b
                        && self.recent_calls[i + 2] == *c
                    {
                        repeats += 1;
                    } else {
                        break;
                    }
                }
                if repeats >= 2 {
                    return repeats;
                }
            }
        }

        0
    }

    /// Compute a SHA-256 hash of the tool name and parameters.
    fn compute_hash(tool_name: &str, params: &serde_json::Value) -> String {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"|");
        // Serialize params deterministically (serde_json sorts object keys)
        let params_str = serde_json::to_string(params).unwrap_or_default();
        hasher.update(params_str.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Compute a SHA-256 hash of the tool name, parameters, AND result.
    ///
    /// Result is truncated to 1000 chars to avoid hashing huge outputs
    /// while still catching identical short results.
    fn compute_outcome_hash(tool_name: &str, params: &serde_json::Value, result: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"|");
        let params_str = serde_json::to_string(params).unwrap_or_default();
        hasher.update(params_str.as_bytes());
        hasher.update(b"|");
        let truncated = crate::str_utils::safe_truncate_str(result, 1000);
        hasher.update(truncated.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ========================================================================
    // Existing tests (preserved unchanged)
    // ========================================================================

    #[test]
    fn allow_below_threshold() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"query": "test"});
        let v = guard.check("web_search", &params);
        assert_eq!(v, LoopGuardVerdict::Allow);
        let v = guard.check("web_search", &params);
        assert_eq!(v, LoopGuardVerdict::Allow);
    }

    #[test]
    fn warn_at_threshold() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"path": "/etc/passwd"});
        // Calls 1, 2 = Allow
        guard.check("file_read", &params);
        guard.check("file_read", &params);
        // Call 3 = Warn (warn_threshold = 3)
        let v = guard.check("file_read", &params);
        assert!(matches!(v, LoopGuardVerdict::Warn(_)));
    }

    #[test]
    fn block_at_threshold() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"command": "ls"});
        for _ in 0..4 {
            guard.check("shell_exec", &params);
        }
        // Call 5 = Block (block_threshold = 5)
        let v = guard.check("shell_exec", &params);
        assert!(matches!(v, LoopGuardVerdict::Block(_)));
    }

    #[test]
    fn different_params_no_collision() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        for i in 0..10 {
            let params = serde_json::json!({"query": format!("query_{}", i)});
            let v = guard.check("web_search", &params);
            assert_eq!(v, LoopGuardVerdict::Allow);
        }
    }

    #[test]
    fn global_circuit_breaker() {
        let config = LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            global_circuit_breaker: 5,
            ..Default::default()
        };
        let mut guard = LoopGuard::new(config);
        for i in 0..5 {
            let params = serde_json::json!({"n": i});
            let v = guard.check("tool", &params);
            assert_eq!(v, LoopGuardVerdict::Allow);
        }
        // Call 6 triggers circuit breaker (> 5)
        let v = guard.check("tool", &serde_json::json!({"n": 5}));
        assert!(matches!(v, LoopGuardVerdict::CircuitBreak(_)));
    }

    #[test]
    fn default_config() {
        let config = LoopGuardConfig::default();
        assert_eq!(config.warn_threshold, 3);
        assert_eq!(config.block_threshold, 5);
        assert_eq!(config.global_circuit_breaker, 30);
    }

    // ========================================================================
    // New tests — Outcome-Aware Detection
    // ========================================================================

    #[test]
    fn test_outcome_aware_warning() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"query": "weather"});
        let result = "sunny 72F";

        // First outcome: no warning
        let w = guard.record_outcome("web_search", &params, result);
        assert!(w.is_none());

        // Second identical outcome: warning (outcome_warn_threshold = 2)
        let w = guard.record_outcome("web_search", &params, result);
        assert!(w.is_some());
        assert!(w.unwrap().contains("identical results"));
    }

    #[test]
    fn test_outcome_aware_blocks_next_call() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"query": "weather"});
        let result = "sunny 72F";

        // Record 3 identical outcomes (outcome_block_threshold = 3)
        guard.record_outcome("web_search", &params, result);
        guard.record_outcome("web_search", &params, result);
        let w = guard.record_outcome("web_search", &params, result);
        assert!(w.is_some());

        // The NEXT check() for this call hash should auto-block
        let v = guard.check("web_search", &params);
        assert!(matches!(v, LoopGuardVerdict::Block(_)));
        if let LoopGuardVerdict::Block(msg) = v {
            assert!(msg.contains("identical results"));
        }
    }

    #[test]
    fn test_outcome_thresholds_use_poll_multiplier() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        // `docker ps` is a known poll prefix, so this call is a poll call.
        // Defaults are outcome_warn = 2, outcome_block = 3 and poll_multiplier = 3, so a poll call warns on the 6th identical result and blocks on the 9th.
        let params = serde_json::json!({"command": "docker ps"});
        let result = "CONTAINER ID   IMAGE   STATUS";

        for i in 1..=5 {
            let w = guard.record_outcome("shell_exec", &params, result);
            assert!(
                w.is_none(),
                "identical poll result {i} warned before the relaxed threshold: {w:?}"
            );
        }

        // Sixth identical result: advisory only — an unchanged answer is what a wait loop looks like, so the call must still be runnable.
        assert!(guard
            .record_outcome("shell_exec", &params, result)
            .is_some());
        assert_eq!(
            guard.check("shell_exec", &params),
            LoopGuardVerdict::Allow,
            "a poll call must keep the relaxed thresholds `check` grants it"
        );

        // Results 7, 8, 9 reach outcome_block_threshold * poll_multiplier.
        for _ in 0..3 {
            guard.record_outcome("shell_exec", &params, result);
        }
        let v = guard.check("shell_exec", &params);
        assert!(
            matches!(v, LoopGuardVerdict::Block(_)),
            "a poll call repeating an identical answer 9 times is still a loop, got: {v:?}"
        );
    }

    #[test]
    fn test_outcome_thresholds_ignore_the_generic_poll_fallback() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        // `is_poll_call`'s third strategy scans the whole serialized parameter object for "status" / "poll" / "wait" for any tool name, so this ordinary listing reads as a poll to `check()`.
        // The outcome rule must not inherit that: tripling its budget here would weaken the guard 3x for exactly the tools it exists to stop.
        let params = serde_json::json!({"status": "pending"});
        let result = "No pending tasks.";

        assert!(LoopGuard::is_poll_call("task_list", &params));
        assert!(!LoopGuard::is_declared_poll_call("task_list", &params));

        assert!(guard.record_outcome("task_list", &params, result).is_none());
        assert!(guard.record_outcome("task_list", &params, result).is_some());
        assert!(guard.record_outcome("task_list", &params, result).is_some());

        let v = guard.check("task_list", &params);
        assert!(
            matches!(&v, LoopGuardVerdict::Block(msg) if msg.contains("identical results")),
            "the fourth identical listing must be blocked on the strict outcome threshold, got: {v:?}"
        );
    }

    #[test]
    fn test_poll_backoff_ignores_the_generic_poll_fallback() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        // The backoff is rendered as "wait roughly 5s before checking again" on the tool result, which is false advice for a call that is not a wait loop.
        let params = serde_json::json!({"goal_id": "g1", "status": "in_progress"});
        for i in 1..=4 {
            assert_eq!(
                guard.get_poll_backoff("goal_update", &params),
                None,
                "repeat {i} of an ordinary call must not be paced"
            );
        }

        // An explicit `"poll": true` is a declaration, so it still is paced.
        let declared = serde_json::json!({"goal_id": "g1", "poll": true});
        assert_eq!(guard.get_poll_backoff("goal_update", &declared), None);
        assert_eq!(guard.get_poll_backoff("goal_update", &declared), Some(5000));
    }

    #[test]
    fn test_is_declared_poll_call_detection() {
        // Explicit declaration, any tool.
        assert!(LoopGuard::is_declared_poll_call(
            "some_tool",
            &serde_json::json!({"poll": true})
        ));
        // Explicit `false` vetoes a command that would otherwise match a prefix.
        assert!(!LoopGuard::is_declared_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "docker ps", "poll": false})
        ));
        // Known prefix on a known poll tool.
        assert!(LoopGuard::is_declared_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "systemctl status nginx.service"})
        ));
        // Keyword anywhere in the command of a known poll tool.
        assert!(LoopGuard::is_declared_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "my-app --health-endpoint /api/v1/healthz"})
        ));
        // The same keyword outside `command` is not a declaration.
        assert!(!LoopGuard::is_declared_poll_call(
            "shell_exec",
            &serde_json::json!({"script": "systemctl status nginx.service"})
        ));
        // A poll-shaped command on a tool that is not in POLL_TOOLS is not a declaration.
        assert!(!LoopGuard::is_declared_poll_call(
            "browser_navigate",
            &serde_json::json!({"command": "docker ps"})
        ));
        // The generic fallback belongs to `is_poll_call` alone.
        assert!(!LoopGuard::is_declared_poll_call(
            "queue",
            &serde_json::json!({"mode": "wait_for_completion"})
        ));
        assert!(LoopGuard::is_poll_call(
            "queue",
            &serde_json::json!({"mode": "wait_for_completion"})
        ));
    }

    // ========================================================================
    // New tests — Ping-Pong Detection
    // ========================================================================

    #[test]
    fn test_ping_pong_ab_detection() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            // Set thresholds high so individual hash counting doesn't interfere
            warn_threshold: 100,
            block_threshold: 100,
            ping_pong_min_repeats: 3,
            ..Default::default()
        });
        let params_a = serde_json::json!({"file": "a.txt"});
        let params_b = serde_json::json!({"file": "b.txt"});

        // A-B-A-B-A-B = 3 repeats of (A,B)
        guard.check("file_read", &params_a);
        guard.check("file_write", &params_b);
        guard.check("file_read", &params_a);
        guard.check("file_write", &params_b);
        guard.check("file_read", &params_a);
        let v = guard.check("file_write", &params_b);

        // Should detect ping-pong and block (3 full repeats)
        assert!(
            matches!(v, LoopGuardVerdict::Block(ref msg) if msg.contains("Ping-pong"))
                || matches!(v, LoopGuardVerdict::Warn(ref msg) if msg.contains("Ping-pong")),
            "Expected ping-pong detection, got: {:?}",
            v
        );
    }

    #[test]
    fn test_ping_pong_abc_detection() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            ping_pong_min_repeats: 3,
            ..Default::default()
        });
        let params_a = serde_json::json!({"a": 1});
        let params_b = serde_json::json!({"b": 2});
        let params_c = serde_json::json!({"c": 3});

        // A-B-C-A-B-C-A-B-C = 3 repeats of (A,B,C)
        for _ in 0..3 {
            guard.check("tool_a", &params_a);
            guard.check("tool_b", &params_b);
            guard.check("tool_c", &params_c);
        }

        // The pattern should be detected by the 9th call
        let stats = guard.stats();
        assert!(stats.ping_pong_detected);
    }

    #[test]
    fn test_no_false_ping_pong() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());

        // Various different calls — no pattern
        for i in 0..10 {
            let params = serde_json::json!({"n": i});
            guard.check("tool", &params);
        }

        let stats = guard.stats();
        assert!(!stats.ping_pong_detected);
    }

    // ========================================================================
    // New tests — Poll Tool Handling
    // ========================================================================

    #[test]
    fn test_poll_tool_relaxed_thresholds() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        // shell_exec with short status-check command = poll call
        // Default thresholds: warn=3, block=5, poll_multiplier=3
        // Effective for poll: warn=9, block=15
        let params = serde_json::json!({"command": "docker ps --status running"});

        // Calls 1..8 should all be Allow (below warn=9)
        for _ in 0..8 {
            let v = guard.check("shell_exec", &params);
            assert_eq!(
                v,
                LoopGuardVerdict::Allow,
                "Poll tool should have relaxed thresholds"
            );
        }

        // Call 9 should be Warn
        let v = guard.check("shell_exec", &params);
        assert!(
            matches!(v, LoopGuardVerdict::Warn(_)),
            "Expected warn at poll threshold, got: {:?}",
            v
        );
    }

    #[test]
    fn test_is_poll_call_detection() {
        // shell_exec with status-check command
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "docker ps --status"})
        ));

        // shell_exec with tail command
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "tail -f /var/log/app.log"})
        ));

        // shell_exec with command but NO poll keywords — NOT a poll
        assert!(!LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "echo hi"})
        ));

        // Long poll command IS correctly detected (no length limit)
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "kubectl get pods -n production-namespace --field-selector=status.phase=Running"})
        ));

        // Non-poll tool with no poll keywords
        assert!(!LoopGuard::is_poll_call(
            "file_read",
            &serde_json::json!({"path": "/etc/hosts"})
        ));

        // Generic poll detection via params containing "status"
        assert!(LoopGuard::is_poll_call(
            "some_tool",
            &serde_json::json!({"check": "status"})
        ));

        // Generic poll detection via params containing "poll"
        assert!(LoopGuard::is_poll_call(
            "api_call",
            &serde_json::json!({"action": "poll_results"})
        ));

        // Generic poll detection via params containing "wait"
        assert!(LoopGuard::is_poll_call(
            "queue",
            &serde_json::json!({"mode": "wait_for_completion"})
        ));

        // Explicit poll=true parameter marks any call as poll
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "some-custom-command", "poll": true})
        ));

        // Explicit poll=false parameter overrides detection
        assert!(!LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "docker ps", "poll": false})
        ));

        // Known prefix: systemctl status
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "systemctl status nginx.service"})
        ));

        // Known prefix: curl -s (health check)
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "curl -s http://localhost:8080/health"})
        ));

        // Known prefix: docker inspect
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "docker inspect --format '{{.State.Status}}' my-container"})
        ));

        // Keyword: health
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "my-app --health-endpoint /api/v1/healthz"})
        ));

        // Keyword: check
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "pg_isready --check --host=db.example.com --port=5432"})
        ));
    }

    // ========================================================================
    // New tests — Backoff Schedule
    // ========================================================================

    #[test]
    fn test_poll_backoff_schedule() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"command": "kubectl get pods --status"});

        // First call: no backoff
        let b = guard.get_poll_backoff("shell_exec", &params);
        assert_eq!(b, None);

        // Second call: 5000ms
        let b = guard.get_poll_backoff("shell_exec", &params);
        assert_eq!(b, Some(5000));

        // Third call: 10000ms
        let b = guard.get_poll_backoff("shell_exec", &params);
        assert_eq!(b, Some(10000));

        // Fourth call: 30000ms
        let b = guard.get_poll_backoff("shell_exec", &params);
        assert_eq!(b, Some(30000));

        // Fifth call: 60000ms
        let b = guard.get_poll_backoff("shell_exec", &params);
        assert_eq!(b, Some(60000));

        // Sixth call: caps at 60000ms
        let b = guard.get_poll_backoff("shell_exec", &params);
        assert_eq!(b, Some(60000));

        // Non-poll tool: no backoff
        let non_poll = serde_json::json!({"path": "/etc/hosts"});
        let b = guard.get_poll_backoff("file_read", &non_poll);
        assert_eq!(b, None);
    }

    // ========================================================================
    // New tests — Warning Bucket
    // ========================================================================

    #[test]
    fn test_warning_bucket_limits() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 2,
            block_threshold: 100, // set very high so only warning bucket triggers block
            max_warnings_per_call: 2,
            ..Default::default()
        });
        let params = serde_json::json!({"x": 1});

        // Call 1: Allow
        let v = guard.check("tool", &params);
        assert_eq!(v, LoopGuardVerdict::Allow);

        // Call 2: Warn (hits warn_threshold=2), warning_count = 1
        let v = guard.check("tool", &params);
        assert!(matches!(v, LoopGuardVerdict::Warn(_)));

        // Call 3: Warn again, warning_count = 2
        let v = guard.check("tool", &params);
        assert!(matches!(v, LoopGuardVerdict::Warn(_)));

        // Call 4: warning_count would be 3, exceeds max_warnings_per_call=2 -> Block
        let v = guard.check("tool", &params);
        assert!(
            matches!(v, LoopGuardVerdict::Block(_)),
            "Expected block after warning limit, got: {:?}",
            v
        );
    }

    #[test]
    fn test_warning_upgrade_to_block() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 1,
            block_threshold: 100,
            max_warnings_per_call: 1,
            ..Default::default()
        });
        let params = serde_json::json!({"y": 2});

        // Call 1: Warn (warn_threshold=1), warning_count = 1
        let v = guard.check("tool", &params);
        assert!(matches!(v, LoopGuardVerdict::Warn(_)));

        // Call 2: warning_count would be 2, exceeds max_warnings_per_call=1 -> Block
        let v = guard.check("tool", &params);
        assert!(
            matches!(v, LoopGuardVerdict::Block(ref msg) if msg.contains("warnings exhausted")),
            "Expected block with 'warnings exhausted', got: {:?}",
            v
        );
    }

    // ========================================================================
    // New tests — Statistics Snapshot
    // ========================================================================

    #[test]
    fn test_stats_snapshot() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params_a = serde_json::json!({"a": 1});
        let params_b = serde_json::json!({"b": 2});

        // 3 calls to tool_a, 1 to tool_b
        guard.check("tool_a", &params_a);
        guard.check("tool_a", &params_a);
        guard.check("tool_a", &params_a);
        guard.check("tool_b", &params_b);

        let stats = guard.stats();
        assert_eq!(stats.total_calls, 4);
        assert_eq!(stats.unique_calls, 2);
        assert_eq!(stats.most_repeated_tool, Some("tool_a".to_string()));
        assert_eq!(stats.most_repeated_count, 3);
        assert!(!stats.ping_pong_detected);
    }

    // ========================================================================
    // New tests — History Ring Buffer
    // ========================================================================

    #[test]
    fn test_history_ring_buffer_limit() {
        let config = LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            global_circuit_breaker: 200,
            ..Default::default()
        };
        let mut guard = LoopGuard::new(config);

        // Push 50 unique calls (exceeds HISTORY_SIZE of 30)
        for i in 0..50 {
            let params = serde_json::json!({"n": i});
            guard.check("tool", &params);
        }

        // Internal ring buffer should be capped at HISTORY_SIZE
        assert_eq!(guard.recent_calls.len(), HISTORY_SIZE);

        // Stats should reflect all 50 calls
        let stats = guard.stats();
        assert_eq!(stats.total_calls, 50);
        assert_eq!(stats.unique_calls, 50);
    }
}
