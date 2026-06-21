//! Shared artifact-spill helpers used by `web_fetch` (primary + legacy)
//! and `web_search` to overflow oversize tool results into the artifact
//! store rather than burning context (#3347 5/N).

/// Resolve `[tool_results]` spill threshold + per-artifact cap from raw
/// `ToolExecContext` fields, falling back to compiled defaults when the
/// caller passed `0` (test call sites that don't populate the ctx).
pub(crate) fn resolve_spill_config(
    spill_threshold_bytes: u64,
    max_artifact_bytes: u64,
) -> (u64, u64) {
    let threshold = if spill_threshold_bytes == 0 {
        librefang_types::config::ToolResultsConfig::default().spill_threshold_bytes
    } else {
        spill_threshold_bytes
    };
    let max = if max_artifact_bytes == 0 {
        crate::artifact_store::DEFAULT_MAX_ARTIFACT_BYTES
    } else {
        max_artifact_bytes
    };
    if threshold > max {
        (max, max)
    } else {
        (threshold, max)
    }
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
    if body.len() as u64 <= threshold {
        return body;
    }
    if let Some(stub) = crate::artifact_store::maybe_spill(
        tool_name,
        body.as_bytes(),
        threshold,
        max_artifact,
        &crate::artifact_store::default_artifact_storage_dir(),
    ) {
        stub
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_small_body_through_unchanged() {
        let body = "short shell output".to_string();
        let out = spill_or_passthrough("shell_exec", body.clone(), 1024, 1 << 20);
        assert_eq!(out, body);
    }

    #[test]
    fn spills_oversized_body_to_a_recoverable_artifact() {
        // Point the artifact store at an isolated temp home so the test does
        // not write to the real `~/.librefang`. nextest runs each test in its
        // own process, so this env mutation does not race other tests.
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("LIBREFANG_HOME", tmp.path());

        let body = "x".repeat(8192);
        let out = spill_or_passthrough("shell_exec", body, 1024, 1 << 20);

        std::env::remove_var("LIBREFANG_HOME");

        // The oversized stream is replaced by a compact stub — not returned
        // verbatim, and not the old lossy `[truncated, N total bytes]` form.
        assert!(
            out.len() < 8192,
            "oversized body must be replaced by a compact stub"
        );
        assert!(!out.contains("[truncated"), "must not lossily truncate");
        // The full bytes were written to the artifact store, so the agent can
        // page them back via read_artifact — the loss the issue (#6242) flags.
        let artifacts = tmp.path().join("data").join("artifacts");
        let wrote_artifact = std::fs::read_dir(&artifacts)
            .map(|d| {
                d.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "bin"))
            })
            .unwrap_or(false);
        assert!(
            wrote_artifact,
            "a recoverable artifact file must be written"
        );
    }
}
