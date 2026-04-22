//! Error types for `librefang-uar-spec`.

use thiserror::Error;

/// Errors produced by the UAR spec parser and translator.
#[derive(Debug, Error)]
pub enum UarSpecError {
    /// The Markdown document has no `# Agent: <name>` heading.
    #[error("missing agent heading — expected `# Agent: <name>` at the top of the document")]
    MissingAgentHeading,

    /// A required UAR-AGENT-MD section was not found in the document.
    #[error("required section `## {section}` is missing from the UAR-AGENT-MD document")]
    MissingRequiredSection { section: &'static str },

    /// A field value could not be parsed.
    #[error("failed to parse field `{field}` in section `{section}`: {reason}")]
    FieldParseFailed {
        field: &'static str,
        section: &'static str,
        reason: String,
    },

    /// The `AgentArtifact` → `AgentManifest` translation failed.
    #[error("translation to AgentManifest failed: {0}")]
    TranslationFailed(String),

    /// JSON serialization / deserialization error.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, UarSpecError>;
