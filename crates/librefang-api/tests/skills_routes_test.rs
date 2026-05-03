//! Integration tests for the skills-domain HTTP routes.
//!
//! Refs #3571 — "~80% of registered HTTP routes have no integration test".
//! This file covers the skills slice: `/api/skills`, `/api/skills/{name}`,
//! `/api/skills/registry`, `/api/skills/reload`, plus the `/api/skills/install`
//! and `/api/skills/uninstall` error paths that don't require shelling out
//! to the real FangHub registry.
//!
//! Mutating endpoints that touch shared global state (network calls to
//! ClawHub / SkillHub / FangHub, GitHub HTTP, etc.) are intentionally
//! skipped; each test boots a fresh kernel against a `TempDir` home, so
//! anything we write stays local to the test process.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use std::path::Path;
use std::sync::Arc;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _state: Arc<AppState>,
    test: TestAppState,
}

impl Harness {
    fn home(&self) -> &Path {
        self.test.tmp_path()
    }
}

async fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new());
    let state = test.state.clone();
    let app = Router::new()
        .nest("/api", routes::skills::router())
        .with_state(state.clone());
    Harness {
        app,
        _state: state,
        test,
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

/// Drop a minimal `skill.toml` into `<home>/skills/<name>/` so the kernel's
/// registry picks it up on the next reload. Mirrors the helper used in
/// `librefang_skills::registry::tests::create_test_skill` so the schema
/// is guaranteed to match what `SkillRegistry::load_all` accepts.
fn install_skill(home: &Path, name: &str, tags: &[&str]) {
    let skill_dir = home.join("skills").join(name);
    std::fs::create_dir_all(&skill_dir).expect("mkdir skill dir");
    let tags_toml = if tags.is_empty() {
        String::new()
    } else {
        let quoted: Vec<String> = tags.iter().map(|t| format!("\"{t}\"")).collect();
        format!("tags = [{}]\n", quoted.join(", "))
    };
    let manifest = format!(
        r#"[skill]
name = "{name}"
version = "0.1.0"
description = "Test skill {name}"
{tags_toml}
[runtime]
type = "python"
entry = "main.py"

[[tools.provided]]
name = "{name}_tool"
description = "A test tool"
input_schema = {{ type = "object" }}
"#
    );
    std::fs::write(skill_dir.join("skill.toml"), manifest).expect("write skill.toml");
}

/// Drop a `SKILL.md`-only entry into `<home>/registry/skills/<name>/` so the
/// `/api/skills/registry` cache walker has something to enumerate.
fn install_registry_skill(home: &Path, name: &str, description: &str) {
    let dir = home.join("registry").join("skills").join(name);
    std::fs::create_dir_all(&dir).expect("mkdir registry skill dir");
    let md = format!(
        "---\nname: {name}\ndescription: \"{description}\"\nversion: \"1.2.3\"\nauthor: tester\ntags: [a, b]\n---\n\n# Body\n"
    );
    std::fs::write(dir.join("SKILL.md"), md).expect("write SKILL.md");
}

// ---------------------------------------------------------------------------
// GET /api/skills
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_list_starts_empty() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/skills", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["items"], serde_json::json!([]));
    assert_eq!(body["categories"], serde_json::json!([]));
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_list_returns_installed_skill_metadata() {
    let h = boot().await;
    install_skill(h.home(), "alpha", &["data"]);
    // Use only non-platform tags. `librefang_skills::registry::skill_matches_platform`
    // (`registry.rs:68`) filters out skills whose tags include a platform hint
    // (`"macos"` / `"linux"` / `"windows"`) when running on a different OS, so
    // a tag set like `["linux", "writing"]` would silently drop "beta" on
    // macOS and Windows runners and the test would observe `total = 1`.
    install_skill(h.home(), "beta", &["writing"]);
    h._state.kernel.reload_skills();

    let (status, body) = json_request(&h, Method::GET, "/api/skills", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["total"], 2, "{body:?}");

    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));

    // Each entry exposes the dashboard-visible flags.
    for s in body["items"].as_array().unwrap() {
        assert_eq!(s["enabled"], true);
        assert_eq!(s["tools_count"], 1);
        assert!(s["source"]["type"].is_string());
        assert!(s["runtime"].is_string());
    }

    // Categories list is sorted (BTreeSet) and non-empty.
    let cats = body["categories"].as_array().unwrap();
    assert!(!cats.is_empty(), "categories should be derived: {body:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_list_filters_by_category() {
    let h = boot().await;
    install_skill(h.home(), "alpha", &["data"]);
    install_skill(h.home(), "beta", &["writing"]);
    h._state.kernel.reload_skills();

    // Pick an actually-present category from the unfiltered call so the
    // assertion doesn't depend on internal `derive_category` rules.
    let (_, full) = json_request(&h, Method::GET, "/api/skills", None).await;
    let pick = full["categories"]
        .as_array()
        .and_then(|cs| cs.first())
        .and_then(|c| c.as_str())
        .expect("at least one category")
        .to_string();

    let (status, body) = json_request(
        &h,
        Method::GET,
        &format!("/api/skills?category={pick}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body["total"].as_u64().unwrap() <= 2,
        "filter should not over-count: {body:?}"
    );
    assert!(
        body["total"].as_u64().unwrap() >= 1,
        "filter should match at least one: {body:?}"
    );
    // Categories list stays unfiltered so the dashboard can still render
    // sibling tabs after a filter is applied.
    assert!(
        body["categories"].as_array().unwrap().len() >= body["total"].as_u64().unwrap() as usize,
        "categories list must reflect all skills, not the filtered subset: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_list_unknown_category_returns_zero() {
    let h = boot().await;
    install_skill(h.home(), "alpha", &["data"]);
    h._state.kernel.reload_skills();
    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/skills?category=__not_a_real_cat__",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["total"], 0);
    assert_eq!(body["offset"], 0);
    assert_eq!(body["items"], serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// GET /api/skills/{name}
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_detail_returns_full_manifest() {
    let h = boot().await;
    install_skill(h.home(), "detail-skill", &["data"]);
    h._state.kernel.reload_skills();

    let (status, body) = json_request(&h, Method::GET, "/api/skills/detail-skill", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["name"], "detail-skill");
    assert_eq!(body["version"], "0.1.0");
    assert_eq!(body["enabled"], true);
    let tools = body["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "detail-skill_tool");
    // `path` must be the absolute on-disk skill dir — dashboards use it
    // to surface a "open in editor" affordance. Normalize the platform
    // separator (Windows reports `...\skills\detail-skill`, sometimes with
    // a `\\?\` UNC prefix) before comparing against the cross-platform
    // forward-slash suffix.
    let normalized_path = body["path"]
        .as_str()
        .unwrap_or("")
        .replace('\\', "/");
    assert!(
        normalized_path.ends_with("skills/detail-skill"),
        "path should point at the skill dir: {body:?}"
    );
    // Evolution metadata block is always present, even for fresh installs.
    assert!(body["evolution"].is_object(), "{body:?}");
    assert_eq!(body["evolution"]["use_count"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_detail_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/skills/ghost", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("not found"),
        "error must mention 'not found': {body:?}"
    );
}

// ---------------------------------------------------------------------------
// GET /api/skills/registry
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_registry_returns_ok_with_well_formed_rows() {
    // Kernel boot seeds a default `registry/skills/` cache, so we don't
    // assert an empty list here — instead we assert that whatever is
    // returned has the dashboard-required shape and a stable schema.
    let h = boot().await;
    let (status, body) = json_request(&h, Method::GET, "/api/skills/registry", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let rows = body["skills"].as_array().expect("skills array");
    assert_eq!(
        body["total"].as_u64().unwrap() as usize,
        rows.len(),
        "total must match array length: {body}"
    );
    for row in rows {
        for key in ["name", "description", "version", "tags", "is_installed"] {
            assert!(
                row.get(key).is_some(),
                "registry row missing '{key}': {row}"
            );
        }
        assert!(row["is_installed"].is_boolean(), "{row}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_registry_lists_cached_entries_and_install_state() {
    let h = boot().await;
    install_registry_skill(h.home(), "cached-one", "first cached skill");
    install_registry_skill(h.home(), "cached-two", "second cached skill");
    // Mark `cached-one` as already installed so the `is_installed` flag
    // round-trips correctly.
    install_skill(h.home(), "cached-one", &[]);
    h._state.kernel.reload_skills();

    let (status, body) = json_request(&h, Method::GET, "/api/skills/registry", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");

    // Find our two seeded entries within the (possibly larger) builtin
    // registry cache. Other rows belong to LibreFang's bundled skills
    // and are out of scope for this test.
    let rows = body["skills"].as_array().unwrap();
    let one = rows
        .iter()
        .find(|r| r["name"] == "cached-one")
        .unwrap_or_else(|| panic!("cached-one row missing in {} rows", rows.len()));
    let two = rows
        .iter()
        .find(|r| r["name"] == "cached-two")
        .unwrap_or_else(|| panic!("cached-two row missing in {} rows", rows.len()));
    assert_eq!(one["description"], "first cached skill");
    assert_eq!(one["version"], "1.2.3");
    assert_eq!(one["author"], "tester");
    assert_eq!(one["tags"], serde_json::json!(["a", "b"]));
    assert_eq!(one["is_installed"], true, "cached-one is installed: {one}");
    assert_eq!(
        two["is_installed"], false,
        "cached-two is registry-only: {two}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/skills/reload
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_reload_picks_up_filesystem_drops() {
    let h = boot().await;
    let (_, before) = json_request(&h, Method::GET, "/api/skills", None).await;
    assert_eq!(before["total"], 0);

    install_skill(h.home(), "dropped", &[]);

    let (status, body) = json_request(&h, Method::POST, "/api/skills/reload", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["status"], "reloaded");
    assert_eq!(body["count"], 1, "{body:?}");

    let (_, after) = json_request(&h, Method::GET, "/api/skills", None).await;
    assert_eq!(after["total"], 1);
    assert_eq!(after["items"][0]["name"], "dropped");
}

// ---------------------------------------------------------------------------
// POST /api/skills/install — error paths only.
// The happy path requires a populated `~/.librefang/registry/skills/<name>`
// AND the kernel's evolution module to recognise the layout; that's
// covered by the kernel's own integration tests. We only assert the two
// 4xx branches that are easy to set up in this harness.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_install_unknown_skill_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/install",
        Some(serde_json::json!({"name": "does-not-exist"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"].as_str().unwrap_or("").contains("not found"),
        "error must mention not-found: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_install_unknown_hand_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/install",
        Some(serde_json::json!({"name": "anything", "hand": "ghost-hand"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("hand"),
        "error must mention the missing hand: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/skills/uninstall — error path only.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn skills_uninstall_unknown_returns_4xx() {
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/uninstall",
        Some(serde_json::json!({"name": "ghost"})),
    )
    .await;
    // The evolution module reports NotFound; we only require a 4xx
    // (the exact code is an evolution-module concern).
    assert!(
        status.is_client_error(),
        "expected 4xx for unknown skill, got {status}: {body:?}"
    );
}
