//! Spawn a sidecar adapter with `--describe` and parse the JSON schema
//! it prints on stdout. Used at daemon boot to populate the Add-picker
//! form for each first-party SIDECAR_CATALOG entry.

use librefang_channels::embedded_sdk::pythonpath_with_embedded;
use librefang_channels::sidecar::{
    format_librefang_sdk_missing_hint, looks_like_librefang_sdk_missing,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
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
    // `--describe` runs the same interpreter, with the same PYTHONPATH resolution, as the eventual sidecar spawn, so the version reported here is the version that will actually serve traffic — precisely what #7140 had no way to find out short of reading logs on the box.
    // `None` for an adapter whose SDK predates the field, and for the compile-time fallback schema used when `--describe` fails outright.
    /// The `librefang-sdk` version the adapter reported on `--describe`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_version: Option<String>,
}

#[derive(Debug, Error)]
pub enum DescribeSidecarError {
    #[error("`{command} ...--describe` timed out after 5s")]
    Timeout { command: String },
    #[error("spawn failed: {0}")]
    Spawn(#[source] std::io::Error),
    #[error("{0}")]
    SdkMissing(String),
    #[error("describe exited {code}: {stderr}")]
    Exited { code: i32, stderr: String },
    #[error("describe stdout was not valid UTF-8: {0}")]
    InvalidUtf8(#[source] std::string::FromUtf8Error),
    #[error("invalid describe JSON: {source}; raw stdout: {stdout}")]
    InvalidJson {
        #[source]
        source: serde_json::Error,
        stdout: String,
    },
}

/// Preserve only non-secret process context needed to locate and run an
/// interpreter. Callers must still supply catalog-controlled commands and
/// arguments; this probe is not a general subprocess execution API.
fn set_describe_environment(cmd: &mut Command) {
    const ALLOWED: &[&str] = &[
        "PATH",
        "PYTHONPATH",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TMPDIR",
        "TEMP",
        "TMP",
        "SYSTEMROOT",
        "WINDIR",
        "PATHEXT",
    ];
    cmd.env_clear();
    for name in ALLOWED {
        if let Some(value) = std::env::var_os(name) {
            cmd.env(name, value);
        }
    }
}

/// Spawn `<command> <args> --describe`, parse stdout as JSON.
///
/// Timeout is 5s — describe should be sub-second; if it hangs (the
/// adapter's __init__ blocks on a network call before reading argv,
/// for example) we'd rather skip than block daemon boot.
/// `home_dir`: pass `KernelApi::home_dir()`, never a recomputed `LIBREFANG_HOME`.
pub async fn describe_sidecar(
    command: &str,
    args: &[String],
    home_dir: &Path,
) -> Result<SidecarSchema, DescribeSidecarError> {
    let mut full_args: Vec<String> = args.to_vec();
    full_args.push("--describe".into());

    // `kill_on_drop(true)`: when the 5s timeout fires, the future is
    // dropped and we want the spawned child reaped with it. Without
    // this flag a hanging adapter would leak after `--describe` returns
    // — the timeout returns to the caller but the child keeps running
    // until it crashes on its own.
    let mut cmd = Command::new(command);
    set_describe_environment(&mut cmd);
    cmd.args(&full_args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Inject the bundled SDK onto PYTHONPATH so --describe succeeds on python3-only hosts; no-op for non-Python commands or when a real install already wins.
    let existing_pythonpath = std::env::var("PYTHONPATH").ok();
    if let Some(composed) =
        pythonpath_with_embedded(command, home_dir, existing_pythonpath.as_deref())
    {
        cmd.env("PYTHONPATH", composed);
    }
    let fut = cmd.output();

    let out = tokio::time::timeout(Duration::from_secs(5), fut)
        .await
        .map_err(|_| DescribeSidecarError::Timeout {
            command: command.to_string(),
        })?
        .map_err(DescribeSidecarError::Spawn)?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(translate_describe_error(
            command,
            out.status.code().unwrap_or(-1),
            stderr.trim(),
        ));
    }
    let stdout = String::from_utf8(out.stdout).map_err(DescribeSidecarError::InvalidUtf8)?;
    serde_json::from_str::<SidecarSchema>(stdout.trim())
        .map_err(|source| DescribeSidecarError::InvalidJson { source, stdout })
}

/// Translate the cryptic Python-side failure mode that fires when
/// `librefang-sdk` is not importable from the interpreter the daemon
/// spawned (the `ModuleNotFoundError: No module named 'librefang'`
/// traceback at boot-time discovery time) into a one-line actionable
/// error that names the install command and warns about the "two
/// different python3 interpreters" footgun under mise / pyenv /
/// conda.
///
/// Detection + message template are shared with
/// `librefang_channels::sidecar` so the discovery-time hint here
/// stays byte-identical to the runtime-time hint emitted from the
/// sidecar supervisor's stderr loop. Edit
/// `librefang_channels::sidecar::format_librefang_sdk_missing_hint`
/// (and the `looks_like_librefang_sdk_missing` detector next to it)
/// to update both paths in lockstep.
fn translate_describe_error(command: &str, code: i32, stderr: &str) -> DescribeSidecarError {
    if looks_like_librefang_sdk_missing(stderr) {
        return DescribeSidecarError::SdkMissing(format_librefang_sdk_missing_hint(command));
    }
    DescribeSidecarError::Exited {
        code,
        stderr: stderr.to_string(),
    }
}
