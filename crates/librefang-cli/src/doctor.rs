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
/// `Pass` reports the green case (showing it built confidence in noisy infra setups), `Info` is informational (no problem, no action), `Warn` surfaces a fixable misconfiguration, `Error` blocks correct operation.
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

/// State a check may consult — paths derived once by the caller so each check doesn't redo the same lookup.
/// Add fields here as new checks need them; keep it cheap to construct.
pub struct AuditContext {
    /// `~/.librefang/` (or `$LIBREFANG_HOME`).
    pub librefang_home: PathBuf,
    /// The `config.toml` the daemon would load, resolved once by the caller.
    ///
    /// Carried rather than recomputed as `librefang_home.join("config.toml")` so a doctor run under `LIBREFANG_CONFIG_PATH` inspects the file the daemon reads instead of an unrelated path that happens to sit in the home directory (#6695).
    pub config_path: PathBuf,
}

pub trait AuditCheck {
    fn run(&self, ctx: &AuditContext) -> AuditResult;
}

/// All currently registered checks.
/// The order here is the order shown to the user — group related checks together.
pub fn registered_checks() -> Vec<Box<dyn AuditCheck>> {
    // `mut` is only exercised by the platform-gated pushes below.
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut checks: Vec<Box<dyn AuditCheck>> = vec![
        Box::new(VaultKeyCheck),
        Box::new(ApiListenAddrCheck),
        Box::new(ConfigTomlSchemaCheck),
        Box::new(EveryApiWiringCheck),
    ];
    // Platform-specific checks are pushed as statements rather than listed above because `#[cfg]` on an element of a `vec![]` literal is not stable.
    // macOS and Windows doctor output is unchanged.
    #[cfg(target_os = "linux")]
    checks.push(Box::new(LinuxDesktopDepsCheck));
    checks
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
        let config_path = ctx.config_path.clone();
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
        let path = ctx.config_path.clone();
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

/// Env vars that redirect a CLI-subprocess driver, paired with the official host that means "not proxied".
/// Order is fixed so the reported var is deterministic.
///
/// Kept in sync by hand with the production probes: `claude_code.rs:claude_code_available` (`ANTHROPIC_BASE_URL`, `ANTHROPIC_API_URL`) and `codex_cli.rs` (`OPENAI_BASE_URL`, `OPENAI_API_BASE`).
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
    /// Credentials file exists but has no usable `relay_key` (missing, blank, or the file is unreadable / not JSON).
    credentials_incomplete: bool,
    /// `{librefang_home}/providers/everyapi.toml` exists.
    provider_entry: Option<PathBuf>,
    /// A CLI-driver base-URL env var pointing somewhere non-official: `(var_name, host)`.
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

/// Grade a gathered [`EveryApiState`].
/// Pure — the whole decision table lives here so it is unit-testable without touching the process env or the disk.
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
        // A provider entry alone is not health. The relay key was copied into
        // the LibreFang dotenv file at connect time and nothing re-validates
        // it, so `everyapi logout`, a key rotation, or a revocation leaves the
        // entry in place while every request through it 401s. Grading that
        // green is the one outcome an operator cannot act on, so the
        // credentials have to be consulted before this returns.
        if !state.credentials_usable {
            return AuditResult::warn(
                NAME,
                i18n::t_args(
                    "doctor-everyapi-provider-without-credentials",
                    &[("path", &path.display().to_string())],
                ),
                Some(i18n::t("doctor-everyapi-provider-without-credentials-hint")),
            );
        }
        return AuditResult::pass(
            NAME,
            i18n::t_args(
                "doctor-everyapi-provider-only",
                &[("path", &path.display().to_string())],
            ),
        );
    }

    // No provider file is required for CLI-managed credentials. Kernel boot
    // registers EveryAPI in memory and resolves the current key per request.
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
/// Mirrors `desktop_install::which_lookup`, which is private to that module (and takes an already-suffixed name — its only caller passes `librefang-desktop.exe` on Windows).
/// Appending the Windows suffix here keeps that helper untouched.
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

/// `$XDG_CONFIG_HOME/everyapi/credentials.json`, falling back to `~/.config/everyapi/credentials.json`.
/// `None` when neither root resolves.
fn everyapi_credentials_path() -> Option<PathBuf> {
    let root = match std::env::var("XDG_CONFIG_HOME") {
        Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => dirs::home_dir()?.join(".config"),
    };
    Some(root.join("everyapi").join("credentials.json"))
}

/// Whether the credentials file carries a non-empty `relay_key`.
///
/// The value is inspected and dropped; it is never returned, logged, or rendered into any message.
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

/// First base-URL env var pointing at a non-official host, as `(var_name, host)`.
///
/// Deliberately diverges from `librefang_llm_drivers::drivers::is_proxied_via_env`, which returns on the first var that is merely *set* — so an `ANTHROPIC_BASE_URL` left at the official host masks a proxied `ANTHROPIC_API_URL`.
/// That is the right trade-off there (it answers "is the primary var redirected"); here a masked redirect is a false negative in exactly the scenario this check exists to catch, so we skip past official and empty values instead of stopping.
/// Do not "fix" this back.
fn detect_cli_driver_env_route() -> Option<(String, String)> {
    for &(var, official_host) in CLI_DRIVER_BASE_URL_VARS {
        let Ok(raw) = std::env::var(var) else {
            continue;
        };
        if let Some(host) = env_route_redirect_host(&raw, official_host) {
            return Some((var.to_string(), host));
        }
    }
    None
}

/// The host `value` actually addresses, or `None` when it is empty or already
/// points at `official_host`.
///
/// Split out from [`detect_cli_driver_env_route`] so the bypasses below are
/// covered by tests rather than needing a process-wide environment mutation,
/// and so the comparison lives in exactly one place.
///
/// The comparison is against the *parsed host*, never a substring of the whole
/// value. A `contains` test is defeated by any URL that carries the official
/// name somewhere other than its authority:
///
/// - `https://api.anthropic.com.attacker.example/v1` — official name as a domain prefix
/// - `https://evil.example/api.anthropic.com` — official name in the path
/// - `https://api.anthropic.com@evil.example/` — official name as userinfo, while every HTTP
///   client actually connects to `evil.example`
///
/// Each of those would have been waved through as "official", making this
/// check report no redirection at all in precisely the case it exists to catch.
///
/// `librefang_llm_drivers::drivers::is_proxied_via_env` has the same weak
/// `contains` shape. That is pre-existing and out of scope here, but this
/// function is new and security-relevant, so it does not reproduce it.
fn env_route_redirect_host(value: &str, official_host: &str) -> Option<String> {
    let normalized = value.trim().trim_end_matches('/').to_lowercase();
    if normalized.is_empty() {
        return None;
    }
    let host = url_host(&normalized).unwrap_or(normalized);
    (host != official_host).then_some(host)
}

/// Extract the host (authority minus userinfo and port) from a URL-ish string.
/// Returns `None` when there is no scheme separator or the authority is empty, letting the caller fall back to the raw value rather than print nothing.
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

// ---------------------------------------------------------------------------
// LinuxDesktopDepsCheck — the GTK/WebKit stack `librefang-desktop` links against, probed through pkg-config.
//
// Linux-only: the desktop app uses the platform webview on macOS and Windows, so the check is not registered there.
//
// This is a soft check by design.
// A CLI-only install is a fully supported configuration, so a missing desktop stack must never reach `Severity::Error` — `Error` is the only severity that clears `all_ok` in `cmd_doctor` (`crates/librefang-cli/src/commands/doctor_cmd.rs:1221`), which in turn drives the `"all_ok"` JSON field (`:1229`) and the closing success-or-failure banner (`:1236`).
// Note that `cmd_doctor` returns `()` and never calls `process::exit`, so no severity changes the process exit code; what a severity decides is what the operator is told, not what the invoking shell sees.
// Missing libraries are `Warn`, which prints the remediation hint while leaving `all_ok` set, and a pkg-config we cannot execute is `Info`, because then we have learned nothing either way.
//
// The remediation hint suggests a *search* command per package manager rather than concrete package names.
// The pkg-config module names below are what the build actually requires and every one of them is verified by the probe; the package that ships a given module differs between distributions and between releases of the same distribution, so naming one would be a guess.
//
// Every item below carries `#[cfg_attr(not(test), cfg(target_os = "linux"))]`, the same idiom as `desktop_install::install_linux_appimage_to`: gone from non-Linux production builds, but still compiled under `cfg(test)` on every host so the os-release and probe mapping stay testable off Linux.
// ---------------------------------------------------------------------------

// Read only from `LinuxDesktopDepsCheck::run`, which a non-Linux test build compiles but never reaches — same reason `pkg_config_probe` carries the allow.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[allow(dead_code)]
const OS_RELEASE_PATH: &str = "/etc/os-release";

/// WebKitGTK pkg-config modules, tried in this order.
/// 4.1 is the current ABI, 4.0 the older one; either satisfies the desktop app.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
const WEBKIT_PKG_MODULES: [&str; 2] = ["webkit2gtk-4.1", "webkit2gtk-4.0"];

/// Probed only when neither WebKitGTK module answers, to tell "GTK is here, WebKitGTK is not" apart from "no GUI stack at all".
#[cfg_attr(not(test), cfg(target_os = "linux"))]
const GTK_PKG_MODULE: &str = "gtk+-3.0";

/// Result of one `pkg-config --exists <module>` invocation.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeOutcome {
    /// pkg-config knows the module.
    Present,
    /// pkg-config ran and does not know the module.
    Absent,
    /// pkg-config itself could not be executed.
    ToolMissing,
}

/// Package manager whose command shape the remediation hint should use.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DistroFamily {
    Debian,
    Arch,
    NixOs,
    Fedora,
    Unknown,
}

#[cfg_attr(not(test), cfg(target_os = "linux"))]
impl DistroFamily {
    /// Map one os-release identifier onto a family.
    /// `id` must already be lowercased.
    fn from_id(id: &str) -> Self {
        match id {
            "debian" | "ubuntu" | "deepin" => DistroFamily::Debian,
            "arch" => DistroFamily::Arch,
            "nixos" => DistroFamily::NixOs,
            "fedora" => DistroFamily::Fedora,
            _ => DistroFamily::Unknown,
        }
    }

    fn hint_key(&self) -> &'static str {
        match self {
            DistroFamily::Debian => "doctor-audit-desktop-deps-hint-apt",
            DistroFamily::Arch => "doctor-audit-desktop-deps-hint-pacman",
            DistroFamily::NixOs => "doctor-audit-desktop-deps-hint-nix",
            DistroFamily::Fedora => "doctor-audit-desktop-deps-hint-dnf",
            DistroFamily::Unknown => "doctor-audit-desktop-deps-hint-generic",
        }
    }
}

/// The `/etc/os-release` fields this check needs.
/// A missing or unreadable file yields `Default` (all fields `None`), which maps to [`DistroFamily::Unknown`] and the distro-agnostic hint.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[derive(Debug, Default)]
struct OsRelease {
    id: Option<String>,
    id_like: Option<String>,
    pretty_name: Option<String>,
}

#[cfg_attr(not(test), cfg(target_os = "linux"))]
impl OsRelease {
    /// Try `ID` first, then each `ID_LIKE` token.
    /// Derivatives commonly name only their base distribution in `ID_LIKE`.
    fn family(&self) -> DistroFamily {
        // The os-release spec says `ID` is lowercase, but shipped files disagree (Deepin 20 writes `ID=Deepin`), so fold case first.
        if let Some(id) = &self.id {
            let family = DistroFamily::from_id(&id.to_ascii_lowercase());
            if family != DistroFamily::Unknown {
                return family;
            }
        }
        if let Some(id_like) = &self.id_like {
            for token in id_like.split_whitespace() {
                let family = DistroFamily::from_id(&token.to_ascii_lowercase());
                if family != DistroFamily::Unknown {
                    return family;
                }
            }
        }
        DistroFamily::Unknown
    }

    /// Distribution name to address the user by in the hint.
    fn display_name(&self) -> String {
        for candidate in [&self.pretty_name, &self.id] {
            if let Some(name) = candidate.as_deref().filter(|n| !n.is_empty()) {
                return name.to_string();
            }
        }
        i18n::t("doctor-audit-desktop-deps-distro-unknown")
    }
}

/// Parse the `KEY=value` lines of an os-release file.
/// Values may be quoted (`PRETTY_NAME="Deepin 20.9"`), and the format allows blank lines and `#` comments.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
fn parse_os_release(content: &str) -> OsRelease {
    let mut parsed = OsRelease::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote_os_release_value(value.trim());
        match key.trim() {
            "ID" => parsed.id = Some(value),
            "ID_LIKE" => parsed.id_like = Some(value),
            "PRETTY_NAME" => parsed.pretty_name = Some(value),
            _ => {}
        }
    }
    parsed
}

/// Strip one layer of matching `"` or `'` quotes, the way a shell does when sourcing os-release.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
fn unquote_os_release_value(value: &str) -> String {
    for quote in ['"', '\''] {
        if let Some(inner) = value
            .strip_prefix(quote)
            .and_then(|rest| rest.strip_suffix(quote))
        {
            return inner.to_string();
        }
    }
    value.to_string()
}

/// Every pkg-config module the desktop app needs, in probe order, for the remediation hint.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
fn required_modules_list() -> String {
    let mut modules = WEBKIT_PKG_MODULES.to_vec();
    modules.push(GTK_PKG_MODULE);
    modules.join(", ")
}

/// Remediation hint for the detected distribution family.
/// `os_release` is the raw file contents, or `None` when the file is absent or unreadable.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
fn desktop_deps_hint(os_release: Option<&str>) -> String {
    let info = os_release.map(parse_os_release).unwrap_or_default();
    i18n::t_args(
        info.family().hint_key(),
        &[
            ("distro", &info.display_name()),
            ("modules", &required_modules_list()),
        ],
    )
}

/// Real probe: ask pkg-config whether it can resolve `module`.
/// A non-zero exit means "unknown module"; a spawn failure means pkg-config itself is unusable, which is a different answer.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[allow(dead_code)]
fn pkg_config_probe(module: &str) -> ProbeOutcome {
    let status = std::process::Command::new("pkg-config")
        .arg("--exists")
        .arg(module)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match status {
        Ok(status) if status.success() => ProbeOutcome::Present,
        Ok(_) => ProbeOutcome::Absent,
        Err(_) => ProbeOutcome::ToolMissing,
    }
}

/// Inner, dependency-injected variant of [`LinuxDesktopDepsCheck::run`]: both the probe and the os-release contents are passed in so tests exercise the mapping without spawning a subprocess or depending on a real `/etc/os-release`.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
fn evaluate_desktop_deps<P>(probe: P, os_release: Option<&str>) -> AuditResult
where
    P: Fn(&str) -> ProbeOutcome,
{
    const NAME: &str = "linux_desktop_deps";

    let mut webkit_module: Option<&str> = None;
    for module in WEBKIT_PKG_MODULES {
        match probe(module) {
            ProbeOutcome::Present => {
                webkit_module = Some(module);
                break;
            }
            ProbeOutcome::Absent => {}
            // The first probe doubles as the pkg-config availability test — once it has spawned, the later ones cannot fail to spawn — so reaching this arm means nothing was learned about the stack.
            ProbeOutcome::ToolMissing => {
                return AuditResult::info(NAME, i18n::t("doctor-audit-desktop-deps-no-pkg-config"));
            }
        }
    }

    if let Some(module) = webkit_module {
        return AuditResult::pass(
            NAME,
            i18n::t_args("doctor-audit-desktop-deps-ok", &[("module", module)]),
        );
    }

    if matches!(probe(GTK_PKG_MODULE), ProbeOutcome::Present) {
        return AuditResult::warn(
            NAME,
            i18n::t_args(
                "doctor-audit-desktop-deps-webkit-missing",
                &[
                    ("gtk", GTK_PKG_MODULE),
                    ("webkit", &WEBKIT_PKG_MODULES.join(", ")),
                ],
            ),
            Some(desktop_deps_hint(os_release)),
        );
    }

    // The tray module is never probed on this path — with no GTK there is nothing to attach a tray icon to.
    // So the summary states what the desktop app *requires* rather than claiming every module was measured and found absent.
    AuditResult::warn(
        NAME,
        i18n::t_args(
            "doctor-audit-desktop-deps-stack-missing",
            &[("modules", &required_modules_list())],
        ),
        Some(desktop_deps_hint(os_release)),
    )
}

#[cfg_attr(not(test), cfg(target_os = "linux"))]
#[allow(dead_code)]
pub struct LinuxDesktopDepsCheck;

#[cfg_attr(not(test), cfg(target_os = "linux"))]
impl AuditCheck for LinuxDesktopDepsCheck {
    fn run(&self, _ctx: &AuditContext) -> AuditResult {
        let os_release = std::fs::read_to_string(OS_RELEASE_PATH).ok();
        evaluate_desktop_deps(pkg_config_probe, os_release.as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with_home(home: PathBuf) -> AuditContext {
        AuditContext {
            config_path: home.join("config.toml"),
            librefang_home: home,
        }
    }

    fn tmp_home() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    // ── VaultKeyCheck ──────────────────────────────────────────────────────

    /// Process-wide lock for tests that mutate `LIBREFANG_VAULT_KEY`.
    /// `cargo test` runs tests in parallel by default, and env-var mutation is process-global, so without serialization these races clobber each other (and `run_all_returns_one_result_per_check`, which also reads the env var).
    /// No external dep needed — std `Mutex` is enough.
    fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// Run a closure with `LIBREFANG_VAULT_KEY` temporarily set to `value`.
    /// Holds [`env_lock`] for the entire body so concurrent vault-key tests (and any other env-var test in this binary) don't race.
    /// The original value is restored before the lock is released.
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

    #[test]
    fn everyapi_credentials_without_provider_entry_report_auto_detection() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            credentials_usable: true,
            ..state()
        });
        assert_eq!(r.severity, Severity::Info);
        assert!(r.summary.contains("auto-detect"));
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

    /// The provider entry is a file on disk and stays there forever; the relay key it depends on can be revoked at any time by `everyapi logout` or a rotation.
    /// Grading on the file alone reported green for a wiring that 401s on every request — the one outcome an operator cannot act on.
    #[test]
    fn everyapi_provider_entry_without_usable_credentials_warns() {
        let r = grade_everyapi_wiring(&EveryApiState {
            cli_on_path: true,
            credentials_usable: false,
            credentials_incomplete: true,
            provider_entry: provider_entry(),
            ..state()
        });
        assert_eq!(r.severity, Severity::Warn);
        assert!(r.summary.contains("everyapi.toml"));
        assert!(r.hint.is_some(), "a warn must carry a remediation hint");
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

    /// Both routes pointing at the *same* gateway is still ambiguous — which one applies depends on the agent's driver, and the two can drift apart later.
    /// Identical hosts must not downgrade the warning.
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

    /// The env route alone is enough to warn: a user can export `ANTHROPIC_BASE_URL` without ever installing the EveryAPI CLI, and the provider entry is still a second, independent route.
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
    fn env_route_redirect_host_is_not_fooled_by_lookalike_urls() {
        // Every value here connects somewhere other than api.anthropic.com while still
        // containing that string, so the previous `normalized.contains(official_host)` test
        // classified them as official and made the check report no redirection at all — a
        // false negative in exactly the case it exists to catch.
        for value in [
            // Official name as a domain prefix.
            "https://api.anthropic.com.attacker.example/v1",
            // Official name in the path.
            "https://evil.example/api.anthropic.com",
            // Official name as userinfo. Every HTTP client connects to evil.example here.
            "https://api.anthropic.com@evil.example/",
            "https://api.anthropic.com:tokenlike@evil.example/v1",
            // Suffix that merely ends with the official name's leading label.
            "https://not-api.anthropic.com.co/v1",
        ] {
            let host = env_route_redirect_host(value, "api.anthropic.com");
            assert!(
                host.is_some(),
                "{value} redirects off the official host and must be reported"
            );
            assert_ne!(
                host.as_deref(),
                Some("api.anthropic.com"),
                "{value} must not resolve to the official host"
            );
        }
    }

    #[test]
    fn env_route_redirect_host_still_skips_genuine_official_values() {
        // The other direction: the check must not cry redirect at operators who
        // set the var explicitly to the vendor's own endpoint. Trailing slash,
        // casing, whitespace, and a set-but-empty value all normalize away.
        for value in [
            "https://api.anthropic.com",
            "https://api.anthropic.com/",
            "  https://API.Anthropic.Com/  ",
            "",
            "   ",
        ] {
            assert_eq!(
                env_route_redirect_host(value, "api.anthropic.com"),
                None,
                "{value:?} must not be reported as a redirect"
            );
        }
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

    // ── LinuxDesktopDepsCheck ─────────────────────────────────────────────

    /// Deepin ships an uppercase `ID`, which the os-release spec says should be lowercase — the parser has to cope with both.
    const DEEPIN_OS_RELEASE: &str =
        "# comment line\nPRETTY_NAME=\"Deepin 20.9\"\nNAME=\"Deepin\"\nID=Deepin\nID_LIKE=debian\n";
    const NIXOS_OS_RELEASE: &str = "ID=nixos\nPRETTY_NAME=\"NixOS 24.05 (Uakari)\"\n";

    /// Probe stub: modules listed in `present` answer `Present`, every other module `Absent`.
    /// Tests never spawn the real pkg-config.
    fn stub_probe<'a>(present: &'a [&'a str]) -> impl Fn(&str) -> ProbeOutcome + 'a {
        move |module| {
            if present.contains(&module) {
                ProbeOutcome::Present
            } else {
                ProbeOutcome::Absent
            }
        }
    }

    fn missing_tool_probe(_module: &str) -> ProbeOutcome {
        ProbeOutcome::ToolMissing
    }

    #[test]
    fn os_release_maps_deepin_to_debian_family() {
        let info = parse_os_release(DEEPIN_OS_RELEASE);
        assert_eq!(info.family(), DistroFamily::Debian);
        assert_eq!(info.display_name(), "Deepin 20.9");
    }

    #[test]
    fn os_release_maps_known_ids_to_their_package_manager() {
        assert_eq!(
            parse_os_release(NIXOS_OS_RELEASE).family(),
            DistroFamily::NixOs
        );
        assert_eq!(parse_os_release("ID=arch\n").family(), DistroFamily::Arch);
        assert_eq!(
            parse_os_release("ID=fedora\nVERSION_ID=40\n").family(),
            DistroFamily::Fedora
        );
        assert_eq!(
            parse_os_release("ID=ubuntu\n").family(),
            DistroFamily::Debian
        );
    }

    #[test]
    fn os_release_unknown_id_is_unknown_family() {
        // An ID we have never seen must not be forced into a package manager — the hint falls back to the distro-agnostic wording.
        let info = parse_os_release("ID=frobnix\nPRETTY_NAME=\"Frobnix 9\"\n");
        assert_eq!(info.family(), DistroFamily::Unknown);
        assert_eq!(info.display_name(), "Frobnix 9");
    }

    #[test]
    fn os_release_falls_back_to_id_like_for_derivatives() {
        let info = parse_os_release("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n");
        assert_eq!(info.family(), DistroFamily::Debian);
    }

    #[test]
    fn os_release_absent_yields_unknown_family_and_localized_name() {
        let info = OsRelease::default();
        assert_eq!(info.family(), DistroFamily::Unknown);
        // Rendered from the locale bundle, so it must not be a missing-key marker.
        assert!(!info.display_name().starts_with('['));
    }

    #[test]
    fn desktop_deps_missing_pkg_config_is_informational() {
        let r = evaluate_desktop_deps(missing_tool_probe, Some(DEEPIN_OS_RELEASE));
        assert_eq!(r.severity, Severity::Info);
        assert!(r.hint.is_none());
    }

    #[test]
    fn desktop_deps_nothing_found_does_not_fail_doctor() {
        // A CLI-only install is fully supported: the missing desktop stack is reported with a hint but must never reach Error, which is the only severity that clears `all_ok` in `cmd_doctor`.
        let r = evaluate_desktop_deps(stub_probe(&[]), Some(DEEPIN_OS_RELEASE));
        assert_ne!(r.severity, Severity::Error);
        assert!(matches!(r.severity, Severity::Info | Severity::Warn));
        assert!(r.hint.is_some());
    }

    #[test]
    fn desktop_deps_full_stack_is_pass() {
        let r = evaluate_desktop_deps(
            stub_probe(&[WEBKIT_PKG_MODULES[0], GTK_PKG_MODULE]),
            Some(DEEPIN_OS_RELEASE),
        );
        assert_eq!(r.severity, Severity::Pass);
        assert!(r.summary.contains(WEBKIT_PKG_MODULES[0]));
    }

    #[test]
    fn desktop_deps_older_webkit_abi_still_passes() {
        let r = evaluate_desktop_deps(
            stub_probe(&[WEBKIT_PKG_MODULES[1]]),
            Some(DEEPIN_OS_RELEASE),
        );
        assert_eq!(r.severity, Severity::Pass);
        assert!(r.summary.contains(WEBKIT_PKG_MODULES[1]));
    }

    #[test]
    fn desktop_deps_gtk_without_webkit_is_warn() {
        let r = evaluate_desktop_deps(stub_probe(&[GTK_PKG_MODULE]), Some(DEEPIN_OS_RELEASE));
        assert_eq!(r.severity, Severity::Warn);
        assert!(r.summary.contains(GTK_PKG_MODULE));
    }

    #[test]
    fn desktop_deps_hint_is_package_manager_specific() {
        let apt = desktop_deps_hint(Some(DEEPIN_OS_RELEASE));
        let nix = desktop_deps_hint(Some(NIXOS_OS_RELEASE));
        let pacman = desktop_deps_hint(Some("ID=arch\n"));
        let dnf = desktop_deps_hint(Some("ID=fedora\n"));
        let generic = desktop_deps_hint(None);

        // The detected distribution is named back to the user.
        assert!(apt.contains("Deepin 20.9"));
        assert!(apt.contains("apt-cache search"));
        assert!(pacman.contains("pacman -Ss"));
        assert!(dnf.contains("dnf search"));
        // NixOS gets pointed at the flake instead of an imperative install, because installing dev libraries by hand does not work there.
        assert!(nix.contains("devShell"));
        assert!(nix.contains("librefang-desktop"));

        for hint in [&apt, &nix, &pacman, &dnf, &generic] {
            // Every hint renders (no `[key]` miss marker) and lists the pkg-config modules the build actually needs.
            assert!(!hint.starts_with('['), "unrendered hint: {hint}");
            assert!(hint.contains(GTK_PKG_MODULE), "hint omits modules: {hint}");
        }
    }

    #[test]
    fn required_modules_list_covers_every_probed_module() {
        let list = required_modules_list();
        for module in WEBKIT_PKG_MODULES {
            assert!(list.contains(module));
        }
        assert!(list.contains(GTK_PKG_MODULE));
    }

    #[test]
    fn unquote_os_release_value_strips_matching_quotes_only() {
        assert_eq!(unquote_os_release_value("\"Deepin 20.9\""), "Deepin 20.9");
        assert_eq!(unquote_os_release_value("'Deepin'"), "Deepin");
        assert_eq!(unquote_os_release_value("nixos"), "nixos");
        assert_eq!(unquote_os_release_value("\"unbalanced"), "\"unbalanced");
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
