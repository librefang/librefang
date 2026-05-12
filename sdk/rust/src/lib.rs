//! LibreFang Rust SDK — AUTO-GENERATED from openapi.json.
//! Do not edit manually. Run: python3 scripts/codegen-sdks.py
//!
//! # Usage
//!
//! ```rust,no_run
//! use librefang::LibreFang;
//!
//! #[tokio::main]
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
use std::sync::Arc;
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

async fn do_req(
    client: &Client,
    base_url: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
    query: &[(&str, Option<&str>)],
) -> Result<Value> {
    let url = format!("{}{}", base_url, path);
    let req = client.request(method, &url);
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
    path: String,
    method: reqwest::Method,
    body: Option<Value>,
    query: Vec<(String, Option<String>)>,
) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let url = format!("{}{}", base_url, path);
        let req = client
            .request(method, &url)
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
        let res = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(serde_json::json!({
                    "error": e.to_string(),
                    "status": 0,
                }));
                return;
            }
        };
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let body = res.text().await.unwrap_or_default();
            let _ = tx.send(serde_json::json!({
                "error": format!("HTTP {}: {}", status, body),
                "status": status,
            }));
            return;
        }
        // Accumulate raw bytes so multi-byte UTF-8 codepoints are not split
        // by chunk boundaries (from_utf8_lossy on individual chunks corrupts
        // non-ASCII content). Split on newline, decode each complete line.
        // MAX_SSE_LINE caps memory on misbehaving streams.
        const MAX_SSE_LINE: usize = 8 * 1024 * 1024;
        let mut stream = res.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        while let Some(Ok(chunk)) = stream.next().await {
            buffer.extend_from_slice(&chunk);
            if buffer.len() > MAX_SSE_LINE {
                let _ = tx.send(serde_json::json!({
                    "error": format!("SSE line exceeded {} bytes", MAX_SSE_LINE),
                    "status": 0,
                }));
                return;
            }
            while let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buffer.drain(..=pos).collect();
                let line = match std::str::from_utf8(&line_bytes) {
                    Ok(s) => s.trim(),
                    Err(e) => {
                        let _ = tx.send(serde_json::json!({
                            "error": format!("invalid utf-8 in SSE line at byte {}", e.valid_up_to()),
                            "status": 0,
                        }));
                        continue;
                    }
                };
                if let Some(data) = line.strip_prefix("data: ") {
                    if data == "[DONE]" {
                        return;
                    }
                    match serde_json::from_str::<Value>(data) {
                        Ok(v) => {
                            let _ = tx.send(v);
                        }
                        Err(_) => {
                            let _ = tx.send(serde_json::json!({"raw": data}));
                        }
                    }
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
    pub hands: Arc<HandsResource>,
    pub mcp: Arc<McpResource>,
    pub memory: Arc<MemoryResource>,
    pub models: Arc<ModelsResource>,
    pub network: Arc<NetworkResource>,
    pub pairing: Arc<PairingResource>,
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
        let base_url = base_url.into().trim_end_matches('/').to_string();
        let client = Client::new();
        Self {
            a2a: Arc::new(A2AResource::new(base_url.clone(), client.clone())),
            agents: Arc::new(AgentsResource::new(base_url.clone(), client.clone())),
            approvals: Arc::new(ApprovalsResource::new(base_url.clone(), client.clone())),
            auth: Arc::new(AuthResource::new(base_url.clone(), client.clone())),
            auto_dream: Arc::new(AutoDreamResource::new(base_url.clone(), client.clone())),
            budget: Arc::new(BudgetResource::new(base_url.clone(), client.clone())),
            channels: Arc::new(ChannelsResource::new(base_url.clone(), client.clone())),
            extensions: Arc::new(ExtensionsResource::new(base_url.clone(), client.clone())),
            hands: Arc::new(HandsResource::new(base_url.clone(), client.clone())),
            mcp: Arc::new(McpResource::new(base_url.clone(), client.clone())),
            memory: Arc::new(MemoryResource::new(base_url.clone(), client.clone())),
            models: Arc::new(ModelsResource::new(base_url.clone(), client.clone())),
            network: Arc::new(NetworkResource::new(base_url.clone(), client.clone())),
            pairing: Arc::new(PairingResource::new(base_url.clone(), client.clone())),
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
            &"/api/a2a/agents".to_string(),
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
            &format!("/api/a2a/agents/{}", id),
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
            &"/api/a2a/discover".to_string(),
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
            &"/api/a2a/send".to_string(),
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
            &format!("/api/a2a/tasks/{}/status", id),
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
            &"/api/agents".to_string(),
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
            &"/api/agents".to_string(),
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
            &"/api/agents/bulk".to_string(),
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
            &"/api/agents/bulk".to_string(),
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
            &"/api/agents/bulk/start".to_string(),
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
            &"/api/agents/bulk/stop".to_string(),
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
            &"/api/agents/identities".to_string(),
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
            &format!("/api/agents/identities/{}/reset", name),
            None,
            &[("confirm", confirm)],
        )
        .await
    }

    pub async fn get_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/agents/{}", id),
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
            &format!("/api/agents/{}", id),
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
            &format!("/api/agents/{}", id),
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
            &format!("/api/agents/{}/clone", id),
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
            &format!("/api/agents/{}/config", id),
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
            &format!("/api/agents/{}/deliveries", id),
            None,
            &[],
        )
        .await
    }

    pub async fn list_agent_events(&self, id: &str, limit: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/agents/{}/events", id),
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
            &format!("/api/agents/{}/files", id),
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
            &format!("/api/agents/{}/files/{}", id, filename),
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
            &format!("/api/agents/{}/files/{}", id, filename),
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
            &format!("/api/agents/{}/files/{}", id, filename),
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
            &format!("/api/agents/{}/hand-runtime-config", id),
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
            &format!("/api/agents/{}/hand-runtime-config", id),
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
            &format!("/api/agents/{}/history", id),
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
            &format!("/api/agents/{}/identity", id),
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
            &format!("/api/agents/{}/inject", id),
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
            &format!("/api/agents/{}/logs", id),
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
            &format!("/api/agents/{}/mcp_servers", id),
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
            &format!("/api/agents/{}/mcp_servers", id),
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
            &format!("/api/agents/{}/message", id),
            Some(data),
            &[],
        )
        .await
    }

    pub fn send_message_stream(
        &self,
        id: &str,
        data: Value,
    ) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            format!("/api/agents/{}/message/stream", id),
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
            &format!("/api/agents/{}/metrics", id),
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
            &format!("/api/agents/{}/mode", id),
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
            &format!("/api/agents/{}/model", id),
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
            &format!("/api/agents/{}/push", id),
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
            &format!("/api/agents/{}/reload", id),
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
            &format!("/api/agents/{}/resume", id),
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
            &format!("/api/agents/{}/runtime", id),
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
            &format!("/api/agents/{}/session", id),
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
            &format!("/api/agents/{}/session/compact", id),
            None,
            &[],
        )
        .await
    }

    pub async fn reboot_session(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &format!("/api/agents/{}/session/reboot", id),
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
            &format!("/api/agents/{}/session/reset", id),
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
            &format!("/api/agents/{}/sessions", id),
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
            &format!("/api/agents/{}/sessions", id),
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
            &format!("/api/agents/{}/sessions/import", id),
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
            &format!("/api/agents/{}/sessions/{}/export", id, session_id),
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
            &format!("/api/agents/{}/sessions/{}/stop", id, session_id),
            None,
            &[],
        )
        .await
    }

    pub fn attach_session_stream(
        &self,
        id: &str,
        session_id: &str,
    ) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            format!("/api/agents/{}/sessions/{}/stream", id, session_id),
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
            &format!("/api/agents/{}/sessions/{}/switch", id, session_id),
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
            &format!("/api/agents/{}/sessions/{}/trajectory", id, session_id),
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
            &format!("/api/agents/{}/skills", id),
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
            &format!("/api/agents/{}/skills", id),
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
            &format!("/api/agents/{}/stats", id),
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
            &format!("/api/agents/{}/stop", id),
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
            &format!("/api/agents/{}/suspend", id),
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
            &format!("/api/agents/{}/tools", id),
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
            &format!("/api/agents/{}/tools", id),
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
            &format!("/api/agents/{}/traces", id),
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
            &format!("/api/agents/{}/upload", id),
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
            &format!("/api/uploads/{}", file_id),
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
            &"/api/approvals".to_string(),
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
            &"/api/approvals".to_string(),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn get_approval(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/approvals/{}", id),
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
            &format!("/api/approvals/{}/approve", id),
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
            &format!("/api/approvals/{}/reject", id),
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
            &"/api/auth/callback".to_string(),
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
            &"/api/auth/callback".to_string(),
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
            &"/api/auth/change-password".to_string(),
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
            &"/api/auth/dashboard-check".to_string(),
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
            &"/api/auth/dashboard-login".to_string(),
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
            &"/api/auth/introspect".to_string(),
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
            &"/api/auth/login".to_string(),
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
            &format!("/api/auth/login/{}", provider),
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
            &"/api/auth/logout".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn auth_providers(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/auth/providers".to_string(),
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
            &"/api/auth/refresh".to_string(),
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
            &"/api/auth/userinfo".to_string(),
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
            &format!("/api/auto-dream/agents/{}/abort", id),
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
            &format!("/api/auto-dream/agents/{}/enabled", id),
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
            &format!("/api/auto-dream/agents/{}/trigger", id),
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
            &"/api/auto-dream/status".to_string(),
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
            &"/api/budget".to_string(),
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
            &"/api/budget".to_string(),
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
            &"/api/budget/agents".to_string(),
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
            &format!("/api/budget/agents/{}", id),
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
            &format!("/api/budget/agents/{}", id),
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
            &"/api/budget/users".to_string(),
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
            &format!("/api/budget/users/{}", user_id),
            None,
            &[],
        )
        .await
    }

    pub async fn usage_stats(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/usage".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn usage_by_model(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/usage/by-model".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn usage_daily(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/usage/daily".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn usage_summary(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/usage/summary".to_string(),
            None,
            &[],
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
            &"/api/channels".to_string(),
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
            &"/api/channels/reload".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn wechat_qr_start(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &"/api/channels/wechat/qr/start".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn wechat_qr_status(&self, qr_code: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/channels/wechat/qr/status".to_string(),
            None,
            &[("qr_code", qr_code)],
        )
        .await
    }

    pub async fn whatsapp_qr_start(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &"/api/channels/whatsapp/qr/start".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn whatsapp_qr_status(&self, session_id: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/channels/whatsapp/qr/status".to_string(),
            None,
            &[("session_id", session_id)],
        )
        .await
    }

    pub async fn configure_channel(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &format!("/api/channels/{}/configure", name),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn remove_channel(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &format!("/api/channels/{}/configure", name),
            None,
            &[],
        )
        .await
    }

    pub async fn list_channel_instances(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/channels/{}/instances", name),
            None,
            &[],
        )
        .await
    }

    pub async fn create_channel_instance(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &format!("/api/channels/{}/instances", name),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn update_channel_instance_handler(
        &self,
        name: &str,
        index: &str,
        data: Value,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &format!("/api/channels/{}/instances/{}", name, index),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn delete_channel_instance(
        &self,
        name: &str,
        index: &str,
        signature: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &format!("/api/channels/{}/instances/{}", name, index),
            None,
            &[("signature", signature)],
        )
        .await
    }

    pub async fn test_channel(&self, name: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &format!("/api/channels/{}/test", name),
            Some(data),
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
            &"/api/extensions".to_string(),
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
            &"/api/extensions/install".to_string(),
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
            &"/api/extensions/uninstall".to_string(),
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
            &format!("/api/extensions/{}", name),
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
            &"/api/hands".to_string(),
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
            &"/api/hands/active".to_string(),
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
            &"/api/hands/install".to_string(),
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
            &format!("/api/hands/instances/{}", id),
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
            &format!("/api/hands/instances/{}/browser", id),
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
            &format!("/api/hands/instances/{}/pause", id),
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
            &format!("/api/hands/instances/{}/resume", id),
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
            &format!("/api/hands/instances/{}/stats", id),
            None,
            &[],
        )
        .await
    }

    pub async fn reload_hands(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &"/api/hands/reload".to_string(),
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
            &format!("/api/hands/{}", hand_id),
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
            &format!("/api/hands/{}/activate", hand_id),
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
            &format!("/api/hands/{}/check-deps", hand_id),
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
            &format!("/api/hands/{}/install-deps", hand_id),
            None,
            &[],
        )
        .await
    }

    pub async fn get_hand_settings(&self, hand_id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/hands/{}/settings", hand_id),
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
            &format!("/api/hands/{}/settings", hand_id),
            Some(data),
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
            &"/api/mcp/catalog".to_string(),
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
            &format!("/api/mcp/catalog/{}", id),
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
            &"/api/mcp/health".to_string(),
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
            &"/api/mcp/reload".to_string(),
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
            &"/api/mcp/servers".to_string(),
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
            &"/api/mcp/servers".to_string(),
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
            &format!("/api/mcp/servers/{}", name),
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
            &format!("/api/mcp/servers/{}", name),
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
            &format!("/api/mcp/servers/{}", name),
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
            &format!("/api/mcp/servers/{}/reconnect", name),
            None,
            &[],
        )
        .await
    }

    pub async fn list_mcp_taint_rules(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/mcp/taint-rules".to_string(),
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
            &format!("/api/agents/{}/memory/export", id),
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
            &format!("/api/agents/{}/memory/import", id),
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
            &format!("/api/memory/agents/{}/kv", id),
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
            &format!("/api/memory/agents/{}/kv/{}", id, key),
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
            &format!("/api/memory/agents/{}/kv/{}", id, key),
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
            &format!("/api/memory/agents/{}/kv/{}", id, key),
            None,
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
            &"/api/catalog/status".to_string(),
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
            &"/api/catalog/update".to_string(),
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
            &"/api/models".to_string(),
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
            &"/api/models/aliases".to_string(),
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
            &"/api/models/aliases".to_string(),
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
            &format!("/api/models/aliases/{}", alias),
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
            &"/api/models/custom".to_string(),
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
            &format!("/api/models/custom/{}", id),
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
            &format!("/api/models/{}", id),
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
            &"/api/providers".to_string(),
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
            &format!("/api/providers/github-copilot/oauth/poll/{}", poll_id),
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
            &"/api/providers/github-copilot/oauth/start".to_string(),
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
            &format!("/api/providers/{}", name),
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
            &format!("/api/providers/{}/default", name),
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
            &format!("/api/providers/{}/enable", name),
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
            &format!("/api/providers/{}/key", name),
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
            &format!("/api/providers/{}/key", name),
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
            &format!("/api/providers/{}/test", name),
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
            &format!("/api/providers/{}/url", name),
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
            &"/api/comms/events".to_string(),
            None,
            &[("limit", limit)],
        )
        .await
    }

    pub fn comms_events_stream(&self) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            "/api/comms/events/stream".to_string(),
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
            &"/api/comms/send".to_string(),
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
            &"/api/comms/task".to_string(),
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
            &"/api/comms/topology".to_string(),
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
            &"/api/network/status".to_string(),
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
            &"/api/peers".to_string(),
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
            &format!("/api/peers/{}", id),
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
            &"/api/pairing/complete".to_string(),
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
            &"/api/pairing/devices".to_string(),
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
            &format!("/api/pairing/devices/{}", id),
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
            &"/api/pairing/notify".to_string(),
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
            &"/api/pairing/request".to_string(),
            None,
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
        offset: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/memory".to_string(),
            None,
            &[("category", category), ("offset", offset), ("limit", limit)],
        )
        .await
    }

    pub async fn memory_add(&self, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &"/api/memory".to_string(),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn memory_list_agent(
        &self,
        id: &str,
        category: Option<&str>,
        offset: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/memory/agents/{}", id),
            None,
            &[("category", category), ("offset", offset), ("limit", limit)],
        )
        .await
    }

    pub async fn memory_reset_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::DELETE,
            &format!("/api/memory/agents/{}", id),
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
            &format!("/api/memory/agents/{}/consolidate", id),
            None,
            &[],
        )
        .await
    }

    pub async fn memory_duplicates(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/memory/agents/{}/duplicates", id),
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
            &format!("/api/memory/agents/{}/export", id),
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
            &format!("/api/memory/agents/{}/import", id),
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
            &format!("/api/memory/agents/{}/level/{}", id, level),
            None,
            &[],
        )
        .await
    }

    pub async fn memory_search_agent(
        &self,
        id: &str,
        q: Option<&str>,
        limit: Option<&str>,
    ) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/memory/agents/{}/search", id),
            None,
            &[("q", q), ("limit", limit)],
        )
        .await
    }

    pub async fn memory_stats_agent(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/memory/agents/{}/stats", id),
            None,
            &[],
        )
        .await
    }

    pub async fn memory_cleanup(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &"/api/memory/cleanup".to_string(),
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
            &format!("/api/memory/items/{}", memory_id),
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
            &format!("/api/memory/items/{}", memory_id),
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
            &format!("/api/memory/items/{}/history", memory_id),
            None,
            &[],
        )
        .await
    }

    pub async fn memory_search(&self, q: Option<&str>, limit: Option<&str>) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/memory/search".to_string(),
            None,
            &[("q", q), ("limit", limit)],
        )
        .await
    }

    pub async fn memory_stats(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/memory/stats".to_string(),
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
            &format!("/api/memory/user/{}", user_id),
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
            &format!("/api/agents/{}/sessions/by-label/{}", id, label),
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
            &"/api/sessions".to_string(),
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
            &"/api/sessions/cleanup".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn get_session(&self, id: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/sessions/{}", id),
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
            &format!("/api/sessions/{}", id),
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
            &format!("/api/sessions/{}/label", id),
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
            &format!("/api/sessions/{}/model", id),
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
            &"/api/clawhub/browse".to_string(),
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
            &"/api/clawhub/install".to_string(),
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
            &"/api/clawhub/search".to_string(),
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
            &format!("/api/clawhub/skill/{}", slug),
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
            &format!("/api/clawhub/skill/{}/code", slug),
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
            &"/api/marketplace/search".to_string(),
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
            &"/api/skills".to_string(),
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
            &"/api/skills/create".to_string(),
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
            &"/api/skills/install".to_string(),
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
            &"/api/skills/pending".to_string(),
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
            &format!("/api/skills/pending/{}", id),
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
            &format!("/api/skills/pending/{}/approve", id),
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
            &format!("/api/skills/pending/{}/reject", id),
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
            &"/api/skills/uninstall".to_string(),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn list_tools(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/tools".to_string(),
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
            &format!("/api/tools/{}", name),
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
            &"/api/audit/export".to_string(),
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
            &"/api/audit/query".to_string(),
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
            &"/api/audit/recent".to_string(),
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
            &"/api/audit/verify".to_string(),
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
            &"/api/backup".to_string(),
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
            &"/api/backups".to_string(),
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
            &format!("/api/backups/{}", filename),
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
            &"/api/bindings".to_string(),
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
            &"/api/bindings".to_string(),
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
            &format!("/api/bindings/{}", index),
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
            &"/api/commands".to_string(),
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
            &format!("/api/commands/{}", name),
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
            &"/api/config".to_string(),
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
            &"/api/config/reload".to_string(),
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
            &"/api/config/schema".to_string(),
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
            &"/api/config/set".to_string(),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn health(&self) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &"/api/health".to_string(),
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
            &"/api/health/detail".to_string(),
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
            &"/api/init".to_string(),
            None,
            &[],
        )
        .await
    }

    pub fn logs_stream(&self) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
        do_stream(
            self.client.clone(),
            self.base_url.clone(),
            "/api/logs/stream".to_string(),
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
            &"/api/metrics".to_string(),
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
            &"/api/migrate".to_string(),
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
            &"/api/migrate/detect".to_string(),
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
            &"/api/migrate/scan".to_string(),
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
            &"/api/profiles".to_string(),
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
            &format!("/api/profiles/{}", name),
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
            &"/api/queue/status".to_string(),
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
            &"/api/restore".to_string(),
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
            &"/api/security".to_string(),
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
            &"/api/shutdown".to_string(),
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
            &"/api/status".to_string(),
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
            &"/api/templates".to_string(),
            None,
            &[],
        )
        .await
    }

    pub async fn get_agent_template(&self, name: &str) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::GET,
            &format!("/api/templates/{}", name),
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
            &"/api/version".to_string(),
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
            &"/api/versions".to_string(),
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
            &format!("/api/tools/{}/invoke", name),
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
            &"/api/users".to_string(),
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
            &"/api/users".to_string(),
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
            &"/api/users/import".to_string(),
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
            &format!("/api/users/{}", name),
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
            &format!("/api/users/{}", name),
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
            &format!("/api/users/{}", name),
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
            &format!("/api/users/{}/rotate-key", name),
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
            &"/api/hooks/agent".to_string(),
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
            &"/api/hooks/wake".to_string(),
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
            &"/api/cron/jobs".to_string(),
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
            &"/api/cron/jobs".to_string(),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn update_cron_job(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &format!("/api/cron/jobs/{}", id),
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
            &format!("/api/cron/jobs/{}", id),
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
            &format!("/api/cron/jobs/{}/enable", id),
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
            &format!("/api/cron/jobs/{}/status", id),
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
            &"/api/schedules".to_string(),
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
            &"/api/schedules".to_string(),
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
            &format!("/api/schedules/{}", id),
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
            &format!("/api/schedules/{}", id),
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
            &format!("/api/schedules/{}", id),
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
            &format!("/api/schedules/{}/run", id),
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
            &"/api/triggers".to_string(),
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
            &"/api/triggers".to_string(),
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
            &format!("/api/triggers/{}", id),
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
            &format!("/api/triggers/{}", id),
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
            &format!("/api/triggers/{}", id),
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
            &"/api/workflows".to_string(),
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
            &"/api/workflows".to_string(),
            Some(data),
            &[],
        )
        .await
    }

    pub async fn update_workflow(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::PUT,
            &format!("/api/workflows/{}", id),
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
            &format!("/api/workflows/{}", id),
            None,
            &[],
        )
        .await
    }

    pub async fn run_workflow(&self, id: &str, data: Value) -> Result<Value> {
        do_req(
            &self.client,
            &self.base_url,
            reqwest::Method::POST,
            &format!("/api/workflows/{}/run", id),
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
            &format!("/api/workflows/{}/runs", id),
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
            &format!("/api/workflows/{}/save-as-template", id),
            None,
            &[],
        )
        .await
    }
}
