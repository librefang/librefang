use crate::{extract_error, Error, Result};
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Provider {
    pub name: String,
    pub status: Option<String>,
    #[serde(rename = "api_key_configured")]
    pub api_key_configured: bool,
    #[serde(rename = "default_model")]
    pub default_model: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProvidersResponse {
    pub providers: Vec<Provider>,
}

pub struct Providers {
    base_url: String,
    client: Client,
}

impl std::fmt::Debug for Providers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Providers").finish()
    }
}

impl Providers {
    pub fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list(&self) -> Result<ProvidersResponse> {
        let url = format!("{}/api/providers", self.base_url);
        let response = self.client.get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(extract_error(status, &body))
        }
    }
}
