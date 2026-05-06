use thiserror::Error;

#[derive(Debug, Error)]
pub enum WikiError {
    #[error("wiki is disabled (set `[memory_wiki] enabled = true` in config.toml)")]
    Disabled,

    #[error("wiki mode `{0}` is not yet implemented in v1 — only `isolated` is wired")]
    ModeNotImplemented(&'static str),

    #[error("invalid topic `{topic}`: {reason}")]
    InvalidTopic { topic: String, reason: &'static str },

    #[error("topic `{0}` not found")]
    NotFound(String),

    #[error(
        "page `{topic}` was modified externally after the last compiler run \
         (disk mtime newer than recorded). Re-read the file and merge manually, \
         or pass `accept_external = true` to overwrite."
    )]
    HandEditConflict { topic: String },

    #[error("frontmatter parse error in `{topic}`: {source}")]
    Frontmatter {
        topic: String,
        #[source]
        source: serde_yaml::Error,
    },

    #[error("vault io error at `{path}`: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

pub type WikiResult<T> = Result<T, WikiError>;

impl WikiError {
    pub(crate) fn io(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
