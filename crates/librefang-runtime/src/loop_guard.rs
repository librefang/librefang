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
use std::collections::{HashMap, HashSet, VecDeque};
use tracing::{debug, warn};

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

/// Substring patterns in commands that strongly indicate polling intent.
/// These are matched with simple `contains_ignore_case` (no word-boundary
/// check needed because the patterns already include spaces or are unique
/// enough to avoid false positives).
const POLL_SUBSTRINGS: &[&str] = &["ps "];

/// Bare-word keywords in commands that indicate polling intent.
/// Matched with word-boundary awareness so e.g. "tail" won't match "detail",
/// and "check" won't match "healthcheck".
const POLL_KEYWORDS: &[&str] = &[
    "status", "poll", "wait", "watch", "tail", "jobs", "pgrep", "health", "ping", "alive", "ready",
    "check",
];

/// Maximum recent call history size for ping-pong detection.
const HISTORY_SIZE: usize = 30;

/// Backoff schedule in milliseconds for polling tools.
const BACKOFF_SCHEDULE_MS: &[u64] = &[5000, 10000, 30000, 60000];

/// Configuration for the loop guard.
///
/// Re-exported from `librefang_types::config::LoopGuardTomlConfig` to avoid
/// duplicating the struct across crates.
pub type LoopGuardConfig = librefang_types::config::LoopGuardTomlConfig;

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
    /// Count of identical (tool_name + params) calls, keyed by SHA-256 raw hash.
    call_counts: HashMap<[u8; 32], u32>,
    /// Total tool calls in this loop execution.
    total_calls: u32,
    /// Count of identical (tool_call_hash + result_hash) pairs.
    outcome_counts: HashMap<[u8; 32], u32>,
    /// Call hashes that are blocked due to repeated identical outcomes.
    blocked_outcomes: HashSet<[u8; 32]>,
    /// Recent tool call hashes (ring buffer of last HISTORY_SIZE).
    recent_calls: VecDeque<[u8; 32]>,
    /// Warnings already emitted (to prevent spam). Key = call hash, value = count emitted.
    warnings_emitted: HashMap<[u8; 32], u32>,
    /// Tracks poll counts per command hash for backoff suggestions.
    poll_counts: HashMap<[u8; 32], u32>,
    /// Total calls that were blocked.
    blocked_calls: u32,
    /// Map from call hash to tool name (for stats reporting).
    hash_to_tool: HashMap<[u8; 32], String>,
    /// Cached hash from last check() call to avoid recomputation.
    last_call_hash: Option<[u8; 32]>,
    /// Cached flag: set once a ping-pong pattern has been detected.
    ping_pong_detected: bool,
}

impl LoopGuard {
    /// Create a new loop guard with the given configuration.
    ///
    /// Nonsensical values (e.g. zero thresholds, warn > block) are clamped and
    /// logged so that a misconfigured TOML file cannot silently disable the guard.
    pub fn new(mut config: LoopGuardConfig) -> Self {
        // Clamp/fix nonsensical values
        if config.global_circuit_breaker == 0 {
            config.global_circuit_breaker = 30;
            warn!("loop_guard: global_circuit_breaker=0 makes no sense, defaulting to 30");
        }
        if config.block_threshold == 0 {
            config.block_threshold = 5;
            warn!("loop_guard: block_threshold=0 makes no sense, defaulting to 5");
        }
        if config.poll_multiplier == 0 {
            config.poll_multiplier = 1;
            warn!("loop_guard: poll_multiplier=0 would block all poll calls, defaulting to 1");
        }
        if config.warn_threshold > config.block_threshold {
            std::mem::swap(&mut config.warn_threshold, &mut config.block_threshold);
            warn!(
                "loop_guard: warn_threshold > block_threshold makes no sense, swapped to warn={}, block={}",
                config.warn_threshold, config.block_threshold
            );
        }

        Self {
            config,
            call_counts: HashMap::with_capacity(16),
            total_calls: 0,
            outcome_counts: HashMap::new(),
            blocked_outcomes: HashSet::new(),
            recent_calls: VecDeque::with_capacity(HISTORY_SIZE),
            warnings_emitted: HashMap::new(),
            poll_counts: HashMap::new(),
            blocked_calls: 0,
            hash_to_tool: HashMap::with_capacity(16),
            last_call_hash: None,
            ping_pong_detected: false,
        }
    }

    /// Check whether a tool call should proceed.
    ///
    /// Returns a verdict indicating whether to allow, warn, block, or
    /// circuit-break. The caller should act on the verdict before executing
    /// the tool.
    pub fn check(&mut self, tool_name: &str, params: &serde_json::Value) -> LoopGuardVerdict {
        self.total_calls += 1;
        debug!(tool = %tool_name, total = self.total_calls, "Loop guard check");

        // Global circuit breaker
        if self.total_calls > self.config.global_circuit_breaker {
            self.blocked_calls += 1;
            warn!(tool = %tool_name, total = self.total_calls, limit = self.config.global_circuit_breaker, "Circuit breaker triggered");
            return LoopGuardVerdict::CircuitBreak(format!(
                "Circuit breaker: exceeded {} total tool calls in this loop. \
                 The agent appears to be stuck.",
                self.config.global_circuit_breaker
            ));
        }

        let hash = Self::compute_hash(tool_name, params);
        self.last_call_hash = Some(hash);
        self.hash_to_tool
            .entry(hash)
            .or_insert_with(|| tool_name.to_string());

        // Track recent calls for ping-pong detection
        if self.recent_calls.len() >= HISTORY_SIZE {
            self.recent_calls.pop_front();
        }
        self.recent_calls.push_back(hash);

        // Check if this call hash was blocked by outcome detection
        if self.blocked_outcomes.contains(&hash) {
            self.blocked_calls += 1;
            warn!(tool = %tool_name, "Blocked by outcome detection");
            return LoopGuardVerdict::Block(format!(
                "Blocked: tool '{}' is returning identical results repeatedly. \
                 The current approach is not working — try something different.",
                tool_name
            ));
        }

        let count = self.call_counts.entry(hash).or_insert(0);
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
            warn!(tool = %tool_name, count = count_val, "Blocked by threshold");
            return LoopGuardVerdict::Block(format!(
                "Blocked: tool '{}' called {} times with identical parameters. \
                 Try a different approach or different parameters.",
                tool_name, count_val
            ));
        }

        if count_val >= effective_warn {
            // Warning bucket: check if we've already warned too many times
            let warning_count = self.warnings_emitted.entry(hash).or_insert(0);
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
            debug!(tool = %tool_name, count = count_val, "Warning threshold reached");
            return LoopGuardVerdict::Warn(format!(
                "Warning: tool '{}' has been called {} times with identical parameters. \
                 Consider a different approach.",
                tool_name, count_val
            ));
        }

        // Ping-pong detection (runs even if individual hash counts are low)
        if let Some((ping_pong_msg, repeats, pattern_hashes)) = self.detect_ping_pong() {
            if repeats >= self.config.ping_pong_min_repeats {
                self.ping_pong_detected = true;
                self.blocked_calls += 1;
                return LoopGuardVerdict::Block(ping_pong_msg);
            }
            // Below min_repeats, just warn
            // Derive a stable key from the pattern hashes for warning dedup.
            let pattern_key = Self::compute_pattern_key(&pattern_hashes);
            let warning_count = self.warnings_emitted.entry(pattern_key).or_insert(0);
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
    pub fn record_outcome(
        &mut self,
        tool_name: &str,
        params: &serde_json::Value,
        result: &str,
    ) -> Option<String> {
        let outcome_hash = Self::compute_outcome_hash(tool_name, params, result);
        let call_hash = self
            .last_call_hash
            .unwrap_or_else(|| Self::compute_hash(tool_name, params));

        let count = self.outcome_counts.entry(outcome_hash).or_insert(0);
        *count += 1;
        let count_val = *count;

        if count_val >= self.config.outcome_block_threshold {
            // Mark the call hash so the NEXT check() auto-blocks it
            self.blocked_outcomes.insert(call_hash);
            warn!(tool = %tool_name, count = count_val, "Identical outcome detected");
            return Some(format!(
                "Tool '{}' is returning identical results — the approach isn't working.",
                tool_name
            ));
        }

        if count_val >= self.config.outcome_warn_threshold {
            warn!(tool = %tool_name, count = count_val, "Identical outcome detected");
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
    pub fn get_poll_backoff(&mut self, tool_name: &str, params: &serde_json::Value) -> Option<u64> {
        if !Self::is_poll_call(tool_name, params) {
            return None;
        }
        let hash = self
            .last_call_hash
            .unwrap_or_else(|| Self::compute_hash(tool_name, params));
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
            ping_pong_detected: self.ping_pong_detected,
            most_repeated_tool,
            most_repeated_count,
        }
    }

    /// Check if a tool call looks like a polling operation.
    ///
    /// Poll tools (like `shell_exec` for status checks) are expected to be
    /// called repeatedly and get relaxed loop detection thresholds.
    ///
    /// Detection uses three strategies (no arbitrary length limits):
    /// 1. Explicit `"poll": true` parameter — callers can mark poll intent directly.
    /// 2. Known command prefix matching — e.g. `docker ps`, `kubectl get`.
    /// 3. Keyword matching — e.g. `status`, `poll`, `health`, `check`.
    fn is_poll_call(tool_name: &str, params: &serde_json::Value) -> bool {
        // Explicit poll intent via params
        if let Some(poll) = params.get("poll").and_then(|v| v.as_bool()) {
            return poll;
        }

        // Known poll tools with poll-like commands (no length restriction)
        if POLL_TOOLS.contains(&tool_name) {
            if let Some(cmd) = params.get("command").and_then(|v| v.as_str()) {
                // Check known poll command prefixes (ASCII case-insensitive, no allocation)
                if POLL_COMMAND_PREFIXES
                    .iter()
                    .any(|prefix| starts_with_ignore_case(cmd, prefix))
                {
                    return true;
                }

                // Check substring patterns (e.g. "ps ") — simple contains, no word boundary
                if POLL_SUBSTRINGS
                    .iter()
                    .any(|pat| contains_ignore_case(cmd, pat))
                {
                    return true;
                }

                // Check bare-word keywords with word-boundary awareness
                if POLL_KEYWORDS
                    .iter()
                    .any(|kw| contains_keyword_ignore_case(cmd, kw))
                {
                    return true;
                }
            }

            // Check param keys for poll-related keywords (values are ignored, no allocation)
            if let Some(obj) = params.as_object() {
                for key in obj.keys() {
                    if contains_keyword_ignore_case(key, "status")
                        || contains_keyword_ignore_case(key, "poll")
                        || contains_keyword_ignore_case(key, "wait")
                    {
                        return true;
                    }
                }
            }
        }

        false
    }

    /// Detect ping-pong patterns (A-B-A-B or A-B-C-A-B-C) in recent call history.
    ///
    /// Checks pattern length 3 first (more specific), then length 2 (more
    /// general), since a length-3 pattern also contains a length-2 subpattern.
    /// Returns `(message, repeats, pattern_hashes)` if a pattern is detected,
    /// `None` otherwise.
    fn detect_ping_pong(&self) -> Option<(String, u32, Vec<[u8; 32]>)> {
        self.detect_pattern(3).or_else(|| self.detect_pattern(2))
    }

    /// Detect a repeating pattern of the given length in recent call history.
    ///
    /// For `pattern_len = 2`, looks for A-B-A-B-A-B (needs >= 6 entries).
    /// For `pattern_len = 3`, looks for A-B-C-A-B-C-A-B-C (needs >= 9 entries).
    ///
    /// Uses direct VecDeque indexing instead of allocating temporary Vecs.
    /// Returns `Some((message, repeats, pattern_hashes))` when detected.
    fn detect_pattern(&self, pattern_len: usize) -> Option<(String, u32, Vec<[u8; 32]>)> {
        let len = self.recent_calls.len();
        // Need at least pattern_len * 3 entries for 3 repeats
        let required = pattern_len * 3;
        if len < required {
            return None;
        }

        // Extract the pattern (first `pattern_len` entries of the last `required` entries)
        let start = len - required;
        let pattern: Vec<[u8; 32]> = (0..pattern_len)
            .map(|j| self.recent_calls[start + j])
            .collect();

        // All pattern entries must not be identical (that's repetition, not ping-pong)
        let all_same = pattern.windows(2).all(|w| w[0] == w[1]);
        if all_same {
            return None;
        }

        // Verify the last `required` entries match the repeating pattern
        for j in 0..required {
            if self.recent_calls[start + j] != pattern[j % pattern_len] {
                return None;
            }
        }

        // Count full pattern repeats backwards from the end
        let mut repeats: u32 = 0;
        let mut i = len;
        while i >= pattern_len {
            i -= pattern_len;
            let chunk_matches = pattern
                .iter()
                .enumerate()
                .all(|(j, &p)| self.recent_calls[i + j] == p);
            if chunk_matches {
                repeats += 1;
            } else {
                break;
            }
        }

        // Resolve tool names from pattern hashes
        let tool_names: Vec<String> = pattern
            .iter()
            .map(|h| {
                self.hash_to_tool
                    .get(h)
                    .cloned()
                    .unwrap_or_else(|| "unknown".to_string())
            })
            .collect();

        debug!(
            repeats = repeats,
            pattern_len = pattern_len,
            "Ping-pong pattern detected"
        );

        let msg = if pattern_len == 2 {
            format!(
                "Ping-pong detected: tools '{}' and '{}' are alternating \
                 repeatedly. Break the cycle by trying a different approach.",
                tool_names[0], tool_names[1]
            )
        } else {
            format!(
                "Ping-pong detected: tools {} are cycling \
                 repeatedly. Break the cycle by trying a different approach.",
                tool_names
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        if i == tool_names.len() - 1 {
                            format!("and '{}'", name)
                        } else {
                            format!("'{}'", name)
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };

        Some((msg, repeats, pattern))
    }

    /// Compute a SHA-256 hash of the tool name and parameters.
    fn compute_hash(tool_name: &str, params: &serde_json::Value) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"|");
        // Serialize params deterministically (serde_json sorts object keys).
        // serde_json::Value IS the JSON AST so this can't actually fail, but
        // guard against programming errors in debug builds.
        let params_bytes = serde_json::to_vec(params).unwrap_or_else(|e| {
            debug_assert!(false, "serde_json::Value should always serialize: {e}");
            Vec::new()
        });
        hasher.update(&params_bytes);
        hasher.finalize().into()
    }

    /// Compute a SHA-256 hash of the tool name, parameters, AND result.
    ///
    /// For results <= 1000 bytes, hashes the entire content. For larger results,
    /// hashes the first 500 bytes + a separator + the last 500 bytes, preserving
    /// both prefix and suffix identity while avoiding huge-hash overhead.
    fn compute_outcome_hash(tool_name: &str, params: &serde_json::Value, result: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"|");
        let params_bytes = serde_json::to_vec(params).unwrap_or_else(|e| {
            debug_assert!(false, "serde_json::Value should always serialize: {e}");
            Vec::new()
        });
        hasher.update(&params_bytes);
        hasher.update(b"|");
        let bytes = result.as_bytes();
        if bytes.len() <= 1000 {
            hasher.update(bytes);
        } else {
            hasher.update(&bytes[..500]);
            hasher.update(b"|TRUNCATED|");
            hasher.update(&bytes[bytes.len() - 500..]);
        }
        hasher.finalize().into()
    }

    /// Derive a stable `[u8; 32]` key from pattern hashes for warning dedup.
    fn compute_pattern_key(pattern_hashes: &[[u8; 32]]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for h in pattern_hashes {
            hasher.update(h);
        }
        hasher.finalize().into()
    }
}

// ---------------------------------------------------------------------------
// Private free helpers — allocation-free, ASCII case-insensitive matching
// ---------------------------------------------------------------------------

/// ASCII case-insensitive prefix check. No allocations.
fn starts_with_ignore_case(haystack: &str, needle: &str) -> bool {
    if haystack.len() < needle.len() {
        return false;
    }
    haystack[..needle.len()].eq_ignore_ascii_case(needle)
}

/// ASCII case-insensitive substring check (sliding window). No allocations.
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if h.len() < n.len() {
        return false;
    }
    h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// ASCII case-insensitive keyword match with word-boundary awareness.
///
/// A match only counts when the keyword is bounded by non-alphanumeric
/// characters (or string start/end). This prevents e.g. "tail" from
/// matching "detail", or "check" from matching "healthcheck".
fn contains_keyword_ignore_case(haystack: &str, needle: &str) -> bool {
    let h = haystack.as_bytes();
    let n = needle.as_bytes();
    if h.len() < n.len() {
        return false;
    }
    for i in 0..=h.len() - n.len() {
        if h[i..i + n.len()].eq_ignore_ascii_case(n) {
            let before_ok = i == 0 || !h[i - 1].is_ascii_alphanumeric();
            let after_ok = i + n.len() >= h.len() || !h[i + n.len()].is_ascii_alphanumeric();
            if before_ok && after_ok {
                return true;
            }
        }
    }
    false
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
            matches!(v, LoopGuardVerdict::Block(ref msg) if msg.contains("Ping-pong")),
            "Expected Block for ping-pong with 3+ repeats, got: {:?}",
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

        // Poll detection via command containing "status" keyword
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "check status of service"})
        ));

        // Poll detection via command containing "poll" keyword
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "poll for results"})
        ));

        // Poll detection via command containing "wait" keyword
        assert!(LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "wait for completion"})
        ));

        // Non-POLL_TOOLS with status in value should NOT be detected as poll
        assert!(!LoopGuard::is_poll_call(
            "some_tool",
            &serde_json::json!({"check": "status"})
        ));

        // Non-POLL_TOOLS with poll in value should NOT be detected as poll
        assert!(!LoopGuard::is_poll_call(
            "api_call",
            &serde_json::json!({"action": "poll_results"})
        ));

        // Non-POLL_TOOLS with wait in value should NOT be detected as poll
        assert!(!LoopGuard::is_poll_call(
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

    // ========================================================================
    // New tests — Blocked Calls Count, Config Correction, Outcome Hashing
    // ========================================================================

    #[test]
    fn test_stats_blocked_calls_count() {
        // warn must be < block to avoid auto-swap in new()
        let config = LoopGuardConfig {
            warn_threshold: 2,
            block_threshold: 3,
            ..LoopGuardConfig::default()
        };
        let mut guard = LoopGuard::new(config);
        let params = serde_json::json!({"x": 1});

        // 3 calls to trigger block at threshold 3
        guard.check("tool", &params); // 1 - Allow
        guard.check("tool", &params); // 2 - Warn (warn_threshold=2)
        guard.check("tool", &params); // 3 - Block

        let stats = guard.stats();
        assert_eq!(stats.blocked_calls, 1, "Should have exactly 1 blocked call");

        // One more blocked call
        guard.check("tool", &params); // 4 - Block again
        let stats = guard.stats();
        assert_eq!(stats.blocked_calls, 2, "Should have 2 blocked calls");
    }

    #[test]
    fn test_different_outcomes_no_false_positive() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"query": "test"});

        // Same call, DIFFERENT results each time
        let w = guard.record_outcome("tool", &params, "result A");
        assert!(w.is_none());
        let w = guard.record_outcome("tool", &params, "result B");
        assert!(w.is_none(), "Different results should NOT trigger warning");
        let w = guard.record_outcome("tool", &params, "result C");
        assert!(w.is_none(), "Different results should NOT trigger warning");

        // Should NOT be blocked — outcomes were all different
        let v = guard.check("tool", &params);
        assert_eq!(
            v,
            LoopGuardVerdict::Allow,
            "Should still be allowed — different outcomes"
        );
    }

    #[test]
    fn test_zero_poll_multiplier_corrected() {
        let config = LoopGuardConfig {
            poll_multiplier: 0,
            ..LoopGuardConfig::default()
        };
        let mut guard = LoopGuard::new(config);
        let params = serde_json::json!({"command": "docker ps"});

        // Should NOT immediately block despite poll_multiplier=0
        // (new() corrects it to 1)
        let v = guard.check("shell_exec", &params);
        assert_eq!(
            v,
            LoopGuardVerdict::Allow,
            "poll_multiplier=0 should be auto-corrected"
        );
    }

    #[test]
    fn test_warn_gt_block_corrected() {
        let config = LoopGuardConfig {
            warn_threshold: 10,
            block_threshold: 2,
            ..LoopGuardConfig::default()
        };
        let mut guard = LoopGuard::new(config);
        let params = serde_json::json!({"x": 1});

        // After correction: warn=2, block=10 (swapped)
        // Call 2 should warn (not block)
        guard.check("tool", &params);
        let v = guard.check("tool", &params);
        // Should get Warn, not Block — because values were swapped
        assert!(
            matches!(v, LoopGuardVerdict::Warn(_)),
            "warn/block swap should produce Warn at call 2, got: {:?}",
            v
        );
    }

    #[test]
    fn test_zero_global_circuit_breaker_corrected() {
        let config = LoopGuardConfig {
            global_circuit_breaker: 0,
            ..LoopGuardConfig::default()
        };
        let mut guard = LoopGuard::new(config);
        let params = serde_json::json!({"x": 1});

        // Should NOT circuit-break on first call (0 was corrected to 30)
        let v = guard.check("tool", &params);
        assert_eq!(
            v,
            LoopGuardVerdict::Allow,
            "global_circuit_breaker=0 should be auto-corrected"
        );
    }


    // ========================================================================
    // New tests — Private Helper Function Coverage
    // ========================================================================

    #[test]
    fn test_starts_with_ignore_case() {
        // haystack shorter than needle
        assert!(!starts_with_ignore_case("ab", "abc"));
        // case-insensitive match
        assert!(starts_with_ignore_case("DOCKER ps", "docker"));
        assert!(starts_with_ignore_case("Hello World", "HELLO"));
        // case-insensitive non-match
        assert!(!starts_with_ignore_case("Hello World", "world"));
        // empty needle matches
        assert!(starts_with_ignore_case("anything", ""));
        // empty haystack with non-empty needle
        assert!(!starts_with_ignore_case("", "x"));
    }

    #[test]
    fn test_contains_ignore_case() {
        // haystack shorter than needle
        assert!(!contains_ignore_case("ab", "abc"));
        // match in middle
        assert!(contains_ignore_case("fooBARbaz", "bar"));
        // no match
        assert!(!contains_ignore_case("hello world", "xyz"));
        // match at start
        assert!(contains_ignore_case("PS aux", "ps "));
        // match at end
        assert!(contains_ignore_case("run ps ", "ps "));
        // case-insensitive
        assert!(contains_ignore_case("xxxPS yyy", "ps "));
    }

    #[test]
    fn test_contains_keyword_ignore_case_word_boundary() {
        // "tail" embedded in "detail" → false (key reason this function exists)
        assert!(!contains_keyword_ignore_case("detail", "tail"));
        // "check" embedded in "healthcheck" → false
        assert!(!contains_keyword_ignore_case("healthcheck", "check"));
        // match at string start → true (no preceding char = word boundary)
        assert!(contains_keyword_ignore_case("tail -f /var/log", "tail"));
        // match at string end → true (no following char = word boundary)
        assert!(contains_keyword_ignore_case("run status", "status"));
        // surrounded by non-alphanumeric (dash) → true
        assert!(contains_keyword_ignore_case("--check-status", "check"));
        assert!(contains_keyword_ignore_case("--check-status", "status"));
        // surrounded by spaces → true
        assert!(contains_keyword_ignore_case("check status now", "status"));
        // case-insensitive with word boundary
        assert!(contains_keyword_ignore_case("HEALTH endpoint", "health"));
    }

    // ========================================================================
    // New tests — Poll Detection Sub-Path Coverage
    // ========================================================================

    #[test]
    fn test_poll_param_non_boolean_falls_through() {
        // "poll": 1 is an integer, not bool — as_bool() returns None, falls through.
        // Detection still succeeds because "docker ps" matches POLL_COMMAND_PREFIXES.
        let result = LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "docker ps", "poll": 1}),
        );
        assert!(result);
    }

    #[test]
    fn test_poll_substring_ps_match() {
        // "ps aux" does NOT match any POLL_COMMAND_PREFIXES (none start with "ps "),
        // but it DOES match POLL_SUBSTRINGS via contains_ignore_case for "ps ".
        let result = LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "ps aux"}),
        );
        assert!(result);
    }

    #[test]
    fn test_poll_detection_via_param_key_status() {
        // "custom-cmd" doesn't match any prefix/substring/keyword,
        // but the param key "status_check" contains "status".
        let result = LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "custom-cmd", "status_check": true}),
        );
        assert!(result);
    }

    #[test]
    fn test_poll_detection_via_param_key_poll() {
        // Param key "poll_interval" contains "poll".
        let result = LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "custom-cmd", "poll_interval": 5000}),
        );
        assert!(result);
    }

    #[test]
    fn test_poll_detection_via_param_key_wait() {
        // Param key "wait_timeout" contains "wait".
        let result = LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!({"command": "custom-cmd", "wait_timeout": 30}),
        );
        assert!(result);
    }

    #[test]
    fn test_is_poll_call_null_params() {
        // Null has no "poll" key, no "command" key, is not an object.
        let result = LoopGuard::is_poll_call("shell_exec", &serde_json::Value::Null);
        assert!(!result);
    }

    #[test]
    fn test_is_poll_call_array_params() {
        // Array has no "poll" key, no "command" key, as_object() returns None.
        let result = LoopGuard::is_poll_call(
            "shell_exec",
            &serde_json::json!([1, 2, 3]),
        );
        assert!(!result);
    }

    #[test]
    fn test_non_poll_tool_explicit_poll_true() {
        // "custom_tool" is NOT in POLL_TOOLS, but explicit poll=true works
        // for any tool (first check in is_poll_call, before POLL_TOOLS gate).
        let result = LoopGuard::is_poll_call(
            "custom_tool",
            &serde_json::json!({"poll": true}),
        );
        assert!(result);
    }

    // ========================================================================
    // Config Correction Edge Cases
    // ========================================================================

    #[test]
    fn test_zero_block_threshold_corrected() {
        let config = LoopGuardConfig {
            block_threshold: 0,
            ..LoopGuardConfig::default()
        };
        let mut guard = LoopGuard::new(config);
        let params = serde_json::json!({"x": 1});

        // block_threshold=0 is corrected to 5 in new()
        // warn_threshold remains default 3
        // Calls 1-4: Allow or Warn (count < corrected block=5)
        for _ in 0..4 {
            let v = guard.check("tool", &params);
            assert!(
                matches!(v, LoopGuardVerdict::Allow | LoopGuardVerdict::Warn(_)),
                "Calls 1-4 should be Allow or Warn, got: {:?}",
                v
            );
        }

        // Call 5: count==5 hits corrected block_threshold, should Block
        let v = guard.check("tool", &params);
        assert!(
            matches!(v, LoopGuardVerdict::Block(_)),
            "5th call should be Block after block_threshold=0 corrected to 5, got: {:?}",
            v
        );
    }

    #[test]
    fn test_warn_eq_block_no_swap() {
        let config = LoopGuardConfig {
            warn_threshold: 5,
            block_threshold: 5,
            ..LoopGuardConfig::default()
        };
        let mut guard = LoopGuard::new(config);
        let params = serde_json::json!({"x": 1});

        // The swap in new() only triggers when warn > block, so equal values
        // should NOT swap. Both thresholds remain at 5.
        // Calls 1-4: Allow (count < 5)
        for _ in 0..4 {
            let v = guard.check("tool", &params);
            assert_eq!(
                v,
                LoopGuardVerdict::Allow,
                "Calls 1-4 should be Allow when warn==block==5, got: {:?}",
                v
            );
        }

        // Call 5: count==5 hits block_threshold (block is checked BEFORE warn
        // in check()), so it should be Block, not Warn.
        let v = guard.check("tool", &params);
        assert!(
            matches!(v, LoopGuardVerdict::Block(_)),
            "5th call should be Block (not Warn) proving no swap happened, got: {:?}",
            v
        );
    }

    // ========================================================================
    // New tests — Record Outcome & Stats Edge Cases
    // ========================================================================

    #[test]
    fn test_outcome_below_block_threshold() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"key": "val"});

        // Establish last_call_hash via check() (normal flow)
        guard.check("tool", &params);

        // First identical outcome: count=1, below outcome_warn_threshold=2 => None
        let w = guard.record_outcome("tool", &params, "same result");
        assert!(w.is_none());

        // Second identical outcome: count=2, hits outcome_warn_threshold=2 => Some
        let w = guard.record_outcome("tool", &params, "same result");
        assert!(w.is_some());
        assert!(w.unwrap().contains("identical results"));

        // count=2 is still below outcome_block_threshold=3, so the call hash
        // should NOT have been added to blocked_outcomes.
        // A subsequent check() may Warn from call counting but must NOT Block
        // with "identical results".
        let v = guard.check("tool", &params);
        match v {
            LoopGuardVerdict::Block(ref msg) => {
                assert!(
                    !msg.contains("identical results"),
                    "Should NOT be blocked by outcome detection (only 2 identical outcomes, need 3), got: {}",
                    msg
                );
            }
            LoopGuardVerdict::Allow | LoopGuardVerdict::Warn(_) | LoopGuardVerdict::CircuitBreak(_) => {
                // Acceptable — call counting may warn, but outcome did not block
            }
        }
    }

    #[test]
    fn test_stats_empty_guard() {
        let guard = LoopGuard::new(LoopGuardConfig::default());

        let stats = guard.stats();
        assert_eq!(stats.total_calls, 0);
        assert_eq!(stats.unique_calls, 0);
        assert_eq!(stats.blocked_calls, 0);
        assert!(!stats.ping_pong_detected);
        assert_eq!(stats.most_repeated_tool, None);
        assert_eq!(stats.most_repeated_count, 0);
    }

    // ========================================================================
    // New tests — Ping-Pong Detection Edge Cases
    // ========================================================================

    #[test]
    fn test_ping_pong_all_same_not_detected() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            ..Default::default()
        });
        let params = serde_json::json!({"x": 1});

        // 9 identical calls — all-same is repetition, NOT ping-pong
        for _ in 0..9 {
            guard.check("tool_a", &params);
        }

        assert!(
            !guard.stats().ping_pong_detected,
            "All-same pattern should NOT trigger ping-pong detection"
        );
    }

    #[test]
    fn test_ping_pong_many_repeats() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            ping_pong_min_repeats: 3,
            ..Default::default()
        });
        let params_a = serde_json::json!({"file": "a.txt"});
        let params_b = serde_json::json!({"file": "b.txt"});

        // A-B pattern repeated 6 times = 12 total calls
        let mut last_verdict = LoopGuardVerdict::Allow;
        for _ in 0..6 {
            guard.check("file_read", &params_a);
            last_verdict = guard.check("file_write", &params_b);
        }

        assert!(
            guard.stats().ping_pong_detected,
            "6 repeats of A-B should trigger ping-pong detection"
        );

        // After the loop, check the last verdict
        assert!(
            matches!(last_verdict, LoopGuardVerdict::Block(ref msg) if msg.contains("file_read") || msg.contains("file_write")),
            "Expected Block for ping-pong with 6 repeats, got: {:?}",
            last_verdict
        );
    }

    #[test]
    fn test_ping_pong_insufficient_history() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            ..Default::default()
        });
        let params_a = serde_json::json!({"a": 1});
        let params_b = serde_json::json!({"b": 2});

        // Only 4 calls: A-B-A-B — need 6 for pattern_len=2 (3 repeats)
        guard.check("tool_a", &params_a);
        guard.check("tool_b", &params_b);
        guard.check("tool_a", &params_a);
        guard.check("tool_b", &params_b);

        assert!(
            !guard.stats().ping_pong_detected,
            "4 calls (2 repeats) should NOT trigger ping-pong — need at least 3 repeats"
        );
    }

    #[test]
    fn test_ping_pong_flag_persists() {
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            ping_pong_min_repeats: 3,
            ..Default::default()
        });
        let params_a = serde_json::json!({"a": 1});
        let params_b = serde_json::json!({"b": 2});

        // Build A-B pattern x 3 to trigger detection
        for _ in 0..3 {
            guard.check("file_read", &params_a);
            guard.check("file_write", &params_b);
        }

        assert!(
            guard.stats().ping_pong_detected,
            "3 repeats of A-B should trigger ping-pong detection"
        );

        // Now make 5 more unique calls (no pattern)
        for i in 0..5 {
            let params = serde_json::json!({"unique": i});
            guard.check("tool_unique", &params);
        }

        // The flag should STILL be true — it is a latching flag
        assert!(
            guard.stats().ping_pong_detected,
            "Ping-pong flag should persist after detection, even with subsequent unique calls"
        );
    }

    // ========================================================================
    // Outcome Hash Truncation Boundaries
    // ========================================================================

    #[test]
    fn test_outcome_hash_long_result_truncated() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"q": 1});

        // result_a = "A" * 2000 (longer than 1000 bytes)
        let result_a = "A".repeat(2000);

        // Call 1: no warning (count = 1)
        let w = guard.record_outcome("tool", &params, &result_a);
        assert!(w.is_none(), "First identical outcome should not warn");

        // Call 2 with identical result: warning (count = 2, outcome_warn_threshold = 2)
        let w = guard.record_outcome("tool", &params, &result_a);
        assert!(w.is_some(), "Second identical outcome should warn");
        assert!(w.unwrap().contains("identical results"));

        // Now build result_b that differs only in the middle (truncated zone):
        // first 500 bytes = "A", middle 1000 bytes = "B", last 500 bytes = "A"
        let mut result_b = "A".repeat(2000);
        {
            let bytes = unsafe { result_b.as_bytes_mut() };
            for b in bytes.iter_mut().take(1500).skip(500) {
                *b = b'B';
            }
        }

        // Both results should produce the same outcome hash because truncation
        // captures only [0..500] and [len-500..len], and the difference is in
        // the truncated middle (bytes 500..1499).
        // Call 3 with result_b: count = 3 -> outcome_block_threshold reached
        let w = guard.record_outcome("tool", &params, &result_b);
        assert!(
            w.is_some(),
            "Third outcome with same truncated hash should warn/block"
        );
        assert!(w.unwrap().contains("identical results"));

        // Negative case: result_c differs from result_a in the FIRST 500 bytes
        // (captured region), so it must produce a DIFFERENT hash.
        let mut result_c = "A".repeat(2000);
        result_c.replace_range(0..10, "BBBBBBBBBB");
        let w = guard.record_outcome("tool", &params, &result_c);
        assert!(
            w.is_none(),
            "Result differing in captured region should produce different hash — no warning"
        );
    }

    #[test]
    fn test_outcome_hash_exactly_1000_bytes() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"q": 1});

        // Exactly 1000 bytes — the full-hash path (no truncation)
        let result = "x".repeat(1000);

        // Call 1: no warning (count = 1)
        let w = guard.record_outcome("tool", &params, &result);
        assert!(w.is_none(), "First outcome should not warn");

        // Call 2 with identical result: warning (count = 2, outcome_warn_threshold = 2)
        let w = guard.record_outcome("tool", &params, &result);
        assert!(w.is_some(), "Second identical outcome should warn");
        assert!(w.unwrap().contains("identical results"));
    }

    #[test]
    fn test_outcome_hash_exactly_1001_bytes() {
        let mut guard = LoopGuard::new(LoopGuardConfig::default());
        let params = serde_json::json!({"q": 1});

        // result_a = "A" * 1001 (truncation path: first 500 + last 500, byte at
        // index 500 is discarded)
        let result_a = "A".repeat(1001);

        // result_b differs from result_a only at byte index 500 (in truncated zone):
        // first 500 bytes = "A", byte 500 = "X", bytes 501..1001 = "A"
        let mut result_b = "A".repeat(1001);
        result_b.replace_range(500..501, "X");

        // Call 1 with result_a: no warning (count = 1)
        let w = guard.record_outcome("tool", &params, &result_a);
        assert!(w.is_none(), "First outcome should not warn");

        // Call 2 with result_b: truncation captures bytes [0..500] and [len-500..len] = [501..1001]
        // byte at index 500 is in the truncated gap — excluded from hash.
        // Both result_a and result_b have identical first 500 and last 500 bytes,
        // hence identical truncated hash.
        // count = 2 -> outcome_warn_threshold reached -> Some(warning)
        let w = guard.record_outcome("tool", &params, &result_b);
        assert!(
            w.is_some(),
            "Second outcome with same truncated hash (differing only in truncated zone) should warn"
        );
        assert!(w.unwrap().contains("identical results"));
    }

    #[test]
    fn test_ping_pong_warning_exhaustion() {
        // When ping-pong is detected below min_repeats, warnings are emitted.
        // After max_warnings_per_call warnings for the pattern key, the guard
        // stops emitting warnings (returns Allow instead of Warn).
        let mut guard = LoopGuard::new(LoopGuardConfig {
            warn_threshold: 100,
            block_threshold: 100,
            global_circuit_breaker: 200,
            ping_pong_min_repeats: 100, // set very high so ping-pong never blocks via repeat count
            max_warnings_per_call: 2,
            ..Default::default()
        });
        let params_a = serde_json::json!({"a": 1});
        let params_b = serde_json::json!({"b": 2});

        // Create AB pattern many times but below min_repeats.
        // Each cycle of 2 calls triggers one ping-pong detection with a Warn.
        // After max_warnings_per_call warnings, the guard stops warning.
        let mut warn_count = 0;
        let mut allow_after_exhaustion = false;
        for _ in 0..20 {
            guard.check("tool_a", &params_a);
            let v = guard.check("tool_b", &params_b);
            match v {
                LoopGuardVerdict::Warn(ref msg) if msg.contains("Ping-pong") => {
                    warn_count += 1;
                }
                LoopGuardVerdict::Allow => {
                    // After warnings are exhausted, ping-pong detection stops warning
                    allow_after_exhaustion = true;
                }
                other => {
                    panic!("Unexpected verdict: {:?}", other);
                }
            }
        }

        // Should have received exactly max_warnings_per_call warnings
        assert_eq!(
            warn_count, 2,
            "Expected exactly 2 ping-pong warnings before exhaustion"
        );
        // After exhaustion, should return Allow (warnings suppressed)
        assert!(
            allow_after_exhaustion,
            "Expected Allow after ping-pong warning exhaustion"
        );
    }
}
