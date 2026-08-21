//! Canonical validation for user-visible resource names that become filenames.
//!
//! Agent templates, workflows, and anything else whose name is used to address an on-disk artifact share one rule: 1–64 characters drawn from `[A-Za-z0-9_-]`.
//! The charset is deliberately narrow — it is path-separator-free, shell-metacharacter-free, and case-insensitively round-trippable on Windows — so a name can be joined onto a directory without a traversal check at every call site.
//!
//! This module is the single source of truth for that rule (#6943 review: the same length + charset check had been copy-pasted verbatim into `librefang-runtime`'s `workflow_create` tool, `librefang-kernel`'s `create_workflow` handle, and `librefang-api`'s `validate_template_name`, so a future tightening would have had to land in three places or silently diverge).

/// Maximum length, in bytes, of a validated resource name.
pub const MAX_RESOURCE_NAME_LEN: usize = 64;

/// Why a resource name was rejected.
///
/// Callers map this onto their own error type — `ToolError::InvalidParameter`, `KernelOpError::Internal`, an HTTP 400 body — so the variant carries the classification and leaves the wording to the surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceNameError {
    /// Empty, or longer than [`MAX_RESOURCE_NAME_LEN`] bytes.
    Length,
    /// Contains a character outside `[A-Za-z0-9_-]`.
    Charset,
}

impl ResourceNameError {
    /// A short human-readable reason, suitable for embedding in an error message aimed at an operator or an LLM.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Length => "must be 1-64 characters",
            Self::Charset => "must contain only [A-Za-z0-9_-]",
        }
    }
}

impl std::fmt::Display for ResourceNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason())
    }
}

impl std::error::Error for ResourceNameError {}

/// Validate a resource name that will be used to address an on-disk artifact.
///
/// Length is measured in bytes rather than `char`s because the charset is ASCII-only, so the two are equal for every accepted input and a byte check rejects multi-byte input earlier.
pub fn validate_resource_name(name: &str) -> Result<(), ResourceNameError> {
    if name.is_empty() || name.len() > MAX_RESOURCE_NAME_LEN {
        return Err(ResourceNameError::Length);
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ResourceNameError::Charset);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_documented_charset() {
        for name in ["assistant", "customer-support", "coder_v2", "a1", "A-_9"] {
            assert!(
                validate_resource_name(name).is_ok(),
                "{name} must be accepted"
            );
        }
        assert!(validate_resource_name(&"a".repeat(MAX_RESOURCE_NAME_LEN)).is_ok());
    }

    #[test]
    fn rejects_empty_and_overlong_names_as_length_errors() {
        assert_eq!(validate_resource_name(""), Err(ResourceNameError::Length));
        assert_eq!(
            validate_resource_name(&"a".repeat(MAX_RESOURCE_NAME_LEN + 1)),
            Err(ResourceNameError::Length)
        );
    }

    #[test]
    fn rejects_path_and_shell_metacharacters_as_charset_errors() {
        // The charset is what keeps a validated name safe to `Path::join` without a separate traversal check.
        for name in [
            "bad name",
            "../escape",
            "a/b",
            "a\\b",
            "a.b",
            "a;b",
            "naïve",
            "a\0b",
        ] {
            assert_eq!(
                validate_resource_name(name),
                Err(ResourceNameError::Charset),
                "{name:?} must be rejected on charset grounds"
            );
        }
    }

    #[test]
    fn multibyte_input_over_the_byte_limit_is_a_length_error() {
        // 'é' is 2 bytes, so 33 of them exceed the 64-byte cap while being only 33 chars.
        assert_eq!(
            validate_resource_name(&"é".repeat(33)),
            Err(ResourceNameError::Length)
        );
    }
}
