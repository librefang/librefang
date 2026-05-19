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
        return Err(format!(
            "describe exited {}: {}",
            out.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }
    let stdout = String::from_utf8_lossy(&out.stdout);
    serde_json::from_str::<SidecarSchema>(stdout.trim())
        .map_err(|e| format!("invalid describe JSON: {e}; raw stdout: {stdout}"))
}
