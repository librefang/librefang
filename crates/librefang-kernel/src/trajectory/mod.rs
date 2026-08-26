//! On-demand session trajectory export with privacy redaction.
//!
//! Produces a structured `.jsonl` (or JSON) audit trail of an agent session
//! — messages, tool calls, model/config metadata — with credentials and
//! workspace-absolute paths redacted. Intended for support, audit, and
//! compliance workflows.
//!
//! # Design
//!
//! - **On-demand only.** Reads an existing session from `MemorySubstrate`
//!   at request time. No background writers, no per-turn file IO, no
//!   kernel loop modifications.
//! - **Read-only.** Never mutates session state; safe to call concurrently
//!   with the agent loop.
//! - **Privacy by default.** Default `RedactionPolicy` masks API-key-shaped
//!   strings, JWTs, and large base64 blobs.
//!
//! # Usage
//!
//! ```ignore
//! let exporter = TrajectoryExporter::new(
//!     kernel.memory_substrate().clone(),
//!     RedactionPolicy::default().with_workspace_root(workspace.clone()),
//! );
//! let bundle = exporter.export_session(agent_id, session_id, agent_meta)?;
//! let jsonl = bundle.to_jsonl();
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use librefang_memory::MemorySubstrate;
use librefang_types::agent::{AgentId, SessionId};
use librefang_types::error::{LibreFangError, LibreFangResult};
use librefang_types::message::{ContentBlock, Message, MessageContent, Role, TokenUsage};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Redaction policy applied to message content before export.
///
/// Defaults to mask credential-shaped strings; callers should set
/// `workspace_root` so absolute paths under the agent workspace can be
/// collapsed to `<WORKSPACE>/...`.
#[derive(Debug, Clone)]
pub struct RedactionPolicy {
    /// Mask anything that looks like an API key, JWT, or large base64 blob.
    pub mask_credentials: bool,
    /// Workspace root — absolute paths starting with this prefix are
    /// rewritten to `<WORKSPACE>/...`. `None` disables path collapsing.
    pub workspace_root: Option<PathBuf>,
    /// Additional caller-provided regex patterns. Matches are replaced with
    /// `<REDACTED>`.
    pub custom_patterns: Vec<Regex>,
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self {
            mask_credentials: true,
            workspace_root: None,
            custom_patterns: Vec::new(),
        }
    }
}

impl RedactionPolicy {
    /// Builder: set the workspace root for path collapsing.
    pub fn with_workspace_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.workspace_root = Some(root.into());
        self
    }

    /// Builder: append a custom regex pattern.
    pub fn with_pattern(mut self, pattern: Regex) -> Self {
        self.custom_patterns.push(pattern);
        self
    }

    /// Builder: disable credential masking (use only when the caller has
    /// already sanitized content out-of-band).
    pub fn without_credential_masking(mut self) -> Self {
        self.mask_credentials = false;
        self
    }
}

// ── Compiled regex set ──────────────────────────────────────────────────

/// Lazy-compiled credential patterns. Compiled once per process via OnceLock.
struct CompiledPatterns {
    /// `sk_live_…`, `api-key=…`, `key_…`, etc.
    api_key: Regex,
    /// JWT-shaped tokens — three base64url segments separated by dots.
    jwt: Regex,
    /// Long opaque base64 blobs (>40 chars). Loose: catches PEM bodies,
    /// long bearer tokens, etc. Intentionally narrow to avoid eating prose.
    long_b64: Regex,
}

impl CompiledPatterns {
    fn get() -> &'static CompiledPatterns {
        use std::sync::OnceLock;
        static PATTERNS: OnceLock<CompiledPatterns> = OnceLock::new();
        PATTERNS.get_or_init(|| {
            CompiledPatterns {
                // Matches "sk", "api", "key", "token", "secret", "bearer"
                // followed by an optional separator and a long alphanumeric
                // run. Case-insensitive.
                api_key: Regex::new(
                    r"(?i)\b(?:sk|api[_-]?key|key|token|secret|bearer)[_\-=:\s]+[A-Za-z0-9_\-]{16,}\b",
                )
                .expect("api_key regex must compile"),
                // JWT: header.payload.signature, each base64url, payload
                // typically >= 20 chars.
                jwt: Regex::new(r"\beyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\b")
                    .expect("jwt regex must compile"),
                // Standalone base64 candidate > 40 chars.
                // The replacement step preserves the candidate only when every character is a hex digit (0-9a-fA-F), so SHA-1/SHA-256-style digests pass through unredacted; anything containing a non-hex letter, `+`, `/`, or padding is masked.
                long_b64: Regex::new(r"\b[A-Za-z0-9+/]{40,}={0,2}")
                    .expect("long_b64 regex must compile"),
            }
        })
    }
}

// ── Bundle types ────────────────────────────────────────────────────────

/// Top-level export bundle. Serializes to JSON or JSONL.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryBundle {
    /// Schema version. Bump when the on-disk shape changes.
    pub schema_version: u32,
    /// Static metadata describing the export.
    pub metadata: TrajectoryMetadata,
    /// Redacted conversation turns, in original order.
    pub messages: Vec<RedactedMessage>,
}

/// Static metadata recorded with each export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryMetadata {
    /// Agent UUID at export time.
    pub agent_id: String,
    /// Human-readable agent name (may have changed since the session began).
    pub agent_name: String,
    /// Session UUID.
    pub session_id: String,
    /// Optional human-readable session label.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_label: Option<String>,
    /// Model identifier at export time (e.g. `groq:llama-3.3-70b-versatile`).
    pub model: String,
    /// Provider name (e.g. `groq`, `anthropic`, `openai`).
    pub provider: String,
    /// SHA-256 hash of the system prompt — fingerprint without leaking content.
    pub system_prompt_sha256: String,
    /// Number of messages in the session.
    pub message_count: usize,
    /// Estimated context window token count at export time.
    pub context_window_tokens: u64,
    /// ISO-8601 UTC timestamp when the export was created.
    pub exported_at: String,
    /// `librefang-kernel` crate version.
    pub librefang_version: String,
    /// Whether credential masking was applied.
    pub redaction_credentials: bool,
    /// Whether workspace path collapsing was applied (root was set).
    pub redaction_workspace_paths: bool,
    /// Cache hit ratio for this trajectory's turns: `cache_read / (cache_read + cache_creation)`.
    /// `None` when the trajectory predates this field or the model didn't
    /// support prompt caching. `Some(0.0)` means caching was active but
    /// nothing hit (cold start / first turn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_ratio: Option<f32>,
}

/// A message turn after redaction. Keeps the original shape so consumers
/// can re-render it; only string contents are rewritten.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactedMessage {
    /// `system` / `user` / `assistant`.
    pub role: String,
    /// Whether the message was pinned.
    pub pinned: bool,
    /// ISO-8601 timestamp if recorded on the original message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    /// Redacted content blocks.
    pub content: Vec<RedactedBlock>,
}

/// A redacted content block. Mirrors `ContentBlock` but with strings already
/// sanitized.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RedactedBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        tool_name: String,
        content: String,
        is_error: bool,
    },
    Image {
        media_type: String,
        /// Base64 data is replaced with a placeholder; emit only the size.
        data_bytes: usize,
    },
    ImageFile {
        media_type: String,
        path: String,
    },
    Unknown,
}

/// Compute the prompt-cache hit ratio for an aggregate `TokenUsage`.
///
/// Thin re-export over [`TokenUsage::cache_hit_ratio`] kept for callers
/// that already pass usage through this module's public API.
pub fn compute_cache_hit_ratio(usage: &TokenUsage) -> Option<f32> {
    usage.cache_hit_ratio()
}

impl TrajectoryBundle {
    /// Serialize to a JSON value (full bundle as a single object).
    pub fn to_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }

    /// Serialize to NDJSON (JSON Lines): first line is the metadata header,
    /// subsequent lines are messages one-per-line. This is the audit-friendly
    /// shape that grep / jq / log tooling expects.
    pub fn to_jsonl(&self) -> String {
        let mut out = String::new();
        let header = serde_json::json!({
            "kind": "metadata",
            "schema_version": self.schema_version,
            "metadata": &self.metadata,
        });
        out.push_str(&header.to_string());
        out.push('\n');
        for (idx, m) in self.messages.iter().enumerate() {
            let line = serde_json::json!({
                "kind": "message",
                "index": idx,
                "message": m,
            });
            out.push_str(&line.to_string());
            out.push('\n');
        }
        out
    }

    /// Stamp the trajectory's metadata with a cache hit ratio computed from
    /// the supplied aggregate `TokenUsage`. Convenience wrapper around
    /// [`TokenUsage::cache_hit_ratio`].
    ///
    /// `TrajectoryExporter` itself never sees per-turn token counts (the
    /// `Session` substrate stores `context_window_tokens` only, not the
    /// `cache_creation` / `cache_read` split). This builder is the API
    /// surface for callers further up the stack — the HTTP export route
    /// and CLI exporter — that aggregate `TokenUsage` from the kernel's
    /// metering layer and stamp the bundle before serialization. Wiring
    /// those call sites is a follow-up.
    pub fn with_cache_hit_ratio(mut self, usage: &TokenUsage) -> Self {
        self.metadata.cache_hit_ratio = usage.cache_hit_ratio();
        self
    }
}

// ── Exporter ────────────────────────────────────────────────────────────

/// Reads sessions from the memory substrate and emits redacted bundles.
pub struct TrajectoryExporter {
    memory: Arc<MemorySubstrate>,
    policy: RedactionPolicy,
}

/// Caller-supplied agent context (so the exporter doesn't need to reach
/// back into the kernel registry).
#[derive(Debug, Clone)]
pub struct AgentContext {
    pub name: String,
    pub model: String,
    pub provider: String,
    pub system_prompt: String,
}

impl TrajectoryExporter {
    /// Create a new exporter.
    pub fn new(memory: Arc<MemorySubstrate>, policy: RedactionPolicy) -> Self {
        Self { memory, policy }
    }

    /// Export a single session. Returns `Err` if the session does not exist
    /// or does not belong to `agent_id`.
    pub fn export_session(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        agent: AgentContext,
    ) -> LibreFangResult<TrajectoryBundle> {
        let session = self.memory.get_session(session_id)?.ok_or_else(|| {
            LibreFangError::memory_msg(format!("session {} not found", session_id.0))
        })?;
        if session.agent_id != agent_id {
            return Err(LibreFangError::memory_msg(format!(
                "session {} does not belong to agent {}",
                session_id.0, agent_id.0
            )));
        }

        let messages: Vec<RedactedMessage> = session
            .messages
            .iter()
            .map(|m| self.redact_message(m))
            .collect();

        let metadata = TrajectoryMetadata {
            agent_id: agent_id.0.to_string(),
            agent_name: agent.name,
            session_id: session_id.0.to_string(),
            session_label: session.label.clone(),
            model: agent.model,
            provider: agent.provider,
            system_prompt_sha256: sha256_hex(agent.system_prompt.as_bytes()),
            message_count: session.messages.len(),
            context_window_tokens: session.context_window_tokens,
            exported_at: Utc::now().to_rfc3339(),
            librefang_version: env!("CARGO_PKG_VERSION").to_string(),
            redaction_credentials: self.policy.mask_credentials,
            redaction_workspace_paths: self.policy.workspace_root.is_some(),
            cache_hit_ratio: None,
        };

        Ok(TrajectoryBundle {
            schema_version: 1,
            metadata,
            messages,
        })
    }

    /// Build an empty bundle without consulting the memory substrate.
    ///
    /// Sessions are persisted lazily — a freshly spawned agent has a
    /// `session_id` but no DB row until the first message is written.
    /// Callers that have already verified ownership via the agent registry
    /// (e.g. `agent_entry.session_id == session_id`) can use this to emit
    /// an empty bundle for that "not yet persisted" case.
    pub fn empty_bundle(
        &self,
        agent_id: AgentId,
        session_id: SessionId,
        agent: AgentContext,
    ) -> TrajectoryBundle {
        let metadata = TrajectoryMetadata {
            agent_id: agent_id.0.to_string(),
            agent_name: agent.name,
            session_id: session_id.0.to_string(),
            session_label: None,
            model: agent.model,
            provider: agent.provider,
            system_prompt_sha256: sha256_hex(agent.system_prompt.as_bytes()),
            message_count: 0,
            context_window_tokens: 0,
            exported_at: Utc::now().to_rfc3339(),
            librefang_version: env!("CARGO_PKG_VERSION").to_string(),
            redaction_credentials: self.policy.mask_credentials,
            redaction_workspace_paths: self.policy.workspace_root.is_some(),
            cache_hit_ratio: None,
        };
        TrajectoryBundle {
            schema_version: 1,
            metadata,
            messages: Vec::new(),
        }
    }

    fn redact_message(&self, m: &Message) -> RedactedMessage {
        let role = match m.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
        .to_string();

        let blocks: Vec<RedactedBlock> = match &m.content {
            MessageContent::Text(s) => vec![RedactedBlock::Text {
                text: self.redact_text(s),
            }],
            MessageContent::Blocks(blocks) => blocks
                .iter()
                .map(|b| self.redact_block(b))
                .collect::<Vec<_>>(),
        };

        RedactedMessage {
            role,
            pinned: m.pinned,
            timestamp: m.timestamp.map(|t| t.to_rfc3339()),
            content: blocks,
        }
    }

    fn redact_block(&self, b: &ContentBlock) -> RedactedBlock {
        match b {
            ContentBlock::Text { text, .. } => RedactedBlock::Text {
                text: self.redact_text(text),
            },
            ContentBlock::Thinking { thinking, .. } => RedactedBlock::Thinking {
                thinking: self.redact_text(thinking),
            },
            ContentBlock::ToolUse {
                id, name, input, ..
            } => RedactedBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: self.redact_json(input.clone()),
            },
            ContentBlock::ToolResult {
                tool_use_id,
                tool_name,
                content,
                is_error,
                ..
            } => RedactedBlock::ToolResult {
                tool_use_id: tool_use_id.clone(),
                tool_name: tool_name.clone(),
                content: self.redact_text(content),
                is_error: *is_error,
            },
            ContentBlock::Image { media_type, data } => RedactedBlock::Image {
                media_type: media_type.clone(),
                data_bytes: data.len(),
            },
            ContentBlock::ImageFile { media_type, path } => RedactedBlock::ImageFile {
                media_type: media_type.clone(),
                path: self.redact_text(path),
            },
            ContentBlock::Unknown => RedactedBlock::Unknown,
        }
    }

    /// Redact a single string. Order matters: collapse paths first (so
    /// they're not eaten by the long-b64 matcher), then mask credentials.
    pub fn redact_text(&self, input: &str) -> String {
        let mut out = collapse_workspace_paths(input, self.policy.workspace_root.as_deref());

        if self.policy.mask_credentials {
            let p = CompiledPatterns::get();
            // JWT first (most specific shape).
            out = p.jwt.replace_all(&out, "<REDACTED:JWT>").into_owned();
            // Then api-key-shaped.
            out = p
                .api_key
                .replace_all(&out, "<REDACTED:CREDENTIAL>")
                .into_owned();
            // Then catch high-confidence standalone base64 (must come last).
            out = p
                .long_b64
                .replace_all(&out, |captures: &regex::Captures<'_>| {
                    let candidate = &captures[0];
                    if candidate.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                        candidate.to_string()
                    } else {
                        "<REDACTED:BLOB>".to_string()
                    }
                })
                .into_owned();
        }

        for re in &self.policy.custom_patterns {
            out = re.replace_all(&out, "<REDACTED>").into_owned();
        }

        out
    }

    /// Redact every string inside a JSON value. Keys are left untouched (they're typically not secret-bearing in tool inputs).
    ///
    /// The traversal is an explicit work stack rather than recursion, and the over-depth value is dropped iteratively too.
    /// `librefang-rl-export`'s `redact_metadata` was hardened this way and this function was not, which left two functions doing the same job in the same repository with only one of them safe on a deeply nested input.
    ///
    /// A `Value` that arrived through `serde_json::from_str` cannot exceed the parser's own 128-deep limit, but the tool inputs and outputs reaching here are not all parsed — a programmatically constructed value nests as far as its builder chose, and both the recursive walk and the recursive `Drop` glue for such a value run on the same stack frame budget.
    fn redact_json(&self, v: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;

        enum Work {
            Visit(Value, usize),
            FinishArray(usize),
            FinishObject(Vec<String>),
        }

        let mut work = vec![Work::Visit(v, 0)];
        let mut output: Vec<Value> = Vec::new();
        while let Some(item) = work.pop() {
            match item {
                Work::Visit(Value::String(s), _) => {
                    output.push(Value::String(self.redact_text(&s)));
                }
                Work::Visit(value @ (Value::Array(_) | Value::Object(_)), depth)
                    if depth >= MAX_REDACT_DEPTH =>
                {
                    drop_value_iteratively(value);
                    output.push(Value::String(TOO_DEEP_SENTINEL.to_string()));
                }
                Work::Visit(Value::Array(values), depth) => {
                    work.push(Work::FinishArray(values.len()));
                    work.extend(
                        values
                            .into_iter()
                            .rev()
                            .map(|value| Work::Visit(value, depth + 1)),
                    );
                }
                Work::Visit(Value::Object(map), depth) => {
                    let (keys, values): (Vec<_>, Vec<_>) = map.into_iter().unzip();
                    work.push(Work::FinishObject(keys));
                    work.extend(
                        values
                            .into_iter()
                            .rev()
                            .map(|value| Work::Visit(value, depth + 1)),
                    );
                }
                Work::Visit(value, _) => output.push(value),
                Work::FinishArray(len) => {
                    let values = output.split_off(output.len() - len);
                    output.push(Value::Array(values));
                }
                Work::FinishObject(keys) => {
                    let values = output.split_off(output.len() - keys.len());
                    output.push(Value::Object(keys.into_iter().zip(values).collect()));
                }
            }
        }
        output.pop().expect("redaction always produces one value")
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Depth past which a nested JSON value is replaced by a sentinel rather than walked.
/// Matches `MAX_METADATA_DEPTH` in `librefang-rl-export`, and matches `serde_json`'s own parser limit, so nothing that arrived over the wire is affected.
const MAX_REDACT_DEPTH: usize = 128;

/// Stand-in for a subtree that exceeded `MAX_REDACT_DEPTH`.
const TOO_DEEP_SENTINEL: &str = "<REDACTED:TOO_DEEP>";

/// Drop a `Value` without recursing.
///
/// Dropping a deeply nested value recurses through `Drop` glue, so a value too deep to walk is also too deep to drop the ordinary way — replacing the walk alone would move the overflow from the traversal to the end of the scope.
fn drop_value_iteratively(value: serde_json::Value) {
    use serde_json::Value;
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        match value {
            Value::Array(values) => pending.extend(values),
            Value::Object(map) => pending.extend(map.into_values()),
            _ => {}
        }
    }
}

fn collapse_workspace_paths(input: &str, root: Option<&std::path::Path>) -> String {
    let Some(root) = root else {
        return input.to_string();
    };
    let root_str = root.to_string_lossy();
    if root_str.is_empty() {
        return input.to_string();
    }
    // Replace forward-slash form. We don't try to handle UNC / Windows
    // backslashes here — the librefang workspace_root is normalized to
    // forward slashes upstream. Callers on Windows can pre-normalize if
    // needed.
    let normalized = trim_workspace_root(&root_str.replace('\\', "/"));
    let mut out = replace_workspace_root(input, &normalized);
    // Also handle the original (non-normalized) form for robustness.
    let original = trim_workspace_root(root_str.as_ref());
    if normalized != original {
        out = replace_workspace_root(&out, &original);
    }
    out
}

fn trim_workspace_root(root: &str) -> String {
    let trimmed = root.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        root.to_string()
    } else {
        trimmed.to_string()
    }
}

fn replace_workspace_root(input: &str, root: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut cursor = 0;
    for (relative_start, _) in input.match_indices(root) {
        let start = relative_start;
        let end = start + root.len();
        let path_start_boundary = input[..start].chars().next_back().is_none_or(|previous| {
            !previous.is_alphanumeric() && !matches!(previous, '/' | '\\' | '.' | '_' | '-')
        });
        let filesystem_root = root == "/" || root == "\\";
        let url_scheme_separator = filesystem_root
            && root == "/"
            && input[..start].ends_with(':')
            && input[end..].starts_with('/');
        let component_boundary = filesystem_root
            || input[end..]
                .chars()
                .next()
                .is_none_or(|next| !next.is_alphanumeric() && !matches!(next, '_' | '-' | '.'));
        if path_start_boundary && component_boundary && !url_scheme_separator {
            output.push_str(&input[cursor..start]);
            output.push_str("<WORKSPACE>");
            if filesystem_root {
                output.push_str(root);
            }
            cursor = end;
        }
    }
    output.push_str(&input[cursor..]);
    output
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

// ── Tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn policy_with_workspace(root: &str) -> RedactionPolicy {
        RedactionPolicy::default().with_workspace_root(PathBuf::from(root))
    }

    fn dummy_exporter(policy: RedactionPolicy) -> TrajectoryExporter {
        // We don't exercise memory in redaction-only tests; build a
        // throwaway in-memory substrate so Arc<MemorySubstrate> exists.
        let mem = MemorySubstrate::open_in_memory(0.01).expect("substrate boots");
        TrajectoryExporter::new(Arc::new(mem), policy)
    }

    /// Build `[[[ ... "leaf" ... ]]]` nested `depth` deep, without recursing.
    fn nested_array(depth: usize, leaf: serde_json::Value) -> serde_json::Value {
        let mut v = leaf;
        for _ in 0..depth {
            v = serde_json::Value::Array(vec![v]);
        }
        v
    }

    #[test]
    fn redact_json_still_rewrites_strings_at_every_level() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let input = serde_json::json!({
            "outer": {
                "list": ["sk_live_abcdef0123456789ABCDEF", 7, true, null],
                "kept": "ordinary text"
            }
        });
        let out = exp.redact_json(input);
        let rendered = out.to_string();
        assert!(
            !rendered.contains("sk_live_abcdef0123456789ABCDEF"),
            "leaked: {rendered}"
        );
        assert!(rendered.contains("<REDACTED"), "no placeholder: {rendered}");
        // Structure and non-string scalars survive the iterative rebuild.
        assert_eq!(out["outer"]["kept"], serde_json::json!("ordinary text"));
        assert_eq!(out["outer"]["list"][1], serde_json::json!(7));
        assert_eq!(out["outer"]["list"][2], serde_json::json!(true));
        assert!(out["outer"]["list"][3].is_null());
    }

    #[test]
    fn redact_json_preserves_object_keys_and_ordering() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let input = serde_json::json!({"b": "two", "a": "one", "c": {"z": "zed"}});
        let out = exp.redact_json(input.clone());
        let before: Vec<&String> = input.as_object().unwrap().keys().collect();
        let after: Vec<&String> = out.as_object().unwrap().keys().collect();
        assert_eq!(before, after, "key order changed");
        assert_eq!(out["c"]["z"], serde_json::json!("zed"));
    }

    #[test]
    fn redact_json_replaces_a_subtree_past_the_depth_cap() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let deep = nested_array(MAX_REDACT_DEPTH + 5, serde_json::json!("leaf"));
        let out = exp.redact_json(deep);

        // Walk down to the sentinel without recursing.
        let mut cur = &out;
        let mut levels = 0usize;
        while let Some(inner) = cur.get(0) {
            cur = inner;
            levels += 1;
        }
        assert_eq!(
            cur,
            &serde_json::Value::String(TOO_DEEP_SENTINEL.to_string()),
            "over-depth subtree was not replaced"
        );
        assert_eq!(levels, MAX_REDACT_DEPTH, "cap applied at the wrong depth");
    }

    /// The recursive walk this replaced overflowed the stack on a value this deep, taking the whole daemon with it rather than returning an error.
    /// A recursion-based implementation cannot pass this test at any depth cap, because the recursive `Drop` glue overflows even if the walk is bounded.
    #[test]
    fn redact_json_survives_a_value_far_deeper_than_any_stack() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let deep = nested_array(200_000, serde_json::json!("sk_live_abcdef0123456789ABCDEF"));
        let out = exp.redact_json(deep);
        // The credential is inside the discarded subtree, so it cannot survive.
        assert!(!out.to_string().contains("sk_live_abcdef0123456789ABCDEF"));
    }

    #[test]
    fn redacts_api_key_shaped_strings() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let s = exp.redact_text("here is my key: sk_live_abcdef0123456789ABCDEF and more text");
        assert!(s.contains("<REDACTED:CREDENTIAL>"), "got: {s}");
        assert!(!s.contains("sk_live_abcdef0123456789ABCDEF"), "leaked: {s}");
    }

    #[test]
    fn redacts_bearer_tokens() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let s = exp.redact_text("Authorization: Bearer abcdef0123456789ABCDEF0123456789");
        assert!(s.contains("<REDACTED"), "got: {s}");
    }

    #[test]
    fn redacts_jwt_shaped_tokens() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.SflKxwRJSMeKKF2QT4fwpMeJf36POk6yJV_adQssw5c";
        let s = exp.redact_text(&format!("token={}", jwt));
        assert!(
            s.contains("<REDACTED:JWT>") || s.contains("<REDACTED"),
            "got: {s}"
        );
        assert!(!s.contains(jwt), "jwt leaked: {s}");
    }

    #[test]
    fn collapses_workspace_paths() {
        let exp = dummy_exporter(policy_with_workspace(
            "/home/alice/.librefang/workspaces/agent42",
        ));
        let s = exp.redact_text("opened /home/alice/.librefang/workspaces/agent42/notes.md ok");
        assert!(s.contains("<WORKSPACE>/notes.md"), "got: {s}");
        assert!(!s.contains("/home/alice"), "leaked path: {s}");
    }

    #[test]
    fn workspace_collapse_requires_component_boundary() {
        let exp = dummy_exporter(policy_with_workspace("/home/alice"));
        let s = exp.redact_text(
            "keep /home/alicey/file and /backup/home/alice/file and https://host/home/alice/file; redact /home/alice/file",
        );
        assert_eq!(
            s,
            "keep /home/alicey/file and /backup/home/alice/file and https://host/home/alice/file; redact <WORKSPACE>/file"
        );
    }

    #[test]
    fn workspace_collapse_requires_component_boundary_at_end_too() {
        // A bare mention of the workspace root followed by punctuation (not another path separator, and not end-of-string) must still count as a boundary — otherwise the path leaks whenever it's not immediately followed by `/` or `\`.
        let exp = dummy_exporter(policy_with_workspace("/home/alice"));
        assert_eq!(
            exp.redact_text("workspace root is /home/alice, please don't touch it"),
            "workspace root is <WORKSPACE>, please don't touch it"
        );
    }

    #[test]
    fn workspace_collapse_accepts_text_delimiters() {
        let exp = dummy_exporter(policy_with_workspace("/home/alice"));
        assert_eq!(
            exp.redact_text("workspace:/home/alice/file and `/home/alice/notes`"),
            "workspace:<WORKSPACE>/file and `<WORKSPACE>/notes`"
        );
    }

    #[test]
    fn workspace_collapse_normalizes_trailing_separators() {
        let unix = dummy_exporter(policy_with_workspace("/home/alice/"));
        assert_eq!(
            unix.redact_text("opened /home/alice/file"),
            "opened <WORKSPACE>/file"
        );

        let windows = dummy_exporter(policy_with_workspace(r"C:\Users\alice\"));
        assert_eq!(
            windows.redact_text(r"opened C:\Users\alice\file"),
            r"opened <WORKSPACE>\file"
        );
    }

    #[test]
    fn workspace_collapse_handles_filesystem_root_once_per_absolute_path() {
        let exp = dummy_exporter(policy_with_workspace("/"));
        assert_eq!(
            exp.redact_text("opened /etc/passwd and /var/lib/data; keep https://host/path"),
            "opened <WORKSPACE>/etc/passwd and <WORKSPACE>/var/lib/data; keep https://host/path"
        );
    }

    #[test]
    fn blob_redaction_preserves_hex_digests() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let blob = "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo/0123456789A";
        let s = exp.redact_text(&format!("sha1={sha1} sha256={sha256} blob={blob}"));
        assert!(s.contains(sha1), "SHA-1 digest was over-redacted: {s}");
        assert!(s.contains(sha256), "SHA-256 digest was over-redacted: {s}");
        assert!(s.contains("<REDACTED:BLOB>"), "base64 blob leaked: {s}");
        assert!(!s.contains(blob), "base64 blob leaked: {s}");
    }

    #[test]
    fn blob_redaction_masks_unpadded_alphanumeric_base64() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let blob = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuv";
        let s = exp.redact_text(&format!("blob={blob}"));
        assert_eq!(s, "blob=<REDACTED:BLOB>");
    }

    #[test]
    fn leaves_short_strings_alone() {
        let exp = dummy_exporter(RedactionPolicy::default());
        let s = exp.redact_text("hello world this is a normal message");
        assert_eq!(s, "hello world this is a normal message");
    }

    #[test]
    fn custom_pattern_applies() {
        let policy = RedactionPolicy::default()
            .with_pattern(Regex::new(r"INTERNAL-[A-Z]{4}-\d{4}").expect("valid"));
        let exp = dummy_exporter(policy);
        let s = exp.redact_text("ticket=INTERNAL-ACME-0042 priority=high");
        assert!(s.contains("<REDACTED>"), "got: {s}");
        assert!(!s.contains("INTERNAL-ACME-0042"), "leaked: {s}");
    }

    #[test]
    fn jsonl_emits_metadata_then_messages() {
        let bundle = TrajectoryBundle {
            schema_version: 1,
            metadata: TrajectoryMetadata {
                agent_id: "00000000-0000-0000-0000-000000000001".into(),
                agent_name: "test".into(),
                session_id: "00000000-0000-0000-0000-000000000002".into(),
                session_label: None,
                model: "test-model".into(),
                provider: "ollama".into(),
                system_prompt_sha256: "deadbeef".into(),
                message_count: 1,
                context_window_tokens: 0,
                exported_at: "2026-01-01T00:00:00Z".into(),
                librefang_version: "0.0.0".into(),
                redaction_credentials: true,
                redaction_workspace_paths: false,
                cache_hit_ratio: None,
            },
            messages: vec![RedactedMessage {
                role: "user".into(),
                pinned: false,
                timestamp: None,
                content: vec![RedactedBlock::Text { text: "hi".into() }],
            }],
        };
        let jsonl = bundle.to_jsonl();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"kind\":\"metadata\""));
        assert!(lines[1].contains("\"kind\":\"message\""));
        let json = bundle.to_json().expect("bundle must serialize");
        assert_eq!(json["schema_version"], 1);
    }

    // ── cache_hit_ratio metadata field (PR-2/2 M2) ─────────────────────────

    fn sample_metadata(cache_hit_ratio: Option<f32>) -> TrajectoryMetadata {
        TrajectoryMetadata {
            agent_id: "00000000-0000-0000-0000-000000000001".into(),
            agent_name: "test".into(),
            session_id: "00000000-0000-0000-0000-000000000002".into(),
            session_label: None,
            model: "test-model".into(),
            provider: "ollama".into(),
            system_prompt_sha256: "deadbeef".into(),
            message_count: 0,
            context_window_tokens: 0,
            exported_at: "2026-01-01T00:00:00Z".into(),
            librefang_version: "0.0.0".into(),
            redaction_credentials: true,
            redaction_workspace_paths: false,
            cache_hit_ratio,
        }
    }

    #[test]
    fn trajectory_metadata_cache_hit_ratio_serde_round_trip() {
        let meta = sample_metadata(Some(0.85));
        let json = serde_json::to_string(&meta).expect("serialize");
        assert!(
            json.contains("\"cache_hit_ratio\":0.85"),
            "field missing in JSON: {json}"
        );
        let back: TrajectoryMetadata = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.cache_hit_ratio, Some(0.85));
    }

    #[test]
    fn trajectory_metadata_cache_hit_ratio_legacy_compat() {
        // Legacy trajectory JSON predating the field — must deserialize
        // cleanly with `cache_hit_ratio == None` and the field must be
        // omitted on re-serialization.
        let legacy = r#"{
            "agent_id":"00000000-0000-0000-0000-000000000001",
            "agent_name":"test",
            "session_id":"00000000-0000-0000-0000-000000000002",
            "model":"test-model",
            "provider":"ollama",
            "system_prompt_sha256":"deadbeef",
            "message_count":0,
            "context_window_tokens":0,
            "exported_at":"2026-01-01T00:00:00Z",
            "librefang_version":"0.0.0",
            "redaction_credentials":true,
            "redaction_workspace_paths":false
        }"#;
        let meta: TrajectoryMetadata = serde_json::from_str(legacy).expect("legacy deserialize");
        assert_eq!(meta.cache_hit_ratio, None);

        let reserialized = serde_json::to_string(&meta).expect("reserialize");
        assert!(
            !reserialized.contains("cache_hit_ratio"),
            "None should be skipped: {reserialized}"
        );
    }

    #[test]
    fn compute_cache_hit_ratio_delegates_to_token_usage() {
        // Smoke test for the public re-export — full coverage of the
        // ratio math lives in `librefang_types::message::TokenUsage`.
        assert_eq!(compute_cache_hit_ratio(&TokenUsage::default()), None);
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 0,
            cache_creation_input_tokens: 30,
            cache_read_input_tokens: 70,
        };
        let ratio = compute_cache_hit_ratio(&usage).expect("ratio set");
        assert!((ratio - 0.7).abs() < 1e-6, "got {ratio}");
    }

    #[test]
    fn bundle_with_cache_hit_ratio_stamps_metadata() {
        let bundle = TrajectoryBundle {
            schema_version: 1,
            metadata: sample_metadata(None),
            messages: Vec::new(),
        };
        let usage = TokenUsage {
            input_tokens: 100,
            output_tokens: 0,
            cache_creation_input_tokens: 30,
            cache_read_input_tokens: 70,
        };
        let stamped = bundle.with_cache_hit_ratio(&usage);
        let ratio = stamped.metadata.cache_hit_ratio.expect("ratio set");
        assert!((ratio - 0.7).abs() < 1e-6, "got {ratio}");
    }

    #[test]
    fn sha256_known_vector() {
        // SHA-256("abc") = ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
