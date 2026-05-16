//! Shared artifact-spill helpers used by `web_fetch` (primary + legacy)
//! and `web_search` to overflow oversize tool results into the artifact
//! store rather than burning context (#3347 5/N).

/// Resolve `[tool_results]` spill threshold + per-artifact cap from raw
/// `ToolExecContext` fields, falling back to compiled defaults when the
/// caller passed `0` (test call sites that don't populate the ctx).
pub(super) fn resolve_spill_config(
    spill_threshold_bytes: u64,
    max_artifact_bytes: u64,
) -> (u64, u64) {
    (
        if spill_threshold_bytes == 0 {
            16_384 // ToolResultsConfig::default().spill_threshold_bytes
        } else {
            spill_threshold_bytes
        },
        if max_artifact_bytes == 0 {
            crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES
        } else {
            max_artifact_bytes
        },
    )
}

/// Apply artifact spill to a tool-result string, returning a compact stub
/// when the body exceeds `threshold` and the spill write succeeds.  Falls
/// through to the original body when below the threshold or when the
/// write fails (e.g. per-artifact cap exceeded, disk full).
pub(super) fn spill_or_passthrough(
    tool_name: &str,
    body: String,
    threshold: u64,
    max_artifact: u64,
) -> String {
    let bytes = body.as_bytes();
    if let Some(stub) = crate::artifact_store::maybe_spill(
        tool_name,
        bytes,
        threshold,
        max_artifact,
        &crate::artifact_store::default_artifact_storage_dir(),
    ) {
        stub
    } else {
        body
    }
}
