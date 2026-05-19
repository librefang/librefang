//! Artifact retrieval tool (#3347).

/// Implementation of the `read_artifact` tool.
///
/// Reads up to `length` bytes from the artifact identified by `handle`,
/// starting at `offset`.  Both parameters are optional (defaults: 0 and 4096).
/// The result is UTF-8 text: binary blobs are lossily decoded.
pub(super) async fn tool_read_artifact(
    input: &serde_json::Value,
    artifact_dir: &std::path::Path,
) -> Result<String, String> {
    let handle = input["handle"]
        .as_str()
        .ok_or("Missing required parameter 'handle'")?;

    let offset = input["offset"].as_u64().unwrap_or(0) as usize;
    let length = input["length"]
        .as_u64()
        .unwrap_or(4096)
        .min(crate::artifact_store::MAX_READ_LENGTH as u64) as usize;

    let bytes = crate::artifact_store::read(handle, offset, length, artifact_dir)?;

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
