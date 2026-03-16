use crate::{extract_error, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub author: Option<String>,
    pub version: Option<String>,
    pub enabled: bool,
    #[serde(rename = "has_prompt_context")]
    pub has_prompt_context: bool,
    #[serde(rename = "tools_count")]
    pub tools_count: u32,
}

#[derive(Debug, Deserialize)]
pub struct SkillsResponse {
    pub skills: Vec<Skill>,
}

#[derive(Debug, Serialize)]
pub struct InstallSkillRequest {
    pub name: String,
}

pub struct Skills {
    base_url: String,
    client: Client,
}

impl std::fmt::Debug for Skills {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Skills").finish()
    }
}

impl Skills {
    pub fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list(&self) -> Result<SkillsResponse> {
        let url = format!("{}/api/skills", self.base_url);
        let response = self.client.get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(extract_error(status, &body))
        }
    }

    pub async fn install(&self, name: &str) -> Result<serde_json::Value> {
        let url = format!("{}/api/skills/install", self.base_url);
        let request = InstallSkillRequest {
            name: name.to_string(),
        };
        let response = self.client.post(&url).json(&request).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(extract_error(status, &body))
        }
    }

    pub async fn uninstall(&self, name: &str) -> Result<serde_json::Value> {
        let url = format!("{}/api/skills/uninstall", self.base_url);
        let request = InstallSkillRequest {
            name: name.to_string(),
        };
        let response = self.client.post(&url).json(&request).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(extract_error(status, &body))
        }
    }
}
