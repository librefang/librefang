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
use std::sync::OnceLock;

// ── TLS configuration ──────────────────────────────────────────────────

/// Cached TLS config — loaded once, reused for every client.
static TLS_CONFIG: OnceLock<rustls::ClientConfig> = OnceLock::new();

fn init_tls_config() -> rustls::ClientConfig {
    let mut root_store = rustls::RootCertStore::empty();

    let result = rustls_native_certs::load_native_certs();
    let (added, _) = root_store.add_parsable_certificates(result.certs);

    if added == 0 {
        tracing::warn!("No system CA certificates found, using bundled Mozilla CA roots");
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
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

/// Global proxy configuration, set once at kernel boot.
static GLOBAL_PROXY: OnceLock<ProxyConfig> = OnceLock::new();

/// Initialise the global proxy configuration.
///
/// Must be called once during daemon startup (before any HTTP client is built).
/// Subsequent calls are silently ignored.
///
/// Config-file values are also exported as environment variables so that
/// crates which build their own `reqwest::Client` (and thus rely on reqwest's
/// built-in env-var detection) automatically pick up the proxy settings.
pub fn init_proxy(cfg: ProxyConfig) {
    // Export config values as env vars for crates that build reqwest clients
    // without going through our builder (e.g. librefang-channels).
    if let Some(ref url) = cfg.http_proxy {
        if !url.is_empty() {
            std::env::set_var("HTTP_PROXY", url);
            std::env::set_var("http_proxy", url);
        }
    }
    if let Some(ref url) = cfg.https_proxy {
        if !url.is_empty() {
            std::env::set_var("HTTPS_PROXY", url);
            std::env::set_var("https_proxy", url);
        }
    }
    if let Some(ref no) = cfg.no_proxy {
        if !no.is_empty() {
            std::env::set_var("NO_PROXY", no);
            std::env::set_var("no_proxy", no);
        }
    }
    let _ = GLOBAL_PROXY.set(cfg);
}

/// Return the active proxy config (global or default-empty).
fn active_proxy() -> &'static ProxyConfig {
    static EMPTY: ProxyConfig = ProxyConfig {
        http_proxy: None,
        https_proxy: None,
        no_proxy: None,
    };
    GLOBAL_PROXY.get().unwrap_or(&EMPTY)
}

// ── Client builders ────────────────────────────────────────────────────

/// Build a [`reqwest::ClientBuilder`] with proxy settings from the global config
/// and TLS that works even when system CA certificates are missing.
///
/// Resolution order for each proxy field:
/// 1. Explicit value from `ProxyConfig` (config.toml `[proxy]` section).
/// 2. Standard environment variables (`HTTP_PROXY`, `HTTPS_PROXY`, `NO_PROXY`).
pub fn proxied_client_builder() -> reqwest::ClientBuilder {
    build_http_client(active_proxy())
}

/// Convenience: build a ready-to-use proxy-aware [`reqwest::Client`].
pub fn proxied_client() -> reqwest::Client {
    proxied_client_builder().build().unwrap_or_default()
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
/// Prefer [`proxied_client_builder`] which reads the global config automatically.
pub fn build_http_client(proxy: &ProxyConfig) -> reqwest::ClientBuilder {
    let mut builder = reqwest::Client::builder()
        .use_preconfigured_tls(tls_config())
        .user_agent(crate::USER_AGENT);

    let http_proxy = proxy
        .http_proxy
        .clone()
        .or_else(|| std::env::var("HTTP_PROXY").ok())
        .or_else(|| std::env::var("http_proxy").ok());

    let https_proxy = proxy
        .https_proxy
        .clone()
        .or_else(|| std::env::var("HTTPS_PROXY").ok())
        .or_else(|| std::env::var("https_proxy").ok());

    let no_proxy = proxy
        .no_proxy
        .clone()
        .or_else(|| std::env::var("NO_PROXY").ok())
        .or_else(|| std::env::var("no_proxy").ok());

    // Build the NoProxy filter once so it can be applied to each Proxy instance.
    let no_proxy_filter = no_proxy
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(reqwest::NoProxy::from_string);

    // Apply HTTP proxy.
    if let Some(ref url) = http_proxy {
        if !url.is_empty() {
            if let Ok(p) = Proxy::http(url) {
                builder = builder.proxy(p.no_proxy(no_proxy_filter.clone()));
            } else {
                tracing::warn!("invalid HTTP proxy URL: {url}");
            }
        }
    }

    // Apply HTTPS proxy.
    if let Some(ref url) = https_proxy {
        if !url.is_empty() {
            if let Ok(p) = Proxy::https(url) {
                builder = builder.proxy(p.no_proxy(no_proxy_filter.clone()));
            } else {
                tracing::warn!("invalid HTTPS proxy URL: {url}");
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
}
