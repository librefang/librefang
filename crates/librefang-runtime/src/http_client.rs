//! Shared HTTP client builder with fallback CA roots.
//!
//! On systems where system CA certificates are unavailable (e.g. musl builds
//! on Termux/Android, minimal Docker images), the default `reqwest` TLS
//! initialization panics. This module provides builders that fall back to
//! bundled Mozilla CA roots via `webpki-roots`.

use std::sync::OnceLock;

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

    rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth()
}

/// Return a `rustls::ClientConfig` that tries system certs first, then falls
/// back to bundled Mozilla CA roots. The result is cached after first call.
pub fn tls_config() -> rustls::ClientConfig {
    TLS_CONFIG.get_or_init(init_tls_config).clone()
}

/// Create an async [`reqwest::ClientBuilder`] with TLS that works even when
/// system CA certificates are missing.
pub fn client_builder() -> reqwest::ClientBuilder {
    reqwest::ClientBuilder::new().use_preconfigured_tls(tls_config())
}

/// Convenience: build a default async client with fallback CA roots.
pub fn new_client() -> reqwest::Client {
    client_builder()
        .build()
        .expect("HTTP client with bundled CA roots should always build")
}
