use crate::{extract_error, Error, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct Agent {
    pub id: String,
    pub name: String,
    pub template: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AgentListResponse {
    pub agents: Vec<Agent>,
}

#[derive(Debug, Serialize)]
pub struct CreateAgentRequest {
    pub template: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MessageResponse {
    pub response: String,
    #[serde(rename = "input_tokens")]
    pub input_tokens: Option<u64>,
    #[serde(rename = "output_tokens")]
    pub output_tokens: Option<u64>,
    pub iterations: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct SendMessageRequest {
    pub message: String,
}

pub struct Agents {
    base_url: String,
    client: Client,
}

impl std::fmt::Debug for Agents {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agents").finish()
    }
}

impl Agents {
    pub fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list(&self) -> Result<AgentListResponse> {
        let url = format!("{}/api/agents", self.base_url);
        let response = self.client.get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(extract_error(status, &body))
        }
    }

    pub async fn get(&self, id: &str) -> Result<Agent> {
        let url = format!("{}/api/agents/{}", self.base_url, id);
        let response = self.client.get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(extract_error(status, &body))
        }
    }

    pub async fn create(&self, request: CreateAgentRequest) -> Result<Agent> {
        let url = format!("{}/api/agents", self.base_url);
        let response = self.client.post(&url).json(&request).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(serde_json::from_str(&body)?)
        } else {
            Err(extract_error(status, &body))
        }
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let url = format!("{}/api/agents/{}", self.base_url, id);
        let response = self.client.delete(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;

        if status.is_success() {
            Ok(())
        } else {
            Err(extract_error(status, &body))
        }
    }

    pub async fn message(&self, id: &str, message: &str) -> Result<MessageResponse> {
        let url = format!("{}/api/agents/{}/message", self.base_url, id);
        let request = SendMessageRequest {
            message: message.to_string(),
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

    /// Stream a message response from the agent.
    /// Returns a future that resolves to a streaming response.
    /// Use this with `futures::stream::StreamExt` to iterate.
    pub async fn stream(&self, id: &str, message: &str) -> Result<reqwest::Response> {
        let url = format!("{}/api/agents/{}/message/stream", self.base_url, id);
        let request = SendMessageRequest {
            message: message.to_string(),
        };

        let response = self.client
            .post(&url)
            .json(&request)
            .send()
            .await?;

        Ok(response)
    }
}
