use crate::{extract_error, Error, Result};
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub provider: Option<String>,
    #[serde(rename = "supports_streaming")]
    pub supports_streaming: Option<bool>,
    #[serde(rename = "supports_function_calling")]
    pub supports_function_calling: Option<bool>,
    #[serde(rename = "max_tokens")]
    pub max_tokens: Option<u32>,
    #[serde(rename = "context_window")]
    pub context_window: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ModelsResponse {
    pub models: Vec<Model>,
}

pub struct Models {
    base_url: String,
    client: Client,
}

impl std::fmt::Debug for Models {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Models").finish()
    }
}

impl Models {
    pub fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list(&self) -> Result<ModelsResponse> {
        let url = format!("{}/api/models", self.base_url);
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
