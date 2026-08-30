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

/// Same as [`boot`] but with the kernel booted in Stable mode, which freezes
/// the skill registry at boot (`KernelConfig::mode`). Used to exercise the
/// frozen-reload honesty path (#6540).
async fn boot_stable() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.mode = librefang_types::config::KernelMode::Stable;
    }));
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
        .expect("path field present and is a string")
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
            .or_else(|| body["error"]["message"].as_str())
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
// GET /api/marketplace/search (#6569)
// ---------------------------------------------------------------------------

/// The registry ships `SKILL.md`, but this endpoint only looked for `skill.toml`, so it returned an empty result set on every install — the same gap `/api/skills/registry` had already solved.
#[tokio::test(flavor = "multi_thread")]
async fn marketplace_search_finds_skillmd_registry_entries_6569() {
    let h = boot().await;
    install_registry_skill(h.home(), "web-search", "Search the web for answers");

    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/marketplace/search?q=web-search",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let rows = body["results"].as_array().expect("results array");
    let row = rows
        .iter()
        .find(|r| r["name"] == "web-search")
        .unwrap_or_else(|| panic!("web-search missing in {body}"));
    assert_eq!(row["description"], "Search the web for answers");
    // The directory name is what the install endpoints take; it can differ from the frontmatter name.
    assert_eq!(row["install_id"], "web-search");
    assert_eq!(
        body["total"].as_u64().unwrap() as usize,
        rows.len(),
        "total must match array length: {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_search_matches_the_description_too_6569() {
    let h = boot().await;
    install_registry_skill(h.home(), "postgres-expert", "Tune slow SQL queries");

    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/marketplace/search?q=slow%20SQL",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert!(
        body["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "postgres-expert"),
        "description match missing in {body}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_search_returns_no_rows_for_a_miss_6569() {
    let h = boot().await;
    install_registry_skill(h.home(), "web-search", "Search the web");

    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/marketplace/search?q=definitely-not-a-skill",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["results"], serde_json::json!([]));
    assert_eq!(body["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn marketplace_search_rows_are_sorted_by_name_6569() {
    let h = boot().await;
    install_registry_skill(h.home(), "zzz-last", "sortable marker skill");
    install_registry_skill(h.home(), "aaa-first", "sortable marker skill");

    let (status, body) = json_request(
        &h,
        Method::GET,
        "/api/marketplace/search?q=sortable%20marker",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let names: Vec<&str> = body["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        names,
        vec!["aaa-first", "zzz-last"],
        "read_dir order is filesystem-dependent, so the handler must sort"
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

#[tokio::test(flavor = "multi_thread")]
async fn skills_reload_frozen_stable_mode_reports_honest_result() {
    // #6540: a frozen (Stable-mode) reload must not silently no-op — the HTTP
    // response must surface `frozen: true` and the skipped new skill dir
    // instead of pretending a full reload happened, and the freeze boundary
    // must actually hold (the new skill must not be loaded).
    let h = boot_stable().await;

    install_skill(h.home(), "added-after-boot", &[]);

    let (status, body) = json_request(&h, Method::POST, "/api/skills/reload", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(body["frozen"], true, "{body:?}");
    assert_eq!(body["status"], "partial", "{body:?}");
    assert_eq!(
        body["skipped_new"],
        serde_json::json!(["added-after-boot"]),
        "{body:?}"
    );
    assert_eq!(body["count"], 0, "{body:?}");

    let (_, after) = json_request(&h, Method::GET, "/api/skills", None).await;
    assert_eq!(
        after["total"], 0,
        "frozen registry must not load the new skill: {after:?}"
    );
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
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .contains("not found"),
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
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("hand"),
        "error must mention the missing hand: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_install_path_traversal_name_rejected_400() {
    // Guards the `validate_skill_identifier` hardening (audit:
    // skill-install-path-traversal): a `name` containing path
    // separators / `..` must be rejected with 400 BEFORE it can reach
    // `Path::join` and probe / write outside `~/.librefang/skills/`.
    // The rejection must fire ahead of the NotFound branch so an
    // attacker gets no filesystem-existence oracle.
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/install",
        Some(serde_json::json!({"name": "../../etc/passwd"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("name"),
        "error must scope the rejection to the bad `name` field: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_install_path_traversal_hand_rejected_400() {
    // Same hardening, applied to the `hand` field. A traversal `hand`
    // is joined onto `workspaces/hands/` pre-fix; it must be rejected
    // with 400 before the `hand_dir.exists()` probe (which would
    // otherwise yield a 404/oracle).
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/install",
        Some(serde_json::json!({"name": "anything", "hand": "../../x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("hand"),
        "error must scope the rejection to the bad `hand` field: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skills_install_already_installed_returns_409() {
    // Regression coverage for #6977: the destination-exists check moved
    // from before the per-skill lock to inside
    // `librefang_skills::evolution::install_local_skill`, and a hit now
    // surfaces through `SkillError::AlreadyInstalled` instead of a
    // pre-lock `dest.exists()` probe. Assert the HTTP-visible behavior
    // (first install succeeds, second is a 409) still holds end-to-end.
    let h = boot().await;
    install_registry_skill(h.home(), "dup-skill", "A skill installed twice");

    let (first_status, first_body) = json_request(
        &h,
        Method::POST,
        "/api/skills/install",
        Some(serde_json::json!({"name": "dup-skill"})),
    )
    .await;
    assert_eq!(first_status, StatusCode::OK, "{first_body:?}");

    let (second_status, second_body) = json_request(
        &h,
        Method::POST,
        "/api/skills/install",
        Some(serde_json::json!({"name": "dup-skill"})),
    )
    .await;
    assert_eq!(second_status, StatusCode::CONFLICT, "{second_body:?}");
    assert_eq!(
        second_body["status"], "already_installed",
        "{second_body:?}"
    );
    assert!(
        second_body["error"]
            .as_str()
            .unwrap_or("")
            .contains("already installed"),
        "error must mention already-installed: {second_body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skillhub_install_path_traversal_hand_rejected_400() {
    // Skillhub has its own install handler. Reject traversal before the
    // hand directory existence probe so an attacker cannot escape the
    // workspace root (or use the response as a filesystem oracle).
    let h = boot().await;
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skillhub/install",
        Some(serde_json::json!({"slug": "anything", "hand": "../../x"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("hand"),
        "error must scope the rejection to the bad `hand` field: {body:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skillhub_install_accepts_canonical_max_length_hand_identifier() {
    let h = boot().await;
    let hand_id = "a".repeat(128);
    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skillhub/install",
        Some(serde_json::json!({"slug": "anything", "hand": hand_id})),
    )
    .await;

    // The hand does not exist in this isolated home, so reaching the lookup's
    // 404 proves the canonical 128-character id passed API validation without
    // allowing the request to continue to the network client.
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("not found"),
        "expected the request to reach the hand lookup: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// POST /api/skills/uninstall
// ---------------------------------------------------------------------------

/// A hub install names the directory after the *slug* it downloaded, while
/// the registry keys every skill by `[skill] name` from the manifest — which
/// is the name the dashboard hands straight back to this endpoint. ClawHub's
/// `frontend-design-2` installs into `skills/frontend-design-2/` and lists as
/// `frontend-design`; the uninstall resolved `<home>/skills/frontend-design`,
/// missed, and answered `404 Skill not found: frontend-design` for a skill
/// the operator had just installed and could see in the list.
#[tokio::test(flavor = "multi_thread")]
async fn skills_uninstall_resolves_a_slug_named_directory() {
    let h = boot().await;
    // Exactly the on-disk shape a ClawHub install leaves behind: directory
    // named after the slug, published name in the SKILL.md frontmatter.
    let skill_dir = h.home().join("skills").join("frontend-design-2");
    std::fs::create_dir_all(&skill_dir).expect("mkdir slug dir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: frontend-design\ndescription: \"Frontend design skill\"\n---\n\n# Body\n",
    )
    .expect("write SKILL.md");
    h._state.kernel.reload_skills();

    let (status, body) = json_request(&h, Method::GET, "/api/skills", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["items"][0]["name"], "frontend-design",
        "the registry publishes the manifest name, not the slug: {body:?}"
    );

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/uninstall",
        Some(serde_json::json!({"name": "frontend-design"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "uninstall must find the skill under its slug directory: {body:?}"
    );

    assert!(
        !skill_dir.exists(),
        "the slug directory must be gone: {}",
        skill_dir.display()
    );
    let (status, body) = json_request(&h, Method::GET, "/api/skills", None).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    assert_eq!(
        body["total"], 0,
        "skill must be gone from the list: {body:?}"
    );
}

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

// ---------------------------------------------------------------------------
// POST /api/skills/{name}/propose
// ---------------------------------------------------------------------------

/// Proposing a skill that does not exist resolves the skill before any
/// token / network concern, so it must 404 regardless of GitHub
/// credentials. This is the network-free contract we can assert
/// deterministically in CI (the fork/PR happy path needs a live GitHub
/// token and is covered by the human Live-Integration checklist).
#[tokio::test(flavor = "multi_thread")]
async fn skills_propose_unknown_returns_404() {
    let h = boot().await;
    let (status, body) = json_request(&h, Method::POST, "/api/skills/ghost/propose", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("not found"),
        "error must mention 'not found': {body:?}"
    );
}

/// When the skill exists but no GitHub token is configured (env or
/// vault), proposing returns 401 — the dashboard surfaces this as a
/// "connect GitHub" affordance. Guarded so it only runs when the test
/// process genuinely has no `GITHUB_TOKEN`, since env state is shared
/// across parallel test binaries and mutating it would be racy
/// (CLAUDE.md env-var flakiness note).
#[tokio::test(flavor = "multi_thread")]
async fn skills_propose_without_token_returns_401() {
    if std::env::var("GITHUB_TOKEN")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false)
    {
        // A token is present in this environment — the 401 branch is
        // unreachable. Skip rather than mutate shared env.
        return;
    }
    let h = boot().await;
    install_skill(h.home(), "proposable-skill", &["data"]);
    h._state.kernel.reload_skills();

    let (status, body) = json_request(
        &h,
        Method::POST,
        "/api/skills/proposable-skill/propose",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:?}");
    assert!(
        body["error"]
            .as_str()
            .or_else(|| body["error"]["message"].as_str())
            .unwrap_or("")
            .to_lowercase()
            .contains("github"),
        "401 error should mention GitHub token: {body:?}"
    );
}

// ---------------------------------------------------------------------------
// A marketplace that answers 200 with its webpage (#7387)
// ---------------------------------------------------------------------------
//
// The failure this covers is not a hub that is down — a down hub already
// produced a sensible upstream error. It is a hub whose API host has been
// retired while the CDN in front of it kept answering, so every path now
// returns `200 OK` with the marketing single-page-app shell. `serde_json` then
// complains about the leading `<`, and that complaint used to reach the reader
// as the whole explanation: a `502` on search and browse, a `404` on detail
// that falsely said the skill did not exist, and a `500` on install whose body
// was scrubbed to "Internal server error".
//
// Every handler below drives exactly that upstream response and asserts the one
// answer they now share: `503 Service Unavailable`, with the actionable text
// intact. The point of testing all of them rather than one is that the previous
// attempt at this fix mapped only the Skillhub routes, leaving the shared
// `ClawHubClient`'s eight ClawHub and ClawHub CN handlers translating the new
// condition into their old, wrong statuses.

/// The shell a dead marketplace serves, complete with the UTF-8 BOM that a
/// CDN-fronted origin tends to prepend. The BOM is deliberate: it is not ASCII
/// whitespace, so a markup detector that skips only whitespace before looking
/// for `<` misses this exact body — the most common real-world shape.
const MARKETPLACE_SPA_SHELL: &str = "\u{feff}<!doctype html>\n<html><head><title>Skill Hub</title></head>\n<body><div id=\"app\"></div></body></html>\n";

/// Base URL of a process-wide server that answers every request with
/// [`MARKETPLACE_SPA_SHELL`], with the marketplace URL overrides already
/// pointed at it.
///
/// Process-wide, and initialised exactly once, because the handlers resolve
/// their hub URLs from the environment and the environment is shared by every
/// test in this binary. One server started once means those variables are
/// written a single time, to a single set of values, before any test reads
/// them. No other test in this file touches a remote hub.
///
/// The listener runs on its own thread with its own runtime rather than on the
/// calling test's. `#[tokio::test]` builds a fresh runtime per test and drops it
/// at the end of that test, which would take a `tokio::spawn`-ed accept loop
/// down with it — every later test would then hit a closed port, and a refused
/// connection is a `Network` error that the client retries with backoff, so the
/// suite hangs for tens of seconds instead of failing.
fn dead_marketplace() -> &'static str {
    use std::io::{Read, Write};
    use std::sync::OnceLock;

    static SERVER: OnceLock<String> = OnceLock::new();

    SERVER.get_or_init(|| {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub marketplace");
        let address = listener.local_addr().expect("stub marketplace address");

        std::thread::spawn(move || {
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: text/html; charset=utf-8\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{}",
                MARKETPLACE_SPA_SHELL.len(),
                MARKETPLACE_SPA_SHELL
            );
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let response = response.clone();
                std::thread::spawn(move || {
                    // Read only until the blank line that ends the request head;
                    // reading to EOF would block on a client waiting for us.
                    let mut request = Vec::new();
                    let mut byte = [0_u8; 1];
                    while stream.read_exact(&mut byte).is_ok() {
                        request.push(byte[0]);
                        if request.ends_with(b"\r\n\r\n") {
                            break;
                        }
                    }
                    let _ = stream.write_all(response.as_bytes());
                    let _ = stream.flush();
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                });
            }
        });

        let base = format!("http://{address}");
        std::env::set_var("LIBREFANG_CLAWHUB_URL", format!("{base}/api/v1"));
        std::env::set_var("LIBREFANG_CLAWHUB_CN_URL", format!("{base}/api/v1"));
        std::env::set_var("LIBREFANG_SKILLHUB_URL", format!("{base}/api/v1"));
        std::env::set_var(
            "LIBREFANG_SKILLHUB_INDEX_URL",
            format!("{base}/skills.json"),
        );
        std::env::set_var("LIBREFANG_SKILLHUB_COS_URL", base.clone());
        base
    })
}

/// Drive one route against the dead marketplace and assert the shared answer.
async fn assert_marketplace_unavailable(
    method: Method,
    path: &str,
    body: Option<serde_json::Value>,
) {
    dead_marketplace();
    let h = boot().await;
    let (status, json) = json_request(&h, method, path, body).await;

    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "{path} should report the hub as unavailable, got {status} with {json}"
    );

    let error = json["error"].as_str().unwrap_or_default();
    assert!(
        error.starts_with("Marketplace unavailable:"),
        "{path} should name the condition, got {error:?}"
    );
    assert!(
        error.contains("webpage instead of"),
        "{path} should say what the hub served, got {error:?}"
    );
    // The scrub that blanks a 500 body must not swallow the one message an
    // operator can act on.
    assert_ne!(
        error, "Internal server error",
        "{path} scrubbed its message"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_search_reports_a_webpage_serving_hub_as_unavailable() {
    assert_marketplace_unavailable(Method::GET, "/api/clawhub/search?q=rust", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_browse_reports_a_webpage_serving_hub_as_unavailable() {
    assert_marketplace_unavailable(Method::GET, "/api/clawhub/browse?limit=5", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_detail_reports_unavailable_rather_than_not_found() {
    assert_marketplace_unavailable(Method::GET, "/api/clawhub/skill/example-skill", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_skill_code_reports_unavailable_rather_than_not_found() {
    assert_marketplace_unavailable(Method::GET, "/api/clawhub/skill/example-skill/code", None)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_install_reports_unavailable_rather_than_a_scrubbed_500() {
    assert_marketplace_unavailable(
        Method::POST,
        "/api/clawhub/install",
        Some(serde_json::json!({"slug": "example-skill"})),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_cn_search_reports_a_webpage_serving_hub_as_unavailable() {
    assert_marketplace_unavailable(Method::GET, "/api/clawhub-cn/search?q=rust", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_cn_browse_reports_a_webpage_serving_hub_as_unavailable() {
    assert_marketplace_unavailable(Method::GET, "/api/clawhub-cn/browse?limit=5", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_cn_detail_reports_unavailable_rather_than_not_found() {
    assert_marketplace_unavailable(Method::GET, "/api/clawhub-cn/skill/example-skill", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_cn_skill_code_reports_unavailable_rather_than_not_found() {
    assert_marketplace_unavailable(
        Method::GET,
        "/api/clawhub-cn/skill/example-skill/code",
        None,
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn clawhub_cn_install_reports_unavailable_rather_than_a_scrubbed_500() {
    assert_marketplace_unavailable(
        Method::POST,
        "/api/clawhub-cn/install",
        Some(serde_json::json!({"slug": "example-skill"})),
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn skillhub_search_reports_a_webpage_serving_hub_as_unavailable() {
    assert_marketplace_unavailable(Method::GET, "/api/skillhub/search?q=rust", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn skillhub_browse_reports_a_webpage_serving_hub_as_unavailable() {
    assert_marketplace_unavailable(Method::GET, "/api/skillhub/browse?limit=5", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn skillhub_detail_reports_unavailable_rather_than_not_found() {
    assert_marketplace_unavailable(Method::GET, "/api/skillhub/skill/example-skill", None).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn skillhub_install_reports_unavailable_rather_than_a_scrubbed_500() {
    assert_marketplace_unavailable(
        Method::POST,
        "/api/skillhub/install",
        Some(serde_json::json!({"slug": "example-skill"})),
    )
    .await;
}
