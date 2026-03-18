//! Shared HTTP client builder with fallback CA roots.

use reqwest::ClientBuilder;

pub fn client_builder() -> ClientBuilder {
    let mut root_store = rustls::RootCertStore::empty();
    let result = rustls_native_certs::load_native_certs();
    let (added, _) = root_store.add_parsable_certificates(result.certs);
    if added == 0 {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }
    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    ClientBuilder::new().use_preconfigured_tls(tls_config)
}
