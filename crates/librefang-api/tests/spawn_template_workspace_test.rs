//! A template instantiation must not alias the template's own agent directory.
//!
//! Spawn resolves the workspace and writes the absolute result back into
//! `agent.toml`, so the file at `workspaces/agents/<template>/agent.toml` ends up
//! carrying `workspace = "<home>/workspaces/agents/<template>"`. Reading that
//! file as a template and honouring the value handed the new agent the
//! *template's* directory — its `.identity/IDENTITY.md`, its sessions and its
//! memory — which is how an agent comes to believe it is the agent it was cloned
//! from.
//!
//! `resolved_workspace_dir` cannot catch it downstream: an absolute path under
//! the workspaces root is accepted there on purpose (#4991, so a recreate or a
//! restart can reuse the same directory), and the template's directory is under
//! that root. The guard therefore has to be at resolution time.
//!
//! The other half is which file is read at all. `agent-types/<name>.toml` is the
//! type — the thing a deployment copies from — and carries no `workspace`;
//! `workspaces/agents/<name>/agent.toml` is a live instance and always does.
//! #6699 taught the ephemeral path to resolve against the type store; the
//! regular spawn path kept reading only the instance.
//!
//! Run: cargo test -p librefang-api --test spawn_template_workspace_test

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::AppState;
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::agent::AgentId;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tower::ServiceExt;

const TEST_TOKEN: &str = "test-secret";

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    home_dir: PathBuf,
    _tmp: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot() -> Harness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: TEST_TOKEN.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: std::collections::BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };

    let home_dir = tmp.path().to_path_buf();
    let kernel = LibreFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;

    Harness {
        app,
        state,
        home_dir,
        _tmp: tmp,
    }
}

async fn post(
    app: axum::Router,
    path: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header("authorization", format!("Bearer {TEST_TOKEN}"))
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let resp = app.oneshot(req).await.expect("oneshot");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let json = if bytes.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    };
    (status, json)
}

/// An agent *instance* on disk, in the shape spawn leaves one: the resolved
/// absolute workspace round-tripped back into its own `agent.toml`. This is what
/// a template lookup used to read.
fn write_instance(home: &Path, name: &str) -> PathBuf {
    let dir = home.join("workspaces").join("agents").join(name);
    std::fs::create_dir_all(dir.join(".identity")).expect("mkdir instance");
    std::fs::write(
        dir.join("agent.toml"),
        format!(
            "name = \"{name}\"\n\
             module = \"builtin:chat\"\n\
             workspace = \"{}\"\n\
             description = \"DE LA INSTANCIA\"\n",
            dir.display()
        ),
    )
    .expect("write instance manifest");
    std::fs::write(
        dir.join(".identity").join("IDENTITY.md"),
        format!("---\nname: {name}\n---\n"),
    )
    .expect("write instance identity");
    dir
}

/// An agent *type*, in the shape `POST /api/templates` writes one. No workspace.
fn write_agent_type(home: &Path, name: &str, description: &str) {
    let dir = home.join("agent-types");
    std::fs::create_dir_all(&dir).expect("mkdir agent-types");
    std::fs::write(
        dir.join(format!("{name}.toml")),
        format!("name = \"{name}\"\nmodule = \"builtin:chat\"\ndescription = \"{description}\"\n"),
    )
    .expect("write agent type");
}

/// The defect. A clone gets its own directory instead of the template's.
#[tokio::test(flavor = "multi_thread")]
async fn a_clone_does_not_inherit_the_templates_workspace() {
    let h = boot().await;
    write_instance(&h.home_dir, "tmpl-inst");

    let (status, body) = post(
        h.app.clone(),
        "/api/agents",
        serde_json::json!({ "template": "tmpl-inst", "name": "clon" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "spawn failed: {body}");

    // The clone's workspace is its own, so the kernel materialised its own
    // directory. Before the fix the clone's workspace WAS the template's —
    // `ensure_workspace` on a directory that already exists is a no-op — so
    // `clon/` was never created at all.
    let clon = h.home_dir.join("workspaces").join("agents").join("clon");
    assert!(
        clon.is_dir(),
        "the clone must get its own workspace directory, not the template's: {}",
        clon.display()
    );

    // And the identity it was given is its own. This is the symptom the whole
    // guard exists to prevent: an agent presenting itself as the agent it was
    // cloned from.
    let identidad = std::fs::read_to_string(clon.join(".identity").join("IDENTITY.md"))
        .expect("the clone's identity file");
    assert!(
        identidad.contains("name: clon"),
        "the clone's identity must name the clone: {identidad}"
    );
}

/// The door the template lookup does not cover: a caller that supplies the
/// manifest itself.
///
/// This is the CLI's shape — `librefang agent spawn --template <name> --name
/// <other>` expands the template and posts the result as `manifest_toml`, so
/// the API never sees `template` and any guard placed at the lookup would be
/// skipped. The kernel decides, because it is the one place every caller passes
/// through.
#[tokio::test(flavor = "multi_thread")]
async fn a_supplied_manifest_cannot_point_at_another_agents_directory() {
    let h = boot().await;
    let victima = write_instance(&h.home_dir, "vivo");

    // Exactly what a template expansion of a live agent produces: its own
    // manifest, with the absolute workspace spawn wrote into it.
    let (status, body) = post(
        h.app.clone(),
        "/api/agents",
        serde_json::json!({
            "name": "nuevo",
            "manifest_toml": format!(
                "name = \"nuevo\"\nmodule = \"builtin:chat\"\nworkspace = \"{}\"\n",
                victima.display()
            ),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "spawn failed: {body}");

    let propio = h.home_dir.join("workspaces").join("agents").join("nuevo");
    assert!(
        propio.join(".identity").join("IDENTITY.md").is_file(),
        "the new agent must get its own workspace, not '{}'",
        victima.display()
    );
    let identidad =
        std::fs::read_to_string(propio.join(".identity").join("IDENTITY.md")).expect("identity");
    assert!(
        identidad.contains("name: nuevo"),
        "the new agent's identity must name the new agent: {identidad}"
    );
}

/// The same door, spelled relatively.
///
/// `workspace = "agents/vivo"` does not begin with the agents root, so an `starts_with` test written against the absolute form skips it — and `resolve_workspace_dir` then joins it onto the workspaces root, landing the new agent in exactly the directory the absolute path was refused.
/// The guard resolves the path against the root spawn joins it onto before judging it, so the two spellings answer the same question.
#[tokio::test(flavor = "multi_thread")]
async fn a_relative_workspace_cannot_point_at_another_agents_directory() {
    let h = boot().await;
    let victima = write_instance(&h.home_dir, "vivo");

    let (status, body) = post(
        h.app.clone(),
        "/api/agents",
        serde_json::json!({
            "name": "nuevo",
            "manifest_toml": "name = \"nuevo\"\nmodule = \"builtin:chat\"\nworkspace = \"agents/vivo\"\n",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "spawn failed: {body}");

    let propio = h.home_dir.join("workspaces").join("agents").join("nuevo");
    assert!(
        propio.join(".identity").join("IDENTITY.md").is_file(),
        "the new agent must get its own workspace, not the directory 'agents/vivo' resolves to"
    );
    let identidad =
        std::fs::read_to_string(propio.join(".identity").join("IDENTITY.md")).expect("identity");
    assert!(
        identidad.contains("name: nuevo"),
        "the new agent's identity must name the new agent: {identidad}"
    );

    // The directory the relative path named stays its own agent's.
    let identidad_vivo =
        std::fs::read_to_string(victima.join(".identity").join("IDENTITY.md")).expect("identity");
    assert!(
        identidad_vivo.contains("name: vivo"),
        "the named directory must keep its own agent's identity: {identidad_vivo}"
    );
}

/// A shared workspace declared on purpose must survive.
///
/// The guard asks whether the path is *this* agent's directory, not what the
/// agent is called, so a directory outside the agents tree is honoured however
/// the spawn is named. Without that, a legitimate shared workspace would be
/// silently dropped on every rename.
#[tokio::test(flavor = "multi_thread")]
async fn a_declared_shared_workspace_outside_the_agents_tree_is_kept() {
    let h = boot().await;
    let compartido = h.home_dir.join("workspaces").join("compartido");

    let (status, body) = post(
        h.app.clone(),
        "/api/agents",
        serde_json::json!({
            "name": "con-compartido",
            "manifest_toml": format!(
                "name = \"con-compartido\"\nmodule = \"builtin:chat\"\nworkspace = \"{}\"\n",
                compartido.display()
            ),
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "spawn failed: {body}");

    assert!(
        compartido.join(".identity").join("IDENTITY.md").is_file(),
        "a declared shared workspace must be honoured, not replaced by the agent's own"
    );
}

/// The registry checkout is the last of the shared template candidates.
///
/// `POST /api/agents {"template": X}` used to hand-roll its own lookup list without the registry candidate, so a registry-only template resolved for the ephemeral and step-agent paths but not here.
#[tokio::test(flavor = "multi_thread")]
async fn a_registry_only_template_resolves_through_the_shared_candidates() {
    let h = boot().await;
    let templ = h
        .home_dir
        .join("registry")
        .join("agents")
        .join("solo-registro");
    std::fs::create_dir_all(&templ).expect("mkdir registry template");
    std::fs::write(
        templ.join("agent.toml"),
        "name = \"solo-registro\"\nmodule = \"builtin:chat\"\ndescription = \"DEL REGISTRO\"\n",
    )
    .expect("write registry template");

    let (status, body) = post(
        h.app.clone(),
        "/api/agents",
        serde_json::json!({ "template": "solo-registro", "name": "desde-registro" }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "a registry-only template must resolve; body: {body}"
    );
}

/// The type is what a deployment copies from, so an `agent-types/<name>.toml`
/// must win over an instance of the same name.
///
/// The instance is deliberately invalid TOML: whichever file is read decides
/// whether the spawn succeeds, so the assertion needs no second observable.
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_type_wins_over_an_instance_of_the_same_name() {
    let h = boot().await;
    let instancia = write_instance(&h.home_dir, "tipo");
    std::fs::write(
        instancia.join("agent.toml"),
        "name = \"tipo\"\nthis line is not valid TOML =\n",
    )
    .expect("write an unparseable instance");
    write_agent_type(&h.home_dir, "tipo", "DEL TIPO");

    let (status, body) = post(
        h.app.clone(),
        "/api/agents",
        serde_json::json!({ "template": "tipo", "name": "desde-tipo" }),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::CREATED,
        "the agent type must be read in preference to the instance; reading the instance \
         would fail to parse. Body: {body}"
    );
}

/// Instantiation yields an independent agent.
/// Two agents created from the same agent type resolve to distinct workspaces and get distinct identities — the type is a spec, not a directory, so nothing of its own is handed over.
#[tokio::test(flavor = "multi_thread")]
async fn two_agents_instantiated_from_the_same_type_do_not_share_paths() {
    let h = boot().await;
    write_agent_type(&h.home_dir, "base", "DEL TIPO");

    let mut workspaces = Vec::new();
    for name in ["alfa", "beta"] {
        let (status, body) = post(
            h.app.clone(),
            "/api/agents",
            serde_json::json!({ "template": "base", "name": name }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "spawn of {name} failed: {body}"
        );

        let agent_id: AgentId = body["agent_id"]
            .as_str()
            .expect("agent_id in the spawn response")
            .parse()
            .expect("agent_id is a UUID");
        let entry = h
            .state
            .kernel
            .agent_registry()
            .get(agent_id)
            .expect("the spawned agent is in the registry");
        workspaces.push(
            entry
                .manifest
                .workspace
                .clone()
                .expect("workspace resolved"),
        );
    }

    let alfa = h.home_dir.join("workspaces").join("agents").join("alfa");
    let beta = h.home_dir.join("workspaces").join("agents").join("beta");
    assert!(alfa.is_dir(), "alfa must materialise its own workspace");
    assert!(beta.is_dir(), "beta must materialise its own workspace");

    // Each instance's resolved workspace is its own directory — not the type's
    // store (which is a spec, not a directory) and not the other instance's.
    assert_eq!(
        workspaces[0], alfa,
        "alfa's workspace must be its own directory"
    );
    assert_eq!(
        workspaces[1], beta,
        "beta's workspace must be its own directory"
    );
    assert_ne!(
        workspaces[0], workspaces[1],
        "the two instances must not share a workspace"
    );

    for (name, dir) in [("alfa", &alfa), ("beta", &beta)] {
        let identidad =
            std::fs::read_to_string(dir.join(".identity").join("IDENTITY.md")).expect("identity");
        assert!(
            identidad.contains(&format!("name: {name}")),
            "{name}'s identity must name it: {identidad}"
        );
    }
}

/// A recreate must re-accept the absolute workspace spawn itself wrote.
///
/// This is the #4991 shape: spawn rewrites `manifest.workspace` to the resolved absolute directory and round-trips it into `agent.toml`, so a later spawn that reads that file back — here through the template door, with the same name — declares a workspace *inside* the agents tree.
/// It must not be rejected (that was #4991's 500) and it must keep the directory the name already owns.
///
/// The declared directory is the one the name resolves to, so the guard evaluates its `resolved != own` branch and leaves it alone; keep-vs-drop is not distinguishable by directory placement here — dropping the path resolves to the same `agents/mismo` — so this pins the acceptance side and the reuse of the existing identity.
/// The delete-then-recreate flow itself is covered in `agents_routes_integration.rs` (`test_recreate_agent_same_name_after_delete_succeeds`).
#[tokio::test(flavor = "multi_thread")]
async fn reusing_the_same_name_accepts_its_own_absolute_workspace() {
    let h = boot().await;
    // Shaped exactly as spawn leaves a live agent: its own absolute directory
    // inside the agents tree, round-tripped into `agent.toml`.
    let propio = write_instance(&h.home_dir, "mismo");

    let (status, body) = post(
        h.app.clone(),
        "/api/agents",
        serde_json::json!({ "template": "mismo", "name": "mismo" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "spawn failed: {body}");

    let agent_id: AgentId = body["agent_id"]
        .as_str()
        .expect("agent_id in the spawn response")
        .parse()
        .expect("agent_id is a UUID");
    let entry = h
        .state
        .kernel
        .agent_registry()
        .get(agent_id)
        .expect("the spawned agent is in the registry");
    assert_eq!(
        entry.manifest.workspace.as_deref(),
        Some(propio.as_path()),
        "the declared absolute workspace must be the one the name resolves to"
    );

    // The identity already in the directory is the agent's own, not a freshly
    // generated stranger's.
    let identidad =
        std::fs::read_to_string(propio.join(".identity").join("IDENTITY.md")).expect("identity");
    assert!(
        identidad.contains("name: mismo"),
        "the existing directory's identity must be reused: {identidad}"
    );
}
