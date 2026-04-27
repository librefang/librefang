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
                // Standalone base64-ish blob > 40 chars. Word-bounded.
                long_b64: Regex::new(r"\b[A-Za-z0-9+/=]{40,}\b")
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
    pub fn to_json(&self) -> serde_json::Value {
        // serde_json on a derive-ser type cannot fail unless a custom impl
        // panics; bundle has only string/usize/Vec primitives so this is safe.
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
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
        let session = self
            .memory
            .get_session(session_id)?
            .ok_or_else(|| LibreFangError::Memory(format!("session {} not found", session_id.0)))?;
        if session.agent_id != agent_id {
            return Err(LibreFangError::Memory(format!(
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
            // Then catch-all long base64 (must come last; broadest).
            out = p.long_b64.replace_all(&out, "<REDACTED:BLOB>").into_owned();
        }

        for re in &self.policy.custom_patterns {
            out = re.replace_all(&out, "<REDACTED>").into_owned();
        }

        out
    }

    /// Recursively redact every string inside a JSON value. Keys are left
    /// untouched (they're typically not secret-bearing in tool inputs).
    fn redact_json(&self, v: serde_json::Value) -> serde_json::Value {
        use serde_json::Value;
        match v {
            Value::String(s) => Value::String(self.redact_text(&s)),
            Value::Array(arr) => {
                Value::Array(arr.into_iter().map(|x| self.redact_json(x)).collect())
            }
            Value::Object(map) => {
                let mut out = serde_json::Map::with_capacity(map.len());
                for (k, val) in map {
                    out.insert(k, self.redact_json(val));
                }
                Value::Object(out)
            }
            other => other,
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────

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
    let normalized = root_str.replace('\\', "/");
    let mut out = input.replace(normalized.as_str(), "<WORKSPACE>");
    // Also handle the original (non-normalized) form for robustness.
    if normalized != root_str.as_ref() {
        out = out.replace(root_str.as_ref(), "<WORKSPACE>");
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    // Minimal SHA-256 via the `sha2` crate would add a dep; we already
    // have a kernel-internal need for prompt fingerprinting elsewhere,
    // so we use a tiny inline implementation here that delegates to the
    // kernel's existing hasher if present. To keep this module self-contained
    // and avoid a new dependency, fall back to FNV-1a 64 if no sha is
    // available — clearly labelled in the field name (`system_prompt_sha256`
    // is misleading then). To avoid that mismatch, depend on the fact that
    // `librefang-kernel` already pulls in `hex` and use a vendored sha256
    // through the `chrono` chain… not viable.
    //
    // Pragmatic answer: use the existing `sha2`-shaped path through the
    // hex crate by leveraging blake3 which the workspace ships? No — keep
    // this module dep-free and emit a stable non-cryptographic digest with
    // a clear field name. Rename field accordingly.
    //
    // Implementation: SHA-256 via a minimal inline implementation.
    sha256(bytes)
}

/// Minimal SHA-256 implementation (RFC 6234). Inlined to keep this module
/// dependency-free; the hash is used only as a stable fingerprint of the
/// system prompt so consumers can correlate exports without leaking the
/// prompt content itself.
fn sha256(input: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // Pre-processing — pad to 512-bit blocks.
    let bit_len = (input.len() as u64).wrapping_mul(8);
    let mut data = input.to_vec();
    data.push(0x80);
    while data.len() % 64 != 56 {
        data.push(0);
    }
    data.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in data.chunks_exact(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh) =
            (h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]);
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }
    let mut out = String::with_capacity(64);
    for word in h {
        out.push_str(&format!("{:08x}", word));
    }
    out
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
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // SHA-256("") = e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
