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

/// Simple glob pattern matching supporting `*` and `**` wildcards.
///
/// # Separator-aware semantics (security-critical)
///
/// For values that look like **file paths** (contain `/`) or **hostnames**
/// (contain `.`), a single `*` is treated as **literal-separator** — it will
/// NOT match across the separator character.  This prevents a capability like
/// `FileRead("/tmp/*")` from matching `/tmp/../etc/passwd` (path traversal) or
/// `NetConnect("*.example.com")` from matching
/// `evil.com?host=good.example.com`.
///
/// Use `**` to match across separators when that is intentional (e.g., a
/// recursive directory grant).
///
/// For values that contain **neither** `/` **nor** `.` (plain identifiers such
/// as tool names, agent names, env-var names), `*` retains its traditional
/// "match anything" behaviour so existing patterns like `"file_*"` or `"mcp_*"`
/// continue to work unchanged.
///
/// # Pattern rules
/// - `"*"` — matches any **single-component** value (no `/` or `.` in result)
///   when the value is path/host-like; matches anything for plain identifiers
/// - `"prefix*"` — matches values whose first component starts with `prefix`
/// - `"*suffix"` — matches values whose last component ends with `suffix`
/// - `"prefix*suffix"` — combined
/// - `"**"` / `"**/*"` style — not yet supported; reserved for future use
/// - Exact string always matches itself
///
/// # Examples
/// ```
/// use librefang_types::capability::glob_matches;
///
/// // File paths: * does NOT cross /
/// assert!(glob_matches("/tmp/*", "/tmp/foo"));
/// assert!(!glob_matches("/tmp/*", "/tmp/foo/bar"));
/// assert!(!glob_matches("/tmp/*", "/tmp/../etc/passwd"));
///
/// // Hostnames: * does NOT cross .
/// assert!(glob_matches("*.example.com:443", "api.example.com:443"));
/// assert!(!glob_matches("*.example.com", "evil.com?host=good.example.com"));
///
/// // Plain identifiers: * matches freely
/// assert!(glob_matches("file_*", "file_read"));
/// assert!(glob_matches("mcp_*", "mcp_server_tool"));
/// ```
pub fn glob_matches(pattern: &str, value: &str) -> bool {
    // Determine whether this is a "structured" value (path or hostname).
    // For structured values we enable literal-separator mode so that a single
    // `*` cannot jump across `/` or `.` boundaries.
    let is_path = value.contains('/');
    // Only apply dot-separator mode when the pattern itself contains dots.
    // Inferring "is host" from the value alone misclassifies file names like
    // "readme.txt" as hostnames, causing `glob_matches("*", "readme.txt")` to
    // incorrectly return false.
    let is_host = !is_path && pattern.contains('.');
    let separator: Option<char> = if is_path {
        Some('/')
    } else if is_host {
        Some('.')
    } else {
        None
    };

    // Fast path: `*` with no separator context keeps backward-compatible
    // behaviour (matches anything).
    if pattern == "*" {
        if let Some(sep) = separator {
            // For structured values `*` must not cross the separator.
            // A lone `*` therefore only matches a single component (no sep in value).
            return !value.contains(sep);
        }
        return true;
    }

    // Exact match always wins.
    if pattern == value {
        return true;
    }

    // For structured values, delegate to a component-aware matcher.
    if let Some(sep) = separator {
        return glob_matches_with_separator(pattern, value, sep);
    }

    // Plain-identifier matching: original behaviour.
    glob_matches_plain(pattern, value)
}

/// Glob matching where `*` must not cross `separator`.
///
/// Splits both the pattern and value on `separator` and matches segment by
/// segment.  A `*` segment matches exactly one value segment; a `*` inside a
/// segment matches within that segment only.
fn glob_matches_with_separator(pattern: &str, value: &str, separator: char) -> bool {
    let pat_parts: Vec<&str> = pattern.split(separator).collect();
    let val_parts: Vec<&str> = value.split(separator).collect();

    if pat_parts.len() != val_parts.len() {
        return false;
    }

    pat_parts
        .iter()
        .zip(val_parts.iter())
        .all(|(p, v)| glob_matches_plain(p, v))
}

/// Original glob logic for plain (non-path, non-host) values.
///
/// `*` matches any substring within the component (no separator awareness).
fn glob_matches_plain(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if pattern == value {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return value.ends_with(suffix);
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    // Middle wildcard: "prefix*suffix"
    if let Some(star_pos) = pattern.find('*') {
        let prefix = &pattern[..star_pos];
        let suffix = &pattern[star_pos + 1..];
        return value.starts_with(prefix)
            && value.ends_with(suffix)
            && value.len() >= prefix.len() + suffix.len();
    }
    false
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
        // Parent grants broad access; child requests a strict subset.
        // FileRead("/data/*") covers a specific file under /data.
        // NetConnect("*.example.com:443") covers a concrete host.
        let parent = vec![
            Capability::FileRead("/data/*".to_string()),
            Capability::NetConnect("*.example.com:443".to_string()),
        ];
        let child = vec![
            Capability::FileRead("/data/output.txt".to_string()),
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
    // Bug #3863 — separator-aware glob: * must not cross / or .
    // -----------------------------------------------------------------------

    #[test]
    fn test_glob_file_star_does_not_cross_directory_separator() {
        // /tmp/* should match /tmp/foo but NOT /tmp/foo/bar
        assert!(
            glob_matches("/tmp/*", "/tmp/foo"),
            "/tmp/* must match /tmp/foo"
        );
        assert!(
            !glob_matches("/tmp/*", "/tmp/foo/bar"),
            "/tmp/* must NOT match /tmp/foo/bar"
        );
    }

    #[test]
    fn test_glob_file_star_does_not_allow_path_traversal() {
        // A malicious guest must not be able to escape /tmp/ via ../
        assert!(
            !glob_matches("/tmp/*", "/tmp/../etc/passwd"),
            "/tmp/* must NOT match /tmp/../etc/passwd"
        );
        assert!(
            !glob_matches("/tmp/*", "/tmp/../../root/.ssh/id_rsa"),
            "/tmp/* must NOT cross directory separators"
        );
    }

    #[test]
    fn test_glob_file_star_single_component_ok() {
        assert!(glob_matches("/data/*", "/data/file.txt"));
        assert!(glob_matches("/var/log/*", "/var/log/app.log"));
        assert!(!glob_matches("/var/log/*", "/var/log/sub/app.log"));
    }

    #[test]
    fn test_glob_host_star_does_not_cross_dot_separator() {
        // *.example.com:443 must match api.example.com:443 but not
        // evil.com?host=good.example.com (which has no '.' structure match)
        assert!(
            glob_matches("*.example.com:443", "api.example.com:443"),
            "*.example.com:443 must match api.example.com:443"
        );
        assert!(
            !glob_matches("*.example.com", "evil.org.example.com"),
            "*.example.com must NOT match evil.org.example.com (two-level prefix)"
        );
    }

    #[test]
    fn test_glob_host_star_single_label_only() {
        // *.example.com should NOT match sub.sub.example.com
        assert!(!glob_matches("*.example.com", "sub.sub.example.com"));
        assert!(glob_matches("*.example.com", "sub.example.com"));
    }

    #[test]
    fn test_glob_plain_identifier_star_unchanged() {
        // Plain identifiers (no / or .) — original behaviour preserved
        assert!(glob_matches("file_*", "file_read"));
        assert!(glob_matches("file_*", "file_write"));
        assert!(!glob_matches("file_*", "shell_exec"));
        assert!(glob_matches("mcp_*", "mcp_server1_tool_a"));
        assert!(glob_matches("*", "anything_plain"));
    }

    #[test]
    fn test_glob_star_alone_on_path_matches_only_single_component() {
        // A bare "*" capability on a path value only matches a value with no /
        // (i.e., a single-component relative path)
        assert!(glob_matches("*", "readme.txt"));
        assert!(!glob_matches("*", "/etc/passwd"));
        assert!(!glob_matches("*", "foo/bar"));
    }

    #[test]
    fn test_glob_exact_path_always_matches() {
        assert!(glob_matches("/etc/passwd", "/etc/passwd"));
        assert!(!glob_matches("/etc/passwd", "/etc/shadow"));
    }
}
