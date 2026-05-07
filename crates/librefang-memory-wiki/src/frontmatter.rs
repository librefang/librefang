//! YAML frontmatter for vault pages.
//!
//! Layout on disk:
//!
//! ```markdown
//! ---
//! topic: project-conventions
//! created: 2026-05-06T10:30:00Z
//! updated: 2026-05-06T11:00:00Z
//! content_sha256: 6a4f...
//! provenance:
//!   - agent: agent_xyz
//!     session: sess_abc
//!     channel: cli
//!     turn: 4
//!     at: 2026-05-06T10:30:00Z
//! ---
//!
//! body markdown ...
//! ```
//!
//! `content_sha256` is the hash of the body *after* the trailing `---` and
//! one separating newline have been stripped. It is the same field the
//! compiler uses to decide whether the on-disk page diverged from the last
//! known compiler output (see `WikiVault::write`).
//!
//! Compatibility note: the parser tolerates a missing or malformed
//! frontmatter block — pages hand-authored in Obsidian without frontmatter
//! still load (`Frontmatter::default_for(topic)` synthesises one with
//! `created = updated = now`, an empty provenance list, and a re-computed
//! body hash on the next compiler pass).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::WikiError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceEntry {
    pub agent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<u64>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub topic: String,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
    #[serde(default)]
    pub content_sha256: String,
    #[serde(default)]
    pub provenance: Vec<ProvenanceEntry>,
}

impl Frontmatter {
    pub fn default_for(topic: &str) -> Self {
        let now = Utc::now();
        Self {
            topic: topic.to_string(),
            created: now,
            updated: now,
            content_sha256: String::new(),
            provenance: Vec::new(),
        }
    }

    /// Compute the canonical content hash of `body` (the markdown that
    /// follows the closing `---` line). Trailing newline is stripped first
    /// so an editor adding/removing a final newline doesn't flip the hash.
    pub fn hash_body(body: &str) -> String {
        let normalized = body.strip_suffix('\n').unwrap_or(body);
        let mut hasher = Sha256::new();
        hasher.update(normalized.as_bytes());
        format!("{:x}", hasher.finalize())
    }
}

/// Split a raw page into its frontmatter block and body. Tolerates pages
/// that were hand-authored without any frontmatter — those return
/// `(None, raw)` and the caller can synthesize a default header.
pub fn split(raw: &str) -> (Option<&str>, &str) {
    let rest = match raw.strip_prefix("---\n") {
        Some(r) => r,
        None => return (None, raw),
    };
    if let Some(end) = rest.find("\n---\n") {
        let yaml = &rest[..end];
        let body = &rest[end + "\n---\n".len()..];
        // Render() emits `---\n\n<body>` so the visual blank line between
        // frontmatter and body is part of the separator, not the body.
        // Strip a single leading newline so `split(render(fm, body)) == body`.
        let body = body.strip_prefix('\n').unwrap_or(body);
        return (Some(yaml), body);
    }
    if let Some(end) = rest.find("\n---") {
        if rest[end + "\n---".len()..].is_empty() {
            let yaml = &rest[..end];
            return (Some(yaml), "");
        }
    }
    (None, raw)
}

/// Parse the frontmatter block into a strongly typed `Frontmatter`.
pub fn parse(yaml: &str, topic: &str) -> Result<Frontmatter, WikiError> {
    serde_yaml::from_str::<Frontmatter>(yaml).map_err(|source| WikiError::Frontmatter {
        topic: topic.to_string(),
        source,
    })
}

/// Serialise `frontmatter` and `body` into the on-disk page representation.
/// `body` is written verbatim — the caller is expected to have already
/// rewritten any `[[link]]` placeholders for the active render mode.
pub fn render(frontmatter: &Frontmatter, body: &str) -> Result<String, WikiError> {
    let yaml = serde_yaml::to_string(frontmatter).map_err(|source| WikiError::Frontmatter {
        topic: frontmatter.topic.clone(),
        source,
    })?;
    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str("---\n");
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str("---\n\n");
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_extracts_frontmatter_and_body() {
        let raw = "---\ntopic: foo\n---\n\nhello\n";
        let (yaml, body) = split(raw);
        assert_eq!(yaml, Some("topic: foo"));
        assert_eq!(body, "hello\n");
    }

    #[test]
    fn split_handles_no_frontmatter() {
        let raw = "just plain markdown\n";
        let (yaml, body) = split(raw);
        assert!(yaml.is_none());
        assert_eq!(body, "just plain markdown\n");
    }

    #[test]
    fn render_then_split_roundtrips() {
        let fm = Frontmatter::default_for("widgets");
        let rendered = render(&fm, "body line\n").unwrap();
        let (yaml, body) = split(&rendered);
        let parsed = parse(yaml.unwrap(), "widgets").unwrap();
        assert_eq!(parsed.topic, "widgets");
        assert_eq!(body, "body line\n");
    }

    #[test]
    fn hash_body_normalises_trailing_newline() {
        let a = Frontmatter::hash_body("hello world");
        let b = Frontmatter::hash_body("hello world\n");
        assert_eq!(a, b);
    }
}
