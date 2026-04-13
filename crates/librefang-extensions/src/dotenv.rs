//! Shared `.env` file loader for LibreFang.
//!
//! Loads `~/.librefang/.env`, `~/.librefang/secrets.env`, and the credential
//! vault into the process environment. Used by the CLI, desktop app, and kernel
//! so that every entry point loads secrets the same way.
//!
//! **Priority order** (highest first):
//! 1. System environment variables (already present in `std::env`)
//! 2. Credential vault (`vault.enc`)
//! 3. `.env` file
//! 4. `secrets.env` file
//!
//! Existing environment variables are **never** overridden.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Once;

/// Gate for [`load_dotenv`] so repeated calls are cheap no-ops.
static DOTENV_LOADED: Once = Once::new();

/// Get the LibreFang home directory, respecting `LIBREFANG_HOME` env var.
fn librefang_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|h| h.join(".librefang"))
}

fn env_file_path() -> Option<PathBuf> {
    librefang_home().map(|h| h.join(".env"))
}

fn secrets_file_path() -> Option<PathBuf> {
    librefang_home().map(|h| h.join("secrets.env"))
}

/// Load vault + `.env` + `secrets.env` into `std::env`. Call from the
/// synchronous `main()` of a binary *before* spawning any tokio runtime —
/// `std::env::set_var` is UB in Rust 1.80+ once other threads exist.
pub fn load_dotenv() {
    DOTENV_LOADED.call_once(|| {
        load_vault();
        if let Some(p) = env_file_path() {
            load_env_file(&p);
        }
        if let Some(p) = secrets_file_path() {
            load_env_file(&p);
        }
    });
}

/// Try to unlock the credential vault and inject secrets into process env.
///
/// Vault secrets have higher priority than `.env` but lower than system env vars.
/// Silently does nothing if vault is not initialized or cannot be unlocked.
fn load_vault() {
    let vault_path = match librefang_home() {
        Some(h) => h.join("vault.enc"),
        None => return,
    };

    if !vault_path.exists() {
        return;
    }

    let vault_display = vault_path.display().to_string();
    let mut vault = crate::vault::CredentialVault::new(vault_path);
    if let Err(e) = vault.unlock() {
        // `eprintln!` rather than `tracing::warn!`: load_dotenv runs from
        // sync main() before any tracing subscriber is installed.
        eprintln!(
            "librefang-dotenv: credential vault at {vault_display} could not be unlocked: {e}; \
             skipping vault-provided secrets"
        );
        return;
    }

    for key in vault.list_keys() {
        if std::env::var(key).is_err() {
            if let Some(val) = vault.get(key) {
                std::env::set_var(key, val.as_str());
            }
        }
    }
}

fn load_env_file(path: &Path) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if let Some((key, value)) = parse_env_line(trimmed) {
            if std::env::var(&key).is_err() {
                std::env::set_var(&key, &value);
            }
        }
    }
}

/// Parse a single `KEY=VALUE` line. Handles optional quotes.
fn parse_env_line(line: &str) -> Option<(String, String)> {
    let eq_pos = line.find('=')?;
    let key = line[..eq_pos].trim().to_string();
    let mut value = line[eq_pos + 1..].trim().to_string();

    if key.is_empty() {
        return None;
    }

    // Strip matching quotes
    if ((value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\'')))
        && value.len() >= 2
    {
        value = value[1..value.len() - 1].to_string();
    }

    Some((key, value))
}

/// Upsert a key in `$LIBREFANG_HOME/.env`.
///
/// Creates the file if missing. Sets 0600 permissions on Unix.
/// Also sets the key in the current process environment.
pub fn save_env_key(key: &str, value: &str) -> Result<(), String> {
    let path = env_file_path().ok_or("Could not determine LibreFang home directory")?;

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
    }

    let mut entries = read_env_file(&path);
    entries.insert(key.to_string(), value.to_string());
    write_env_file(&path, &entries)?;

    // Also set in current process
    std::env::set_var(key, value);

    Ok(())
}

/// Remove a key from `$LIBREFANG_HOME/.env`.
pub fn remove_env_key(key: &str) -> Result<(), String> {
    let path = env_file_path().ok_or("Could not determine LibreFang home directory")?;

    let mut entries = read_env_file(&path);
    entries.remove(key);
    write_env_file(&path, &entries)?;

    std::env::remove_var(key);

    Ok(())
}

/// List key names (without values) from `$LIBREFANG_HOME/.env`.
#[allow(dead_code)]
pub fn list_env_keys() -> Vec<String> {
    let path = match env_file_path() {
        Some(p) => p,
        None => return Vec::new(),
    };

    read_env_file(&path).into_keys().collect()
}

/// Check if the `.env` file exists.
#[allow(dead_code)]
pub fn env_file_exists() -> bool {
    env_file_path().map(|p| p.exists()).unwrap_or(false)
}

fn read_env_file(path: &Path) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();

    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return map,
    };

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = parse_env_line(trimmed) {
            map.insert(key, value);
        }
    }

    map
}

fn write_env_file(path: &Path, entries: &BTreeMap<String, String>) -> Result<(), String> {
    let mut content =
        String::from("# LibreFang environment — managed by `librefang config set-key`\n");
    content.push_str("# Do not edit while the daemon is running.\n\n");

    for (key, value) in entries {
        if value.contains(' ') || value.contains('#') || value.contains('"') {
            content.push_str(&format!("{key}=\"{}\"\n", value.replace('"', "\\\"")));
        } else {
            content.push_str(&format!("{key}={value}\n"));
        }
    }

    std::fs::write(path, &content).map_err(|e| format!("Failed to write .env file: {e}"))?;

    // Set 0600 permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_line_simple() {
        let (k, v) = parse_env_line("FOO=bar").unwrap();
        assert_eq!(k, "FOO");
        assert_eq!(v, "bar");
    }

    #[test]
    fn test_parse_env_line_quoted() {
        let (k, v) = parse_env_line("KEY=\"hello world\"").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "hello world");
    }

    #[test]
    fn test_parse_env_line_single_quoted() {
        let (k, v) = parse_env_line("KEY='value'").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "value");
    }

    #[test]
    fn test_parse_env_line_spaces() {
        let (k, v) = parse_env_line("  KEY  =  value  ").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "value");
    }

    #[test]
    fn test_parse_env_line_no_value() {
        let (k, v) = parse_env_line("KEY=").unwrap();
        assert_eq!(k, "KEY");
        assert_eq!(v, "");
    }

    #[test]
    fn test_parse_env_line_no_equals() {
        assert!(parse_env_line("NOEQUALS").is_none());
    }

    #[test]
    fn test_parse_env_line_empty_key() {
        assert!(parse_env_line("=value").is_none());
    }

    #[test]
    fn test_load_env_file_nonexistent() {
        // Should not panic
        load_env_file(&PathBuf::from("/nonexistent/.env"));
    }

    /// `load_env_file` must not override existing process env vars —
    /// this is the invariant that anchors the system > file priority.
    ///
    /// `#[serial]` because `cargo test` runs multi-threaded by default
    /// and `std::env::{set,remove}_var` is UB while other threads exist.
    #[test]
    #[serial_test::serial]
    fn test_load_env_file_does_not_override_existing_var() {
        let key = "LIBREFANG_DOTENV_TEST_PRIORITY_OVERRIDE";
        // SAFETY: #[serial] serialises every env-mutating test in this
        // binary, so no other thread reads or writes `key` here.
        unsafe { std::env::set_var(key, "system-value") };

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(".env");
        std::fs::write(&path, format!("{key}=file-value\n")).unwrap();

        load_env_file(&path);

        assert_eq!(std::env::var(key).unwrap(), "system-value");

        // SAFETY: same as above.
        unsafe { std::env::remove_var(key) };
    }
}
