//! Capability-based security types.
//!
//! LibreFang uses capability-based security: an agent can only perform actions
//! that it has been explicitly granted permission to do. Capabilities are
//! immutable after agent creation and enforced at the kernel level.

use serde::{Deserialize, Serialize};

/// A specific permission granted to an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum Capability {
    // -- File system --
    /// Read files matching the given glob pattern.
    FileRead(String),
    /// Write files matching the given glob pattern.
    FileWrite(String),

    // -- Network --
    /// Connect to hosts matching the pattern (e.g., "api.openai.com:443").
    NetConnect(String),
    /// Listen on a specific port.
    NetListen(u16),

    // -- Tools --
    /// Invoke a specific tool by ID.
    ToolInvoke(String),
    /// Invoke any tool (dangerous, requires explicit grant).
    ToolAll,

    // -- LLM --
    /// Query models matching the pattern.
    LlmQuery(String),
    /// Maximum token budget.
    LlmMaxTokens(u64),

    // -- Agent interaction --
    /// Can spawn sub-agents.
    AgentSpawn,
    /// Can send messages to agents matching the pattern.
    AgentMessage(String),
    /// Can kill agents matching the pattern (or "*" for any).
    AgentKill(String),

    // -- Memory --
    /// Read from memory scopes matching the pattern.
    MemoryRead(String),
    /// Write to memory scopes matching the pattern.
    MemoryWrite(String),

    // -- Shell --
    /// Execute shell commands matching the pattern.
    ShellExec(String),
    /// Read environment variables matching the pattern.
    EnvRead(String),

    // -- OFP (LibreFang Wire Protocol) --
    /// Can discover remote agents.
    OfpDiscover,
    /// Can connect to remote peers matching the pattern.
    OfpConnect(String),
    /// Can advertise services on the network.
    OfpAdvertise,

    // -- Economic --
    /// Can spend up to the given amount in USD.
    EconSpend(f64),
    /// Can accept incoming payments.
    EconEarn,
    /// Can transfer funds to agents matching the pattern.
    EconTransfer(String),
}

/// Result of a capability check.
#[derive(Debug, Clone)]
pub enum CapabilityCheck {
    /// The capability is granted.
    Granted,
    /// The capability is denied with a reason.
    Denied(String),
}

impl CapabilityCheck {
    /// Returns true if the capability is granted.
    pub fn is_granted(&self) -> bool {
        matches!(self, Self::Granted)
    }

    /// Returns an error if denied, Ok(()) if granted.
    pub fn require(&self) -> Result<(), crate::error::LibreFangError> {
        match self {
            Self::Granted => Ok(()),
            Self::Denied(reason) => Err(crate::error::LibreFangError::CapabilityDenied(
                reason.clone(),
            )),
        }
    }
}

/// Checks whether a required capability matches any granted capability.
///
/// Pattern matching rules:
/// - Exact match: "api.openai.com:443" matches "api.openai.com:443"
/// - Wildcard: "*" matches anything
/// - Glob: "*.openai.com:443" matches "api.openai.com:443"
pub fn capability_matches(granted: &Capability, required: &Capability) -> bool {
    match (granted, required) {
        // ToolAll grants any ToolInvoke
        (Capability::ToolAll, Capability::ToolInvoke(_)) => true,

        // Same variant, check pattern matching
        (Capability::FileRead(pattern), Capability::FileRead(path)) => glob_matches(pattern, path),
        (Capability::FileWrite(pattern), Capability::FileWrite(path)) => {
            glob_matches(pattern, path)
        }
        (Capability::NetConnect(pattern), Capability::NetConnect(host)) => {
            glob_matches(pattern, host)
        }
        (Capability::ToolInvoke(granted_id), Capability::ToolInvoke(required_id)) => {
            glob_matches(granted_id, required_id)
        }
        (Capability::LlmQuery(pattern), Capability::LlmQuery(model)) => {
            glob_matches(pattern, model)
        }
        (Capability::AgentMessage(pattern), Capability::AgentMessage(target)) => {
            glob_matches(pattern, target)
        }
        (Capability::AgentKill(pattern), Capability::AgentKill(target)) => {
            glob_matches(pattern, target)
        }
        (Capability::MemoryRead(pattern), Capability::MemoryRead(scope)) => {
            glob_matches(pattern, scope)
        }
        (Capability::MemoryWrite(pattern), Capability::MemoryWrite(scope)) => {
            glob_matches(pattern, scope)
        }
        (Capability::ShellExec(pattern), Capability::ShellExec(cmd)) => glob_matches(pattern, cmd),
        (Capability::EnvRead(pattern), Capability::EnvRead(var)) => glob_matches(pattern, var),
        (Capability::OfpConnect(pattern), Capability::OfpConnect(peer)) => {
            glob_matches(pattern, peer)
        }
        (Capability::EconTransfer(pattern), Capability::EconTransfer(target)) => {
            glob_matches(pattern, target)
        }

        // Simple boolean capabilities
        (Capability::AgentSpawn, Capability::AgentSpawn) => true,
        (Capability::OfpDiscover, Capability::OfpDiscover) => true,
        (Capability::OfpAdvertise, Capability::OfpAdvertise) => true,
        (Capability::EconEarn, Capability::EconEarn) => true,

        // Numeric capabilities
        (Capability::NetListen(granted_port), Capability::NetListen(required_port)) => {
            granted_port == required_port
        }
        (Capability::LlmMaxTokens(granted_max), Capability::LlmMaxTokens(required_max)) => {
            granted_max >= required_max
        }
        (Capability::EconSpend(granted_max), Capability::EconSpend(required_amount)) => {
            granted_max >= required_amount
        }

        // Different variants never match
        _ => false,
    }
}

/// Validate that child capabilities are a subset of parent capabilities.
/// This prevents privilege escalation: a restricted parent cannot create
/// an unrestricted child.
pub fn validate_capability_inheritance(
    parent_caps: &[Capability],
    child_caps: &[Capability],
) -> Result<(), String> {
    for child_cap in child_caps {
        let is_covered = parent_caps
            .iter()
            .any(|parent_cap| capability_matches(parent_cap, child_cap));
        if !is_covered {
            return Err(format!(
                "Privilege escalation denied: child requests {:?} but parent does not have a matching grant",
                child_cap
            ));
        }
    }
    Ok(())
}

/// Glob pattern matching supporting `*` and `**` wildcards.
///
/// # Pattern rules
///
/// **Single-segment wildcard `*`** — matches any characters **except** the
/// path/URL separator `/`. This prevents path traversal via capability globs:
/// `data/*` matches `data/file.txt` but NOT `data/../../etc/passwd`.
///
/// **Double-segment wildcard `**`** — matches any characters including `/`,
/// so `data/**` matches `data/a/b/c/file.txt`.
///
/// **Bare `*`** (the entire pattern is just `"*"`) — matches anything, for
/// backward compatibility with the universal wildcard grant.
///
/// **No `/` in pattern** — falls back to the original single-wildcard
/// matching so non-path patterns (tool names, hostnames, memory scopes, etc.)
/// continue to work as before: `file_*` matches `file_read`, `*.openai.com`
/// matches `api.openai.com`, and so on.
///
/// # Rationale
///
/// The original `*` implementation used `str::ends_with` / `str::starts_with`
/// which let `*` silently cross `/` separators. A grant of `FileRead("data/*")`
/// was supposed to allow reads inside the `data/` directory but instead also
/// matched `data/../../etc/passwd` (containing the traversal after the `*`).
/// Fixing `*` to stop at `/` closes this class of capability bypass.
pub fn glob_matches(pattern: &str, value: &str) -> bool {
    // Bare "*" is the universal match — keep it fast and unchanged.
    if pattern == "*" {
        return true;
    }
    // Exact match is always valid.
    if pattern == value {
        return true;
    }

    // If the pattern contains a path separator we apply segment-aware matching
    // so that a single `*` cannot cross a `/`.
    if pattern.contains('/') {
        return glob_matches_path(pattern, value);
    }

    // SECURITY: hostname-style patterns (`*.example.com`) must use the
    // dot-separator-aware matcher, otherwise the legacy `value.ends_with(suffix)`
    // path lets a value like `evil.com?host=good.example.com` match
    // `*.example.com` (it ends with `.example.com`). That bypass was caught by
    // the original #3902 host-mode check; this branch restores it.
    if pattern.contains('.') && value.contains('.') {
        return glob_matches_with_separator(pattern, value, '.');
    }

    // No separator in pattern: use the original wildcard logic so tool
    // names ("file_*"), memory scope patterns, etc. keep working exactly
    // as before.
    glob_matches_simple(pattern, value)
}

/// Generic separator-aware matcher: split both sides on `sep` and match
/// segment-by-segment. A single `*` segment matches one segment; `**` matches
/// across segments. Used by both the path and hostname matchers.
fn glob_matches_with_separator(pattern: &str, value: &str, sep: char) -> bool {
    let pat_segs: Vec<&str> = pattern.split(sep).collect();
    let val_segs: Vec<&str> = value.split(sep).collect();
    glob_match_segments(&pat_segs, &val_segs)
}

/// Original (legacy) single-wildcard matching used when the pattern has no `/`.
///
/// `*` matches any sequence of characters (including `.`, `:`, etc.).
fn glob_matches_simple(pattern: &str, value: &str) -> bool {
    if let Some(suffix) = pattern.strip_prefix('*') {
        // "*suffix" — but only if there's no second '*' in suffix.
        // Multi-wildcard patterns without '/' fall through to the find() branch.
        if !suffix.contains('*') {
            return value.ends_with(suffix);
        }
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        if !prefix.contains('*') {
            return value.starts_with(prefix);
        }
    }
    // Middle wildcard or multi-wildcard: find first '*' and check prefix+suffix.
    if let Some(star_pos) = pattern.find('*') {
        let prefix = &pattern[..star_pos];
        let suffix = &pattern[star_pos + 1..];
        // Recursively handle the suffix in case it contains more wildcards.
        if suffix.contains('*') {
            // For simplicity, only support one level of recursion here.
            if let Some(rest) = value.strip_prefix(prefix) {
                return glob_matches_simple(suffix, rest);
            }
            return false;
        }
        return value.starts_with(prefix)
            && value.ends_with(suffix)
            && value.len() >= prefix.len() + suffix.len();
    }
    false
}

/// Path-aware glob matching used when the pattern contains `/`.
///
/// Splits both pattern and value on `/` and matches segment by segment.
/// A single `*` within a segment matches any characters **except** `/`.
/// A `**` segment matches zero or more complete path segments (like `/**/`).
fn glob_matches_path(pattern: &str, value: &str) -> bool {
    let pat_segs: Vec<&str> = pattern.split('/').collect();
    let val_segs: Vec<&str> = value.split('/').collect();
    glob_match_segments(&pat_segs, &val_segs)
}

/// Recursive segment-by-segment matcher.
fn glob_match_segments(pat: &[&str], val: &[&str]) -> bool {
    match (pat.first(), val.first()) {
        // Both exhausted at the same time: success.
        (None, None) => true,
        // Pattern exhausted but value still has segments: no match.
        // (The single-segment `*` cannot silently consume extra segments.)
        (None, _) => false,
        // Value exhausted but pattern still has segments: only succeed if every
        // remaining pattern segment is `**` (which can match zero segments).
        (_, None) => pat.iter().all(|s| *s == "**"),
        (Some(&"**"), _) => {
            // "**" can match zero or more segments. Try consuming 0, 1, 2, …
            // segments from val until we find a match or exhaust val.
            let rest_pat = &pat[1..];
            // Match zero segments consumed by **
            if glob_match_segments(rest_pat, val) {
                return true;
            }
            // Match one or more segments consumed by **
            for i in 1..=val.len() {
                if glob_match_segments(rest_pat, &val[i..]) {
                    return true;
                }
            }
            false
        }
        (Some(p), Some(v)) => {
            // Match this segment, then recurse on the rest.
            if segment_matches(p, v) {
                glob_match_segments(&pat[1..], &val[1..])
            } else {
                false
            }
        }
    }
}

/// Match a single path segment (`*` = any chars except `/`).
fn segment_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" || pattern == value {
        return true;
    }
    // Use the simple matcher restricted to a single segment (no `/` in either).
    // Because we've already split on `/`, neither string should contain `/`;
    // but double-check to be safe.
    if value.contains('/') {
        return false;
    }
    glob_matches_simple(pattern, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_match() {
        assert!(capability_matches(
            &Capability::NetConnect("api.openai.com:443".to_string()),
            &Capability::NetConnect("api.openai.com:443".to_string()),
        ));
    }

    #[test]
    fn test_wildcard_match() {
        assert!(capability_matches(
            &Capability::NetConnect("*.openai.com:443".to_string()),
            &Capability::NetConnect("api.openai.com:443".to_string()),
        ));
    }

    #[test]
    fn test_star_matches_all() {
        assert!(capability_matches(
            &Capability::AgentMessage("*".to_string()),
            &Capability::AgentMessage("any-agent".to_string()),
        ));
    }

    #[test]
    fn test_tool_all_grants_specific() {
        assert!(capability_matches(
            &Capability::ToolAll,
            &Capability::ToolInvoke("web_search".to_string()),
        ));
    }

    #[test]
    fn test_different_variants_dont_match() {
        assert!(!capability_matches(
            &Capability::FileRead("*".to_string()),
            &Capability::FileWrite("/tmp/test".to_string()),
        ));
    }

    #[test]
    fn test_numeric_capability_bounds() {
        assert!(capability_matches(
            &Capability::LlmMaxTokens(10000),
            &Capability::LlmMaxTokens(5000),
        ));
        assert!(!capability_matches(
            &Capability::LlmMaxTokens(1000),
            &Capability::LlmMaxTokens(5000),
        ));
    }

    #[test]
    fn test_capability_check_require() {
        assert!(CapabilityCheck::Granted.require().is_ok());
        assert!(CapabilityCheck::Denied("no".to_string()).require().is_err());
    }

    #[test]
    fn test_glob_matches_middle_wildcard() {
        assert!(glob_matches("api.*.com", "api.openai.com"));
        assert!(!glob_matches("api.*.com", "api.openai.org"));
    }

    #[test]
    fn test_agent_kill_capability() {
        assert!(capability_matches(
            &Capability::AgentKill("*".to_string()),
            &Capability::AgentKill("agent-123".to_string()),
        ));
        assert!(!capability_matches(
            &Capability::AgentKill("agent-1".to_string()),
            &Capability::AgentKill("agent-2".to_string()),
        ));
    }

    #[test]
    fn test_capability_inheritance_subset_ok() {
        let parent = vec![
            Capability::FileRead("*".to_string()),
            Capability::NetConnect("*.example.com:443".to_string()),
        ];
        let child = vec![
            Capability::FileRead("/data/*".to_string()),
            Capability::NetConnect("api.example.com:443".to_string()),
        ];
        assert!(validate_capability_inheritance(&parent, &child).is_ok());
    }

    #[test]
    fn test_capability_inheritance_escalation_denied() {
        let parent = vec![Capability::FileRead("/data/*".to_string())];
        let child = vec![
            Capability::FileRead("*".to_string()),
            Capability::ShellExec("*".to_string()),
        ];
        assert!(validate_capability_inheritance(&parent, &child).is_err());
    }

    // -----------------------------------------------------------------------
    // glob_matches (pub) — tool name style patterns
    // -----------------------------------------------------------------------

    #[test]
    fn test_glob_matches_tool_prefix_wildcard() {
        assert!(glob_matches("file_*", "file_read"));
        assert!(glob_matches("file_*", "file_write"));
        assert!(glob_matches("file_*", "file_delete"));
        assert!(!glob_matches("file_*", "shell_exec"));
        assert!(!glob_matches("file_*", "web_fetch"));
    }

    #[test]
    fn test_glob_matches_tool_suffix_wildcard() {
        assert!(glob_matches("*_exec", "shell_exec"));
        assert!(!glob_matches("*_exec", "shell_read"));
    }

    #[test]
    fn test_glob_matches_tool_star_all() {
        assert!(glob_matches("*", "file_read"));
        assert!(glob_matches("*", "shell_exec"));
        assert!(glob_matches("*", "anything"));
    }

    #[test]
    fn test_glob_matches_tool_exact() {
        assert!(glob_matches("file_read", "file_read"));
        assert!(!glob_matches("file_read", "file_write"));
    }

    #[test]
    fn test_glob_matches_mcp_prefix() {
        assert!(glob_matches("mcp_*", "mcp_server1_tool_a"));
        assert!(glob_matches("mcp_*", "mcp_myserver_mytool"));
        assert!(!glob_matches("mcp_*", "file_read"));
    }

    // Verifies the resolution strategy used in tool_timeout_secs_for:
    // when multiple glob patterns match, longest pattern (most specific) wins.
    #[test]
    fn test_glob_tool_timeout_resolution_longest_wins() {
        // "mcp_browser_*" (14 chars) must beat "mcp_*" (5 chars)
        let patterns: &[(&str, u64)] = &[("mcp_*", 300), ("mcp_browser_*", 900)];
        let tool = "mcp_browser_navigate";
        let best = patterns
            .iter()
            .filter(|(p, _)| glob_matches(p, tool))
            .max_by_key(|(p, _)| p.len());
        assert_eq!(best.map(|(_, t)| *t), Some(900));
    }

    #[test]
    fn test_glob_tool_timeout_resolution_star_loses_to_specific() {
        let patterns: &[(&str, u64)] = &[("*", 60), ("shell_*", 300)];
        let tool = "shell_exec";
        let best = patterns
            .iter()
            .filter(|(p, _)| glob_matches(p, tool))
            .max_by_key(|(p, _)| p.len());
        assert_eq!(best.map(|(_, t)| *t), Some(300));
    }

    #[test]
    fn test_glob_tool_timeout_resolution_no_match_returns_none() {
        let patterns: &[(&str, u64)] = &[("mcp_*", 900), ("shell_*", 300)];
        let tool = "file_read";
        let best = patterns
            .iter()
            .filter(|(p, _)| glob_matches(p, tool))
            .max_by_key(|(p, _)| p.len());
        assert!(best.is_none());
    }

    // -----------------------------------------------------------------------
    // Bug #3863: glob separator safety — `*` must not cross `/`
    // -----------------------------------------------------------------------

    /// `data/*` must match a file directly inside `data/` but NOT a path
    /// that uses `..` to escape the directory boundary.
    #[test]
    fn test_glob_star_does_not_cross_path_separator() {
        // Should match — `*` covers a single segment "file.txt"
        assert!(
            glob_matches("data/*", "data/file.txt"),
            "data/* must match data/file.txt"
        );
        // Must NOT match — `*` cannot span across the `/..` traversal segments
        assert!(
            !glob_matches("data/*", "data/../../etc/passwd"),
            "data/* must NOT match data/../../etc/passwd"
        );
        // Must NOT match — extra segments beyond the single `*`
        assert!(
            !glob_matches("data/*", "data/subdir/file.txt"),
            "data/* must NOT match data/subdir/file.txt (use data/** for that)"
        );
    }

    /// `**` must be able to match across path segments.
    #[test]
    fn test_glob_double_star_crosses_path_separator() {
        assert!(
            glob_matches("data/**", "data/subdir/file.txt"),
            "data/** must match data/subdir/file.txt"
        );
        assert!(
            glob_matches("data/**", "data/file.txt"),
            "data/** must match data/file.txt"
        );
    }

    /// URL capability patterns: `*` in the host portion must NOT match an
    /// entirely different domain. The host string is already extracted as
    /// `hostname:port` before this function is called, so the separator to
    /// guard against is `.`, which is intentionally NOT blocked by `*` (we
    /// want `*.openai.com:443` to work). However, `/` in the scheme-stripped
    /// URL path portion MUST be blocked. This test exercises the scheme-level
    /// guard via the `https://…/*` pattern.
    #[test]
    fn test_glob_url_star_does_not_cross_scheme_host_boundary() {
        // Pattern allows a specific path on example.com
        assert!(
            glob_matches("https://example.com/*", "https://example.com/foo"),
            "https://example.com/* must match https://example.com/foo"
        );
        // Must NOT match a different host — `*` cannot cross the scheme+host boundary
        assert!(
            !glob_matches("https://example.com/*", "https://evil.com/foo"),
            "https://example.com/* must NOT match https://evil.com/foo"
        );
    }

    /// Bare `*` (universal grant) must still match everything — including paths
    /// with `/`, so that `FileRead("*")` continues to work as a super-grant.
    #[test]
    fn test_glob_bare_star_still_matches_all() {
        assert!(glob_matches("*", "/etc/passwd"));
        assert!(glob_matches("*", "data/../../etc/passwd"));
        assert!(glob_matches("*", "any-tool-name"));
        assert!(glob_matches("*", "https://example.com/path"));
    }

    /// Non-path patterns (tool names, hostnames) must continue to work
    /// with `*` crossing `.`, `-`, `_`, and other non-`/` separators.
    #[test]
    fn test_glob_non_path_patterns_unchanged() {
        // Tool name patterns
        assert!(glob_matches("file_*", "file_read"));
        assert!(glob_matches("mcp_*", "mcp_server_tool"));
        // Hostname patterns (no `/` in pattern)
        assert!(glob_matches("*.openai.com:443", "api.openai.com:443"));
        assert!(glob_matches("api.*.com", "api.openai.com"));
        // Memory scope patterns
        assert!(glob_matches("agent:*", "agent:abc123"));
    }

    /// A path capability grant of `/data/*` must work for files at that level
    /// but not grant access to the traversal escape `/data/../../etc/passwd`.
    #[test]
    fn test_capability_file_read_path_glob_blocks_traversal() {
        assert!(capability_matches(
            &Capability::FileRead("/data/*".to_string()),
            &Capability::FileRead("/data/myfile.txt".to_string()),
        ));
        assert!(!capability_matches(
            &Capability::FileRead("/data/*".to_string()),
            &Capability::FileRead("/data/../../etc/passwd".to_string()),
        ));
    }

    /// Regression: hostname-style patterns must NOT use the legacy
    /// `value.ends_with(suffix)` path, because that lets a value with the
    /// pattern's suffix anywhere in it match (SSRF amplifier).
    ///
    /// `*.example.com` should match `api.example.com` but NOT
    /// `evil.com?host=good.example.com` (which ends with `.example.com`).
    /// This was the original #3902 protection that #3925 inadvertently
    /// regressed.
    #[test]
    fn test_glob_host_separator_blocks_endswith_smuggling() {
        assert!(glob_matches("*.example.com", "api.example.com"));
        assert!(glob_matches("*.openai.com:443", "api.openai.com:443"));
        // The smuggle: value ends with ".example.com" but `evil.com?...` is
        // NOT the legitimate first-segment match. Must be rejected.
        assert!(!glob_matches(
            "*.example.com",
            "evil.com?host=good.example.com"
        ));
        // Two-segment value cannot match three-segment pattern.
        assert!(!glob_matches("*.example.com", "evil.com"));
    }
}
