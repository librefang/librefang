//! LibreFang Rust SDK
//!
//! Official Rust client for the LibreFang Agent OS REST API.
//!
//! # Usage
//!
//! ```rust,no_run
//! use librefang::LibreFang;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = LibreFang::new("http://localhost:4545");
//!
//!     // List skills
//!     let skills = client.skills().list().await?;
//!     println!("Skills: {}", skills.skills.len());
//!
//!     // List models
//!     let models = client.models().list().await?;
//!     println!("Models: {}", models.models.len());
//!
//!     Ok(())
//! }
//! ```

use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

pub mod agents;
pub mod models;
pub mod providers;
pub mod skills;

pub use agents::Agents;
pub use models::Models;
pub use providers::Providers;
pub use skills::Skills;

#[derive(Error, Debug)]
pub enum Error {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error: {0}")]
    Api(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone)]
pub struct LibreFang {
    base_url: String,
    #[allow(dead_code)]
    client: Client,
    agents: Arc<Agents>,
    skills: Arc<Skills>,
    models: Arc<Models>,
    providers: Arc<Providers>,
}

impl LibreFang {
    pub fn new(base_url: impl Into<String>) -> Self {
        let base_url = base_url.into();
        let client = Client::new();

        let agents = Arc::new(Agents::new(base_url.clone(), client.clone()));
        let skills = Arc::new(Skills::new(base_url.clone(), client.clone()));
        let models = Arc::new(Models::new(base_url.clone(), client.clone()));
        let providers = Arc::new(Providers::new(base_url.clone(), client.clone()));

        Self {
            base_url,
            client,
            agents,
            skills,
            models,
            providers,
        }
    }

    pub fn agents(&self) -> &Arc<Agents> {
        &self.agents
    }

    pub fn skills(&self) -> &Arc<Skills> {
        &self.skills
    }

    pub fn models(&self) -> &Arc<Models> {
        &self.models
    }

    pub fn providers(&self) -> &Arc<Providers> {
        &self.providers
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

// Common response types
#[derive(Debug, Deserialize, Serialize)]
pub struct ApiResponse<T> {
    pub data: Option<T>,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ApiError {
    pub error: String,
}

pub fn extract_error(
    status: reqwest::StatusCode,
    body: &str,
) -> Error {
    if let Ok(api_err) = serde_json::from_str::<ApiError>(body) {
        Error::Api(api_err.error)
    } else {
        Error::Api(format!("HTTP {}", status))
    }
}
