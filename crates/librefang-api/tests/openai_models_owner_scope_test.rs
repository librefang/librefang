//! Owner scoping for `GET /v1/models` (#6753).
//!
//! `GET /api/agents` forces `owner = caller` for roles below Admin so a User or Viewer key cannot enumerate other users' agents.
//! The OpenAI-compatible model list returns the same enumeration (`librefang:<agent name>`), so it must apply the same rule.
//! The request runs through the real auth middleware so `AuthenticatedApiUser` extraction is exercised end to end.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::middleware;
use librefang_api::routes::AppState;
use librefang_kernel::auth::UserRole as KernelUserRole;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::agent::{AgentManifest, UserId};
use librefang_types::config::UserConfig;
use std::sync::Arc;
use tower::ServiceExt;

const MASTER_KEY: &str = "models-scope-master-key";
const ALICE_KEY: &str = "alice-models-scope-key";
const VIEWER_KEY: &str = "viewer-models-scope-key";
const ADMIN_KEY: &str = "admin-models-scope-key";

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

async fn boot() -> Harness {
    let users = [
        ("Alice", "user", ALICE_KEY),
        ("Eve", "viewer", VIEWER_KEY),
        ("Admin", "admin", ADMIN_KEY),
    ];
    let mut configs = Vec::new();
    let mut auth_users = Vec::new();
    for (name, role, key) in users {
        let hash = librefang_api::password_hash::hash_password(key).expect("hash test key");
        configs.push(UserConfig {
            name: name.to_string(),
            role: role.to_string(),
            api_key_hash: Some(hash.clone()),
            ..Default::default()
        });
        auth_users.push(middleware::ApiUserAuth {
            name: name.to_string(),
            role: KernelUserRole::from_str_role(role),
            api_key_hash: hash,
            user_id: UserId::from_name(name),
        });
    }

    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.api_key = MASTER_KEY.to_string();
        cfg.users = configs;
    }))
    .with_api_key(MASTER_KEY)
    .with_user_api_keys(auth_users);
    let (state, tmp, _) = test.into_parts();
    state.kernel.clone().set_self_handle();

    let auth_state = middleware::AuthState {
        api_key_lock: state.api_key_lock.clone(),
        master_key: state.master_key.clone(),
        active_sessions: state.active_sessions.clone(),
        dashboard_auth_enabled: state.dashboard_auth_enabled.clone(),
        user_api_keys: state.user_api_keys.clone(),
        // Production turns this on whenever `[[users]]` are configured; without it a GET would never populate `AuthenticatedApiUser` and the scoping assertions would pass vacuously.
        require_auth_for_reads: true,
        allow_no_auth: false,
        audit_log: Some(state.kernel.audit().clone()),
    };
    let app = Router::new()
        .route(
            "/v1/models",
            axum::routing::get(librefang_api::openai_compat::list_models),
        )
        .layer(axum::middleware::from_fn_with_state(
            auth_state,
            middleware::auth,
        ))
        .with_state(state.clone());

    Harness {
        app,
        state,
        _tmp: tmp,
    }
}

fn spawn_authored(state: &AppState, author: &str) -> String {
    let name = format!("models-scope-{}", uuid::Uuid::new_v4());
    state
        .kernel
        .spawn_agent_typed(AgentManifest {
            name: name.clone(),
            source_template: None,
            author: author.to_string(),
            ..AgentManifest::default()
        })
        .expect("spawn authored test agent");
    format!("librefang:{name}")
}

async fn model_ids(h: &Harness, bearer: &str) -> Vec<String> {
    let resp = h
        .app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/models")
                .header("authorization", format!("Bearer {bearer}"))
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("route response");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 4 << 20)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(body["object"], "list");
    body["data"]
        .as_array()
        .expect("data array")
        .iter()
        .map(|m| m["id"].as_str().expect("model id").to_string())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn v1_models_is_owner_scoped_below_admin() {
    let h = boot().await;
    let alice_model = spawn_authored(&h.state, "Alice");
    let bob_model = spawn_authored(&h.state, "Bob");

    let alice_sees = model_ids(&h, ALICE_KEY).await;
    assert!(
        alice_sees.contains(&alice_model),
        "a user sees its own agent"
    );
    assert!(
        !alice_sees.contains(&bob_model),
        "a user must not enumerate another user's agent through /v1/models"
    );

    let viewer_sees = model_ids(&h, VIEWER_KEY).await;
    assert!(!viewer_sees.contains(&alice_model));
    assert!(!viewer_sees.contains(&bob_model));

    for key in [ADMIN_KEY, MASTER_KEY] {
        let sees = model_ids(&h, key).await;
        assert!(sees.contains(&alice_model) && sees.contains(&bob_model));
    }
}
