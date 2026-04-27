//! Audit framework for `librefang doctor`.
//!
//! `cmd_doctor` in `main.rs` is a long, hand-rolled chain of inline checks.
//! Adding a new check there means appending another 30-line `if librefang_dir.exists() ...`
//! branch — the existing checks are not addressable individually, can't be
//! tested in isolation, and can't be enumerated.
//!
//! This module introduces a small trait-based registry so each new check is
//! its own struct that anyone can grep for. It currently runs *alongside*
//! the legacy inline checks in `cmd_doctor` rather than replacing them, to
//! keep the change minimal and reviewable. Migration of the legacy checks
//! can happen incrementally in follow-up PRs.
//!
//! ## Adding a new check
//!
//! 1. Add a unit struct implementing [`AuditCheck`] below.
//! 2. Add it to [`registered_checks`].
//!
//! That's it. The check shows up the next time `librefang doctor` runs.
//! Each check should be a leaf operation that doesn't bleed into others —
//! tests for one check shouldn't have to set up state for another.

use base64::Engine;
use std::path::PathBuf;

/// Severity of a single audit finding.
///
/// `Pass` reports the green case (showing it built confidence in noisy
/// infra setups), `Info` is informational (no problem, no action), `Warn`
/// surfaces a fixable misconfiguration, `Error` blocks correct operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Pass,
    Info,
    Warn,
    Error,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Pass => "pass",
            Severity::Info => "info",
            Severity::Warn => "warn",
            Severity::Error => "error",
        }
    }
}

/// Outcome of a single audit check.
#[derive(Debug, Clone)]
pub struct AuditResult {
    /// Stable machine-readable identifier (use snake_case; goes into JSON).
    pub name: &'static str,
    pub severity: Severity,
    /// Human-readable one-line summary.
    pub summary: String,
    /// Optional remediation hint shown to the user when severity is Warn/Error.
    pub hint: Option<String>,
}

impl AuditResult {
    fn pass(name: &'static str, summary: impl Into<String>) -> Self {
        Self {
            name,
            severity: Severity::Pass,
            summary: summary.into(),
            hint: None,
        }
    }

    fn info(name: &'static str, summary: impl Into<String>) -> Self {
        Self {
            name,
            severity: Severity::Info,
            summary: summary.into(),
            hint: None,
        }
    }

    fn warn(name: &'static str, summary: impl Into<String>, hint: Option<String>) -> Self {
        Self {
            name,
            severity: Severity::Warn,
            summary: summary.into(),
            hint,
        }
    }

    fn error(name: &'static str, summary: impl Into<String>, hint: Option<String>) -> Self {
        Self {
            name,
            severity: Severity::Error,
            summary: summary.into(),
            hint,
        }
    }
}

/// State a check may consult — paths derived once by the caller so each
/// check doesn't redo the same lookup. Add fields here as new checks need
/// them; keep it cheap to construct.
pub struct AuditContext {
    /// `~/.librefang/` (or `$LIBREFANG_HOME`).
    pub librefang_home: PathBuf,
}

pub trait AuditCheck {
    fn run(&self, ctx: &AuditContext) -> AuditResult;
}

/// All currently registered checks. The order here is the order shown to
/// the user — group related checks together.
pub fn registered_checks() -> Vec<Box<dyn AuditCheck>> {
    vec![
        Box::new(VaultKeyCheck),
        Box::new(ApiListenAddrCheck),
        Box::new(ConfigTomlSchemaCheck),
    ]
}

pub fn run_all(ctx: &AuditContext) -> Vec<AuditResult> {
    registered_checks()
        .into_iter()
        .map(|c| c.run(ctx))
        .collect()
}

// ---------------------------------------------------------------------------
// VaultKeyCheck — LIBREFANG_VAULT_KEY must base64-decode to exactly 32 bytes.
//
// CLAUDE.md "Common Gotchas" calls this out specifically:
//
// > LIBREFANG_VAULT_KEY env var must base64-decode to exactly 32 bytes
// > (use `openssl rand -base64 32` which gives 44 chars). 32 ASCII chars ≠
// > 32 bytes.
//
// People keep tripping on this because the env var "looks 32 chars long"
// to the eye.
// ---------------------------------------------------------------------------

pub struct VaultKeyCheck;

impl AuditCheck for VaultKeyCheck {
    fn run(&self, _ctx: &AuditContext) -> AuditResult {
        const NAME: &str = "vault_key_length";
        let raw = match std::env::var("LIBREFANG_VAULT_KEY") {
            Ok(v) => v,
            Err(_) => {
                return AuditResult::info(
                    NAME,
                    "LIBREFANG_VAULT_KEY not set — vault encryption disabled.",
                );
            }
        };
        // Match production: `decode_master_key` in librefang-extensions/src/vault.rs
        // does NOT trim — so neither do we. A trailing newline that production
        // would reject must also fail here, otherwise this check is a false
        // negative (says OK while real vault unlock errors out).
        match base64::engine::general_purpose::STANDARD.decode(raw.as_bytes()) {
            Err(e) => AuditResult::error(
                NAME,
                format!("LIBREFANG_VAULT_KEY is not valid base64: {e}"),
                Some("Generate one with: openssl rand -base64 32".into()),
            ),
            Ok(bytes) if bytes.len() != 32 => AuditResult::error(
                NAME,
                format!(
                    "LIBREFANG_VAULT_KEY decodes to {} bytes; must be exactly 32. \
                     Note that 32 ASCII characters is NOT 32 bytes after base64 decode.",
                    bytes.len()
                ),
                Some(
                    "Generate a fresh 32-byte key: openssl rand -base64 32 (44-char output)".into(),
                ),
            ),
            Ok(_) => AuditResult::pass(NAME, "LIBREFANG_VAULT_KEY decodes to 32 bytes."),
        }
    }
}

// ---------------------------------------------------------------------------
// ApiListenAddrCheck — config.api_listen must parse as SocketAddr; warn on
// privileged ports (<1024) since the daemon won't be able to bind without
// root.
// ---------------------------------------------------------------------------

pub struct ApiListenAddrCheck;

impl AuditCheck for ApiListenAddrCheck {
    fn run(&self, ctx: &AuditContext) -> AuditResult {
        const NAME: &str = "api_listen_addr";
        let config_path = ctx.librefang_home.join("config.toml");
        let raw = match std::fs::read_to_string(&config_path) {
            Ok(s) => s,
            Err(_) => {
                return AuditResult::info(
                    NAME,
                    "config.toml not found — skipping api_listen check.",
                );
            }
        };
        // Accept any TOML and just look at api_listen if present. Don't
        // hard-depend on the full KernelConfig schema here; this check is
        // meant to be cheap and forward-compatible with future fields.
        let value: toml::Value = match toml::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                return AuditResult::error(
                    NAME,
                    format!("config.toml is not valid TOML: {e}"),
                    Some(
                        "Edit ~/.librefang/config.toml or run `librefang doctor --repair`.".into(),
                    ),
                );
            }
        };
        let listen = match value.get("api_listen").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return AuditResult::info(
                    NAME,
                    "api_listen not set in config — kernel will use the default.",
                );
            }
        };
        match listen.parse::<std::net::SocketAddr>() {
            Err(e) => AuditResult::error(
                NAME,
                format!("api_listen `{listen}` is not a valid socket address: {e}"),
                Some(
                    "Use `host:port` form, e.g. `127.0.0.1:4545` or `[::1]:4545`."
                        .into(),
                ),
            ),
            Ok(addr) if addr.port() == 0 => AuditResult::warn(
                NAME,
                format!(
                    "api_listen `{addr}` uses port 0 (OS-assigned ephemeral); clients can't \
                     discover the daemon URL after bind."
                ),
                Some(
                    "Pick an explicit port (default 4545), e.g. `127.0.0.1:4545`."
                        .into(),
                ),
            ),
            Ok(addr) if addr.port() < 1024 => AuditResult::warn(
                NAME,
                format!(
                    "api_listen port {} is privileged (<1024); daemon will fail to bind without root.",
                    addr.port()
                ),
                Some(
                    "Use a port >= 1024 (default 4545) unless you intentionally need root."
                        .into(),
                ),
            ),
            Ok(addr) => AuditResult::pass(NAME, format!("api_listen `{addr}` parses cleanly.")),
        }
    }
}

// ---------------------------------------------------------------------------
// ConfigTomlSchemaCheck — config.toml exists and parses as TOML. Distinct
// from the legacy syntax check in `cmd_doctor` only in that it lives in the
// framework so future schema-level checks can land here without growing
// the inline doctor function further.
// ---------------------------------------------------------------------------

pub struct ConfigTomlSchemaCheck;

impl AuditCheck for ConfigTomlSchemaCheck {
    fn run(&self, ctx: &AuditContext) -> AuditResult {
        const NAME: &str = "config_toml_schema";
        let path = ctx.librefang_home.join("config.toml");
        if !path.exists() {
            return AuditResult::warn(
                NAME,
                format!("{} does not exist.", path.display()),
                Some("Run `librefang init` to create a default config.".into()),
            );
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                return AuditResult::error(
                    NAME,
                    format!("Failed to read {}: {e}", path.display()),
                    None,
                );
            }
        };
        match toml::from_str::<toml::Value>(&raw) {
            Ok(_) => AuditResult::pass(NAME, format!("{} parses as TOML.", path.display())),
            Err(e) => AuditResult::error(
                NAME,
                format!("{} has TOML syntax errors: {e}", path.display()),
                Some(format!("Edit {} or restore from a backup.", path.display())),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_home(home: PathBuf) -> AuditContext {
        AuditContext {
            librefang_home: home,
        }
    }

    fn tmp_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    // ── VaultKeyCheck ──────────────────────────────────────────────────────

    /// Process-wide lock for tests that mutate `LIBREFANG_VAULT_KEY`. `cargo
    /// test` runs tests in parallel by default, and env-var mutation is
    /// process-global, so without serialization these races clobber each
    /// other (and `run_all_returns_one_result_per_check`, which also reads
    /// the env var). No external dep needed — std `Mutex` is enough.
    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// Run a closure with `LIBREFANG_VAULT_KEY` temporarily set to `value`.
    /// Holds [`env_lock`] for the entire body so concurrent vault-key tests
    /// (and any other env-var test in this binary) don't race. The original
    /// value is restored before the lock is released.
    fn with_vault_key<F: FnOnce() -> AuditResult>(value: Option<&str>, f: F) -> AuditResult {
        // poison is fine — a panicking sibling test shouldn't make the rest
        // hang or incorrectly skip.
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("LIBREFANG_VAULT_KEY").ok();
        match value {
            Some(v) => std::env::set_var("LIBREFANG_VAULT_KEY", v),
            None => std::env::remove_var("LIBREFANG_VAULT_KEY"),
        }
        let result = f();
        match prev {
            Some(p) => std::env::set_var("LIBREFANG_VAULT_KEY", p),
            None => std::env::remove_var("LIBREFANG_VAULT_KEY"),
        }
        result
    }

    #[test]
    fn vault_key_unset_is_info() {
        let tmp = tmp_home();
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = with_vault_key(None, || VaultKeyCheck.run(&ctx));
        assert_eq!(r.severity, Severity::Info);
    }

    #[test]
    fn vault_key_invalid_base64_is_error() {
        let tmp = tmp_home();
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = with_vault_key(Some("!!!not-base64!!!"), || VaultKeyCheck.run(&ctx));
        assert_eq!(r.severity, Severity::Error);
        assert!(r.summary.contains("not valid base64"));
    }

    #[test]
    fn vault_key_wrong_length_is_error() {
        let tmp = tmp_home();
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        // 32 ASCII chars → base64 → 24 bytes (the classic gotcha).
        let r = with_vault_key(Some("MDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAw"), || {
            VaultKeyCheck.run(&ctx)
        });
        assert_eq!(r.severity, Severity::Error);
        assert!(r.summary.contains("must be exactly 32"));
    }

    #[test]
    fn vault_key_correct_length_is_pass() {
        let tmp = tmp_home();
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        // Real 32-byte key, base64 → 44 chars.
        let real_32_byte_key_b64 = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
        let r = with_vault_key(Some(real_32_byte_key_b64), || VaultKeyCheck.run(&ctx));
        assert_eq!(r.severity, Severity::Pass);
    }

    // ── ApiListenAddrCheck ────────────────────────────────────────────────

    fn write_config(home: &std::path::Path, body: &str) {
        std::fs::write(home.join("config.toml"), body).expect("write config");
    }

    #[test]
    fn api_listen_missing_config_is_info() {
        let tmp = tmp_home();
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ApiListenAddrCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Info);
    }

    #[test]
    fn api_listen_invalid_addr_is_error() {
        let tmp = tmp_home();
        write_config(tmp.path(), "api_listen = \"not-an-addr\"\n");
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ApiListenAddrCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Error);
    }

    #[test]
    fn api_listen_privileged_port_is_warn() {
        let tmp = tmp_home();
        write_config(tmp.path(), "api_listen = \"127.0.0.1:80\"\n");
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ApiListenAddrCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Warn);
    }

    #[test]
    fn api_listen_port_zero_is_warn() {
        // Port 0 = OS-assigned ephemeral. Daemon binds, but the chosen port
        // is unknowable to clients — practically broken for a service users
        // are supposed to connect to. Must NOT silently pass.
        let tmp = tmp_home();
        write_config(tmp.path(), "api_listen = \"127.0.0.1:0\"\n");
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ApiListenAddrCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Warn);
    }

    #[test]
    fn api_listen_normal_port_is_pass() {
        let tmp = tmp_home();
        write_config(tmp.path(), "api_listen = \"127.0.0.1:4545\"\n");
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ApiListenAddrCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Pass);
    }

    // ── ConfigTomlSchemaCheck ─────────────────────────────────────────────

    #[test]
    fn config_missing_is_warn() {
        let tmp = tmp_home();
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ConfigTomlSchemaCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Warn);
    }

    #[test]
    fn config_malformed_is_error() {
        let tmp = tmp_home();
        write_config(tmp.path(), "this is = not [valid toml");
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ConfigTomlSchemaCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Error);
    }

    #[test]
    fn config_valid_is_pass() {
        let tmp = tmp_home();
        write_config(tmp.path(), "api_listen = \"127.0.0.1:4545\"\n");
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let r = ConfigTomlSchemaCheck.run(&ctx);
        assert_eq!(r.severity, Severity::Pass);
    }

    // ── Registry sanity ──────────────────────────────────────────────────

    #[test]
    fn registered_checks_is_non_empty() {
        assert!(!registered_checks().is_empty());
    }

    #[test]
    fn run_all_returns_one_result_per_check() {
        // `run_all` invokes `VaultKeyCheck`, which reads `LIBREFANG_VAULT_KEY`.
        // Hold `env_lock` so this can't race with `with_vault_key` callers
        // mid-flight — otherwise the result count is fine, but the
        // observed env state is non-deterministic for any future asserts here.
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let tmp = tmp_home();
        let ctx = ctx_with_home(tmp.path().to_path_buf());
        let results = run_all(&ctx);
        assert_eq!(results.len(), registered_checks().len());
    }
}
