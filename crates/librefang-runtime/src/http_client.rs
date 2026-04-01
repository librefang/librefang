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
use std::sync::{OnceLock, RwLock};

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
pub fn init_proxy(cfg: ProxyConfig) {
    // Only export env vars during initial bootstrap (single-threaded context).
    // During hot-reload GLOBAL_PROXY already has a value, and calling
    // `std::env::set_var` from a multi-threaded tokio runtime is unsound.
    let is_initial = GLOBAL_PROXY.read().map(|g| g.is_none()).unwrap_or(true);

    if is_initial {
        if let Some(ref url) = cfg.http_proxy {
            if !url.is_empty() {
                if is_valid_proxy_url(url) {
                    std::env::set_var("HTTP_PROXY", url);
                    std::env::set_var("http_proxy", url);
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
                    std::env::set_var("HTTPS_PROXY", url);
                    std::env::set_var("https_proxy", url);
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
                std::env::set_var("NO_PROXY", no);
                std::env::set_var("no_proxy", no);
            }
        }
    }

    if let Ok(mut guard) = GLOBAL_PROXY.write() {
        *guard = Some(cfg);
    }
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
    GLOBAL_PROXY
        .read()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_default()
}

// ── Client builders ────────────────────────────────────────────────────

/// Build a [`reqwest::ClientBuilder`] with proxy settings from the global config
/// and TLS that works even when system CA certificates are missing.
///
/// Proxy resolution:
/// - Explicit values from `ProxyConfig` (config.toml `[proxy]` section) are applied directly.
/// - When `ProxyConfig` fields are `None`, reqwest's built-in env var detection
///   (`HTTP_PROXY` / `HTTPS_PROXY` / `NO_PROXY`) provides the fallback automatically.
/// - `init_proxy()` also exports config values as env vars, ensuring consistency
///   for crates that don't use this builder (e.g. `librefang-channels`).
pub fn proxied_client_builder() -> reqwest::ClientBuilder {
    build_http_client(&active_proxy())
}

/// Convenience: build a ready-to-use proxy-aware [`reqwest::Client`].
pub fn proxied_client() -> reqwest::Client {
    proxied_client_builder()
        .build()
        .expect("HTTP client with proxy/TLS config should always build")
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
/// Only explicit `ProxyConfig` values are set on the builder. When fields are
/// `None`, reqwest's built-in env var detection handles the fallback, avoiding
/// double-application of proxy settings that `init_proxy` already exported.
///
/// Prefer [`proxied_client_builder`] which reads the global config automatically.
pub fn build_http_client(proxy: &ProxyConfig) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .use_preconfigured_tls(tls_config())
        .user_agent(crate::USER_AGENT);

    // Build the NoProxy filter from explicit config only.
    let no_proxy_filter = proxy
        .no_proxy
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(reqwest::NoProxy::from_string);

    // Apply HTTP proxy from config. When None, reqwest reads HTTP_PROXY env var.
    if let Some(ref url) = proxy.http_proxy {
        if !url.is_empty() {
            if let Ok(p) = Proxy::http(url) {
                builder = builder.proxy(p.no_proxy(no_proxy_filter.clone()));
            } else {
                tracing::warn!(
                    "invalid HTTP proxy URL: {}",
                    librefang_types::config::redact_proxy_url(url)
                );
            }
        }
    }

    // Apply HTTPS proxy from config. When None, reqwest reads HTTPS_PROXY env var.
    if let Some(ref url) = proxy.https_proxy {
        if !url.is_empty() {
            if let Ok(p) = Proxy::https(url) {
                builder = builder.proxy(p.no_proxy(no_proxy_filter.clone()));
            } else {
                tracing::warn!(
                    "invalid HTTPS proxy URL: {}",
                    librefang_types::config::redact_proxy_url(url)
                );
            }
        }
    }

    builder
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_proxy_config_builds_client() {
        let proxy = ProxyConfig::default();
        let client = build_http_client(&proxy).build().unwrap();
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
}
