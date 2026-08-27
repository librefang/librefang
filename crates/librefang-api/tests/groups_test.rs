//! Integration tests for the user-group endpoints (#7745).
//!
//! Same harness as `users_test.rs`: a real axum router built from the production `routes::groups::router()`, a freshly-booted kernel, and a temp-dir `config.toml` on disk, so every write goes through `toml_edit` serialization, `validate_config_for_reload`, an atomic file write and a kernel reload before the assertion reads it back.
//! That is what makes these regression tests rather than tautologies — a mutation that fails to reach the file, or that produces TOML the kernel then refuses to reload, fails here even though the handler returned the right status.
//!
//! Every write endpoint is followed by an independent read (`GET /api/groups/{name}`, `GET /api/groups`, or `GET /api/users/{name}/groups`) that asserts the side effect, per the project's integration-test rule (refs #3721).
//! Router registration in `server.rs` is separately guarded by `dead_route_audit_test.rs`, which boots the whole production router and would report every path here as dead if the `.merge(routes::groups::router())` line were dropped.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{GroupConfig, UserConfig};
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

async fn boot() -> Harness {
    boot_with(vec![], vec![]).await
}

async fn boot_with(users: Vec<UserConfig>, groups: Vec<GroupConfig>) -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.default_model = librefang_types::config::DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        };
        cfg.users = users;
        cfg.groups = groups;
    }));

    let config_path = test.tmp_path().join("config.toml");
    let test = test.with_config_path(config_path);

    let state = test.state.clone();
    // Both routers are mounted because the user-delete cascade into group
    // membership is a cross-surface behaviour and has to be driven through the
    // real `DELETE /api/users/{name}` handler to be worth asserting.
    let app = Router::new()
        .nest("/api", routes::groups::router())
        .nest("/api", routes::users::router())
        .with_state(state.clone());

    Harness {
        app,
        state,
        _test: test,
    }
}

fn user(name: &str) -> UserConfig {
    UserConfig {
        name: name.to_string(),
        ..Default::default()
    }
}

async fn json_request(
    h: &Harness,
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(method).uri(path);
    let body_bytes = match body {
        Some(v) => {
            builder = builder.header("content-type", "application/json");
            serde_json::to_vec(&v).unwrap()
        }
        None => Vec::new(),
    };
    let req = builder.body(Body::from(body_bytes)).unwrap();
    let resp = h.app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let value: serde_json::Value = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, value)
}

/// Read the on-disk `config.toml` back as text. Used where the assertion is
/// about what was *persisted*, not about what the reloaded kernel reports.
async fn raw_config(h: &Harness) -> String {
    tokio::fs::read_to_string(h.state.kernel.config_path())
        .await
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_list_starts_empty() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_create_then_get_then_delete_round_trips() {
    let h = boot_with(vec![user("alice"), user("bob")], vec![]).await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/groups",
        Some(serde_json::json!({
            "name": "oncall",
            "description": "Support rota",
            "members": ["bob", "alice"],
            "roles": ["approver"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create: {body:?}");
    assert_eq!(body["name"], "oncall");
    assert_eq!(body["description"], "Support rota");
    // Stored sorted, so the response echoes the canonical order rather than the
    // order the client happened to send.
    assert_eq!(body["members"], serde_json::json!(["alice", "bob"]));
    assert_eq!(body["member_count"], 2);
    assert_eq!(body["unknown_members"], serde_json::json!([]));

    // Independent read — proves the write reached the file and survived the
    // kernel reload, not just that the handler built a response.
    let (status, body) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["roles"], serde_json::json!(["approver"]));

    let (status, body) = json_request(&h, Method::GET, "/api/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.as_array().unwrap().len(), 1);

    let (status, _) = json_request(&h, Method::DELETE, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_create_persists_array_of_tables_to_config_toml() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/groups",
        Some(serde_json::json!({ "name": "platform", "members": ["carol"] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);

    let raw = raw_config(&h).await;
    assert!(
        raw.contains("[[groups]]"),
        "expected a [[groups]] array-of-tables in config.toml, got:\n{raw}"
    );
    assert!(raw.contains("carol"), "membership missing from:\n{raw}");
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_create_rejects_duplicate() {
    let h = boot_with(
        vec![],
        vec![GroupConfig {
            name: "oncall".to_string(),
            ..Default::default()
        }],
    )
    .await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/groups",
        Some(serde_json::json!({ "name": "oncall" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_create_rejects_empty_name() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/groups",
        Some(serde_json::json!({ "name": "   " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("error envelope must carry a message: {body}"))
        .contains("must not be empty"));
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_create_normalizes_members_sorted_deduped_and_trimmed() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/groups",
        Some(serde_json::json!({
            "name": "team",
            "members": ["zoe", " alice ", "alice", "", "  ", "mallory"],
            "roles": ["b", "a", "a"],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    assert_eq!(
        body["members"],
        serde_json::json!(["alice", "mallory", "zoe"])
    );
    assert_eq!(body["roles"], serde_json::json!(["a", "b"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_create_flags_members_without_a_user_row() {
    // A member that does not resolve to a `[[users]]` entry is accepted, not
    // rejected — #7746 syncs membership from an IdP claim that can name someone
    // before they have ever authenticated here. The discrepancy is reported so
    // the dashboard can badge it.
    let h = boot_with(vec![user("alice")], vec![]).await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/groups",
        Some(serde_json::json!({ "name": "team", "members": ["alice", "ghost"] })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");
    assert_eq!(body["unknown_members"], serde_json::json!(["ghost"]));
    assert_eq!(body["member_count"], 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_update_renames_and_replaces_membership() {
    let h = boot_with(
        vec![user("alice"), user("bob")],
        vec![GroupConfig {
            name: "oncall".to_string(),
            description: "old".to_string(),
            members: vec!["alice".to_string()],
            roles: vec!["approver".to_string()],
        }],
    )
    .await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/groups/oncall",
        Some(serde_json::json!({
            "name": "support",
            "description": "new",
            "members": ["bob"],
            "roles": [],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "support");

    let (status, body) = json_request(&h, Method::GET, "/api/groups/support", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["description"], "new");
    assert_eq!(body["members"], serde_json::json!(["bob"]));
    assert_eq!(body["roles"], serde_json::json!([]));

    let (status, _) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "old name must be gone");
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_update_rename_onto_existing_name_conflicts() {
    let h = boot_with(
        vec![],
        vec![
            GroupConfig {
                name: "a".to_string(),
                ..Default::default()
            },
            GroupConfig {
                name: "b".to_string(),
                ..Default::default()
            },
        ],
    )
    .await;
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/groups/a",
        Some(serde_json::json!({ "name": "b" })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_update_unknown_returns_404() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::PUT,
        "/api/groups/nope",
        Some(serde_json::json!({ "name": "nope" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_member_add_and_remove_round_trip() {
    let h = boot_with(
        vec![user("alice"), user("bob")],
        vec![GroupConfig {
            name: "oncall".to_string(),
            ..Default::default()
        }],
    )
    .await;

    let (status, body) =
        json_request(&h, Method::PUT, "/api/groups/oncall/members/bob", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["members"], serde_json::json!(["bob"]));

    let (status, body) =
        json_request(&h, Method::PUT, "/api/groups/oncall/members/alice", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    // Sorted invariant re-established after the append.
    assert_eq!(body["members"], serde_json::json!(["alice", "bob"]));

    // Independent read.
    let (status, body) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["member_count"], 2);

    let (status, body) =
        json_request(&h, Method::DELETE, "/api/groups/oncall/members/bob", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["members"], serde_json::json!(["alice"]));

    let (status, body) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"], serde_json::json!(["alice"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_member_add_is_idempotent() {
    let h = boot_with(
        vec![user("alice")],
        vec![GroupConfig {
            name: "oncall".to_string(),
            members: vec!["alice".to_string()],
            ..Default::default()
        }],
    )
    .await;
    let (status, body) =
        json_request(&h, Method::PUT, "/api/groups/oncall/members/alice", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["members"], serde_json::json!(["alice"]));
    assert_eq!(body["member_count"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_member_remove_absent_is_idempotent() {
    let h = boot_with(
        vec![],
        vec![GroupConfig {
            name: "oncall".to_string(),
            ..Default::default()
        }],
    )
    .await;
    let (status, body) =
        json_request(&h, Method::DELETE, "/api/groups/oncall/members/ghost", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["members"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_member_mutation_on_unknown_group_is_404() {
    let h = boot().await;
    let (status, _) = json_request(&h, Method::PUT, "/api/groups/nope/members/alice", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) =
        json_request(&h, Method::DELETE, "/api/groups/nope/members/alice", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn user_groups_reverse_lookup_resolves_membership_and_roles() {
    let h = boot_with(
        vec![user("alice")],
        vec![
            GroupConfig {
                name: "oncall".to_string(),
                members: vec!["alice".to_string()],
                roles: vec!["approver".to_string()],
                ..Default::default()
            },
            // Declared second but sorts first, so the response ordering proves
            // the resolver sorts by name rather than echoing declaration order.
            GroupConfig {
                name: "billing".to_string(),
                members: vec!["alice".to_string()],
                roles: vec!["approver".to_string(), "auditor".to_string()],
                ..Default::default()
            },
            GroupConfig {
                name: "unrelated".to_string(),
                members: vec!["bob".to_string()],
                ..Default::default()
            },
        ],
    )
    .await;

    let (status, body) = json_request(&h, Method::GET, "/api/users/alice/groups", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "alice");
    assert_eq!(body["groups"], serde_json::json!(["billing", "oncall"]));
    // The union: each group's own name plus its declared roles, de-duplicated
    // (`approver` is conferred by both groups) and sorted.
    assert_eq!(
        body["roles"],
        serde_json::json!(["approver", "auditor", "billing", "oncall"])
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn user_groups_for_unregistered_name_is_empty_not_404() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/users/ghost/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["groups"], serde_json::json!([]));
    assert_eq!(body["roles"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_user_strips_the_name_from_every_group() {
    // The cascade that makes group-conferred roles safe: without it a deleted
    // user keeps whatever `roles_for_user` derives from their memberships.
    let h = boot_with(
        vec![user("alice"), user("bob")],
        vec![
            GroupConfig {
                name: "oncall".to_string(),
                members: vec!["alice".to_string(), "bob".to_string()],
                roles: vec!["approver".to_string()],
                ..Default::default()
            },
            GroupConfig {
                name: "billing".to_string(),
                members: vec!["alice".to_string()],
                ..Default::default()
            },
        ],
    )
    .await;

    let (status, _) = json_request(&h, Method::DELETE, "/api/users/alice", None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"], serde_json::json!(["bob"]));

    let (status, body) = json_request(&h, Method::GET, "/api/groups/billing", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"], serde_json::json!([]));

    // And the resolver agrees, which is the fact that actually matters.
    let (status, body) = json_request(&h, Method::GET, "/api/users/alice/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["groups"], serde_json::json!([]));
    assert_eq!(body["roles"], serde_json::json!([]));

    // The cascade must not have been the only thing written — bob's user row
    // and the untouched group both survive the same config rewrite.
    let (status, body) = json_request(&h, Method::GET, "/api/users/bob", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_user_carries_the_name_through_group_membership() {
    // Without the carry-through a rename reads as "this person left every group
    // they were on": the old name lingers as a dangling member and the new one
    // belongs to nothing, silently dropping the roles the rename never meant to
    // touch.
    let h = boot_with(
        vec![user("alice")],
        vec![
            GroupConfig {
                name: "oncall".to_string(),
                members: vec!["alice".to_string(), "bob".to_string()],
                roles: vec!["approver".to_string()],
                ..Default::default()
            },
            GroupConfig {
                name: "unrelated".to_string(),
                members: vec!["bob".to_string()],
                ..Default::default()
            },
        ],
    )
    .await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/alice",
        Some(serde_json::json!({ "name": "alicia", "role": "user" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (status, body) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"], serde_json::json!(["alicia", "bob"]));

    // A group the user was not in is untouched.
    let (status, body) = json_request(&h, Method::GET, "/api/groups/unrelated", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"], serde_json::json!(["bob"]));

    // And the resolver follows the rename, which is the fact that matters.
    let (status, body) = json_request(&h, Method::GET, "/api/users/alicia/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["groups"], serde_json::json!(["oncall"]));
    assert_eq!(body["roles"], serde_json::json!(["approver", "oncall"]));

    let (status, body) = json_request(&h, Method::GET, "/api/users/alice/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["groups"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn renaming_a_user_onto_an_existing_group_member_does_not_duplicate() {
    // The rename target may already be listed in the group (an operator
    // pre-created the membership). Appending unconditionally would leave the
    // name twice and break the de-duplicated invariant every other write holds.
    let h = boot_with(
        vec![user("alice")],
        vec![GroupConfig {
            name: "oncall".to_string(),
            members: vec!["alice".to_string(), "alicia".to_string()],
            ..Default::default()
        }],
    )
    .await;

    let (status, body) = json_request(
        &h,
        Method::PUT,
        "/api/users/alice",
        Some(serde_json::json!({ "name": "alicia", "role": "user" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    let (status, body) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["members"], serde_json::json!(["alicia"]));
    assert_eq!(body["member_count"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn user_mutation_round_trips_the_groups_section_unchanged() {
    // `persist_identity_sections` rewrites both arrays on every identity write.
    // A users-only edit must therefore leave `[[groups]]` semantically intact —
    // if it did not, editing a user would silently empty every group.
    let h = boot_with(
        vec![user("alice")],
        vec![GroupConfig {
            name: "oncall".to_string(),
            description: "Support rota".to_string(),
            members: vec!["alice".to_string()],
            roles: vec!["approver".to_string()],
        }],
    )
    .await;

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/users",
        Some(serde_json::json!({ "name": "bob", "role": "viewer" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body:?}");

    let (status, body) = json_request(&h, Method::GET, "/api/groups/oncall", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["description"], "Support rota");
    assert_eq!(body["members"], serde_json::json!(["alice"]));
    assert_eq!(body["roles"], serde_json::json!(["approver"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_list_is_ordered_by_name_not_declaration_order() {
    let h = boot_with(
        vec![],
        vec![
            GroupConfig {
                name: "zulu".to_string(),
                ..Default::default()
            },
            GroupConfig {
                name: "alpha".to_string(),
                ..Default::default()
            },
        ],
    )
    .await;
    let (status, body) = json_request(&h, Method::GET, "/api/groups", None).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["alpha", "zulu"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn groups_create_rejects_unknown_fields() {
    let h = boot().await;
    let (status, _) = json_request(
        &h,
        Method::POST,
        "/api/groups",
        Some(serde_json::json!({ "name": "team", "member": ["alice"] })),
    )
    .await;
    assert!(
        status == StatusCode::UNPROCESSABLE_ENTITY || status == StatusCode::BAD_REQUEST,
        "a typo'd key must be rejected, got {status}"
    );
}
