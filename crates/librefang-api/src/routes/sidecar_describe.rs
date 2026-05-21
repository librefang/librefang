//! Spawn a sidecar adapter with `--describe` and parse the JSON schema
//! it prints on stdout. Used at daemon boot to populate the Add-picker
//! form for each first-party SIDECAR_CATALOG entry.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SidecarSchemaField {
    pub key: String,
    pub label: String,
    #[serde(rename = "type")]
    pub field_type: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub placeholder: String,
    #[serde(default)]
    pub advanced: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct SidecarSchema {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub fields: Vec<SidecarSchemaField>,
}

/// Spawn `<command> <args> --describe`, parse stdout as JSON.
///
/// Timeout is 5s — describe should be sub-second; if it hangs (the
/// adapter's __init__ blocks on a network call before reading argv,
/// for example) we'd rather skip than block daemon boot.
pub async fn describe_sidecar(command: &str, args: &[String]) -> Result<SidecarSchema, String> {
    let mut full_args: Vec<String> = args.to_vec();
    full_args.push("--describe".into());

    // `kill_on_drop(true)`: when the 5s timeout fires, the future is
    // dropped and we want the spawned child reaped with it. Without
    // this flag a hanging adapter would leak after `--describe` returns
    // — the timeout returns to the caller but the child keeps running
    // until it crashes on its own.
    let fut = Command::new(command)
        .args(&full_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output();

    let out = tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .map_err(|_| format!("`{command} ...--describe` timed out after 5s"))?
        .map_err(|e| format!("spawn failed: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(translate_describe_error(
            command,
            out.status.code().unwrap_or(-1),
            stderr.trim(),
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<SidecarSchema>(stdout.trim())
        .map_err(|e| format!("invalid describe JSON: {e}; raw stdout: {stdout}"))
}

/// Translate the cryptic interpreter-level failure modes (specifically
/// the Python "module not found" traceback when `librefang-sdk` is
/// not installed in the daemon's Python interpreter) into a one-line
/// actionable error that names the install command and warns about the
/// "two different `python3` interpreters" footgun under
/// mise / pyenv / conda.
///
/// Falls through to the raw `describe exited N: <stderr>` shape on
/// every other failure mode so we don't silently mask other bugs.
fn translate_describe_error(command: &str, code: i32, stderr: &str) -> String {
    // Python's specific failure pattern when `librefang-sdk` isn't
    // installed in the interpreter the daemon picked. Match the
    // canonical phrase rather than substring-matching "librefang" —
    // a malformed adapter that just raises `ImportError: librefang
    // something else` shouldn't trip this hint.
    let is_sdk_missing =
        stderr.contains("ModuleNotFoundError") && stderr.contains("No module named 'librefang'");
    let is_spec_missing =
        stderr.contains("Error while finding module specification for 'librefang.sidecar");
    if is_sdk_missing || is_spec_missing {
        return format!(
            "librefang-sdk is not installed in the Python interpreter \
             resolved by `{command}`. Install with `pip install \
             librefang-sdk` (or `pip install -e sdk/python/` from a \
             source checkout). The daemon and your shell can resolve \
             different `python3` binaries under mise / pyenv / conda — \
             verify with `{command} -c 'import librefang.sidecar; \
             print(librefang.__file__)'`."
        );
    }
    format!("describe exited {code}: {stderr}")
}
