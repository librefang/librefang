//! Integration test for `DELETE /api/channels/sidecar/{name}` (channel removal).
//!
//! Tempdir-backed kernel so the config.toml rewrite lands in the sandbox.
//! The block is written to disk after boot, so the kernel's in-memory config
//! never carried the channel — removing it yields no `ReloadChannels` action,
//! keeping the test free of sidecar-spawn side effects.

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{header, Method, Request, StatusCode};
use futures::Stream;
use librefang_api::server;
use librefang_channels::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, KernelConfig};
use std::pin::Pin;
use std::sync::Arc;
use tower::ServiceExt;

const API_KEY: &str = "test-secret-key";

struct RouterHarness {
    app: axum::Router,
    home: std::path::PathBuf,
    _tmp: tempfile::TempDir,
    state: Arc<librefang_api::routes::AppState>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

async fn boot_router() -> RouterHarness {
    boot_router_with_sidecars(Vec::new()).await
}

/// Boot with `sidecar_channels` already in the kernel's *in-memory* config,
/// which is what a running daemon holds and what `GET /api/channels` reports —
/// independently of whatever config.toml says at any later moment.
async fn boot_router_with_sidecars(
    sidecar_channels: Vec<librefang_types::config::SidecarChannelConfig>,
) -> RouterHarness {
    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());
    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: API_KEY.to_string(),
        sidecar_channels,
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
    let home = config.home_dir.clone();
    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    RouterHarness {
        app,
        home,
        _tmp: tmp,
        state,
    }
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, Vec<u8>) {
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

fn auth_delete(path: &str) -> Request<Body> {
    Request::builder()
        .method(Method::DELETE)
        .uri(path)
        .header(header::AUTHORIZATION, format!("Bearer {API_KEY}"))
        .body(Body::empty())
        .unwrap()
}

const TELEGRAM_BLOCK: &str = "[[sidecar_channels]]\n\
     name = \"telegram\"\n\
     channel_type = \"telegram\"\n\
     command = \"python3\"\n\
     args = [\"-m\", \"librefang.sidecar.adapters.telegram\"]\n\
     \n\
     [sidecar_channels.env]\n\
     ALLOWED_USERS = \"1,2\"\n";

#[tokio::test(flavor = "multi_thread")]
async fn delete_removes_configured_sidecar_then_404s_on_repeat() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    std::fs::write(&config_path, TELEGRAM_BLOCK).expect("seed config.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    let written = std::fs::read_to_string(&config_path).expect("config.toml still present");
    assert!(
        !written.contains("[[sidecar_channels]]") && !written.contains("name = \"telegram\""),
        "block must be gone: {written}"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}

const EMAIL_BLOCK: &str = "[[sidecar_channels]]\n\
     name = \"email\"\n\
     channel_type = \"email\"\n\
     command = \"python3\"\n\
     args = [\"-m\", \"librefang.sidecar.adapters.email\"]\n";

/// A sidecar declared in an `include = [...]` file is a fully live channel:
/// the kernel merges included files into the running config, so it spawns,
/// it supervises, and `list_channels` renders it as `configured` — which is
/// the only state in which the dashboard offers the delete button. Deleting
/// used to rewrite the root config.toml only, find nothing, and answer
/// "404 no configured sidecar channel named email" for a channel that was
/// running at that moment.
#[tokio::test(flavor = "multi_thread")]
async fn delete_removes_sidecar_declared_in_an_included_file() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    let included_path = h.home.join("channels.toml");
    std::fs::write(&config_path, "include = [\"channels.toml\"]\n").expect("seed config.toml");
    std::fs::write(&included_path, EMAIL_BLOCK).expect("seed channels.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    let written = std::fs::read_to_string(&included_path).expect("included file still present");
    assert!(
        !written.contains("[[sidecar_channels]]") && !written.contains("name = \"email\""),
        "block must be gone from the included file: {written}"
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("root config still present"),
        "include = [\"channels.toml\"]\n",
        "the root config must be left untouched"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}

/// Reconcile path: the daemon is running a sidecar whose `[[sidecar_channels]]`
/// block is no longer on disk. The dashboard reads `configured` from the live
/// in-memory config, so the card — and its delete button — are still there,
/// while the delete found nothing to strip and answered
/// "404 no configured sidecar channel named email" for a channel whose child
/// process was alive. Observed on the rodela deployment on 2026-08-24: config
/// carried telegram only, `GET /api/channels` reported email as configured,
/// supervised and connected, and its adapter process was running.
///
/// The delete must complete: nothing to remove on disk is not "does not
/// exist", and the reload that follows is what stops the orphaned child.
#[tokio::test(flavor = "multi_thread")]
async fn delete_reconciles_a_live_sidecar_that_is_no_longer_on_disk() {
    // Deliberately unspawnable (and `restart = false`, so the supervisor does
    // not retry): the point of the test is that a channel must be deletable
    // regardless of whether its sidecar is up.
    let email: librefang_types::config::SidecarChannelConfig = toml::from_str(
        "name = \"email\"\n\
         channel_type = \"email\"\n\
         command = \"/nonexistent/librefang-test-email-sidecar\"\n\
         restart = false\n",
    )
    .expect("parse sidecar entry");
    let h = boot_router_with_sidecars(vec![email]).await;
    // config.toml never declared it — the file is the post-edit state.
    std::fs::write(h.home.join("config.toml"), "# no sidecar_channels\n").expect("seed config");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a live channel must be deletable even with no block left on disk; body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    // Reconciled: the reload dropped it from the live config, so the channel
    // really is gone and a repeat delete is a genuine 404.
    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}

#[tokio::test(flavor = "multi_thread")]
async fn delete_unknown_sidecar_404s() {
    let h = boot_router().await;
    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/nope")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A name declared in the root config.toml AND an included file at once: the
/// early-returning walk this test replaces stripped only the root block,
/// reported `removed`, and the reload re-merged the survivor from the include,
/// so the channel came back — a fresh version of the bug this handler fixes.
/// Both blocks must go in one delete, and a repeat delete must be a real 404.
#[tokio::test(flavor = "multi_thread")]
async fn delete_strips_a_sidecar_declared_in_root_and_include_at_once() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    let included_path = h.home.join("channels.toml");
    // `include` must precede the first table header: written after
    // TELEGRAM_BLOCK it lands under `[sidecar_channels.env]`, not at the
    // document root, and the included file is never scanned.
    std::fs::write(
        &config_path,
        format!("include = [\"channels.toml\"]\n{TELEGRAM_BLOCK}"),
    )
    .expect("seed config.toml");
    std::fs::write(&included_path, TELEGRAM_BLOCK).expect("seed channels.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    let root = std::fs::read_to_string(&config_path).expect("root config still present");
    let included = std::fs::read_to_string(&included_path).expect("included file still present");
    assert!(
        !root.contains("[[sidecar_channels]]") && !root.contains("name = \"telegram\""),
        "block must be gone from the root config: {root}"
    );
    assert!(
        !included.contains("[[sidecar_channels]]") && !included.contains("name = \"telegram\""),
        "block must be gone from the included file: {included}"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete must 404");
}

/// A channel declared two include-levels deep (`config.toml` -> `a.toml` ->
/// `channels.toml`), the case the old first-level-only substring scan could
/// not see at all. The kernel's own `load_config` resolves includes
/// recursively, so this channel is fully live and the dashboard offers its
/// delete button; the old scan found no block in `a.toml` (no
/// `[[sidecar_channels]]` substring there), concluded "declared nowhere",
/// and answered `removed` while the block in `channels.toml` survived and
/// was re-merged on the next reload.
#[tokio::test(flavor = "multi_thread")]
async fn delete_strips_a_sidecar_declared_two_include_levels_deep() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    let mid_path = h.home.join("a.toml");
    let leaf_path = h.home.join("channels.toml");
    std::fs::write(&config_path, "include = [\"a.toml\"]\n").expect("seed config.toml");
    std::fs::write(&mid_path, "include = [\"channels.toml\"]\n").expect("seed a.toml");
    std::fs::write(&leaf_path, EMAIL_BLOCK).expect("seed channels.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "removed");

    let leaf = std::fs::read_to_string(&leaf_path).expect("leaf include still present");
    assert!(
        !leaf.contains("[[sidecar_channels]]") && !leaf.contains("name = \"email\""),
        "block must be gone from the two-levels-deep include: {leaf}"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "second delete must be a real 404, not a repeat of the still-present block"
    );
}

/// The multi-file walk must be all-or-nothing. `telegram` is declared in
/// both the root config and an included file; the included file's
/// `sidecar_channels` is a scalar string rather than an array-of-tables, so
/// `remove_sidecar_block` errors on it deterministically (independent of
/// filesystem permissions or run-as-root) — but only *after* the root has
/// already been stripped and committed to disk, since the root is processed
/// first. Without a snapshot/restore across the whole set, the request
/// 500s with the root's block already gone and no way to retry short of a
/// hand-edit; with it, the root is rolled back to exactly its pre-request
/// bytes.
#[tokio::test(flavor = "multi_thread")]
async fn delete_restores_the_root_when_a_later_include_write_fails() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    let included_path = h.home.join("channels.toml");
    let root_before = format!("include = [\"channels.toml\"]\n{TELEGRAM_BLOCK}");
    std::fs::write(&config_path, &root_before).expect("seed config.toml");
    std::fs::write(&included_path, "sidecar_channels = \"not-a-table\"\n")
        .expect("seed channels.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(
        status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "the malformed include must fail the request; body: {}",
        String::from_utf8_lossy(&body)
    );

    assert_eq!(
        std::fs::read_to_string(&config_path).expect("root config still present"),
        root_before,
        "the root's already-committed strip must be rolled back when a later file fails"
    );
    assert_eq!(
        std::fs::read_to_string(&included_path).expect("included file still present"),
        "sidecar_channels = \"not-a-table\"\n",
        "the malformed include must be left exactly as it was"
    );
}

/// Adapter that only exists to occupy a `channel_adapters_ref()` key; the
/// orphan-convergence test below never calls `start`/`send`/`stop`.
struct OrphanAdapter {
    name: String,
}

#[async_trait]
impl ChannelAdapter for OrphanAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Email
    }

    async fn start(
        &self,
    ) -> Result<
        Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        Ok(Box::pin(futures::stream::empty::<ChannelMessage>()))
    }

    async fn send(
        &self,
        _user: &ChannelUser,
        _content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

/// `has_adapter`-only orphan: nothing in `sidecar_channels` (neither on disk
/// nor in the kernel's in-memory config), but a stale adapter is still
/// registered under both the plain and a qualified key. The qualified key is
/// `{name}:{account_id}` (`channel_bridge.rs` — `format!("{name}:{aid}")`),
/// where `aid` is the *adapter's* account id; `email:email` is the shape a
/// single-account deployment happens to produce, and
/// `delete_clears_a_stale_adapter_entry_keyed_by_a_real_account_id` covers the
/// multi-account one that the removal used to miss.
///
/// Before the fix, `HotAction::ReloadChannels` is the only thing that clears
/// `channel_adapters_ref()`, and it never dispatches here because the
/// reload's plan diff is empty (memory and disk both already say "no such
/// channel"). Without a direct removal, `has_adapter` stays `true` forever
/// and a repeat delete keeps taking the reconcile branch and 200s instead of
/// 404ing once the orphan is actually gone.
#[tokio::test(flavor = "multi_thread")]
async fn delete_clears_a_stale_adapter_entry_with_no_config_anywhere() {
    let h = boot_router().await;
    std::fs::write(h.home.join("config.toml"), "# no sidecar_channels\n").expect("seed config");

    let adapter: Arc<dyn ChannelAdapter> = Arc::new(OrphanAdapter {
        name: "email".to_string(),
    });
    h.state
        .kernel
        .channel_adapters_ref()
        .insert("email".to_string(), adapter.clone());
    h.state
        .kernel
        .channel_adapters_ref()
        .insert("email:email".to_string(), adapter);

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an adapter-only orphan must be deletable; body: {}",
        String::from_utf8_lossy(&body)
    );

    assert!(
        !h.state.kernel.channel_adapters_ref().contains_key("email"),
        "the plain adapter key must be cleared directly, not left for a reload that never fires"
    );
    assert!(
        !h.state
            .kernel
            .channel_adapters_ref()
            .contains_key("email:email"),
        "the qualified adapter key must be cleared directly, not left for a reload that never fires"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "once the orphan is actually gone, a repeat delete must converge to 404"
    );
}

/// Regression (#7971 review M5): the direct removal used to take the plain key
/// plus a guessed `{name}:{name}`, which only matches when the adapter's
/// `account_id` happens to equal the channel name. `channel_bridge.rs` builds
/// the qualified key as `format!("{name}:{aid}")` from the adapter's own
/// account id, so on any multi-account deployment the real entry survived and
/// the orphan never converged — the exact failure this branch was added to fix.
/// A neighbouring channel whose name merely shares the prefix must not be
/// caught by the same sweep.
#[tokio::test(flavor = "multi_thread")]
async fn delete_clears_a_stale_adapter_entry_keyed_by_a_real_account_id() {
    let h = boot_router().await;
    std::fs::write(h.home.join("config.toml"), "# no sidecar_channels\n").expect("seed config");

    let adapter: Arc<dyn ChannelAdapter> = Arc::new(OrphanAdapter {
        name: "email".to_string(),
    });
    let neighbour: Arc<dyn ChannelAdapter> = Arc::new(OrphanAdapter {
        name: "email-support".to_string(),
    });
    let adapters = h.state.kernel.channel_adapters_ref();
    adapters.insert("email".to_string(), adapter.clone());
    adapters.insert("email:acct-42".to_string(), adapter.clone());
    adapters.insert("email:acct-7".to_string(), adapter);
    adapters.insert("email-support:acct-1".to_string(), neighbour);

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an adapter-only orphan must be deletable; body: {}",
        String::from_utf8_lossy(&body)
    );

    for key in ["email", "email:acct-42", "email:acct-7"] {
        assert!(
            !h.state.kernel.channel_adapters_ref().contains_key(key),
            "`{key}` must be cleared — the qualified key is keyed by the adapter's \
             account id, not by the channel name"
        );
    }
    assert!(
        h.state
            .kernel
            .channel_adapters_ref()
            .contains_key("email-support:acct-1"),
        "a different channel that merely shares the name prefix must survive"
    );

    let (status, _) = send(h.app.clone(), auth_delete("/api/channels/sidecar/email")).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "once every key this channel owns is gone, a repeat delete must converge to 404"
    );
}

/// Deleting a root-declared channel while an included file declares another
/// must not state a root-level `sidecar_channels = []`.
///
/// The empty section the reload overlay (#8459/#8460) needs to express a
/// deletion is a per-document statement, and the root wins the include merge:
/// a root-level `[]` written while the include still declares entries would
/// replace the include's whole list and silently drop a channel whose delete
/// nobody asked for. The explicit empty belongs in this walk only when no
/// reachable file states the section any more.
#[tokio::test(flavor = "multi_thread")]
async fn delete_from_the_root_does_not_shadow_an_included_channels_sibling() {
    let h = boot_router().await;
    let config_path = h.home.join("config.toml");
    let included_path = h.home.join("channels.toml");
    std::fs::write(
        &config_path,
        format!("include = [\"channels.toml\"]\n{TELEGRAM_BLOCK}"),
    )
    .expect("seed config.toml");
    std::fs::write(&included_path, EMAIL_BLOCK).expect("seed channels.toml");

    let (status, body) = send(h.app.clone(), auth_delete("/api/channels/sidecar/telegram")).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "body: {}",
        String::from_utf8_lossy(&body)
    );

    let root = std::fs::read_to_string(&config_path).expect("root config still present");
    let included = std::fs::read_to_string(&included_path).expect("included file still present");
    assert!(
        !root.contains("[[sidecar_channels]]") && !root.contains("name = \"telegram\""),
        "telegram's block must be gone from the root: {root}"
    );
    assert!(
        !root.contains("sidecar_channels = []"),
        "a root-level empty array would shadow the include's entries: {root}"
    );
    assert!(
        included.contains("[[sidecar_channels]]") && included.contains("name = \"email\""),
        "the include's unrelated channel must be untouched on disk: {included}"
    );
    assert!(
        h.state
            .kernel
            .config_ref()
            .sidecar_channels
            .iter()
            .any(|sc| sc.name == "email"),
        "the reload must keep the include's channel live instead of dropping it \
         behind the deletion's empty section"
    );
}
