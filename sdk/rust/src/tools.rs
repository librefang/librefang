//! Tools resource for the LibreFang Rust SDK.

use crate::{extract_error, Error, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ToolListResponse {
    pub tools: Vec<ToolDefinition>,
    pub total: usize,
}

#[derive(Debug, Clone)]
pub struct Tools {
    base_url: String,
    client: Client,
}

impl Tools {
    pub fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    /// List all available tools.
    pub async fn list(&self) -> Result<ToolListResponse> {
        let url = format!("{}/api/tools", self.base_url);
        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            return Err(extract_error(status, &body));
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// Get a single tool definition by name.
    pub async fn get(&self, name: &str) -> Result<ToolDefinition> {
        let url = format!("{}/api/tools/{}", self.base_url, name);
        let res = self.client.get(&url).send().await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            return Err(extract_error(status, &body));
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// Invoke a tool by name with the given input.
    pub async fn invoke(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let url = format!("{}/api/tools/{}/invoke", self.base_url, name);
        let res = self.client.post(&url).json(&input).send().await?;
        let status = res.status();
        let body = res.text().await?;
        if !status.is_success() {
            return Err(extract_error(status, &body));
        }
        Ok(serde_json::from_str(&body)?)
    }
}
