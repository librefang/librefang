//! LibreFang Rust SDK — AUTO-GENERATED from openapi.json.
//! Do not edit manually. Run: python3 scripts/codegen-sdks.py
//!
//! # Usage
//!
//! ```rust,no_run
//! use librefang::LibreFang;
//!
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let client = LibreFang::new("http://localhost:4545");
//!     let health = client.system.health().await?;
//!     println!("{:?}", health);
//!     Ok(())
//! }
//! ```

use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::{sync::Arc, time::Duration};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("HTTP {status}: {body}")]
    Api { status: u16, body: String },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

fn build_url<'a>(
    client: &Client,
    base_url: &str,
    path_segments: impl IntoIterator<Item = &'a str>,
) -> Result<reqwest::Url> {
    let path_segments: Vec<&str> = path_segments.into_iter().collect();
    if let Some(segment) = path_segments
        .iter()
        .copied()
        .find(|segment| matches!(*segment, "." | ".."))
    {
        return Err(Error::Api {
            status: 0,
            body: format!("invalid path segment: {}", segment),
        });
    }
    let mut url = client.get(base_url).build()?.url().clone();
    url.set_query(None);
    url.set_fragment(None);
    let mut segments = url.path_segments_mut().map_err(|_| Error::Api {
        status: 0,
        body: "base URL cannot contain path segments".to_string(),
    })?;
    segments.pop_if_empty();
    segments.extend(path_segments);
    drop(segments);
    Ok(url)
}

async fn do_req(
    client: &Client,
    base_url: &str,
    method: reqwest::Method,
    path_segments: &[&str],
    body: Option<Value>,
    query: &[(&str, Option<&str>)],
) -> Result<Value> {
    let url = build_url(client, base_url, path_segments.iter().copied())?;
    let req = client.request(method, url).timeout(DEFAULT_REQUEST_TIMEOUT);
    let filtered: Vec<(&str, &str)> = query
        .iter()
        .filter_map(|(k, v)| v.map(|vv| (*k, vv)))
        .collect();
    let req = if filtered.is_empty() {
        req
    } else {
        req.query(&filtered)
    };
    let req = if let Some(b) = body {
        req.json(&b)
    } else {
        req
    };
    let res = req.send().await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        return Err(Error::Api {
            status: status.as_u16(),
            body: text,
        });
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn do_stream(
    client: Client,
    base_url: String,
    path_segments: Vec<String>,
    method: reqwest::Method,
    body: Option<Value>,
    query: Vec<(String, Option<String>)>,
) -> tokio::sync::mpsc::Receiver<Value> {
    const STREAM_CHANNEL_CAPACITY: usize = 256;
    let (tx, rx) = tokio::sync::mpsc::channel(STREAM_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        let url = match build_url(&client, &base_url, path_segments.iter().map(String::as_str)) {
            Ok(url) => url,
            Err(e) => {
                let error = match e {
                    Error::Api { status: 0, body } => body,
                    other => other.to_string(),
                };
                let _ = tx
                    .send(serde_json::json!({
                        "error": error,
                        "status": 0,
                    }))
                    .await;
                return;
            }
        };
        let req = client
            .request(method, url)
            .header("Accept", "text/event-stream");
        let filtered: Vec<(String, String)> = query
            .into_iter()
            .filter_map(|(k, v)| v.map(|vv| (k, vv)))
            .collect();
        let req = if filtered.is_empty() {
            req
        } else {
            req.query(&filtered)
        };
        let req = if let Some(b) = body {
            req.json(&b)
        } else {
            req
        };
        let res = tokio::select! {
            _ = tx.closed() => return,
            result = req.send() => match result {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(serde_json::json!({
                        "error": e.to_string(),
                        "status": 0,
                    })).await;
                    return;
                }
            }
        };
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = tokio::select! {
                _ = tx.closed() => return,
                body = res.text() => body.unwrap_or_default(),
            };
            let _ = tx
                .send(serde_json::json!({
                    "error": format!("HTTP {}: {}", status, body),
                    "status": status,
                }))
                .await;
            return;
        }
        // Accumulate raw bytes so multi-byte UTF-8 codepoints are not split
        // by chunk boundaries (from_utf8_lossy on individual chunks corrupts
        // non-ASCII content). Split on newline, decode each complete line.
        // MAX_SSE_LINE caps memory on misbehaving streams.
        const MAX_SSE_LINE: usize = 8 * 1024 * 1024;
        let mut stream = res.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        loop {
            let chunk = tokio::select! {
                _ = tx.closed() => return,
                next = stream.next() => match next {
                    Some(Ok(chunk)) => chunk,
                    Some(Err(e)) => {
                        let _ = tx.send(serde_json::json!({
                            "error": format!("stream error: {}", e),
                            "status": 0,
                        })).await;
                        return;
                    }
                    None => break,
                },
            };
            buffer.extend_from_slice(&chunk);
            if buffer.len() > MAX_SSE_LINE {
                let _ = tx
                    .send(serde_json::json!({
                        "error": format!("SSE line exceeded {} bytes", MAX_SSE_LINE),
                        "status": 0,
                    }))
                    .await;
                return;
            }
            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buffer.drain(..=pos).collect();
                let line = match std::str::from_utf8(&line_bytes) {
                    Ok(s) => s.trim(),
                    Err(e) => {
                        if tx.send(serde_json::json!({
                            "error": format!("invalid utf-8 in SSE line at byte {}", e.valid_up_to()),
                            "status": 0,
                        })).await.is_err() {
                            return;
                        }
                        continue;
                    }
                };
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return;
                    }
                    match serde_json::from_str::<Value>(data) {
                        Ok(v) => {
                            if tx.send(v).await.is_err() {
                                return;
                            }
                        }
                        Err(_) => {
                            if tx.send(serde_json::json!({"raw": data})).await.is_err() {
                                return;
                            }
                        }
                    }
                }
            }
        }
        // A clean EOF can arrive without a trailing newline, leaving the last event in the buffer.
        // Parse it here rather than dropping it; the loop above only fires on a newline.
        if !buffer.is_empty() {
            match std::str::from_utf8(&buffer) {
                Ok(line) => {
                    if let Some(data) = line.trim().strip_prefix("data: ") {
                        if data != "[DONE]" {
                            let event = serde_json::from_str::<Value>(data)
                                .unwrap_or_else(|_| serde_json::json!({"raw": data}));
                            let _ = tx.send(event).await;
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(serde_json::json!({
                        "error": format!("invalid utf-8 in SSE line at byte {}", e.valid_up_to()),
                        "status": 0,
                    })).await;
                }
            }
        }
    });
    rx
}

#[derive(Debug, Clone)]
pub struct LibreFang {
    pub a2a: Arc<A2AResource>,
    pub agents: Arc<AgentsResource>,
    pub approvals: Arc<ApprovalsResource>,
    pub auth: Arc<AuthResource>,
    pub auto_dream: Arc<AutoDreamResource>,
    pub budget: Arc<BudgetResource>,
    pub channels: Arc<ChannelsResource>,
    pub extensions: Arc<ExtensionsResource>,
    pub goals: Arc<GoalsResource>,
    pub groups: Arc<GroupsResource>,
    pub hands: Arc<HandsResource>,
    pub inbox: Arc<InboxResource>,
    pub mcp: Arc<McpResource>,
    pub memory: Arc<MemoryResource>,
    pub models: Arc<ModelsResource>,
    pub network: Arc<NetworkResource>,
    pub pairing: Arc<PairingResource>,
    pub plugins: Arc<PluginsResource>,
    pub proactive_memory: Arc<ProactiveMemoryResource>,
    pub sessions: Arc<SessionsResource>,
    pub skills: Arc<SkillsResource>,
    pub system: Arc<SystemResource>,
    pub tools: Arc<ToolsResource>,
    pub users: Arc<UsersResource>,
    pub webhooks: Arc<WebhooksResource>,
    pub workflows: Arc<WorkflowsResource>,
    _base_url: String,
    _client: Client,
}

impl LibreFang {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .connect_timeout(DEFAULT_CONNECT_TIMEOUT)
            .build()
            .expect("failed to build HTTP client");
        Self::with_client(base_url, client)
    }

    /// Creates an SDK client using a caller-configured HTTP client.
    ///
    /// Use this to configure authentication headers, cookies, proxies,
    /// TLS, or other [`reqwest::Client`] behavior shared by all resources.
    pub fn with_client(base_url: impl Into<String>, client: Client) -> Self {
        let base_url = base_url.into().trim_end_matches('/').to_string();
        Self {
            a2a: Arc::new(A2AResource::new(base_url.clone(), client.clone())),
            agents: Arc::new(AgentsResource::new(base_url.clone(), client.clone())),
            approvals: Arc::new(ApprovalsResource::new(base_url.clone(), client.clone())),
            auth: Arc::new(AuthResource::new(base_url.clone(), client.clone())),
            auto_dream: Arc::new(AutoDreamResource::new(base_url.clone(), client.clone())),
            budget: Arc::new(BudgetResource::new(base_url.clone(), client.clone())),
            channels: Arc::new(ChannelsResource::new(base_url.clone(), client.clone())),
            extensions: Arc::new(ExtensionsResource::new(base_url.clone(), client.clone())),
            goals: Arc::new(GoalsResource::new(base_url.clone(), client.clone())),
            groups: Arc::new(GroupsResource::new(base_url.clone(), client.clone())),
            hands: Arc::new(HandsResource::new(base_url.clone(), client.clone())),
            inbox: Arc::new(InboxResource::new(base_url.clone(), client.clone())),
            mcp: Arc::new(McpResource::new(base_url.clone(), client.clone())),
            memory: Arc::new(MemoryResource::new(base_url.clone(), client.clone())),
            models: Arc::new(ModelsResource::new(base_url.clone(), client.clone())),
            network: Arc::new(NetworkResource::new(base_url.clone(), client.clone())),
            pairing: Arc::new(PairingResource::new(base_url.clone(), client.clone())),
            plugins: Arc::new(PluginsResource::new(base_url.clone(), client.clone())),
            proactive_memory: Arc::new(ProactiveMemoryResource::new(
                base_url.clone(),
                client.clone(),
            )),
            sessions: Arc::new(SessionsResource::new(base_url.clone(), client.clone())),
            skills: Arc::new(SkillsResource::new(base_url.clone(), client.clone())),
            system: Arc::new(SystemResource::new(base_url.clone(), client.clone())),
            tools: Arc::new(ToolsResource::new(base_url.clone(), client.clone())),
            users: Arc::new(UsersResource::new(base_url.clone(), client.clone())),
            webhooks: Arc::new(WebhooksResource::new(base_url.clone(), client.clone())),
            workflows: Arc::new(WorkflowsResource::new(base_url.clone(), client.clone())),
            _base_url: base_url,
            _client: client,
        }
    }
}

// ── A2A ──

#[derive(Debug, Clone)]
pub struct A2AResource {
    base_url: String,
    client: Client,
}

impl A2AResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn a2a_list_external_agents(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "a2a", "agents"],
            None,
            &[],
        )
        .await
    }

    pub async fn a2a_get_external_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "a2a", "agents", id],
            None,
            &[],
        )
        .await
    }

    pub async fn a2a_approve_external(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "a2a", "agents", id, "approve"],
            None,
            &[],
        )
        .await
    }

    pub async fn a2a_discover_external(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "a2a", "discover"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn a2a_send_external(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "a2a", "send"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn a2a_external_task_status(&self, id: &str, url: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "a2a", "tasks", id, "status"],
            None,
            &[("url", url)],
        )
        .await
    }
}

// ── Agents ──

#[derive(Debug, Clone)]
pub struct AgentsResource {
    base_url: String,
    client: Client,
}

impl AgentsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_agents(
        &self,
        q: Option<&str>,
        status: Option<&str>,
        limit: Option<&str>,
        offset: Option<&str>,
        sort: Option<&str>,
        order: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents"],
            None,
            &[
                ("q", q),
                ("status", status),
                ("limit", limit),
                ("offset", offset),
                ("sort", sort),
                ("order", order),
            ],
        )
        .await
    }

    pub async fn spawn_agent(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn bulk_create_agents(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", "bulk"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn bulk_delete_agents(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "agents", "bulk"],
            None,
            &[],
        )
        .await
    }

    pub async fn bulk_start_agents(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", "bulk", "start"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn bulk_stop_agents(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", "bulk", "stop"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_agent_identities(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", "identities"],
            None,
            &[],
        )
        .await
    }

    pub async fn reset_agent_identity(&self, name: &str, confirm: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", "identities", name, "reset"],
            None,
            &[("confirm", confirm)],
        )
        .await
    }

    pub async fn spawn_ephemeral_agent(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", "spawn-ephemeral"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id],
            None,
            &[],
        )
        .await
    }

    pub async fn kill_agent(&self, id: &str, confirm: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "agents", id],
            None,
            &[("confirm", confirm)],
        )
        .await
    }

    pub async fn patch_agent(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "agents", id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_agent_channels(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "channels"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_agent_channels(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "channels"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn clone_agent(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "clone"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn patch_agent_config(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "agents", id, "config"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_agent_deliveries(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "deliveries"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_agent_ephemeral_runs(&self, id: &str, limit: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "ephemeral-runs"],
            None,
            &[("limit", limit)],
        )
        .await
    }

    pub async fn list_agent_events(&self, id: &str, limit: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "events"],
            None,
            &[("limit", limit)],
        )
        .await
    }

    pub async fn list_agent_files(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "files"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_agent_file(&self, id: &str, filename: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "files", filename],
            None,
            &[],
        )
        .await
    }

    pub async fn set_agent_file(&self, id: &str, filename: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "files", filename],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_agent_file(&self, id: &str, filename: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "agents", id, "files", filename],
            None,
            &[],
        )
        .await
    }

    pub async fn delete_hand_agent_runtime_config(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "agents", id, "hand-runtime-config"],
            None,
            &[],
        )
        .await
    }

    pub async fn patch_hand_agent_runtime_config(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "agents", id, "hand-runtime-config"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn clear_agent_history(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "agents", id, "history"],
            None,
            &[],
        )
        .await
    }

    pub async fn update_agent_identity(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "agents", id, "identity"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn inject_message(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "inject"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn agent_logs(
        &self,
        id: &str,
        n: Option<&str>,
        level: Option<&str>,
        offset: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "logs"],
            None,
            &[("n", n), ("level", level), ("offset", offset)],
        )
        .await
    }

    pub async fn get_agent_mcp_servers(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "mcp_servers"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_agent_mcp_servers(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "mcp_servers"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn send_message(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "message"],
            Some(data),
            &[],
        )
        .await
    }

    pub fn send_message_stream(&self, id: &str, data: Value) -> tokio::sync::mpsc::Receiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            vec![
                "api".to_string(),
                "agents".to_string(),
                id.to_string(),
                "message".to_string(),
                "stream".to_string(),
            ],
            reqwest::Method::POST,
            Some(data),
            Vec::new(),
        )
    }

    pub async fn agent_metrics(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "metrics"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_agent_mode(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "mode"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn set_model(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "model"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn push_message(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "push"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn reload_agent_manifest(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "reload"],
            None,
            &[],
        )
        .await
    }

    pub async fn resume_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "resume"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_agent_runtime(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "runtime"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_agent_session(&self, id: &str, session_id: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "session"],
            None,
            &[("session_id", session_id)],
        )
        .await
    }

    pub async fn compact_session(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "session", "compact"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_agent_session_context(
        &self,
        id: &str,
        session_id: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "session", "context"],
            None,
            &[("session_id", session_id)],
        )
        .await
    }

    pub async fn reboot_session(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "session", "reboot"],
            None,
            &[],
        )
        .await
    }

    pub async fn reset_session(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "session", "reset"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_agent_sessions(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "sessions"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_agent_session(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "sessions"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn import_session(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "sessions", "import"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn export_session(&self, id: &str, session_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "sessions", session_id, "export"],
            None,
            &[],
        )
        .await
    }

    pub async fn stop_session(&self, id: &str, session_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "sessions", session_id, "stop"],
            None,
            &[],
        )
        .await
    }

    pub fn attach_session_stream(
        &self,
        id: &str,
        session_id: &str,
    ) -> tokio::sync::mpsc::Receiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            vec![
                "api".to_string(),
                "agents".to_string(),
                id.to_string(),
                "sessions".to_string(),
                session_id.to_string(),
                "stream".to_string(),
            ],
            reqwest::Method::GET,
            None,
            Vec::new(),
        )
    }

    pub async fn switch_agent_session(&self, id: &str, session_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "sessions", session_id, "switch"],
            None,
            &[],
        )
        .await
    }

    pub async fn export_session_trajectory(
        &self,
        id: &str,
        session_id: &str,
        format: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "sessions", session_id, "trajectory"],
            None,
            &[("format", format)],
        )
        .await
    }

    pub async fn get_agent_skills(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "skills"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_agent_skills(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "skills"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_agent_stats(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "stats"],
            None,
            &[],
        )
        .await
    }

    pub async fn stop_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "stop"],
            None,
            &[],
        )
        .await
    }

    pub async fn suspend_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "suspend"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_agent_tools(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "tools"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_agent_tools(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "agents", id, "tools"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_agent_traces(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "traces"],
            None,
            &[],
        )
        .await
    }

    pub async fn upload_file(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "upload"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn serve_upload(&self, file_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "uploads", file_id],
            None,
            &[],
        )
        .await
    }
}

// ── Approvals ──

#[derive(Debug, Clone)]
pub struct ApprovalsResource {
    base_url: String,
    client: Client,
}

impl ApprovalsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_approvals(&self, limit: Option<&str>, offset: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "approvals"],
            None,
            &[("limit", limit), ("offset", offset)],
        )
        .await
    }

    pub async fn create_approval(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "approvals"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn audit_log(
        &self,
        limit: Option<&str>,
        offset: Option<&str>,
        agent_id: Option<&str>,
        tool_name: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "approvals", "audit"],
            None,
            &[
                ("limit", limit),
                ("offset", offset),
                ("agent_id", agent_id),
                ("tool_name", tool_name),
            ],
        )
        .await
    }

    pub async fn batch_resolve(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "approvals", "batch"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn approval_count(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "approvals", "count"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_approvals_for_session(&self, session_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "approvals", "session", session_id],
            None,
            &[],
        )
        .await
    }

    pub async fn approve_all_for_session(&self, session_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "approvals", "session", session_id, "approve_all"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn reject_all_for_session(&self, session_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "approvals", "session", session_id, "reject_all"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_approval(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "approvals", id],
            None,
            &[],
        )
        .await
    }

    pub async fn approve_request(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "approvals", id, "approve"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn modify_request(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "approvals", id, "modify"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn reject_request(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "approvals", id, "reject"],
            None,
            &[],
        )
        .await
    }
}

// ── Auth ──

#[derive(Debug, Clone)]
pub struct AuthResource {
    base_url: String,
    client: Client,
}

impl AuthResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn auth_callback(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auth", "callback"],
            None,
            &[],
        )
        .await
    }

    pub async fn auth_callback_post(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "callback"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn change_password(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "change-password"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn dashboard_auth_check(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auth", "dashboard-check"],
            None,
            &[],
        )
        .await
    }

    pub async fn dashboard_login(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "dashboard-login"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn auth_introspect(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "introspect"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn auth_login(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auth", "login"],
            None,
            &[],
        )
        .await
    }

    pub async fn auth_login_provider(&self, provider: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auth", "login", provider],
            None,
            &[],
        )
        .await
    }

    pub async fn dashboard_logout(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "logout"],
            None,
            &[],
        )
        .await
    }

    pub async fn authentication_options(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "passkey", "authentication-options"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn authentication_verify(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "passkey", "authentication-verify"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_credentials(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auth", "passkey", "credentials"],
            None,
            &[],
        )
        .await
    }

    pub async fn revoke_credential(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "auth", "passkey", "credentials", id],
            None,
            &[],
        )
        .await
    }

    pub async fn registration_options(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "passkey", "registration-options"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn registration_verify(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "passkey", "registration-verify"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn auth_providers(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auth", "providers"],
            None,
            &[],
        )
        .await
    }

    pub async fn auth_refresh(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auth", "refresh"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn auth_userinfo(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auth", "userinfo"],
            None,
            &[],
        )
        .await
    }
}

// ── AutoDream ──

#[derive(Debug, Clone)]
pub struct AutoDreamResource {
    base_url: String,
    client: Client,
}

impl AutoDreamResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn auto_dream_abort(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auto-dream", "agents", id, "abort"],
            None,
            &[],
        )
        .await
    }

    pub async fn auto_dream_set_enabled(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "auto-dream", "agents", id, "enabled"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn auto_dream_trigger(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "auto-dream", "agents", id, "trigger"],
            None,
            &[],
        )
        .await
    }

    pub async fn auto_dream_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "auto-dream", "status"],
            None,
            &[],
        )
        .await
    }
}

// ── Budget ──

#[derive(Debug, Clone)]
pub struct BudgetResource {
    base_url: String,
    client: Client,
}

impl BudgetResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn budget_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "budget"],
            None,
            &[],
        )
        .await
    }

    pub async fn update_budget(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "budget"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn agent_budget_ranking(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "budget", "agents"],
            None,
            &[],
        )
        .await
    }

    pub async fn agent_budget_status(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "budget", "agents", id],
            None,
            &[],
        )
        .await
    }

    pub async fn update_agent_budget(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "budget", "agents", id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn provider_budget_list(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "budget", "providers"],
            None,
            &[],
        )
        .await
    }

    pub async fn update_provider_budget(&self, provider_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "budget", "providers", provider_id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn user_budget_ranking(&self, limit: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "budget", "users"],
            None,
            &[("limit", limit)],
        )
        .await
    }

    pub async fn user_budget_detail(&self, user_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "budget", "users", user_id],
            None,
            &[],
        )
        .await
    }

    pub async fn update_user_budget(&self, user_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "budget", "users", user_id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_user_budget(&self, user_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "budget", "users", user_id],
            None,
            &[],
        )
        .await
    }

    pub async fn usage_stats(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "usage"],
            None,
            &[("start_date", start_date), ("end_date", end_date)],
        )
        .await
    }

    pub async fn usage_by_model(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "usage", "by-model"],
            None,
            &[("start_date", start_date), ("end_date", end_date)],
        )
        .await
    }

    pub async fn usage_by_model_performance(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "usage", "by-model", "performance"],
            None,
            &[("start_date", start_date), ("end_date", end_date)],
        )
        .await
    }

    pub async fn usage_daily(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
        days: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "usage", "daily"],
            None,
            &[
                ("start_date", start_date),
                ("end_date", end_date),
                ("days", days),
            ],
        )
        .await
    }

    pub async fn usage_export(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
        format: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "usage", "export"],
            None,
            &[
                ("start_date", start_date),
                ("end_date", end_date),
                ("format", format),
            ],
        )
        .await
    }

    pub async fn usage_summary(
        &self,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "usage", "summary"],
            None,
            &[("start_date", start_date), ("end_date", end_date)],
        )
        .await
    }
}

// ── Channels ──

#[derive(Debug, Clone)]
pub struct ChannelsResource {
    base_url: String,
    client: Client,
}

impl ChannelsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_channels(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "channels"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_channel_registry(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "channels", "registry"],
            None,
            &[],
        )
        .await
    }

    pub async fn reload_channels(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "channels", "reload"],
            None,
            &[],
        )
        .await
    }

    pub async fn delete_sidecar_channel(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "channels", "sidecar", name],
            None,
            &[],
        )
        .await
    }

    pub async fn configure_sidecar_channel(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "channels", "sidecar", name, "configure"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_channel_qr(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "channels", name, "qr"],
            None,
            &[],
        )
        .await
    }
}

// ── Extensions ──

#[derive(Debug, Clone)]
pub struct ExtensionsResource {
    base_url: String,
    client: Client,
}

impl ExtensionsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_extensions(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "extensions"],
            None,
            &[],
        )
        .await
    }

    pub async fn install_extension(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "extensions", "install"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn uninstall_extension(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "extensions", "uninstall"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_extension(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "extensions", name],
            None,
            &[],
        )
        .await
    }
}

// ── Goals ──

#[derive(Debug, Clone)]
pub struct GoalsResource {
    base_url: String,
    client: Client,
}

impl GoalsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_goal_templates(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "goals", "templates"],
            None,
            &[],
        )
        .await
    }
}

// ── Groups ──

#[derive(Debug, Clone)]
pub struct GroupsResource {
    base_url: String,
    client: Client,
}

impl GroupsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_groups(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "groups"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_group(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "groups"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_group(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "groups", name],
            None,
            &[],
        )
        .await
    }

    pub async fn update_group(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "groups", name],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_group(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "groups", name],
            None,
            &[],
        )
        .await
    }

    pub async fn add_group_member(&self, name: &str, user: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "groups", name, "members", user],
            None,
            &[],
        )
        .await
    }

    pub async fn remove_group_member(&self, name: &str, user: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "groups", name, "members", user],
            None,
            &[],
        )
        .await
    }

    pub async fn user_groups(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "users", name, "groups"],
            None,
            &[],
        )
        .await
    }
}

// ── Hands ──

#[derive(Debug, Clone)]
pub struct HandsResource {
    base_url: String,
    client: Client,
}

impl HandsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_hands(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "hands"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_active_hands(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "hands", "active"],
            None,
            &[],
        )
        .await
    }

    pub async fn install_hand(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", "install"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn deactivate_hand(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "hands", "instances", id],
            None,
            &[],
        )
        .await
    }

    pub async fn hand_instance_browser(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "hands", "instances", id, "browser"],
            None,
            &[],
        )
        .await
    }

    pub async fn pause_hand(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", "instances", id, "pause"],
            None,
            &[],
        )
        .await
    }

    pub async fn resume_hand(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", "instances", id, "resume"],
            None,
            &[],
        )
        .await
    }

    pub async fn hand_stats(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "hands", "instances", id, "stats"],
            None,
            &[],
        )
        .await
    }

    pub async fn install_hand_from_marketplace(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", "marketplace", "install"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn reload_hands(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", "reload"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_hand(&self, hand_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "hands", hand_id],
            None,
            &[],
        )
        .await
    }

    pub async fn uninstall_hand(&self, hand_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "hands", hand_id],
            None,
            &[],
        )
        .await
    }

    pub async fn activate_hand(&self, hand_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", hand_id, "activate"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn check_hand_deps(&self, hand_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", hand_id, "check-deps"],
            None,
            &[],
        )
        .await
    }

    pub async fn install_hand_deps(&self, hand_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", hand_id, "install-deps"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_hand_manifest(&self, hand_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "hands", hand_id, "manifest"],
            None,
            &[],
        )
        .await
    }

    pub async fn update_hand_manifest(&self, hand_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "hands", hand_id, "manifest"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn set_hand_secret(&self, hand_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hands", hand_id, "secret"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_hand_settings(&self, hand_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "hands", hand_id, "settings"],
            None,
            &[],
        )
        .await
    }

    pub async fn update_hand_settings(&self, hand_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "hands", hand_id, "settings"],
            Some(data),
            &[],
        )
        .await
    }
}

// ── Inbox ──

#[derive(Debug, Clone)]
pub struct InboxResource {
    base_url: String,
    client: Client,
}

impl InboxResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn inbox_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "inbox", "status"],
            None,
            &[],
        )
        .await
    }
}

// ── Mcp ──

#[derive(Debug, Clone)]
pub struct McpResource {
    base_url: String,
    client: Client,
}

impl McpResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_mcp_catalog(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "mcp", "catalog"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_mcp_catalog_entry(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "mcp", "catalog", id],
            None,
            &[],
        )
        .await
    }

    pub async fn mcp_health_handler(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "mcp", "health"],
            None,
            &[],
        )
        .await
    }

    pub async fn reload_mcp_handler(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "mcp", "reload"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_mcp_servers(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "mcp", "servers"],
            None,
            &[],
        )
        .await
    }

    pub async fn add_mcp_server(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "mcp", "servers"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_mcp_server(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "mcp", "servers", name],
            None,
            &[],
        )
        .await
    }

    pub async fn update_mcp_server(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "mcp", "servers", name],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_mcp_server(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "mcp", "servers", name],
            None,
            &[],
        )
        .await
    }

    pub async fn auth_revoke(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "mcp", "servers", name, "auth", "revoke"],
            None,
            &[],
        )
        .await
    }

    pub async fn auth_start(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "mcp", "servers", name, "auth", "start"],
            None,
            &[],
        )
        .await
    }

    pub async fn auth_status(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "mcp", "servers", name, "auth", "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn reconnect_mcp_server_handler(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "mcp", "servers", name, "reconnect"],
            None,
            &[],
        )
        .await
    }

    pub async fn patch_mcp_server_taint(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "mcp", "servers", name, "taint"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_mcp_taint_rules(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "mcp", "taint-rules"],
            None,
            &[],
        )
        .await
    }
}

// ── Memory ──

#[derive(Debug, Clone)]
pub struct MemoryResource {
    base_url: String,
    client: Client,
}

impl MemoryResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn export_agent_memory(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "memory", "export"],
            None,
            &[],
        )
        .await
    }

    pub async fn import_agent_memory(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "agents", id, "memory", "import"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_agent_kv(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "kv"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_agent_kv_key(&self, id: &str, key: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "kv", key],
            None,
            &[],
        )
        .await
    }

    pub async fn set_agent_kv_key(&self, id: &str, key: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "memory", "agents", id, "kv", key],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_agent_kv_key(&self, id: &str, key: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "memory", "agents", id, "kv", key],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_config_get(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "config"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_config_patch(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "memory", "config"],
            Some(data),
            &[],
        )
        .await
    }
}

// ── Models ──

#[derive(Debug, Clone)]
pub struct ModelsResource {
    base_url: String,
    client: Client,
}

impl ModelsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn catalog_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "catalog", "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn catalog_update(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "catalog", "update"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_credential_pools(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "credential-pools"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_all_models(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "models"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_aliases(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "models", "aliases"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_alias(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "models", "aliases"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_alias(&self, alias: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "models", "aliases", alias],
            None,
            &[],
        )
        .await
    }

    pub async fn add_custom_model(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "models", "custom"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn remove_custom_model(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "models", "custom", id],
            None,
            &[],
        )
        .await
    }

    pub async fn get_model_overrides(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "models", "overrides", id],
            None,
            &[],
        )
        .await
    }

    pub async fn set_model_overrides(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "models", "overrides", id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_model_overrides(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "models", "overrides", id],
            None,
            &[],
        )
        .await
    }

    pub async fn get_model(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "models", id],
            None,
            &[],
        )
        .await
    }

    pub async fn list_providers(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "providers"],
            None,
            &[],
        )
        .await
    }

    pub async fn copilot_oauth_poll(&self, poll_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &[
                "api",
                "providers",
                "github-copilot",
                "oauth",
                "poll",
                poll_id,
            ],
            None,
            &[],
        )
        .await
    }

    pub async fn copilot_oauth_start(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "providers", "github-copilot", "oauth", "start"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_provider(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "providers", name],
            None,
            &[],
        )
        .await
    }

    pub async fn set_default_provider(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "providers", name, "default"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn set_provider_discovery(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "providers", name, "discovery"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn enable_provider(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "providers", name, "enable"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_provider_key(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "providers", name, "key"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_provider_key(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "providers", name, "key"],
            None,
            &[],
        )
        .await
    }

    pub async fn test_provider(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "providers", name, "test"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_provider_url(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "providers", name, "url"],
            Some(data),
            &[],
        )
        .await
    }
}

// ── Network ──

#[derive(Debug, Clone)]
pub struct NetworkResource {
    base_url: String,
    client: Client,
}

impl NetworkResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn comms_events(&self, limit: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "comms", "events"],
            None,
            &[("limit", limit)],
        )
        .await
    }

    pub fn comms_events_stream(&self) -> tokio::sync::mpsc::Receiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            vec![
                "api".to_string(),
                "comms".to_string(),
                "events".to_string(),
                "stream".to_string(),
            ],
            reqwest::Method::GET,
            None,
            Vec::new(),
        )
    }

    pub async fn comms_send(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "comms", "send"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn comms_task(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "comms", "task"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn comms_topology(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "comms", "topology"],
            None,
            &[],
        )
        .await
    }

    pub async fn network_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "network", "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn network_trusted_peers(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "network", "trusted-peers"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_peers(&self, offset: Option<&str>, limit: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "peers"],
            None,
            &[("offset", offset), ("limit", limit)],
        )
        .await
    }

    pub async fn get_peer(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "peers", id],
            None,
            &[],
        )
        .await
    }
}

// ── Pairing ──

#[derive(Debug, Clone)]
pub struct PairingResource {
    base_url: String,
    client: Client,
}

impl PairingResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn pairing_complete(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "pairing", "complete"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn pairing_devices(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "pairing", "devices"],
            None,
            &[],
        )
        .await
    }

    pub async fn pairing_remove_device(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "pairing", "devices", id],
            None,
            &[],
        )
        .await
    }

    pub async fn pairing_notify(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "pairing", "notify"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn pairing_request(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "pairing", "request"],
            None,
            &[],
        )
        .await
    }
}

// ── Plugins ──

#[derive(Debug, Clone)]
pub struct PluginsResource {
    base_url: String,
    client: Client,
}

impl PluginsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn context_engine_chain(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "context-engine", "chain"],
            None,
            &[],
        )
        .await
    }

    pub async fn context_engine_config(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "context-engine", "config"],
            None,
            &[],
        )
        .await
    }

    pub async fn context_engine_health(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "context-engine", "health"],
            None,
            &[],
        )
        .await
    }

    pub async fn context_engine_metrics(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "context-engine", "metrics"],
            None,
            &[],
        )
        .await
    }

    pub async fn context_engine_sandbox_policy(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "context-engine", "sandbox-policy"],
            None,
            &[],
        )
        .await
    }

    pub async fn context_engine_traces(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "context-engine", "traces"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_plugins(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins"],
            None,
            &[],
        )
        .await
    }

    pub async fn plugin_doctor(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins", "doctor"],
            None,
            &[],
        )
        .await
    }

    pub async fn install_plugin(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", "install"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_plugin_registries(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins", "registries"],
            None,
            &[],
        )
        .await
    }

    pub async fn scaffold_plugin(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", "scaffold"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn uninstall_plugin(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", "uninstall"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_plugin(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins", name],
            None,
            &[],
        )
        .await
    }

    pub async fn plugin_advanced_config(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins", name, "advanced-config"],
            None,
            &[],
        )
        .await
    }

    pub async fn disable_plugin(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "disable"],
            None,
            &[],
        )
        .await
    }

    pub async fn enable_plugin(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "enable"],
            None,
            &[],
        )
        .await
    }

    pub async fn plugin_env(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins", name, "env"],
            None,
            &[],
        )
        .await
    }

    pub async fn install_plugin_deps(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "install-deps"],
            None,
            &[],
        )
        .await
    }

    pub async fn lint_plugin(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins", name, "lint"],
            None,
            &[],
        )
        .await
    }

    pub async fn prewarm_plugin(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "prewarm"],
            None,
            &[],
        )
        .await
    }

    pub async fn reload_plugin(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "reload"],
            None,
            &[],
        )
        .await
    }

    pub async fn sign_plugin(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "sign"],
            None,
            &[],
        )
        .await
    }

    pub async fn plugin_status(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "plugins", name, "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn test_plugin_hook(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "test-hook"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn upgrade_plugin(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "plugins", name, "upgrade"],
            Some(data),
            &[],
        )
        .await
    }
}

// ── ProactiveMemory ──

#[derive(Debug, Clone)]
pub struct ProactiveMemoryResource {
    base_url: String,
    client: Client,
}

impl ProactiveMemoryResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn memory_list(
        &self,
        category: Option<&str>,
        level: Option<&str>,
        offset: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory"],
            None,
            &[
                ("category", category),
                ("level", level),
                ("offset", offset),
                ("limit", limit),
            ],
        )
        .await
    }

    pub async fn memory_add(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "memory"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn memory_list_agent(
        &self,
        id: &str,
        category: Option<&str>,
        level: Option<&str>,
        offset: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id],
            None,
            &[
                ("category", category),
                ("level", level),
                ("offset", offset),
                ("limit", limit),
            ],
        )
        .await
    }

    pub async fn memory_reset_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "memory", "agents", id],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_consolidate(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "memory", "agents", id, "consolidate"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_count_agent(&self, id: &str, level: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "count"],
            None,
            &[("level", level)],
        )
        .await
    }

    pub async fn memory_duplicates(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "duplicates"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_export_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "export"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_import_agent(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "memory", "agents", id, "import"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn memory_clear_level(&self, id: &str, level: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "memory", "agents", id, "level", level],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_query_relations(
        &self,
        id: &str,
        source: Option<&str>,
        relation: Option<&str>,
        target: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "relations"],
            None,
            &[
                ("source", source),
                ("relation", relation),
                ("target", target),
            ],
        )
        .await
    }

    pub async fn memory_store_relations(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "memory", "agents", id, "relations"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn memory_search_agent(
        &self,
        id: &str,
        q: Option<&str>,
        level: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "search"],
            None,
            &[("q", q), ("level", level), ("limit", limit)],
        )
        .await
    }

    pub async fn memory_stats_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "agents", id, "stats"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_bulk_delete(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "memory", "bulk-delete"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn memory_cleanup(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "memory", "cleanup"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_decay(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "memory", "decay"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_update(&self, memory_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "memory", "items", memory_id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn memory_delete(&self, memory_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "memory", "items", memory_id],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_history(&self, memory_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "items", memory_id, "history"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_search(
        &self,
        q: Option<&str>,
        level: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "search"],
            None,
            &[("q", q), ("level", level), ("limit", limit)],
        )
        .await
    }

    pub async fn memory_stats(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "stats"],
            None,
            &[],
        )
        .await
    }

    pub async fn memory_get_user(&self, user_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "memory", "user", user_id],
            None,
            &[],
        )
        .await
    }
}

// ── Sessions ──

#[derive(Debug, Clone)]
pub struct SessionsResource {
    base_url: String,
    client: Client,
}

impl SessionsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn find_session_by_label(&self, id: &str, label: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "agents", id, "sessions", "by-label", label],
            None,
            &[],
        )
        .await
    }

    pub async fn list_sessions(&self, limit: Option<&str>, offset: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "sessions"],
            None,
            &[("limit", limit), ("offset", offset)],
        )
        .await
    }

    pub async fn session_cleanup(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "sessions", "cleanup"],
            None,
            &[],
        )
        .await
    }

    pub async fn search_sessions(
        &self,
        q: Option<&str>,
        agent_id: Option<&str>,
        limit: Option<&str>,
        offset: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "sessions", "search"],
            None,
            &[
                ("q", q),
                ("agent_id", agent_id),
                ("limit", limit),
                ("offset", offset),
            ],
        )
        .await
    }

    pub async fn get_session(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "sessions", id],
            None,
            &[],
        )
        .await
    }

    pub async fn delete_session(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "sessions", id],
            None,
            &[],
        )
        .await
    }

    pub async fn set_session_label(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "sessions", id, "label"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn patch_session_model(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "sessions", id, "model"],
            Some(data),
            &[],
        )
        .await
    }
}

// ── Skills ──

#[derive(Debug, Clone)]
pub struct SkillsResource {
    base_url: String,
    client: Client,
}

impl SkillsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn clawhub_browse(&self, q: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "clawhub", "browse"],
            None,
            &[("q", q)],
        )
        .await
    }

    pub async fn clawhub_install(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "clawhub", "install"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn clawhub_search(&self, q: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "clawhub", "search"],
            None,
            &[("q", q)],
        )
        .await
    }

    pub async fn clawhub_skill_detail(&self, slug: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "clawhub", "skill", slug],
            None,
            &[],
        )
        .await
    }

    pub async fn clawhub_skill_code(&self, slug: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "clawhub", "skill", slug, "code"],
            None,
            &[],
        )
        .await
    }

    pub async fn marketplace_search(&self, q: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "marketplace", "search"],
            None,
            &[("q", q)],
        )
        .await
    }

    pub async fn list_skills(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "skills"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_skill(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", "create"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn install_skill(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", "install"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_pending_candidates(&self, agent: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "skills", "pending"],
            None,
            &[("agent", agent)],
        )
        .await
    }

    pub async fn show_pending_candidate(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "skills", "pending", id],
            None,
            &[],
        )
        .await
    }

    pub async fn approve_pending_candidate(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", "pending", id, "approve"],
            None,
            &[],
        )
        .await
    }

    pub async fn propose_pending_to_registry(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", "pending", id, "propose-to-registry"],
            None,
            &[],
        )
        .await
    }

    pub async fn reject_pending_candidate(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", "pending", id, "reject"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_skill_registry(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "skills", "registry"],
            None,
            &[],
        )
        .await
    }

    pub async fn reload_skills(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", "reload"],
            None,
            &[],
        )
        .await
    }

    pub async fn uninstall_skill(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", "uninstall"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_skill_detail(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "skills", name],
            None,
            &[],
        )
        .await
    }

    pub async fn evolve_delete_skill(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", name, "evolve", "delete"],
            None,
            &[],
        )
        .await
    }

    pub async fn evolve_write_file(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", name, "evolve", "file"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn evolve_remove_file(&self, name: &str, path: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "skills", name, "evolve", "file"],
            None,
            &[("path", path)],
        )
        .await
    }

    pub async fn evolve_patch_skill(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", name, "evolve", "patch"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn evolve_rollback_skill(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", name, "evolve", "rollback"],
            None,
            &[],
        )
        .await
    }

    pub async fn evolve_update_skill(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", name, "evolve", "update"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_supporting_file(&self, name: &str, path: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "skills", name, "file"],
            None,
            &[("path", path)],
        )
        .await
    }

    pub async fn propose_skill_to_registry(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "skills", name, "propose"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_tools(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "tools"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_tool(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "tools", name],
            None,
            &[],
        )
        .await
    }
}

// ── System ──

#[derive(Debug, Clone)]
pub struct SystemResource {
    base_url: String,
    client: Client,
}

impl SystemResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn audit_export(
        &self,
        format: Option<&str>,
        user: Option<&str>,
        action: Option<&str>,
        agent: Option<&str>,
        channel: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "audit", "export"],
            None,
            &[
                ("format", format),
                ("user", user),
                ("action", action),
                ("agent", agent),
                ("channel", channel),
                ("from", from),
                ("to", to),
                ("limit", limit),
            ],
        )
        .await
    }

    pub async fn audit_query(
        &self,
        user: Option<&str>,
        action: Option<&str>,
        agent: Option<&str>,
        channel: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "audit", "query"],
            None,
            &[
                ("user", user),
                ("action", action),
                ("agent", agent),
                ("channel", channel),
                ("from", from),
                ("to", to),
                ("limit", limit),
            ],
        )
        .await
    }

    pub async fn audit_recent(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "audit", "recent"],
            None,
            &[],
        )
        .await
    }

    pub async fn audit_verify(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "audit", "verify"],
            None,
            &[],
        )
        .await
    }

    pub async fn check(
        &self,
        user: Option<&str>,
        action: Option<&str>,
        channel: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "authz", "check"],
            None,
            &[("user", user), ("action", action), ("channel", channel)],
        )
        .await
    }

    pub async fn effective_permissions(&self, user_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "authz", "effective", user_id],
            None,
            &[],
        )
        .await
    }

    pub async fn whoami(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "authz", "whoami"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_backup(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "backup"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_backups(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "backups"],
            None,
            &[],
        )
        .await
    }

    pub async fn delete_backup(&self, filename: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "backups", filename],
            None,
            &[],
        )
        .await
    }

    pub async fn list_bindings(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "bindings"],
            None,
            &[],
        )
        .await
    }

    pub async fn add_binding(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "bindings"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn remove_binding(&self, index: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "bindings", index],
            None,
            &[],
        )
        .await
    }

    pub async fn list_commands(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "commands"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_command(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "commands", name],
            None,
            &[],
        )
        .await
    }

    pub async fn get_config(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "config"],
            None,
            &[],
        )
        .await
    }

    pub async fn export_config(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "config", "export"],
            None,
            &[],
        )
        .await
    }

    pub async fn config_reload(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "config", "reload"],
            None,
            &[],
        )
        .await
    }

    pub async fn config_schema(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "config", "schema"],
            None,
            &[],
        )
        .await
    }

    pub async fn config_set(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "config", "set"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn config_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "config", "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn health(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "health"],
            None,
            &[],
        )
        .await
    }

    pub async fn health_detail(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "health", "detail"],
            None,
            &[],
        )
        .await
    }

    pub async fn quick_init(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "init"],
            None,
            &[],
        )
        .await
    }

    pub fn logs_stream(&self) -> tokio::sync::mpsc::Receiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            vec!["api".to_string(), "logs".to_string(), "stream".to_string()],
            reqwest::Method::GET,
            None,
            Vec::new(),
        )
    }

    pub async fn prometheus_metrics(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "metrics"],
            None,
            &[],
        )
        .await
    }

    pub async fn run_migrate(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "migrate"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn migrate_detect(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "migrate", "detect"],
            None,
            &[],
        )
        .await
    }

    pub async fn migrate_scan(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "migrate", "scan"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_profiles(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "profiles"],
            None,
            &[],
        )
        .await
    }

    pub async fn get_profile(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "profiles", name],
            None,
            &[],
        )
        .await
    }

    pub async fn provisioning_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "provisioning", "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn queue_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "queue", "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn ready(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "ready"],
            None,
            &[],
        )
        .await
    }

    pub async fn restore_backup(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "restore"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn security_status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "security"],
            None,
            &[],
        )
        .await
    }

    pub async fn shutdown(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "shutdown"],
            None,
            &[],
        )
        .await
    }

    pub async fn status(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_agent_templates(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "templates"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_agent_type(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "templates"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_agent_template(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "templates", name],
            None,
            &[],
        )
        .await
    }

    pub async fn update_agent_type(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "templates", name],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_agent_type(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "templates", name],
            None,
            &[],
        )
        .await
    }

    pub async fn get_agent_template_toml(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "templates", name, "toml"],
            None,
            &[],
        )
        .await
    }

    pub async fn version(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "version"],
            None,
            &[],
        )
        .await
    }

    pub async fn api_versions(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "versions"],
            None,
            &[],
        )
        .await
    }
}

// ── Tools ──

#[derive(Debug, Clone)]
pub struct ToolsResource {
    base_url: String,
    client: Client,
}

impl ToolsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn invoke_tool(
        &self,
        name: &str,
        data: Value,
        agent_id: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "tools", name, "invoke"],
            Some(data),
            &[("agent_id", agent_id)],
        )
        .await
    }
}

// ── Users ──

#[derive(Debug, Clone)]
pub struct UsersResource {
    base_url: String,
    client: Client,
}

impl UsersResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_users(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "users"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_user(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "users"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn import_users(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "users", "import"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_user(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "users", name],
            None,
            &[],
        )
        .await
    }

    pub async fn update_user(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "users", name],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_user(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "users", name],
            None,
            &[],
        )
        .await
    }

    pub async fn get_user_policy(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "users", name, "policy"],
            None,
            &[],
        )
        .await
    }

    pub async fn update_user_policy(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "users", name, "policy"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_user_provider_keys(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "users", name, "provider-keys"],
            None,
            &[],
        )
        .await
    }

    pub async fn set_user_provider_key(
        &self,
        name: &str,
        provider: &str,
        data: Value,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "users", name, "provider-keys", provider],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_user_provider_key(&self, name: &str, provider: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "users", name, "provider-keys", provider],
            None,
            &[],
        )
        .await
    }

    pub async fn rotate_user_key(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "users", name, "rotate-key"],
            None,
            &[],
        )
        .await
    }
}

// ── Webhooks ──

#[derive(Debug, Clone)]
pub struct WebhooksResource {
    base_url: String,
    client: Client,
}

impl WebhooksResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn webhook_agent(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hooks", "agent"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn webhook_wake(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "hooks", "wake"],
            Some(data),
            &[],
        )
        .await
    }
}

// ── Workflows ──

#[derive(Debug, Clone)]
pub struct WorkflowsResource {
    base_url: String,
    client: Client,
}

impl WorkflowsResource {
    fn new(base_url: String, client: Client) -> Self {
        Self { base_url, client }
    }

    pub async fn list_cron_jobs(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "cron", "jobs"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_cron_job(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "cron", "jobs"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_cron_job(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "cron", "jobs", id],
            None,
            &[],
        )
        .await
    }

    pub async fn update_cron_job(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "cron", "jobs", id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_cron_job(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "cron", "jobs", id],
            None,
            &[],
        )
        .await
    }

    pub async fn toggle_cron_job(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "cron", "jobs", id, "enable"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn cron_job_status(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "cron", "jobs", id, "status"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_schedules(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "schedules"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_schedule(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "schedules"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_schedule(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "schedules", id],
            None,
            &[],
        )
        .await
    }

    pub async fn update_schedule(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "schedules", id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_schedule(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "schedules", id],
            None,
            &[],
        )
        .await
    }

    pub async fn run_schedule(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "schedules", id, "run"],
            None,
            &[],
        )
        .await
    }

    pub async fn list_triggers(&self, agent_id: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "triggers"],
            None,
            &[("agent_id", agent_id)],
        )
        .await
    }

    pub async fn create_trigger(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "triggers"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_trigger(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "triggers", id],
            None,
            &[],
        )
        .await
    }

    pub async fn delete_trigger(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "triggers", id],
            None,
            &[],
        )
        .await
    }

    pub async fn update_trigger(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PATCH,
            &["api", "triggers", id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_workflow_templates(
        &self,
        q: Option<&str>,
        category: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "workflow-templates"],
            None,
            &[("q", q), ("category", category)],
        )
        .await
    }

    pub async fn get_workflow_template(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "workflow-templates", id],
            None,
            &[],
        )
        .await
    }

    pub async fn instantiate_template(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflow-templates", id, "instantiate"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_workflows(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "workflows"],
            None,
            &[],
        )
        .await
    }

    pub async fn create_workflow(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_workflow_run(&self, run_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "workflows", "runs", run_id],
            None,
            &[],
        )
        .await
    }

    pub async fn cancel_workflow_run(&self, run_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", "runs", run_id, "cancel"],
            None,
            &[],
        )
        .await
    }

    pub async fn operator_action_workflow_run(&self, run_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", "runs", run_id, "operator"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn pause_workflow_run(&self, run_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", "runs", run_id, "pause"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn rerun_workflow_run(&self, run_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", "runs", run_id, "rerun"],
            None,
            &[],
        )
        .await
    }

    pub async fn resume_workflow_run(&self, run_id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", "runs", run_id, "resume"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_workflow(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "workflows", id],
            None,
            &[],
        )
        .await
    }

    pub async fn update_workflow(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &["api", "workflows", id],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_workflow(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &["api", "workflows", id],
            None,
            &[],
        )
        .await
    }

    pub async fn dry_run_workflow(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", id, "dry-run"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn run_workflow(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", id, "run"],
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_workflow_runs(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &["api", "workflows", id, "runs"],
            None,
            &[],
        )
        .await
    }

    pub async fn save_workflow_as_template(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &["api", "workflows", id, "save-as-template"],
            None,
            &[],
        )
        .await
    }
}
