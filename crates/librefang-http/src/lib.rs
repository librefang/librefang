//! Centralized HTTP client builder with proxy support and fallback CA roots.
//!
//! All outbound HTTP connections should use [`proxied_client_builder`] (or the
//! convenience [`proxied_client`]) so that proxy settings from the config file
//! and environment variables are applied uniformly.
//!
//! On systems where system CA certificates are unavailable (e.g. musl builds
//! on Termux/Android, minimal Docker images), the default `reqwest` TLS
//! initialization panics. This module provides builders that fall back to
//! bundled Mozilla CA roots via `webpki-roots`.
//!
//! At daemon startup, call [`init_proxy`] once with the `[proxy]` section from
//! config.toml.  After that, every call to [`proxied_client_builder`] /
//! [`proxied_client`] will include the configured proxy settings.

use librefang_types::config::ProxyConfig;
use reqwest::Proxy;
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};

const USER_AGENT: &str = concat!("librefang/", env!("CARGO_PKG_VERSION"));

// ── TLS configuration ──────────────────────────────────────────────────

/// Cached TLS config — loaded once, reused for every client.
static TLS_CONFIG: OnceLock<rustls::ClientConfig> = OnceLock::new();

fn init_tls_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();

    // Always seed with bundled Mozilla CA roots first so common public CAs are
    // trusted even on systems with incomplete or outdated system cert stores
    // (minimal Docker images, Termux, corporate Linux with partial CA bundles).
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    // Supplement with system CA certificates (adds org-internal / self-signed CAs
    // and keeps trust anchors up-to-date without a librefang release).
    let result = rustls_native_certs::load_native_certs();
    let (added, _) = root_store.add_parsable_certificates(result.certs);
    if added == 0 {
        tracing::debug!("No system CA certificates found; relying on bundled Mozilla CA roots");
    }

    rustls::ClientConfig::builder_with_provider(
        rustls::crypto::aws_lc_rs::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .expect("default protocol versions")
    .with_root_certificates(root_store)
    .with_no_client_auth()
}

/// Return a `rustls::ClientConfig` that tries system certs first, then falls
/// back to bundled Mozilla CA roots. The result is cached after first call.
pub fn tls_config() -> rustls::ClientConfig {
    TLS_CONFIG.get_or_init(init_tls_config).clone()
}

// ── Proxy configuration ────────────────────────────────────────────────

/// Global proxy configuration, updated on boot and hot-reload.
static GLOBAL_PROXY: RwLock<Option<ProxyConfig>> = RwLock::new(None);

/// Every environment variable proxy resolution reads, in the spelling reqwest's system detection uses.
const PROXY_ENV_KEYS: &[&str] = &[
    "HTTP_PROXY",
    "http_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
    "REQUEST_METHOD",
];

/// The proxy environment as it was before the boot-time [`init_proxy`] export overwrote it.
///
/// Set only when that export actually wrote a variable.
/// Once set, every later resolution reads this snapshot instead of the live environment, because the live values are the boot config's and would otherwise survive a hot-reload that removes or narrows the `[proxy]` section.
static ORIGINAL_PROXY_ENV: OnceLock<HashMap<&'static str, String>> = OnceLock::new();

/// Read a proxy environment variable as the operator set it, ignoring what [`init_proxy`] exported.
fn original_proxy_env(key: &str) -> Option<String> {
    match ORIGINAL_PROXY_ENV.get() {
        Some(snapshot) => snapshot.get(key).cloned(),
        None => std::env::var(key).ok(),
    }
}

/// Snapshot the proxy environment once, before the first export mutates it.
fn snapshot_original_proxy_env() {
    ORIGINAL_PROXY_ENV.get_or_init(|| {
        PROXY_ENV_KEYS
            .iter()
            .filter_map(|key| std::env::var(key).ok().map(|value| (*key, value)))
            .collect()
    });
}

fn read_proxy_recover(
    proxy: &RwLock<Option<ProxyConfig>>,
) -> RwLockReadGuard<'_, Option<ProxyConfig>> {
    proxy.read().unwrap_or_else(|poisoned| {
        tracing::warn!("Global proxy configuration lock poisoned; recovering inner state");
        proxy.clear_poison();
        poisoned.into_inner()
    })
}

fn write_proxy_recover(
    proxy: &RwLock<Option<ProxyConfig>>,
) -> (RwLockWriteGuard<'_, Option<ProxyConfig>>, bool) {
    match proxy.write() {
        Ok(guard) => (guard, false),
        Err(poisoned) => {
            tracing::warn!("Global proxy configuration lock poisoned; recovering inner state");
            proxy.clear_poison();
            (poisoned.into_inner(), true)
        }
    }
}

fn update_proxy(
    proxy: &RwLock<Option<ProxyConfig>>,
    cfg: ProxyConfig,
    export_initial: impl FnOnce(&ProxyConfig),
) {
    let (mut guard, recovered) = write_proxy_recover(proxy);
    if guard.is_none() && !recovered {
        export_initial(&cfg);
    }
    *guard = Some(cfg);
}

/// Updates the global proxy configuration.
///
/// Can be called multiple times (e.g. during hot-reload). Previous values
/// are overwritten.
///
/// Config-file values are also exported as environment variables so that
/// crates which build their own `reqwest::Client` (and thus rely on reqwest's
/// built-in env-var detection) automatically pick up the proxy settings.
///
/// # Thread safety
///
/// `std::env::set_var` is inherently racy in a multi-threaded process.
/// Environment variables are only set during the initial bootstrap call
/// (when `GLOBAL_PROXY` is still `None`), which happens before the Tokio
/// runtime spawns worker threads. Subsequent calls (hot-reload) update
/// `GLOBAL_PROXY` only, avoiding the unsound `set_var` in a
/// multi-threaded context.
///
/// Because the exported variables outlive the config that produced them, the original environment is snapshotted before the first export and every later resolution reads that snapshot (see [`build_http_client`]).
/// That is what lets a hot-reload that removes or narrows `[proxy]` actually stop using the boot-time proxy for clients built through this crate.
pub fn init_proxy(cfg: ProxyConfig) {
    update_proxy(&GLOBAL_PROXY, cfg, |cfg| {
        // SAFETY: `is_initial` is only `true` during the synchronous bootstrap
        // call that happens before the tokio runtime (and its worker threads)
        // are started. Callers must preserve that bootstrap contract. The
        // write guard additionally makes the initial check and state update
        // one transaction. A poisoned lock never enters this branch because
        // recovery can happen during hot reload, after worker threads exist.
        if let Some(ref url) = cfg.http_proxy {
            if !url.is_empty() {
                if is_valid_proxy_url(url) {
                    snapshot_original_proxy_env();
                    unsafe {
                        std::env::set_var("HTTP_PROXY", url);
                        std::env::set_var("http_proxy", url);
                    }
                } else {
                    tracing::warn!(
                        "http_proxy has invalid scheme (expected http://, https://, socks5://, or socks5h://): {}",
                        librefang_types::config::redact_proxy_url(url)
                    );
                }
            }
        }
        if let Some(ref url) = cfg.https_proxy {
            if !url.is_empty() {
                if is_valid_proxy_url(url) {
                    snapshot_original_proxy_env();
                    unsafe {
                        std::env::set_var("HTTPS_PROXY", url);
                        std::env::set_var("https_proxy", url);
                    }
                } else {
                    tracing::warn!(
                        "https_proxy has invalid scheme (expected http://, https://, socks5://, or socks5h://): {}",
                        librefang_types::config::redact_proxy_url(url)
                    );
                }
            }
        }
        if let Some(ref no) = cfg.no_proxy {
            if !no.is_empty() {
                snapshot_original_proxy_env();
                unsafe {
                    std::env::set_var("NO_PROXY", no);
                    std::env::set_var("no_proxy", no);
                }
            }
        }
    });
}

/// Check if a proxy URL has a valid scheme.
fn is_valid_proxy_url(url: &str) -> bool {
    url.starts_with("http://")
        || url.starts_with("https://")
        || url.starts_with("socks5://")
        || url.starts_with("socks5h://")
}

/// Return the active proxy config (global or default-empty).
fn active_proxy() -> ProxyConfig {
    read_proxy_recover(&GLOBAL_PROXY)
        .clone()
        .unwrap_or_default()
}

// ── Client builders ────────────────────────────────────────────────────

/// Build a [`reqwest::ClientBuilder`] with proxy settings from the global config
/// and TLS that works even when system CA certificates are missing.
///
/// Proxy resolution:
/// - When no `ProxyConfig` field is set and `init_proxy()` has exported nothing, reqwest's built-in system proxy detection (`HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` / `NO_PROXY`, plus the platform proxy settings on macOS and Windows) applies unchanged.
/// - When any field is set, each field is resolved on its own: the config value wins, otherwise the matching environment variable (as it was before the boot export) is used. See [`build_http_client`].
/// - `init_proxy()` also exports config values as env vars, ensuring consistency
///   for crates that don't use this builder (e.g. `librefang-channels`).
pub fn proxied_client_builder() -> reqwest::ClientBuilder {
    build_http_client(&active_proxy())
}

/// Build an HTTP client that never uses configured or environment proxies.
///
/// SSRF-sensitive callers that pin a hostname to locally validated socket addresses must connect directly.
/// A proxy can resolve the original hostname itself, bypassing reqwest's DNS overrides and reopening DNS rebinding.
pub fn direct_client_builder() -> reqwest::ClientBuilder {
    build_http_client(&ProxyConfig::default()).no_proxy()
}

/// Convenience: build a ready-to-use proxy-aware [`reqwest::Client`].
pub fn proxied_client() -> reqwest::Client {
    proxied_client_builder()
        .build()
        .expect("HTTP client with proxy/TLS config should always build")
}

/// Proxy-aware [`reqwest::ClientBuilder`] that NEVER follows HTTP redirects, for OAuth machine-to-machine endpoints (metadata discovery, token exchange, dynamic client registration, token refresh).
///
/// OAuth endpoints return JSON directly and have no legitimate reason to emit a 3xx mid-flow.
/// Following a redirect on these calls is a security hole the per-URL SSRF guard cannot catch, because that guard validates only the initial URL — never the redirect `Location`.
/// A `307`/`308` on a credential-bearing POST replays the request body (`client_secret`, `code_verifier`, `refresh_token`) to an attacker-controlled redirect target; a `302` on discovery becomes a blind SSRF / cloud-metadata pivot.
///
/// This mirrors `librefang_runtime::web_fetch`'s pinned client, which disables reqwest's automatic redirect following for the same reason.
/// Use this for every outbound OAuth request; do not reach for [`proxied_client_builder`] there, whose default policy follows up to 10 hops.
pub fn oauth_client_builder() -> reqwest::ClientBuilder {
    proxied_client_builder().redirect(reqwest::redirect::Policy::none())
}

/// Convenience: ready-to-use proxy-aware OAuth [`reqwest::Client`] with redirect following disabled. See [`oauth_client_builder`].
pub fn oauth_client() -> reqwest::Client {
    oauth_client_builder()
        .build()
        .expect("OAuth HTTP client with proxy/TLS config should always build")
}

/// Build a fallback [`reqwest::Client`] used when a per-provider proxy override
/// is invalid. Identical to [`proxied_client`] but also enforces a total
/// per-request timeout so a stuck upstream cannot wedge the agent loop
/// indefinitely (#3756). Callers should `tracing::warn!` before invoking.
pub fn proxied_client_fallback() -> reqwest::Client {
    proxied_client_builder()
        // Total per-request budget on top of connect/read timeouts. 300s aligns
        // with the read_timeout already set in build_http_client.
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("HTTP fallback client with proxy/TLS config should always build")
}

/// Build a [`reqwest::Client`] that routes all traffic through the given proxy URL,
/// ignoring the global proxy config. Used for per-provider proxy overrides.
///
/// Returns `Err` if `proxy_url` is invalid or the client cannot be built.
/// Callers that want a best-effort client should call [`proxied_client`] on error
/// and log a warning explicitly — this function never silently falls back.
pub fn proxied_client_with_override(proxy_url: &str) -> reqwest::Result<reqwest::Client> {
    let proxy = Proxy::all(proxy_url)?;
    reqwest::Client::builder()
        .use_preconfigured_tls(tls_config())
        .user_agent(USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(300))
        .proxy(proxy)
        .build()
}

/// Backward-compatible alias for [`proxied_client_builder`].
pub fn client_builder() -> reqwest::ClientBuilder {
    proxied_client_builder()
}

/// Backward-compatible alias for [`proxied_client`].
pub fn new_client() -> reqwest::Client {
    proxied_client()
}

/// Build a [`reqwest::ClientBuilder`] with the given proxy settings applied
/// and TLS fallback to bundled Mozilla CA roots.
///
/// When every `ProxyConfig` field is unset, the builder keeps reqwest's automatic system proxy detection, unless [`init_proxy`] has already exported config values into the environment; from then on the operator's pre-export environment is applied explicitly (or a direct connection is used) so the stale export cannot outlive a hot-reload.
/// When any field is set, each one is resolved separately (config value first, then the matching environment variable) and applied explicitly.
/// That per-field fallback has to be done here because adding any explicit proxy to a reqwest builder disables its automatic environment detection entirely, which would otherwise drop the user's `NO_PROXY` and `HTTPS_PROXY` as soon as config sets only `http_proxy`.
///
/// Prefer [`proxied_client_builder`] which reads the global config automatically.
pub fn build_http_client(proxy: &ProxyConfig) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .use_preconfigured_tls(tls_config())
        .user_agent(USER_AGENT)
        // Default timeouts so the agent loop never hangs forever when an
        // upstream stalls. These are per-request defaults that any caller
        // may override via `.timeout()`, `.connect_timeout()`, etc. on the
        // returned builder. Issue #2340.
        //
        // - `connect_timeout`: cap TCP / TLS handshake. 30s is generous
        //   even for slow international links to LLM providers.
        // - `read_timeout`: per-read inactivity timeout, NOT total request
        //   time. Streaming LLM responses keep this alive as long as
        //   tokens trickle in; a true upstream stall will fire it. 300s
        //   gives slow models room while still bounding hangs.
        .connect_timeout(std::time::Duration::from_secs(30))
        .read_timeout(std::time::Duration::from_secs(300));

    let env_exported = ORIGINAL_PROXY_ENV.get().is_some();
    let resolved = match effective_proxy(proxy, original_proxy_env, env_exported) {
        // Nothing configured and the environment is untouched: keep reqwest's automatic system proxy detection.
        EffectiveProxy::System => return builder,
        // Nothing configured, the boot export polluted the environment, and the operator's own environment has no proxy: connect directly instead of letting reqwest pick up the stale export.
        EffectiveProxy::Direct => return builder.no_proxy(),
        EffectiveProxy::Explicit(resolved) => resolved,
    };

    // After the boot export, reqwest's automatic detection reads the stale exported values.
    // Turn it off up front so a resolution that ends up adding no proxy (a config with only `no_proxy`, or an invalid proxy URL that is skipped below) connects directly instead of falling back to the boot-time proxy.
    if env_exported {
        builder = builder.no_proxy();
    }

    let no_proxy_filter = resolved
        .no_proxy
        .as_deref()
        .and_then(reqwest::NoProxy::from_string);

    if let Some(ref url) = resolved.http {
        if let Ok(p) = Proxy::http(url) {
            builder = builder.proxy(p.no_proxy(no_proxy_filter.clone()));
        } else {
            tracing::warn!(
                "invalid HTTP proxy URL: {}",
                librefang_types::config::redact_proxy_url(url)
            );
        }
    }

    if let Some(ref url) = resolved.https {
        if let Ok(p) = Proxy::https(url) {
            builder = builder.proxy(p.no_proxy(no_proxy_filter.clone()));
        } else {
            tracing::warn!(
                "invalid HTTPS proxy URL: {}",
                librefang_types::config::redact_proxy_url(url)
            );
        }
    }

    builder
}

/// Proxy settings after merging `ProxyConfig` with the environment.
#[derive(Debug, Default, PartialEq, Eq)]
struct ResolvedProxy {
    http: Option<String>,
    https: Option<String>,
    no_proxy: Option<String>,
}

/// How [`build_http_client`] should route a client.
#[derive(Debug, PartialEq, Eq)]
enum EffectiveProxy {
    /// Leave reqwest's own system proxy detection in charge.
    System,
    /// Use no proxy at all.
    Direct,
    /// Apply exactly these proxies.
    Explicit(ResolvedProxy),
}

/// Decide the routing for `proxy` given the operator's original environment.
///
/// `env_exported` is true once [`init_proxy`] has written config values into the process environment.
/// From then on reqwest's system detection would read those stale values, so an empty config is resolved explicitly from `env` (the pre-export snapshot) instead of being handed back to reqwest.
/// The cost is that platform proxy settings (macOS / Windows system configuration) are not consulted in that one case; an explicit environment proxy still is.
fn effective_proxy(
    proxy: &ProxyConfig,
    env: impl Fn(&str) -> Option<String>,
    env_exported: bool,
) -> EffectiveProxy {
    let resolved = match resolve_proxy(proxy, &env) {
        Some(resolved) => resolved,
        None if !env_exported => return EffectiveProxy::System,
        None => merge_with_env(ResolvedProxy::default(), &env),
    };
    // A resolution with no proxy URL (for example a config that sets only `no_proxy`) adds nothing to the builder; after the export that must mean a direct connection, not reqwest's detection of the stale exported values.
    if env_exported && resolved.http.is_none() && resolved.https.is_none() {
        EffectiveProxy::Direct
    } else {
        EffectiveProxy::Explicit(resolved)
    }
}

/// Merge the `[proxy]` config with the environment, field by field.
///
/// Returns `None` when no config field is set, so the caller can leave reqwest's own system proxy detection in charge.
/// Otherwise each field takes the non-empty config value, falling back to the same environment variables (and precedence) that reqwest's system detection reads: `HTTP_PROXY` / `http_proxy`, `HTTPS_PROXY` / `https_proxy`, both falling back to `ALL_PROXY` / `all_proxy`, and `NO_PROXY` / `no_proxy`.
/// Like reqwest, the environment is ignored under CGI (`REQUEST_METHOD` set), where `HTTP_PROXY` can be injected through a request header.
fn resolve_proxy(
    proxy: &ProxyConfig,
    env: impl Fn(&str) -> Option<String>,
) -> Option<ResolvedProxy> {
    fn non_empty(value: Option<&String>) -> Option<String> {
        value.filter(|v| !v.is_empty()).cloned()
    }

    let config = ResolvedProxy {
        http: non_empty(proxy.http_proxy.as_ref()),
        https: non_empty(proxy.https_proxy.as_ref()),
        no_proxy: non_empty(proxy.no_proxy.as_ref()),
    };
    if config == ResolvedProxy::default() {
        return None;
    }
    Some(merge_with_env(config, env))
}

/// Fill every unset field of `config` from the environment, with reqwest's precedence and CGI rule.
fn merge_with_env(config: ResolvedProxy, env: impl Fn(&str) -> Option<String>) -> ResolvedProxy {
    let is_cgi = env("REQUEST_METHOD").is_some();
    let first_env = |keys: &[&str]| -> Option<String> {
        if is_cgi {
            return None;
        }
        keys.iter()
            .find_map(|key| env(key).filter(|v| !v.is_empty()))
    };
    let all = || first_env(&["ALL_PROXY", "all_proxy"]);

    ResolvedProxy {
        http: config
            .http
            .or_else(|| first_env(&["HTTP_PROXY", "http_proxy"]))
            .or_else(all),
        https: config
            .https
            .or_else(|| first_env(&["HTTPS_PROXY", "https_proxy"]))
            .or_else(all),
        no_proxy: config
            .no_proxy
            .or_else(|| first_env(&["NO_PROXY", "no_proxy"])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    #[test]
    fn test_empty_proxy_config_builds_client() {
        let proxy = ProxyConfig::default();
        let client = build_http_client(&proxy).build().unwrap();
        drop(client);
    }

    fn env_of(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let vars: std::collections::HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| vars.get(key).cloned()
    }

    #[test]
    fn resolve_proxy_defers_to_system_detection_when_config_is_empty() {
        let env = env_of(&[("HTTPS_PROXY", "http://corp:3128")]);
        assert_eq!(resolve_proxy(&ProxyConfig::default(), &env), None);
        let empty_strings = ProxyConfig {
            http_proxy: Some(String::new()),
            https_proxy: Some(String::new()),
            no_proxy: Some(String::new()),
        };
        assert_eq!(resolve_proxy(&empty_strings, &env), None);
    }

    #[test]
    fn resolve_proxy_falls_back_to_env_per_field() {
        // Config sets only http_proxy; reqwest disables env detection once any proxy is added, so the env HTTPS_PROXY and NO_PROXY must still be picked up here.
        let proxy = ProxyConfig {
            http_proxy: Some("http://config:3128".to_string()),
            ..ProxyConfig::default()
        };
        let env = env_of(&[
            ("HTTP_PROXY", "http://env-http:3128"),
            ("https_proxy", "http://env-https:3128"),
            ("NO_PROXY", "localhost,127.0.0.1"),
        ]);
        assert_eq!(
            resolve_proxy(&proxy, env),
            Some(ResolvedProxy {
                http: Some("http://config:3128".to_string()),
                https: Some("http://env-https:3128".to_string()),
                no_proxy: Some("localhost,127.0.0.1".to_string()),
            })
        );
    }

    #[test]
    fn resolve_proxy_config_wins_and_all_proxy_is_last_resort() {
        let proxy = ProxyConfig {
            no_proxy: Some("internal.example".to_string()),
            ..ProxyConfig::default()
        };
        let env = env_of(&[
            ("ALL_PROXY", "socks5://all:1080"),
            ("HTTPS_PROXY", "http://env-https:3128"),
            ("NO_PROXY", "localhost"),
        ]);
        assert_eq!(
            resolve_proxy(&proxy, env),
            Some(ResolvedProxy {
                http: Some("socks5://all:1080".to_string()),
                https: Some("http://env-https:3128".to_string()),
                no_proxy: Some("internal.example".to_string()),
            })
        );
    }

    #[test]
    fn resolve_proxy_ignores_env_under_cgi() {
        let proxy = ProxyConfig {
            https_proxy: Some("http://config:3128".to_string()),
            ..ProxyConfig::default()
        };
        let env = env_of(&[
            ("REQUEST_METHOD", "GET"),
            ("HTTP_PROXY", "http://injected:80"),
        ]);
        assert_eq!(
            resolve_proxy(&proxy, env),
            Some(ResolvedProxy {
                http: None,
                https: Some("http://config:3128".to_string()),
                no_proxy: None,
            })
        );
    }

    /// Hot-reload removing `[proxy]` after the boot export: the stale exported `HTTPS_PROXY` must not keep routing traffic.
    /// The resolver reads the pre-export snapshot, so an operator environment with no proxy means a direct connection, not reqwest's polluted system detection.
    #[test]
    fn effective_proxy_ignores_boot_export_after_proxy_removed() {
        let empty_snapshot = env_of(&[]);
        assert_eq!(
            effective_proxy(&ProxyConfig::default(), &empty_snapshot, true),
            EffectiveProxy::Direct
        );
        // Before any export, an empty config still defers to reqwest's system detection.
        assert_eq!(
            effective_proxy(&ProxyConfig::default(), &empty_snapshot, false),
            EffectiveProxy::System
        );
        // The operator's own environment proxy survives the removal.
        let operator_env = env_of(&[("HTTPS_PROXY", "http://operator:3128")]);
        assert_eq!(
            effective_proxy(&ProxyConfig::default(), &operator_env, true),
            EffectiveProxy::Explicit(ResolvedProxy {
                http: None,
                https: Some("http://operator:3128".to_string()),
                no_proxy: None,
            })
        );
    }

    /// Narrowing `[proxy]` from `https_proxy = old` to `http_proxy = new` must not keep sending HTTPS through `old`.
    #[test]
    fn effective_proxy_narrowed_config_does_not_inherit_boot_export() {
        let narrowed = ProxyConfig {
            http_proxy: Some("http://new:8080".to_string()),
            ..ProxyConfig::default()
        };
        assert_eq!(
            effective_proxy(&narrowed, env_of(&[]), true),
            EffectiveProxy::Explicit(ResolvedProxy {
                http: Some("http://new:8080".to_string()),
                https: None,
                no_proxy: None,
            })
        );
    }

    /// Narrowing `[proxy]` to only `no_proxy` after the boot export adds no proxy URL, which must route directly rather than through the stale exported `HTTPS_PROXY`.
    #[test]
    fn effective_proxy_no_proxy_only_config_after_export_is_direct() {
        let no_proxy_only = ProxyConfig {
            no_proxy: Some("internal.example".to_string()),
            ..ProxyConfig::default()
        };
        assert_eq!(
            effective_proxy(&no_proxy_only, env_of(&[]), true),
            EffectiveProxy::Direct
        );
    }

    #[test]
    fn test_direct_client_builder_builds_client() {
        let client = direct_client_builder().build().unwrap();
        drop(client);
    }

    #[test]
    fn test_proxy_config_with_values() {
        let proxy = ProxyConfig {
            http_proxy: Some("http://proxy.example.com:8080".to_string()),
            https_proxy: Some("http://proxy.example.com:8443".to_string()),
            no_proxy: Some("localhost,127.0.0.1".to_string()),
        };
        let client = build_http_client(&proxy).build().unwrap();
        drop(client);
    }

    #[test]
    fn test_proxied_client_without_init() {
        // Before init_proxy is called, should still work (empty config).
        let client = proxied_client();
        drop(client);
    }

    #[test]
    fn test_is_valid_proxy_url() {
        assert!(is_valid_proxy_url("http://proxy:8080"));
        assert!(is_valid_proxy_url("https://proxy:8080"));
        assert!(is_valid_proxy_url("socks5://proxy:1080"));
        assert!(is_valid_proxy_url("socks5h://proxy:1080"));
        assert!(!is_valid_proxy_url("ftp://proxy:21"));
        assert!(!is_valid_proxy_url("proxy:8080"));
        assert!(!is_valid_proxy_url(""));
    }

    #[test]
    fn concurrent_proxy_updates_export_only_once() {
        let proxy = Arc::new(RwLock::new(None));
        let exports = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();

        for n in 0..8 {
            let proxy = proxy.clone();
            let exports = exports.clone();
            let barrier = barrier.clone();
            threads.push(std::thread::spawn(move || {
                barrier.wait();
                update_proxy(
                    &proxy,
                    ProxyConfig {
                        http_proxy: Some(format!("http://proxy-{n}.example.com")),
                        ..ProxyConfig::default()
                    },
                    |_| {
                        exports.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    },
                );
            }));
        }

        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(exports.load(Ordering::SeqCst), 1);
        assert!(proxy.read().unwrap().is_some());
    }

    #[test]
    fn proxy_update_recovers_poison_without_exporting_environment() {
        let proxy = Arc::new(RwLock::new(None));
        let poisoned_proxy = proxy.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned_proxy.write().unwrap();
            panic!("poison proxy lock");
        });
        assert!(proxy.is_poisoned());

        let exports = AtomicUsize::new(0);
        let expected = ProxyConfig {
            https_proxy: Some("http://recovered.example.com".to_string()),
            ..ProxyConfig::default()
        };
        update_proxy(&proxy, expected.clone(), |_| {
            exports.fetch_add(1, Ordering::SeqCst);
        });

        assert_eq!(exports.load(Ordering::SeqCst), 0);
        assert!(!proxy.is_poisoned());
        let guard = proxy.read().unwrap();
        assert_eq!(guard.as_ref().unwrap().https_proxy, expected.https_proxy);
    }

    #[test]
    fn proxy_read_recovers_and_clears_poison() {
        let expected = ProxyConfig {
            no_proxy: Some("localhost".to_string()),
            ..ProxyConfig::default()
        };
        let proxy = Arc::new(RwLock::new(Some(expected.clone())));
        let poisoned_proxy = proxy.clone();
        let _ = std::panic::catch_unwind(move || {
            let _guard = poisoned_proxy.write().unwrap();
            panic!("poison proxy lock");
        });

        let guard = read_proxy_recover(&proxy);
        assert_eq!(guard.as_ref().unwrap().no_proxy, expected.no_proxy);
        drop(guard);
        assert!(!proxy.is_poisoned());
        assert!(proxy.read().is_ok());
    }
}
