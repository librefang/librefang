//! Artifact retrieval tool (#3347).
//!
//! Migrated from `Result<String, String>` to `Result<String, ToolError>`
//! (#3576, slice 2). Completes the slice — `event` / `goal` / `sandbox` /
//! `system` were already typed; this was the last `Result<String, String>`
//! return in the group.

use super::error::{ToolError, ToolResult};

/// Implementation of the `read_artifact` tool.
///
/// Reads up to `length` bytes from the artifact identified by `handle`,
/// starting at `offset`.  Both parameters are optional (defaults: 0 and 4096).
/// The result is UTF-8 text: binary blobs are lossily decoded.
pub(super) async fn tool_read_artifact(
    input: &serde_json::Value,
    artifact_dir: &std::path::Path,
) -> ToolResult {
    let handle = input["handle"]
        .as_str()
        .ok_or(ToolError::MissingParameter("handle"))?;

    let offset: usize = match input.get("offset") {
        Some(v) => {
            let n = v.as_u64().ok_or(ToolError::InvalidParameter {
                name: "offset",
                reason: "must be a non-negative integer".into(),
            })?;
            usize::try_from(n).map_err(|_| ToolError::InvalidParameter {
                name: "offset",
                reason: "is too large for this platform".into(),
            })?
        }
        None => 0,
    };

    let length: usize = match input.get("length") {
        Some(v) => {
            let n = v.as_u64().ok_or(ToolError::InvalidParameter {
                name: "length",
                reason: "must be a non-negative integer".into(),
            })?;
            let n = usize::try_from(n).map_err(|_| ToolError::InvalidParameter {
                name: "length",
                reason: "is too large for this platform".into(),
            })?;
            if n == 0 {
                return Err(ToolError::InvalidParameter {
                    name: "length",
                    reason: "must be greater than 0".into(),
                });
            }
            n
        }
        None => 4096,
    };

    let length = length.min(crate::artifact_store::MAX_READ_LENGTH);

    let handle_owned = handle.to_string();
    let dir_owned = artifact_dir.to_path_buf();
    let bytes = tokio::task::spawn_blocking(move || {
        crate::artifact_store::read(&handle_owned, offset, length, &dir_owned)
    })
    .await
    .map_err(|e| ToolError::Internal(format!("read_artifact task panicked: {e}")))?
    .map_err(ToolError::upstream_msg)?;

    if bytes.is_empty() {
        return Ok(format!(
            "[read_artifact: {handle} | offset={offset}] — no more content (past end of artifact)"
        ));
    }

    let text = String::from_utf8_lossy(&bytes);
    Ok(format!(
        "[read_artifact: {handle} | offset={offset} | {} bytes read]\n{text}",
        bytes.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn read_artifact_without_handle_returns_missing_parameter() {
        let dir = std::env::temp_dir();
        let r = tool_read_artifact(&json!({}), &dir).await;
        assert!(matches!(r, Err(ToolError::MissingParameter("handle"))));
    }

    #[tokio::test]
    async fn read_artifact_non_integer_offset_returns_invalid_parameter() {
        let dir = std::env::temp_dir();
        let r = tool_read_artifact(&json!({"handle": "h", "offset": "nope"}), &dir).await;
        assert!(matches!(
            r,
            Err(ToolError::InvalidParameter { name: "offset", .. })
        ));
    }

    #[tokio::test]
    async fn read_artifact_zero_length_returns_invalid_parameter() {
        let dir = std::env::temp_dir();
        let r = tool_read_artifact(&json!({"handle": "h", "length": 0}), &dir).await;
        assert!(matches!(
            r,
            Err(ToolError::InvalidParameter { name: "length", .. })
        ));
    }

    #[tokio::test]
    async fn read_artifact_missing_artifact_surfaces_as_upstream() {
        // A well-formed but nonexistent handle (passes `ArtifactHandle::parse`'s
        // `sha256:` + 64-hex check, then misses `path.exists()`): the not-found
        // lookup failure inside `artifact_store::read` (still `Result<_, String>`)
        // is lifted onto the `Upstream` variant verbatim, not flattened into
        // `Internal`. Asserting the message keeps this on the not-found path —
        // a bare-prefix handle would short-circuit in `parse` and exercise a
        // different error entirely while still matching `Upstream`.
        let dir = std::env::temp_dir();
        let handle = "sha256:".to_string() + &"0".repeat(64);
        let r = tool_read_artifact(&json!({ "handle": handle }), &dir).await;
        match r {
            Err(ToolError::Upstream { message, .. }) => {
                assert!(message.contains("not found"), "got: {message}");
            }
            other => panic!("expected ToolError::Upstream, got: {other:?}"),
        }
    }
}
