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
    // `mut` is only exercised by the platform-gated pushes below.
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut checks: Vec<Box<dyn AuditCheck>> = vec![
        Box::new(VaultKeyCheck),
        Box::new(ApiListenAddrCheck),
        Box::new(ConfigTomlSchemaCheck),
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

/// Tray icon support.
#[cfg_attr(not(test), cfg(target_os = "linux"))]
const TRAY_PKG_MODULE: &str = "libayatana-appindicator3-0.1";

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
    modules.push(TRAY_PKG_MODULE);
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
        if matches!(probe(TRAY_PKG_MODULE), ProbeOutcome::Present) {
            return AuditResult::pass(
                NAME,
                i18n::t_args(
                    "doctor-audit-desktop-deps-ok",
                    &[("module", module), ("tray", TRAY_PKG_MODULE)],
                ),
            );
        }
        // The hint names the whole stack rather than the tray alone; every package manager below no-ops on what is already installed.
        return AuditResult::warn(
            NAME,
            i18n::t_args(
                "doctor-audit-desktop-deps-tray-missing",
                &[("module", module), ("tray", TRAY_PKG_MODULE)],
            ),
            Some(desktop_deps_hint(os_release)),
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
            stub_probe(&[WEBKIT_PKG_MODULES[0], GTK_PKG_MODULE, TRAY_PKG_MODULE]),
            Some(DEEPIN_OS_RELEASE),
        );
        assert_eq!(r.severity, Severity::Pass);
        assert!(r.summary.contains(WEBKIT_PKG_MODULES[0]));
    }

    #[test]
    fn desktop_deps_older_webkit_abi_still_passes() {
        let r = evaluate_desktop_deps(
            stub_probe(&[WEBKIT_PKG_MODULES[1], TRAY_PKG_MODULE]),
            Some(DEEPIN_OS_RELEASE),
        );
        assert_eq!(r.severity, Severity::Pass);
        assert!(r.summary.contains(WEBKIT_PKG_MODULES[1]));
    }

    #[test]
    fn desktop_deps_tray_missing_is_warn() {
        let r = evaluate_desktop_deps(
            stub_probe(&[WEBKIT_PKG_MODULES[0]]),
            Some(DEEPIN_OS_RELEASE),
        );
        assert_eq!(r.severity, Severity::Warn);
        assert!(r.summary.contains(TRAY_PKG_MODULE));
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
            assert!(hint.contains(TRAY_PKG_MODULE), "hint omits modules: {hint}");
        }
    }

    #[test]
    fn required_modules_list_covers_every_probed_module() {
        let list = required_modules_list();
        for module in WEBKIT_PKG_MODULES {
            assert!(list.contains(module));
        }
        assert!(list.contains(GTK_PKG_MODULE));
        assert!(list.contains(TRAY_PKG_MODULE));
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
