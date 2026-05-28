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

    let offset: usize = match input.get("offset") {
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "Parameter 'offset' must be a non-negative integer".to_string())
            .and_then(|n| {
                usize::try_from(n)
                    .map_err(|_| "Parameter 'offset' is too large for this platform".to_string())
            })?,
        None => 0,
    };

    let length: usize = match input.get("length") {
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| "Parameter 'length' must be a non-negative integer".to_string())
                .and_then(|n| {
                    usize::try_from(n).map_err(|_| {
                        "Parameter 'length' is too large for this platform".to_string()
                    })
                })?;
            if n == 0 {
                return Err("Parameter 'length' must be greater than 0".to_string());
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
    .map_err(|e| format!("read_artifact task panicked: {e}"))??;

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
