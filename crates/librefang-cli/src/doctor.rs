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

use crate::i18n;
use base64::Engine;
use std::path::{Path, PathBuf};

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
        Box::new(EveryApiWiringCheck),
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
                return AuditResult::info(NAME, i18n::t("doctor-audit-vault-key-unset"));
            }
        };
        // Match production: `decode_master_key` in librefang-extensions/src/vault.rs
        // does NOT trim — so neither do we. A trailing newline that production
        // would reject must also fail here, otherwise this check is a false
        // negative (says OK while real vault unlock errors out).
        match base64::engine::general_purpose::STANDARD.decode(raw.as_bytes()) {
            Err(e) => AuditResult::error(
                NAME,
                i18n::t_args(
                    "doctor-audit-vault-key-invalid-base64",
                    &[("error", &e.to_string())],
                ),
                Some(i18n::t("doctor-audit-vault-key-invalid-base64-hint")),
            ),
            Ok(bytes) if bytes.len() != 32 => AuditResult::error(
                NAME,
                i18n::t_args(
                    "doctor-audit-vault-key-wrong-length",
                    &[("count", &bytes.len().to_string())],
                ),
                Some(i18n::t("doctor-audit-vault-key-wrong-length-hint")),
            ),
            Ok(_) => AuditResult::pass(NAME, i18n::t("doctor-audit-vault-key-ok")),
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
                return AuditResult::info(NAME, i18n::t("doctor-audit-api-listen-no-config"));
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
                    i18n::t_args(
                        "doctor-audit-api-listen-invalid-toml",
                        &[("error", &e.to_string())],
                    ),
                    Some(i18n::t("doctor-audit-api-listen-invalid-toml-hint")),
                );
            }
        };
        let listen = match value.get("api_listen").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => {
                return AuditResult::info(NAME, i18n::t("doctor-audit-api-listen-unset"));
            }
        };
        match listen.parse::<std::net::SocketAddr>() {
            Err(e) => AuditResult::error(
                NAME,
                i18n::t_args(
                    "doctor-audit-api-listen-invalid-addr",
                    &[("address", listen), ("error", &e.to_string())],
                ),
                Some(i18n::t("doctor-audit-api-listen-invalid-addr-hint")),
            ),
            Ok(addr) if addr.port() == 0 => AuditResult::warn(
                NAME,
                i18n::t_args(
                    "doctor-audit-api-listen-port-zero",
                    &[("address", &addr.to_string())],
                ),
                Some(i18n::t("doctor-audit-api-listen-port-zero-hint")),
            ),
            Ok(addr) if addr.port() < 1024 => AuditResult::warn(
                NAME,
                i18n::t_args(
                    "doctor-audit-api-listen-privileged",
                    &[("port", &addr.port().to_string())],
                ),
                Some(i18n::t("doctor-audit-api-listen-privileged-hint")),
            ),
            Ok(addr) => AuditResult::pass(
                NAME,
                i18n::t_args(
                    "doctor-audit-api-listen-ok",
                    &[("address", &addr.to_string())],
                ),
            ),
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
                i18n::t_args(
                    "doctor-audit-config-not-found",
                    &[("path", &path.display().to_string())],
                ),
                Some(i18n::t("doctor-audit-config-not-found-hint")),
            );
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                return AuditResult::error(
                    NAME,
                    i18n::t_args(
                        "doctor-audit-config-read-fail",
                        &[
                            ("path", &path.display().to_string()),
                            ("error", &e.to_string()),
                        ],
                    ),
                    None,
                );
            }
        };
        match toml::from_str::<toml::Value>(&raw) {
            Ok(_) => AuditResult::pass(
                NAME,
                i18n::t_args(
                    "doctor-audit-config-ok",
                    &[("path", &path.display().to_string())],
                ),
            ),
            Err(e) => AuditResult::error(
                NAME,
                i18n::t_args(
                    "doctor-audit-config-syntax-error",
                    &[
                        ("path", &path.display().to_string()),
                        ("error", &e.to_string()),
                    ],
                ),
                Some(i18n::t_args(
                    "doctor-audit-config-syntax-error-hint",
                    &[("path", &path.display().to_string())],
                )),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// EveryApiWiringCheck — report whether an EveryAPI gateway environment exists
// and whether LibreFang is actually wired to it.
//
// EveryAPI can reach LibreFang through two INDEPENDENT routes, and they do not
// see each other:
//
//   1. Config route — `{librefang_home}/providers/everyapi.toml` registers a
//      custom OpenAI-shaped provider. This is what API drivers use.
//   2. Env route — `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL` (and the `_API_URL`
//      / `_API_BASE` aliases) exported into the process. CLI-subprocess drivers
//      (claude-code, codex-cli) inherit these and get redirected regardless of
//      any provider entry. `everyapi use` injects exactly this shape.
//
// When both are live, which one applies depends on the agent's driver, not on
// anything the user can see in one place. Saying so explicitly is the whole
// point of this check.
//
// Read-only: `AuditCheck::run` has no repair path, and the relay key is never
// printed.
// ---------------------------------------------------------------------------

/// Env vars that redirect a CLI-subprocess driver, paired with the official
/// host that means "not proxied". Order is fixed so the reported var is
/// deterministic.
///
/// Kept in sync by hand with the production probes:
/// `claude_code.rs:claude_code_available` (`ANTHROPIC_BASE_URL`,
/// `ANTHROPIC_API_URL`) and `codex_cli.rs` (`OPENAI_BASE_URL`,
/// `OPENAI_API_BASE`).
const CLI_DRIVER_BASE_URL_VARS: &[(&str, &str)] = &[
    ("ANTHROPIC_BASE_URL", "api.anthropic.com"),
    ("ANTHROPIC_API_URL", "api.anthropic.com"),
    ("OPENAI_BASE_URL", "api.openai.com"),
    ("OPENAI_API_BASE", "api.openai.com"),
];

/// Everything the grader needs, gathered from the environment once.
#[derive(Debug, Default)]
struct EveryApiState {
    /// `everyapi` binary found on `PATH`.
    cli_on_path: bool,
    /// Credentials file exists AND carries a non-empty `relay_key`.
    /// The key value itself is never retained.
    credentials_usable: bool,
    /// Credentials file exists but has no usable `relay_key` (missing,
    /// blank, or the file is unreadable / not JSON).
    credentials_incomplete: bool,
    /// `{librefang_home}/providers/everyapi.toml` exists.
    provider_entry: Option<PathBuf>,
    /// A CLI-driver base-URL env var pointing somewhere non-official:
    /// `(var_name, host)`.
    env_route: Option<(String, String)>,
}

pub struct EveryApiWiringCheck;

impl AuditCheck for EveryApiWiringCheck {
    fn run(&self, ctx: &AuditContext) -> AuditResult {
        grade_everyapi_wiring(&gather_everyapi_state(ctx))
    }
}

fn gather_everyapi_state(ctx: &AuditContext) -> EveryApiState {
    let credentials_file = everyapi_credentials_path().filter(|p| p.exists());
    let credentials_usable = credentials_file.as_deref().is_some_and(relay_key_present);
    let credentials_incomplete = credentials_file.is_some() && !credentials_usable;
    let provider_path = ctx.librefang_home.join("providers").join("everyapi.toml");
    EveryApiState {
        cli_on_path: everyapi_cli_on_path(),
        credentials_usable,
        credentials_incomplete,
        provider_entry: provider_path.exists().then_some(provider_path),
        env_route: detect_cli_driver_env_route(),
    }
}

/// Grade a gathered [`EveryApiState`]. Pure — the whole decision table lives
/// here so it is unit-testable without touching the process env or the disk.
fn grade_everyapi_wiring(state: &EveryApiState) -> AuditResult {
    const NAME: &str = "everyapi_wiring";

    // Both routes live: the one case that genuinely needs a warning, because
    // the effective gateway differs per driver and neither surface mentions
    // the other.
    if let (Some((var, env_host)), Some(path)) = (&state.env_route, &state.provider_entry) {
        return AuditResult::warn(
            NAME,
            i18n::t_args(
                "doctor-everyapi-both-routes",
                &[
                    ("var", var),
                    ("host", env_host),
                    ("path", &path.display().to_string()),
                ],
            ),
            Some(i18n::t_args(
                "doctor-everyapi-both-routes-hint",
                &[("var", var), ("path", &path.display().to_string())],
            )),
        );
    }

    if let Some((var, env_host)) = &state.env_route {
        return AuditResult::info(
            NAME,
            i18n::t_args(
                "doctor-everyapi-env-route-only",
                &[("var", var), ("host", env_host)],
            ),
        );
    }

    if let Some(path) = &state.provider_entry {
        return AuditResult::pass(
            NAME,
            i18n::t_args(
                "doctor-everyapi-provider-only",
                &[("path", &path.display().to_string())],
            ),
        );
    }

    // Credentials exist but nothing routes to them. Deliberately Info, not
    // Warn: having the EveryAPI CLI installed without wiring LibreFang to it
    // is a legitimate state, and `doctor` should not paint it yellow every
    // run. The remediation command therefore lives in the summary — the
    // human path only renders `hint` for Warn/Error (see
    // `commands/doctor_cmd.rs`).
    if state.credentials_usable {
        return AuditResult::info(NAME, i18n::t("doctor-everyapi-not-connected"));
    }

    if state.credentials_incomplete {
        return AuditResult::info(NAME, i18n::t("doctor-everyapi-credentials-incomplete"));
    }

    if state.cli_on_path {
        return AuditResult::info(NAME, i18n::t("doctor-everyapi-cli-not-logged-in"));
    }

    AuditResult::info(NAME, i18n::t("doctor-everyapi-absent"))
}

/// PATH lookup for the `everyapi` binary.
///
/// Mirrors `desktop_install::which_lookup`, which is private to that module
/// (and takes an already-suffixed name — its only caller passes
/// `librefang-desktop.exe` on Windows). Appending the Windows suffix here
/// keeps that helper untouched.
fn everyapi_cli_on_path() -> bool {
    let name = if cfg!(windows) {
        "everyapi.exe"
    } else {
        "everyapi"
    };
    let Ok(path_var) = std::env::var("PATH") else {
        return false;
    };
    let separator = if cfg!(windows) { ';' } else { ':' };
    path_var
        .split(separator)
        .any(|dir| PathBuf::from(dir).join(name).exists())
}

/// `$XDG_CONFIG_HOME/everyapi/credentials.json`, falling back to
/// `~/.config/everyapi/credentials.json`. `None` when neither root resolves.
fn everyapi_credentials_path() -> Option<PathBuf> {
    let root = match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => dirs::home_dir()?.join(".config"),
    };
    Some(root.join("everyapi").join("credentials.json"))
}

/// Whether the credentials file carries a non-empty `relay_key`.
///
/// The value is inspected and dropped; it is never returned, logged, or
/// rendered into any message.
fn relay_key_present(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .get("relay_key")
        .and_then(|v| v.as_str())
        .is_some_and(|k| !k.trim().is_empty())
}

/// First base-URL env var pointing at a non-official host, as
/// `(var_name, host)`.
///
/// Deliberately diverges from `librefang_llm_drivers::drivers::is_proxied_via_env`,
/// which returns on the first var that is merely *set* — so an
/// `ANTHROPIC_BASE_URL` left at the official host masks a proxied
/// `ANTHROPIC_API_URL`. That is the right trade-off there (it answers "is the
/// primary var redirected"); here a masked redirect is a false negative in
/// exactly the scenario this check exists to catch, so we skip past official
/// and empty values instead of stopping. Do not "fix" this back.
fn detect_cli_driver_env_route() -> Option<(String, String)> {
    for &(var, official_host) in CLI_DRIVER_BASE_URL_VARS {
        let Ok(raw) = std::env::var(var) else {
            continue;
        };
        let normalized = raw.trim().trim_end_matches('/').to_lowercase();
        if normalized.is_empty() || normalized.contains(official_host) {
            continue;
        }
        let host = url_host(&normalized).unwrap_or(normalized);
        return Some((var.to_string(), host));
    }
    None
}

/// Extract the host (authority minus userinfo and port) from a URL-ish string.
/// Returns `None` when there is no scheme separator or the authority is empty,
/// letting the caller fall back to the raw value rather than print nothing.
fn url_host(url: &str) -> Option<String> {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest)?;
    let authority = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or(after_scheme);
    // Strip userinfo, then the port. A bracketed IPv6 literal keeps its
    // brackets so the result stays an unambiguous host.
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = match host.strip_prefix('[') {
        Some(rest) => match rest.split_once(']') {
            Some((v6, _)) => return Some(format!("[{v6}]")),
            None => host,
        },
        None => host.split_once(':').map_or(host, |(h, _)| h),
    };
    (!host.is_empty()).then(|| host.to_string())
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
        // SAFETY: guarded by env_lock() mutex; no concurrent thread reads/writes
        // LIBREFANG_VAULT_KEY while the lock is held.
        unsafe {
            match value {
                Some(v) => std::env::set_var("LIBREFANG_VAULT_KEY", v),
                None => std::env::remove_var("LIBREFANG_VAULT_KEY"),
            }
        }
        let result = f();
        // SAFETY: same as above.
        unsafe {
            match prev {
                Some(p) => std::env::set_var("LIBREFANG_VAULT_KEY", p),
                None => std::env::remove_var("LIBREFANG_VAULT_KEY"),
            }
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

    // ── EveryApiWiringCheck ───────────────────────────────────────────────
    //
    // The grading table is exercised through the pure `grade_everyapi_wiring`
    // so no test touches the process env or the real `~/.config`. The PATH
    // probe (`everyapi_cli_on_path`) is intentionally untested — it is a
    // one-line, host-dependent lookup with no decision logic.

    fn state() -> EveryApiState {
        EveryApiState::default()
    }

    fn env_route(host: &str) -> Option<(String, String)> {
        Some(("ANTHROPIC_BASE_URL".to_string(), host.to_string()))
    }

    fn provider_entry() -> Option<PathBuf> {
        Some(PathBuf::from("/home/u/.librefang/providers/everyapi.toml"))
    }

    #[test]
    fn everyapi_absent_everywhere_is_info() {
        let r = grade_everyapi_wiring(&state());
        assert_eq!(r.severity, Severity::Info);
        assert!(r.hint.is_none());
        assert!(r.summary.contains("No EveryAPI"));
    }

    #[test]
    fn everyapi_cli_present_without_credentials_is_info() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            ..state()
        });
        assert_eq!(r.severity, Severity::Info);
        assert!(r.summary.contains("everyapi login"));
    }

    #[test]
    fn everyapi_credentials_without_relay_key_is_info() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            credentials_incomplete: true,
            ..state()
        });
        assert_eq!(r.severity, Severity::Info);
        assert!(r.summary.contains("relay_key"));
    }

    /// The actionable case. Info (not Warn) on purpose — see the comment in
    /// `grade_everyapi_wiring`. Because the human path only renders `hint`
    /// for Warn/Error, the remediation command MUST be in the summary; this
    /// assertion is what stops someone from silently moving it to `hint`.
    #[test]
    fn everyapi_credentials_without_provider_entry_names_connect_command_in_summary() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            credentials_usable: true,
            ..state()
        });
        assert_eq!(r.severity, Severity::Info);
        assert!(r.summary.contains("librefang models connect everyapi"));
    }

    #[test]
    fn everyapi_provider_entry_only_is_pass() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            credentials_usable: true,
            provider_entry: provider_entry(),
            ..state()
        });
        assert_eq!(r.severity, Severity::Pass);
        assert!(r.summary.contains("everyapi.toml"));
    }

    #[test]
    fn everyapi_env_route_only_is_info() {
        let r = grade_everyapi_wiring(&EveryApiState {
            env_route: env_route("api.everyapi.ai"),
            ..state()
        });
        assert_eq!(r.severity, Severity::Info);
        assert!(r.summary.contains("ANTHROPIC_BASE_URL"));
        assert!(r.summary.contains("api.everyapi.ai"));
    }

    #[test]
    fn everyapi_both_routes_is_warn_naming_both_surfaces() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            credentials_usable: true,
            provider_entry: provider_entry(),
            env_route: env_route("api.everyapi.ai"),
            ..state()
        });
        assert_eq!(r.severity, Severity::Warn);
        assert!(r.hint.is_some());
        assert!(r.summary.contains("ANTHROPIC_BASE_URL"));
        assert!(r.summary.contains("everyapi.toml"));
    }

    /// Both routes pointing at the *same* gateway is still ambiguous — which
    /// one applies depends on the agent's driver, and the two can drift apart
    /// later. Identical hosts must not downgrade the warning.
    #[test]
    fn everyapi_both_routes_same_host_is_still_warn() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            credentials_usable: true,
            provider_entry: Some(PathBuf::from("/h/.librefang/providers/everyapi.toml")),
            env_route: env_route("api.everyapi.ai"),
            ..state()
        });
        assert_eq!(r.severity, Severity::Warn);
    }

    /// The env route alone is enough to warn: a user can export
    /// `ANTHROPIC_BASE_URL` without ever installing the EveryAPI CLI, and the
    /// provider entry is still a second, independent route.
    #[test]
    fn everyapi_both_routes_without_cli_or_credentials_is_warn() {
        let r = grade_everyapi_wiring(&EveryApiState {
            provider_entry: provider_entry(),
            env_route: env_route("gateway.internal"),
            ..state()
        });
        assert_eq!(r.severity, Severity::Warn);
    }

    #[test]
    fn url_host_extracts_authority() {
        assert_eq!(
            url_host("https://api.everyapi.ai/v1"),
            Some("api.everyapi.ai".to_string())
        );
        assert_eq!(
            url_host("http://user:pw@gw.internal:8080/v1"),
            Some("gw.internal".to_string())
        );
        assert_eq!(url_host("http://[::1]:4545/v1"), Some("[::1]".to_string()));
        // No scheme separator, or an empty authority: caller falls back to
        // the raw value rather than printing nothing.
        assert_eq!(url_host("api.everyapi.ai"), None);
        assert_eq!(url_host("https:///v1"), None);
    }

    #[test]
    fn relay_key_present_requires_non_empty_string() {
        let tmp = tmp_home();
        let path = tmp.path().join("credentials.json");

        std::fs::write(&path, r#"{"relay_key":"sk-abc","api_base":"https://x"}"#).unwrap();
        assert!(relay_key_present(&path));

        std::fs::write(&path, r#"{"relay_key":"   "}"#).unwrap();
        assert!(!relay_key_present(&path));

        std::fs::write(&path, r#"{"api_base":"https://x"}"#).unwrap();
        assert!(!relay_key_present(&path));

        // Non-JSON, and a missing file, are both "no usable key" rather than
        // a panic.
        std::fs::write(&path, "not json at all").unwrap();
        assert!(!relay_key_present(&path));
        assert!(!relay_key_present(&tmp.path().join("nope.json")));
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
