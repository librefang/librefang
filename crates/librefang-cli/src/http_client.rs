//! Blocking HTTP client builder with fallback CA roots.

pub fn client_builder() -> reqwest::blocking::ClientBuilder {
    let tls = librefang_runtime::http_client::tls_config();
    reqwest::blocking::ClientBuilder::new().use_preconfigured_tls(tls)
}

pub fn new_client() -> reqwest::blocking::Client {
    client_builder()
        .build()
        .expect("HTTP blocking client with bundled CA roots should always build")
}
