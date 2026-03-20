//! Shared HTTP client builder with fallback CA roots.

use std::sync::OnceLock;

static INIT: OnceLock<()> = OnceLock::new();

fn ensure_crypto_provider() {
    INIT.get_or_init(|| {
        // Initialize default crypto provider to prevent "CryptoProvider not found" errors
        // when multiple TLS backends are available
        use rustls::crypto::{aws_lc_rs, CryptoProvider};
        let _ = aws_lc_rs::default_provider().install_default();
    });
}

static TLS_CONFIG: OnceLock<rustls::ClientConfig> = OnceLock::new();

fn init_tls_config() -> rustls::ClientConfig {
    ensure_crypto_provider();
    let mut root_store = rustls::RootCertStore::empty();
    let result = rustls_native_certs::load_native_certs();
    let (added, _) = root_store.add_parsable_certificates(result.certs);
    if added == 0 {
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

pub fn client_builder() -> reqwest::ClientBuilder {
    let tls = TLS_CONFIG.get_or_init(init_tls_config).clone();
    reqwest::ClientBuilder::new().use_preconfigured_tls(tls)
}

pub fn new_client() -> reqwest::Client {
    client_builder()
        .build()
        .expect("HTTP client with bundled CA roots should always build")
}
