//! EveryAPI's local credential-process integration.
//!
//! EveryAPI remains the authority for relay-key selection, OAuth refresh, gateway-region resolution, and rejected-key invalidation.
//! LibreFang only consumes the versioned machine response and never persists the secret.

use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PROTOCOL_VERSION: u64 = 1;
// EveryAPI's credential refresh HTTP client has a 30-second deadline.
// Leave enough time for the CLI to persist a rotated refresh token before terminating it; killing at a shorter deadline can strand OAuth credentials.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(45);
const MAX_COMMAND_OUTPUT_BYTES: u64 = 64 * 1024;
const GLOBAL_API_BASE: &str = "https://api.everyapi.ai";
const CHINA_API_BASE: &str = "https://api-cn.everyapi.ai";

/// A live bearer credential resolved by the EveryAPI CLI.
#[derive(Clone)]
pub struct EveryApiCredential {
    pub base_url: String,
    pub api_key: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for EveryApiCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EveryApiCredential")
            .field("base_url", &self.base_url)
            .field("api_key", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialError {
    #[error("EveryAPI CLI is not installed and no compatible cached credential exists")]
    NotInstalled,
    #[error("EveryAPI is not logged in")]
    NotLoggedIn,
    #[error("EveryAPI account has no enabled relay key")]
    NoRelayKey,
    #[error("EveryAPI credentials are invalid: {0}")]
    InvalidCredentials(String),
    #[error("unsupported EveryAPI credential protocol version {0}")]
    UnsupportedVersion(u64),
    #[error("invalid EveryAPI credential response: {0}")]
    InvalidOutput(String),
    #[error("EveryAPI credential command timed out")]
    Timeout,
    #[error("EveryAPI credential command is unavailable")]
    Unavailable,
    #[error("EveryAPI executable was not found")]
    ExecutableNotFound,
}

#[derive(Clone)]
struct ProcessResult {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl std::fmt::Debug for ProcessResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessResult")
            .field("success", &self.success)
            .field("stdout_len", &self.stdout.len())
            .field("stderr_len", &self.stderr.len())
            .finish()
    }
}

trait CredentialCommand: Send + Sync {
    fn run(
        &self,
        executable: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<ProcessResult, CredentialError>;
}

struct SystemCredentialCommand;

impl CredentialCommand for SystemCredentialCommand {
    fn run(
        &self,
        executable: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<ProcessResult, CredentialError> {
        let mut child = Command::new(executable)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    CredentialError::ExecutableNotFound
                } else {
                    CredentialError::Unavailable
                }
            })?;

        let stdout = child.stdout.take().ok_or(CredentialError::Unavailable)?;
        let stderr = child.stderr.take().ok_or(CredentialError::Unavailable)?;
        let stdout_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout
                .take(MAX_COMMAND_OUTPUT_BYTES)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });
        let stderr_reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr
                .take(MAX_COMMAND_OUTPUT_BYTES)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        });

        let deadline = Instant::now() + timeout;
        let success = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    // Do not block the timeout path on pipe-reader joins.
                    // A misbehaving executable may have spawned descendants that inherited stdout/stderr; those descendants can keep the pipe open even after the direct child is dead.
                    // Dropping the JoinHandles detaches the readers, which exit when the inherited descriptors finally close.
                    drop(stdout_reader);
                    drop(stderr_reader);
                    return Err(CredentialError::Timeout);
                }
                Err(_) => return Err(CredentialError::Unavailable),
            }
        };

        let stdout = stdout_reader
            .join()
            .map_err(|_| CredentialError::Unavailable)?
            .map_err(|_| CredentialError::Unavailable)?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| CredentialError::Unavailable)?
            .map_err(|_| CredentialError::Unavailable)?;
        Ok(ProcessResult {
            success,
            stdout,
            stderr,
        })
    }
}

#[derive(Deserialize)]
struct CredentialProcessOutput {
    version: u64,
    base_url: String,
    api_key: String,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    models: Vec<ModelCatalogProcessEntry>,
}

#[derive(Deserialize)]
struct ModelCatalogProcessEntry {
    id: String,
    #[serde(default)]
    supported_endpoint_types: Vec<String>,
}

fn parse_credential_output(bytes: &[u8]) -> Result<EveryApiCredential, CredentialError> {
    let output: CredentialProcessOutput = serde_json::from_slice(bytes)
        .map_err(|error| CredentialError::InvalidOutput(error.to_string()))?;
    if output.version != PROTOCOL_VERSION {
        return Err(CredentialError::UnsupportedVersion(output.version));
    }
    let base_url = output.base_url.trim().trim_end_matches('/').to_string();
    if base_url.is_empty() || output.api_key.trim().is_empty() {
        return Err(CredentialError::InvalidOutput(
            "base_url and api_key must be non-empty".to_string(),
        ));
    }
    let expires_at = output
        .expires_at
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|value| value.with_timezone(&Utc))
                .map_err(|error| CredentialError::InvalidOutput(error.to_string()))
        })
        .transpose()?;
    Ok(EveryApiCredential {
        base_url,
        api_key: output.api_key,
        expires_at,
    })
}

/// Resolve the current default EveryAPI relay credential.
///
/// `invalidate` is used only after an authenticated request returned 401.
/// It asks EveryAPI to discard an ordinary cached relay key before resolving again.
/// The caller remains responsible for bounding its retry count.
pub fn resolve(invalidate: bool) -> Result<EveryApiCredential, CredentialError> {
    let config_dir = everyapi_config_dir().ok_or(CredentialError::NotInstalled)?;
    resolve_with(
        &SystemCredentialCommand,
        invalidate,
        &candidate_executables(),
        &config_dir,
    )
}

/// Return the current relay-key-scoped chat model IDs from the EveryAPI CLI.
///
/// This is used only while auto-selecting an initial model.
/// Ordinary requests do not spawn this extra command.
pub fn resolve_available_models() -> Result<Vec<String>, CredentialError> {
    resolve_available_models_with(&SystemCredentialCommand, &candidate_executables())
}

fn resolve_available_models_with(
    runner: &dyn CredentialCommand,
    candidates: &[PathBuf],
) -> Result<Vec<String>, CredentialError> {
    for executable in candidates {
        let result = match runner.run(
            executable,
            &["auth", "credential", "--format=json", "--include-models"],
            COMMAND_TIMEOUT,
        ) {
            Ok(result) => result,
            Err(CredentialError::ExecutableNotFound) => continue,
            Err(error) => return Err(error),
        };
        if !result.success {
            return Err(CredentialError::Unavailable);
        }
        let output: CredentialProcessOutput = serde_json::from_slice(&result.stdout)
            .map_err(|error| CredentialError::InvalidOutput(error.to_string()))?;
        if output.version != PROTOCOL_VERSION {
            return Err(CredentialError::UnsupportedVersion(output.version));
        }
        return Ok(output
            .models
            .into_iter()
            .filter(|model| {
                model
                    .supported_endpoint_types
                    .iter()
                    .any(|endpoint| endpoint.eq_ignore_ascii_case("openai"))
            })
            .map(|model| model.id.trim().to_string())
            .filter(|id| !id.is_empty())
            .collect());
    }
    Err(CredentialError::NotInstalled)
}

fn resolve_with(
    runner: &dyn CredentialCommand,
    invalidate: bool,
    candidates: &[PathBuf],
    config_dir: &Path,
) -> Result<EveryApiCredential, CredentialError> {
    for executable in candidates {
        let mut args = vec!["auth", "credential", "--format=json"];
        if invalidate {
            args.push("--invalidate");
        }
        let result = match runner.run(executable, &args, COMMAND_TIMEOUT) {
            Ok(result) => result,
            Err(CredentialError::ExecutableNotFound) => continue,
            Err(error) => return Err(error),
        };
        if result.success {
            return parse_credential_output(&result.stdout);
        }
        let stderr = String::from_utf8_lossy(&result.stderr);
        if stderr.contains("EVERYAPI_CREDENTIAL_ERROR:not_logged_in") {
            return Err(CredentialError::NotLoggedIn);
        }
        if stderr.contains("EVERYAPI_CREDENTIAL_ERROR:no_relay_key") {
            return Err(CredentialError::NoRelayKey);
        }
        if stderr.contains("EVERYAPI_CREDENTIAL_ERROR:invalid_credentials") {
            return Err(CredentialError::InvalidCredentials(
                "the EveryAPI CLI rejected its credential file".to_string(),
            ));
        }
        let old_cli = stderr.contains("unknown") && stderr.contains("credential");
        if old_cli && !invalidate {
            return resolve_legacy_cache(config_dir);
        }
        return Err(CredentialError::Unavailable);
    }

    if !invalidate {
        return resolve_legacy_cache(config_dir).map_err(|error| {
            if matches!(error, CredentialError::NotLoggedIn) {
                CredentialError::NotInstalled
            } else {
                error
            }
        });
    }
    Err(CredentialError::NotInstalled)
}

fn candidate_executables() -> Vec<PathBuf> {
    if let Ok(path) = std::env::var("EVERYAPI_CLI_PATH") {
        if !path.trim().is_empty() {
            return vec![PathBuf::from(path)];
        }
    }
    let mut candidates = vec![PathBuf::from(if cfg!(windows) {
        "everyapi.exe"
    } else {
        "everyapi"
    })];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local").join("bin").join(if cfg!(windows) {
            "everyapi.exe"
        } else {
            "everyapi"
        }));
    }
    if !cfg!(windows) {
        candidates.push(PathBuf::from("/usr/local/bin/everyapi"));
        candidates.push(PathBuf::from("/opt/homebrew/bin/everyapi"));
    }
    candidates
}

fn everyapi_config_dir() -> Option<PathBuf> {
    match std::env::var("XDG_CONFIG_HOME") {
        Ok(value) if !value.trim().is_empty() => Some(PathBuf::from(value).join("everyapi")),
        _ => dirs::home_dir().map(|home| home.join(".config").join("everyapi")),
    }
}

#[derive(Deserialize)]
struct LegacyCredentials {
    api_base: String,
    relay_key: String,
    #[serde(default)]
    relay_key_expires_at: i64,
}

#[derive(Default, Deserialize)]
struct LegacySettings {
    #[serde(default)]
    gateway_region: String,
}

fn resolve_legacy_cache(config_dir: &Path) -> Result<EveryApiCredential, CredentialError> {
    let bytes = std::fs::read(config_dir.join("credentials.json")).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            CredentialError::NotLoggedIn
        } else {
            CredentialError::InvalidCredentials(error.to_string())
        }
    })?;
    let credentials: LegacyCredentials = serde_json::from_slice(&bytes)
        .map_err(|error| CredentialError::InvalidCredentials(error.to_string()))?;
    if credentials.relay_key.trim().is_empty() {
        return Err(CredentialError::NoRelayKey);
    }
    let expires_at = (credentials.relay_key_expires_at > 0)
        .then(|| DateTime::from_timestamp(credentials.relay_key_expires_at, 0))
        .flatten();
    if expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(CredentialError::Unavailable);
    }

    let login_base = credentials.api_base.trim().trim_end_matches('/');
    let settings = std::fs::read(config_dir.join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<LegacySettings>(&bytes).ok())
        .unwrap_or_default();
    let official =
        login_base.is_empty() || login_base == GLOBAL_API_BASE || login_base == CHINA_API_BASE;
    let origin = if !official {
        login_base.to_string()
    } else if login_base == CHINA_API_BASE
        || matches!(
            settings.gateway_region.trim().to_ascii_lowercase().as_str(),
            "cn" | "china"
        )
    {
        CHINA_API_BASE.to_string()
    } else {
        GLOBAL_API_BASE.to_string()
    };
    Ok(EveryApiCredential {
        base_url: format!("{origin}/v1"),
        api_key: credentials.relay_key,
        expires_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    struct FakeRunner {
        result: ProcessResult,
        seen_args: std::sync::Mutex<Vec<String>>,
    }

    impl CredentialCommand for FakeRunner {
        fn run(
            &self,
            _executable: &Path,
            args: &[&str],
            _timeout: Duration,
        ) -> Result<ProcessResult, CredentialError> {
            *self.seen_args.lock().unwrap() = args.iter().map(|s| (*s).to_string()).collect();
            Ok(self.result.clone())
        }
    }

    fn success(stdout: &str) -> ProcessResult {
        ProcessResult {
            success: true,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn parses_version_one_credential() {
        let credential = parse_credential_output(
            br#"{"version":1,"base_url":"https://api.everyapi.ai/v1","api_key":"secret","expires_at":"2026-07-30T12:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(credential.base_url, "https://api.everyapi.ai/v1");
        assert_eq!(credential.api_key, "secret");
        assert!(credential.expires_at.is_some());
    }

    #[test]
    fn rejects_unknown_version_and_empty_secret() {
        let unknown = parse_credential_output(
            br#"{"version":2,"base_url":"https://api.everyapi.ai/v1","api_key":"secret"}"#,
        )
        .unwrap_err();
        assert!(matches!(unknown, CredentialError::UnsupportedVersion(2)));

        let empty = parse_credential_output(
            br#"{"version":1,"base_url":"https://api.everyapi.ai/v1","api_key":" "}"#,
        )
        .unwrap_err();
        assert!(matches!(empty, CredentialError::InvalidOutput(_)));
    }

    #[test]
    fn invalidating_resolution_passes_the_machine_flag() {
        let runner = FakeRunner {
            result: success(
                r#"{"version":1,"base_url":"https://api.everyapi.ai/v1","api_key":"fresh"}"#,
            ),
            seen_args: std::sync::Mutex::new(Vec::new()),
        };
        let credential = resolve_with(
            &runner,
            true,
            &[PathBuf::from("/opt/everyapi")],
            Path::new("/unused"),
        )
        .unwrap();
        assert_eq!(credential.api_key, "fresh");
        assert_eq!(
            *runner.seen_args.lock().unwrap(),
            ["auth", "credential", "--format=json", "--invalidate"]
        );
    }

    #[test]
    fn live_model_resolution_uses_the_versioned_cli_catalog() {
        let runner = FakeRunner {
            result: success(
                r#"{"version":1,"base_url":"https://api.everyapi.ai/v1","api_key":"secret","models":[{"id":"image-only","supported_endpoint_types":["image-generation"]},{"id":"claude-sonnet-5","supported_endpoint_types":["openai","anthropic"]}]}"#,
            ),
            seen_args: std::sync::Mutex::new(Vec::new()),
        };

        let models =
            resolve_available_models_with(&runner, &[PathBuf::from("/opt/everyapi")]).unwrap();

        assert_eq!(models, vec!["claude-sonnet-5"]);
        assert_eq!(
            *runner.seen_args.lock().unwrap(),
            ["auth", "credential", "--format=json", "--include-models"]
        );
    }

    #[test]
    fn old_cli_falls_back_to_region_aware_cached_credentials() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("credentials.json"),
            r#"{"api_base":"https://api.everyapi.ai","relay_key":"cached"}"#,
        )
        .unwrap();
        fs::write(
            root.path().join("settings.json"),
            r#"{"gateway_region":"cn"}"#,
        )
        .unwrap();
        let runner = FakeRunner {
            result: ProcessResult {
                success: false,
                stdout: Vec::new(),
                stderr: b"Error: unknown auth subcommand credential".to_vec(),
            },
            seen_args: std::sync::Mutex::new(Vec::new()),
        };

        let credential =
            resolve_with(&runner, false, &[PathBuf::from("everyapi")], root.path()).unwrap();
        assert_eq!(credential.base_url, "https://api-cn.everyapi.ai/v1");
        assert_eq!(credential.api_key, "cached");
    }

    #[test]
    fn machine_not_logged_in_does_not_reuse_a_stale_file() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("credentials.json"),
            r#"{"api_base":"https://api.everyapi.ai","relay_key":"stale"}"#,
        )
        .unwrap();
        let runner = FakeRunner {
            result: ProcessResult {
                success: false,
                stdout: Vec::new(),
                stderr: b"EVERYAPI_CREDENTIAL_ERROR:not_logged_in".to_vec(),
            },
            seen_args: std::sync::Mutex::new(Vec::new()),
        };

        let error =
            resolve_with(&runner, false, &[PathBuf::from("everyapi")], root.path()).unwrap_err();
        assert!(matches!(error, CredentialError::NotLoggedIn));
    }

    #[cfg(unix)]
    #[test]
    fn process_runner_enforces_the_timeout() {
        let started = std::time::Instant::now();
        let error = SystemCredentialCommand
            .run(
                Path::new("/bin/sh"),
                &["-c", "sleep 2"],
                Duration::from_millis(50),
            )
            .unwrap_err();
        assert!(matches!(error, CredentialError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
