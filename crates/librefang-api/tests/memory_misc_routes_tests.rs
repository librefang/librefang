//! Integration tests for remaining API route gaps in memory and misc admin endpoints.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::collections::HashMap;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

impl Harness {
    async fn send(&self, req: Request<Body>) -> axum::response::Response {
        self.app.clone().oneshot(req).await.unwrap()
    }
}

async fn read_json(resp: axum::response::Response) -> serde_json::Value {
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

async fn start_mt_router_with_admin(admin_id: &str) -> Harness {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        multi_tenant: true,
        admin_accounts: vec![admin_id.to_string()],
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
        },
        ..KernelConfig::default()
    };

    let kernel = LibreFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let (app, state) = server::build_router(
        kernel,
        "127.0.0.1:0".parse().expect("listen addr should parse"),
    )
    .await;

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn json_request(
    method: axum::http::Method,
    uri: String,
    account_id: &str,
    body: serde_json::Value,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-account-id", account_id)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap()
}

fn empty_request(method: axum::http::Method, uri: String, account_id: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("x-account-id", account_id)
        .body(Body::empty())
        .unwrap()
}

const TEST_MANIFEST_TEMPLATE: &str = r#"
name = "{name}"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

async fn spawn_agent(h: &Harness, tenant: &str, name: &str) -> String {
    let resp = h
        .send(json_request(
            axum::http::Method::POST,
            "/api/agents".to_string(),
            tenant,
            serde_json::json!({
                "manifest_toml": TEST_MANIFEST_TEMPLATE.replace("{name}", name),
            }),
        ))
        .await;

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = read_json(resp).await;
    body["agent_id"].as_str().unwrap().to_string()
}

fn insert_test_memory(h: &Harness, agent_id_str: &str, tenant: &str, content: &str) -> String {
    use librefang_types::memory::MemorySource;

    let agent_id: librefang_types::agent::AgentId = agent_id_str.parse().unwrap();

    let mut metadata = HashMap::new();
    metadata.insert(
        "account_id".to_string(),
        serde_json::Value::String(tenant.to_string()),
    );

    h.state
        .kernel
        .memory_substrate()
        .remember_with_embedding(
            agent_id,
            content,
            MemorySource::UserProvided,
            "user",
            metadata,
            None,
        )
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn inbox_status_is_admin_only_and_reports_counts() {
    let h = start_mt_router_with_admin("tenant-admin").await;
    let inbox_dir = h.state.kernel.home_dir().join("inbox");
    let processed_dir = inbox_dir.join("processed");
    std::fs::create_dir_all(&processed_dir).unwrap();
    std::fs::write(inbox_dir.join("pending.txt"), "hello").unwrap();
    std::fs::write(processed_dir.join("done.txt"), "done").unwrap();

    let forbidden = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/inbox/status".to_string(),
            "tenant-a",
        ))
        .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let ok = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/inbox/status".to_string(),
            "tenant-admin",
        ))
        .await;
    assert_eq!(ok.status(), StatusCode::OK);
    let json = read_json(ok).await;
    assert_eq!(json["pending_count"], 1);
    assert_eq!(json["processed_count"], 1);
    assert_eq!(json["enabled"], false);
    assert!(json["directory"].as_str().unwrap().contains("inbox"));
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_detect_is_admin_only_and_returns_shape() {
    let h = start_mt_router_with_admin("tenant-admin").await;

    let forbidden = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/migrate/detect".to_string(),
            "tenant-a",
        ))
        .await;
    assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);

    let ok = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/migrate/detect".to_string(),
            "tenant-admin",
        ))
        .await;
    assert_eq!(ok.status(), StatusCode::OK);
    let json = read_json(ok).await;
    assert!(json.get("detected").is_some());
    assert!(json.get("source").is_some());
    assert!(json.get("path").is_some());
    assert!(json.get("scan").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_import_export_and_count_work_for_owner() {
    let h = start_mt_router_with_admin("tenant-admin").await;
    let agent_id = spawn_agent(&h, "tenant-a", "memory-agent").await;

    let import = h
        .send(json_request(
            axum::http::Method::POST,
            format!("/api/memory/agents/{agent_id}/import"),
            "tenant-a",
            serde_json::json!([
                {
                    "content": "tenant a user preference",
                    "level": "user",
                    "category": "user_preference",
                    "confidence": 0.9,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": null,
                    "metadata": {}
                },
                {
                    "content": "tenant a session context",
                    "level": "session",
                    "category": "task_context",
                    "confidence": 0.8,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": null,
                    "metadata": {}
                }
            ]),
        ))
        .await;
    assert_eq!(import.status(), StatusCode::OK);
    let import_json = read_json(import).await;
    assert_eq!(import_json["imported"], 2);

    let total_count = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/memory/agents/{agent_id}/count"),
            "tenant-a",
        ))
        .await;
    assert_eq!(total_count.status(), StatusCode::OK);
    assert_eq!(read_json(total_count).await["count"], 2);

    let user_count = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/memory/agents/{agent_id}/count?level=user"),
            "tenant-a",
        ))
        .await;
    assert_eq!(user_count.status(), StatusCode::OK);
    let user_count_json = read_json(user_count).await;
    assert_eq!(user_count_json["count"], 1);
    assert_eq!(user_count_json["level"], "user");

    let export = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/memory/agents/{agent_id}/export"),
            "tenant-a",
        ))
        .await;
    assert_eq!(export.status(), StatusCode::OK);
    let export_json = read_json(export).await;
    assert_eq!(export_json["count"], 2);
    let memories = export_json["memories"].as_array().unwrap();
    assert!(memories
        .iter()
        .any(|m| m["content"] == "tenant a user preference"));
    assert!(memories
        .iter()
        .any(|m| m["content"] == "tenant a session context"));
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_update_creates_history_entry() {
    let h = start_mt_router_with_admin("tenant-admin").await;
    let agent_id = spawn_agent(&h, "tenant-a", "history-agent").await;
    let memory_id = insert_test_memory(&h, &agent_id, "tenant-a", "original content");

    let update = h
        .send(json_request(
            axum::http::Method::PUT,
            format!("/api/memory/items/{memory_id}"),
            "tenant-a",
            serde_json::json!({ "content": "updated content" }),
        ))
        .await;
    assert_eq!(update.status(), StatusCode::OK);
    assert_eq!(read_json(update).await["updated"], true);

    let history = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/memory/items/{memory_id}/history"),
            "tenant-a",
        ))
        .await;
    assert_eq!(history.status(), StatusCode::OK);
    let history_json = read_json(history).await;
    assert_eq!(history_json["version_count"], 1);
    let versions = history_json["versions"].as_array().unwrap();
    assert_eq!(versions[0]["content"], "original content");
    assert!(versions[0].get("replaced_at").is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_history_denies_cross_tenant_access() {
    let h = start_mt_router_with_admin("tenant-admin").await;
    let agent_id = spawn_agent(&h, "tenant-a", "history-owner").await;
    let memory_id = insert_test_memory(&h, &agent_id, "tenant-a", "tenant-a only history");

    let history = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/memory/items/{memory_id}/history"),
            "tenant-b",
        ))
        .await;

    assert_eq!(
        history.status(),
        StatusCode::NOT_FOUND,
        "cross-tenant history lookup must return 404"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_import_overrides_malicious_account_id_in_request_payload() {
    let h = start_mt_router_with_admin("tenant-admin").await;
    let agent_id = spawn_agent(&h, "tenant-a", "import-owner").await;

    let import = h
        .send(json_request(
            axum::http::Method::POST,
            format!("/api/memory/agents/{agent_id}/import"),
            "tenant-a",
            serde_json::json!([
                {
                    "content": "tenant-a imported memory",
                    "level": "user",
                    "category": "malicious_override",
                    "confidence": 0.9,
                    "created_at": chrono::Utc::now().to_rfc3339(),
                    "updated_at": null,
                    "metadata": {
                        "account_id": "tenant-b"
                    }
                }
            ]),
        ))
        .await;
    assert_eq!(import.status(), StatusCode::OK);

    let tenant_a_search = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/memory/search?q=tenant-a+imported+memory&limit=10".to_string(),
            "tenant-a",
        ))
        .await;
    assert_eq!(tenant_a_search.status(), StatusCode::OK);
    let tenant_a_json = read_json(tenant_a_search).await;
    let tenant_a_items = tenant_a_json["memories"].as_array().unwrap();
    assert!(
        !tenant_a_items.is_empty(),
        "tenant-a should find the imported memory under its authenticated account"
    );

    let tenant_b_search = h
        .send(empty_request(
            axum::http::Method::GET,
            "/api/memory/search?q=tenant-a+imported+memory&limit=10".to_string(),
            "tenant-b",
        ))
        .await;
    assert_eq!(tenant_b_search.status(), StatusCode::OK);
    let tenant_b_json = read_json(tenant_b_search).await;
    let tenant_b_items = tenant_b_json["memories"].as_array().unwrap();
    assert!(
        tenant_b_items.is_empty(),
        "payload metadata.account_id must be overridden so tenant-b cannot see tenant-a's imported memory"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_duplicates_returns_groups_for_duplicate_content() {
    let h = start_mt_router_with_admin("tenant-admin").await;
    let agent_id = spawn_agent(&h, "tenant-a", "dup-agent").await;
    insert_test_memory(&h, &agent_id, "tenant-a", "same duplicate phrase");
    insert_test_memory(&h, &agent_id, "tenant-a", "same duplicate phrase");

    let duplicates = h
        .send(empty_request(
            axum::http::Method::GET,
            format!("/api/memory/agents/{agent_id}/duplicates"),
            "tenant-a",
        ))
        .await;
    assert_eq!(duplicates.status(), StatusCode::OK);
    let json = read_json(duplicates).await;
    assert!(json["duplicate_groups"].as_u64().unwrap() >= 1);
    let groups = json["groups"].as_array().unwrap();
    assert!(!groups.is_empty());
    assert!(groups[0].as_array().unwrap().len() >= 2);
}
