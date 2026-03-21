//! Minimal `.env` file loader for `~/.librefang/.env`.
//!
//! Mirrors the CLI's dotenv module so the desktop app loads environment
//! variables (API keys, etc.) the same way the CLI does.

use std::path::PathBuf;

/// Get the LibreFang home directory, respecting LIBREFANG_HOME env var.
fn dotenv_librefang_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("LIBREFANG_HOME") {
        return Some(PathBuf::from(home));
    }
    dirs::home_dir().map(|h| h.join(".librefang"))
}

/// Return the path to `~/.librefang/.env`.
fn env_file_path() -> Option<PathBuf> {
    dotenv_librefang_home().map(|h| h.join(".env"))
}

/// Return the path to `~/.librefang/secrets.env`.
fn secrets_env_path() -> Option<PathBuf> {
    dotenv_librefang_home().map(|h| h.join("secrets.env"))
}

/// Load `~/.librefang/.env` and `~/.librefang/secrets.env` into `std::env`.
///
/// System env vars take priority — existing vars are NOT overridden.
/// `secrets.env` is loaded second so `.env` values take priority over secrets
/// (but both yield to system env vars).
/// Silently does nothing if the files don't exist.
pub fn load_dotenv() {
    // Vault takes highest priority (after system env vars).
    load_vault();
    load_env_file(env_file_path());
    // Also load secrets.env (written by dashboard "Set API Key" button)
    load_env_file(secrets_env_path());
}

/// Try to unlock the credential vault and inject secrets into process env.
///
/// Vault secrets have higher priority than `.env` but lower than system env vars.
/// Silently does nothing if vault is not initialized or cannot be unlocked.
fn load_vault() {
    let vault_path = match dotenv_librefang_home() {
        Some(h) => h.join("vault.enc"),
        None => return,
    };

    if !vault_path.exists() {
        return;
    }

    let mut vault = librefang_extensions::vault::CredentialVault::new(vault_path);
    if vault.unlock().is_err() {
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

fn load_env_file(path: Option<PathBuf>) {
    let path = match path {
        Some(p) => p,
        None => return,
    };

    let content = match std::fs::read_to_string(&path) {
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
