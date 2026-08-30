//! Native browser automation via Chrome DevTools Protocol (CDP).
//!
//! Direct WebSocket connection to Chromium. No Python, no Playwright.
//! Launches a Chromium process, connects over CDP WebSocket, and sends
//! JSON-RPC commands for navigation, interaction, screenshots, etc.
//!
//! # Security
//! - SSRF check runs in Rust before navigate commands
//! - All page content wrapped with `wrap_external_content()` markers
//! - Session limits: max concurrent, idle timeout, 1 per agent
//! - No subprocess bridge, no env leakage, no Python code execution

use dashmap::DashMap;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use librefang_types::config::BrowserConfig;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::io::AsyncBufReadExt;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info, warn};

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn run_chromium_discovery_job<F, T>(job: F) -> Result<T, tokio::task::JoinError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(job).await
}

// ── Constants ──────────────────────────────────────────────────────────────

const CDP_CONNECT_TIMEOUT_SECS: u64 = 15;
const CDP_COMMAND_TIMEOUT_SECS: u64 = 30;
const PAGE_LOAD_POLL_INTERVAL_MS: u64 = 200;
const PAGE_LOAD_MAX_POLLS: u32 = 150; // 30 seconds
const BROWSER_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "TMPDIR",
    "APPDATA",
    "LOCALAPPDATA",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
];

// ── Public types ───────────────────────────────────────────────────────────

/// Command sent to the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum BrowserCommand {
    Navigate { url: String },
    Click { selector: String },
    Type { selector: String, text: String },
    Screenshot,
    ReadPage,
    Close,
    Scroll { direction: String, amount: i32 },
    Wait { selector: String, timeout_ms: u64 },
    RunJs { expression: String },
    Back,
}

/// Response from a browser command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl BrowserResponse {
    fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
    fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

// ── CDP connection ─────────────────────────────────────────────────────────

/// Build one CDP wire message.
///
/// `params` is always emitted, even when empty — some CDP servers (Lightpanda) reject commands that omit the key.
/// `sessionId` is added only when the command targets an attached session.
fn build_cdp_message(
    id: u64,
    method: &str,
    params: serde_json::Value,
    session_id: Option<&str>,
) -> serde_json::Value {
    let mut msg = serde_json::json!({ "id": id, "method": method, "params": params });
    if let Some(sid) = session_id {
        msg["sessionId"] = serde_json::Value::String(sid.to_string());
    }
    msg
}

/// Low-level Chrome DevTools Protocol connection over WebSocket.
struct CdpConnection {
    write: Arc<Mutex<SplitSink<WsStream, WsMessage>>>,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>>,
    next_id: AtomicU64,
    /// Flattened CDP session, set when we attached to a target over this connection.
    /// While present, every command carries `sessionId` so the browser routes it to that target instead of the browser itself.
    session_id: Option<String>,
    _reader_handle: tokio::task::JoinHandle<()>,
}

impl CdpConnection {
    /// Connect to a CDP WebSocket endpoint.
    async fn connect(ws_url: &str) -> Result<Self, String> {
        let (stream, _) = tokio::time::timeout(
            Duration::from_secs(CDP_CONNECT_TIMEOUT_SECS),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .map_err(|_| format!("CDP WebSocket connect timed out: {ws_url}"))?
        .map_err(|e| format!("CDP WebSocket connect failed: {e}"))?;
        Self::from_stream(stream)
    }

    /// Wrap an already-connected WebSocket stream in a CdpConnection.
    fn from_stream(stream: WsStream) -> Result<Self, String> {
        let (write, read) = stream.split();
        let write = Arc::new(Mutex::new(write));
        let pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>> =
            Arc::new(DashMap::new());

        let reader_pending = Arc::clone(&pending);
        let reader_handle = tokio::spawn(Self::reader_loop(read, reader_pending));

        Ok(Self {
            write,
            pending,
            next_id: AtomicU64::new(1),
            session_id: None,
            _reader_handle: reader_handle,
        })
    }

    /// Background task: read WebSocket messages and route responses.
    async fn reader_loop(
        mut read: SplitStream<WsStream>,
        pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>>,
    ) {
        while let Some(msg) = read.next().await {
            let text = match msg {
                Ok(WsMessage::Text(t)) => t.to_string(),
                Ok(WsMessage::Close(_)) => break,
                Err(e) => {
                    debug!("CDP WebSocket read error: {e}");
                    break;
                }
                _ => continue,
            };

            let json: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Route response to waiting caller by id
            if let Some(id) = json.get("id").and_then(|v| v.as_u64()) {
                if let Some((_, sender)) = pending.remove(&id) {
                    if let Some(error) = json.get("error") {
                        let msg = error["message"].as_str().unwrap_or("CDP error").to_string();
                        let _ = sender.send(Err(msg));
                    } else {
                        let result = json
                            .get("result")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let _ = sender.send(Ok(result));
                    }
                }
            }
            // Events (method field, no id) are ignored for now.
            // Future: handle Fetch.requestPaused for CDP-level SSRF.
        }

        // The socket is gone, so nothing will ever answer the commands still waiting on it.
        // Left alone they sit until `CDP_COMMAND_TIMEOUT_SECS` elapses and then report a timeout, which reads as "the browser is slow" rather than "the connection died" — and costs 30s per in-flight command to say the wrong thing.
        let waiting: Vec<u64> = pending.iter().map(|e| *e.key()).collect();
        for id in waiting {
            if let Some((_, sender)) = pending.remove(&id) {
                let _ = sender.send(Err("CDP connection closed".to_string()));
            }
        }
    }

    /// Send a CDP command and wait for the response.
    ///
    /// Routed to the attached target when a flattened session exists.
    async fn send(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.dispatch(method, params, self.session_id.as_deref())
            .await
    }

    /// Send a CDP command to the browser itself, bypassing any attached session.
    /// Needed for `Target.*`, which the browser handles and a page session does not.
    async fn send_browser(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.dispatch(method, params, None).await
    }

    /// Frame, send, and await one CDP command.
    async fn dispatch(
        &self,
        method: &str,
        params: serde_json::Value,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);

        let msg = build_cdp_message(id, method, params, session_id);
        self.write
            .lock()
            .await
            .send(WsMessage::Text(msg.to_string().into()))
            .await
            .map_err(|e| format!("CDP send failed: {e}"))?;

        match tokio::time::timeout(Duration::from_secs(CDP_COMMAND_TIMEOUT_SECS), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("CDP response channel closed".to_string()),
            Err(_) => {
                self.pending.remove(&id);
                Err("CDP command timed out".to_string())
            }
        }
    }

    /// Evaluate JavaScript in the browser page and return the value.
    async fn run_js(&self, expression: &str) -> Result<serde_json::Value, String> {
        let result = self
            .send(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;

        // Check for JS exceptions
        if let Some(desc) = result
            .get("exceptionDetails")
            .and_then(|e| e.get("text"))
            .and_then(|t| t.as_str())
        {
            return Err(format!("JS error: {desc}"));
        }

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }
}

impl Drop for CdpConnection {
    fn drop(&mut self) {
        self._reader_handle.abort();
    }
}

// ── HTTP target discovery ──────────────────────────────────────────────────

/// Ask a Chrome-style HTTP discovery endpoint for a fresh tab.
///
/// Returns the tab's WebSocket URL and its target ID, when the endpoint reports one.
///
/// `/json/new` is requested with PUT.
/// Chrome has required that verb since 111 and answers a GET with `405 Method Not Allowed` and the body "Using unsafe HTTP verb GET to invoke /json/new. This action supports only PUT verb.", which this code used to walk straight into.
/// Older Chromium builds and third-party CDP proxies may route only GET, so a 405 falls back to it rather than failing the session.
async fn http_discover_target(cdp_endpoint: &str) -> Result<(String, Option<String>), String> {
    let base = cdp_endpoint.trim_end_matches('/');
    let new_url = format!("{base}/json/new");
    let client = crate::http_client::new_client();

    let send = |req: reqwest::RequestBuilder| async {
        tokio::time::timeout(Duration::from_secs(CDP_CONNECT_TIMEOUT_SECS), req.send())
            .await
            .map_err(|_| format!("Timed out connecting to CDP endpoint: {cdp_endpoint}"))?
            .map_err(|e| format!("Failed to reach CDP endpoint {cdp_endpoint}: {e}"))
    };

    let mut resp = send(client.put(&new_url)).await?;
    if resp.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
        debug!("PUT /json/new rejected with 405; retrying with GET");
        resp = send(client.get(&new_url)).await?;
    }

    let status = resp.status();
    if !status.is_success() {
        return Err(format!("/json/new returned {status}"));
    }

    let target: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Invalid JSON from /json/new: {e}"))?;

    let page_ws = target["webSocketDebuggerUrl"]
        .as_str()
        .ok_or("Missing webSocketDebuggerUrl in /json/new response")?
        .to_string();
    let target_id = target["id"].as_str().map(|s| s.to_string());
    Ok((page_ws, target_id))
}

// ── Browser session ────────────────────────────────────────────────────────

/// A live browser session: one CDP connection per agent.
///
/// `process` is `Some` for locally-spawned Chromium, `None` when attaching to
/// a remote CDP endpoint (the operator manages the browser lifecycle).
/// `attached_target_id` tracks the tab created during attach so it can be
/// closed when the session ends.
struct BrowserSession {
    process: Option<tokio::process::Child>,
    cdp: CdpConnection,
    #[allow(dead_code)]
    last_active: Instant,
    _user_data_dir: Option<TempDir>,
    /// Target ID of the tab created during attach mode.
    /// `None` for local-launch sessions.
    attached_target_id: Option<String>,
    /// Whether that tab was created over CDP (`Target.createTarget`) instead of HTTP discovery.
    /// Decides how it gets closed — a `ws://` endpoint has no `/json/close` route to call.
    target_created_over_cdp: bool,
}

fn navigation_error_text(result: &serde_json::Value) -> Option<&str> {
    result
        .get("errorText")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

impl BrowserSession {
    /// Launch Chromium and establish a CDP connection.
    async fn launch(config: &BrowserConfig) -> Result<Self, String> {
        let discovery_config = config.clone();
        let chrome_path = run_chromium_discovery_job(move || find_chromium(&discovery_config))
            .await
            .map_err(|error| format!("Chromium discovery task failed: {error}"))??;
        debug!(path = %chrome_path.display(), "Launching Chromium");

        let user_data_dir = tempfile::Builder::new()
            .prefix("librefang-chrome-")
            .tempdir()
            .map_err(|e| format!("Failed to create Chromium user-data-dir: {e}"))?;
        // `mut` is only exercised by the Linux root/--no-sandbox branch below.
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut args = chromium_launch_args(config, user_data_dir.path());

        // On Linux, Chromium refuses to start as root without --no-sandbox.
        // This is common in Docker containers and server installs.
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::MetadataExt;
            let is_root = std::fs::metadata("/proc/self")
                .map(|m| m.uid() == 0)
                .unwrap_or(false);
            if is_root {
                warn!("Running as root — adding --no-sandbox flag for Chromium");
                args.push("--no-sandbox".to_string());
            }
        }

        let mut cmd = tokio::process::Command::new(&chrome_path);
        cmd.args(&args);
        cmd.stderr(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());
        cmd.stdin(std::process::Stdio::null());

        // SECURITY: clear environment, pass only essentials
        cmd.env_clear();
        for key in BROWSER_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to launch Chromium at {}: {e}",
                chrome_path.display()
            )
        })?;

        // Parse stderr for the DevTools WebSocket URL
        let stderr = child.stderr.take().ok_or("No stderr from Chromium")?;
        let ws_url = Self::read_devtools_url(stderr).await?;
        debug!(ws_url = %ws_url, "Got CDP WebSocket URL");

        // GET /json/list to find the page target
        let port = ws_url
            .split("://")
            .nth(1)
            .and_then(|s| s.split(':').nth(1))
            .and_then(|s| s.split('/').next())
            .ok_or("Cannot parse port from CDP URL")?;
        let list_url = format!("http://127.0.0.1:{port}/json/list");

        // Try 127.0.0.1 first; fall back to localhost in case Chrome bound to IPv6
        let page_ws = match Self::find_page_ws(&list_url).await {
            Ok(ws) => ws,
            Err(original_err) => {
                let fallback_url = format!("http://localhost:{port}/json/list");
                debug!(
                    "127.0.0.1 unreachable ({}), falling back to localhost for /json/list",
                    original_err
                );
                Self::find_page_ws(&fallback_url).await?
            }
        };
        debug!(page_ws = %page_ws, "Connecting to page");

        let cdp = CdpConnection::connect(&page_ws).await?;

        // Enable required domains. If these fail, downstream navigation /
        // eval fails with no signal pointing back here — abort the session
        // now with a clear error instead (#5137).
        cdp.send("Page.enable", serde_json::json!({}))
            .await
            .map_err(|e| format!("CDP Page.enable failed: {e}"))?;
        cdp.send("Runtime.enable", serde_json::json!({}))
            .await
            .map_err(|e| format!("CDP Runtime.enable failed: {e}"))?;

        Ok(Self {
            process: Some(child),
            cdp,
            last_active: Instant::now(),
            _user_data_dir: Some(user_data_dir),
            attached_target_id: None,
            target_created_over_cdp: false,
        })
    }

    /// Attach to a remote CDP endpoint instead of spawning a local Chromium.
    ///
    /// Accepted formats for `cdp_endpoint`:
    /// - `http[s]://host:port` — HTTP discovery; `/json/new` creates a fresh tab (PUT, falling back to GET on a 405) and returns its WebSocket URL.
    ///   The created target ID is stored for cleanup when the session ends.
    /// - `ws[s]://…` — Direct WebSocket attach.
    ///   Works with both page-level endpoints and browser-level ones: on a browser-level endpoint we create a target and attach to it over CDP (see below).
    ///
    /// `auth_token` is sent as `Authorization: Bearer <token>` on the WS upgrade,
    /// for CDP proxies that require authentication (e.g. Browserless).
    async fn attach(cdp_endpoint: &str, auth_token: Option<&str>) -> Result<Self, String> {
        let page_ws: String;
        let mut target_id: Option<String> = None;
        // Only the ws:// path may land on a browser-level endpoint; HTTP discovery always hands back a page.
        let mut needs_target_handshake = false;
        let mut cdp_created_target = false;

        let lower = cdp_endpoint.to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            let (ws, id) = http_discover_target(cdp_endpoint).await?;
            page_ws = ws;
            target_id = id;
            debug!(ws = %page_ws, "Attached via HTTP discovery (/json/new)");
        } else if lower.starts_with("ws://") || lower.starts_with("wss://") {
            page_ws = cdp_endpoint.to_string();
            needs_target_handshake = true;
            debug!(ws = %page_ws, "Attaching to CDP WebSocket directly");
        } else {
            return Err(format!(
                "Unsupported cdp_endpoint scheme. Use http://, https://, ws://, or wss://. Got: {cdp_endpoint}"
            ));
        }

        let mut cdp = Self::connect_with_auth(&page_ws, auth_token).await?;

        // A `ws://` endpoint may be page-level or browser-level, and the two need different setup.
        // Ask the endpoint which it is: `Target.getTargetInfo` reports `type: "page"` on a page-level connection and `type: "browser"` on a browser-level one.
        // Verified against Chrome 148 (both shapes) and Lightpanda, which reports `browser`.
        //
        // Reading the answer off the protocol rather than inferring it from a failed command matters for what happens when something goes wrong.
        // A transient error — a busy target, a proxy hiccup, a timeout — is not evidence about the shape of the endpoint, and treating it as such on a page-level connection would create a second blank tab and move the session onto it, which is the exact regression this handshake is meant to avoid.
        // Anything other than a definite `browser` therefore stays on the page-level path, so an endpoint that does not implement `Target.getTargetInfo` keeps behaving exactly as it did before this handshake existed.
        //
        // Asking costs one command that was not sent before, and a strict server is free to answer an unknown method by dropping the socket rather than returning a JSON-RPC error.
        // The reply says nothing about which of the two happened, so the failure path reconnects unconditionally before continuing: on a live socket that is one redundant connect, and on a dropped one it is the difference between the page-level fallback working and `attach()` failing where it used to succeed.
        if needs_target_handshake {
            let endpoint_kind = match cdp
                .send_browser("Target.getTargetInfo", serde_json::json!({}))
                .await
            {
                Ok(info) => info["targetInfo"]["type"]
                    .as_str()
                    .unwrap_or("page")
                    .to_string(),
                Err(e) => {
                    debug!(error = %e, "Target.getTargetInfo failed; reconnecting and treating endpoint as page-level");
                    cdp = Self::connect_with_auth(&page_ws, auth_token).await?;
                    "page".to_string()
                }
            };

            if endpoint_kind == "browser" {
                debug!("Endpoint reports type=browser; creating a target to attach to");
                let created = cdp
                    .send_browser(
                        "Target.createTarget",
                        serde_json::json!({ "url": "about:blank" }),
                    )
                    .await
                    .map_err(|e| format!("CDP Target.createTarget failed: {e}"))?;
                let tid = created["targetId"]
                    .as_str()
                    .ok_or("Target.createTarget returned no targetId")?
                    .to_string();

                // The target now exists on the remote browser and no `BrowserSession` tracks it yet, so every failure from here on has to close it before propagating.
                // Otherwise a partial handshake abandons a blank tab that nothing will ever reap — on a long-lived remote endpoint it accumulates one per failed attach.
                match Self::attach_to_target(&mut cdp, &tid).await {
                    Ok(()) => {
                        target_id = Some(tid);
                        cdp_created_target = true;
                    }
                    Err(e) => {
                        Self::close_target_best_effort(&cdp, &tid).await;
                        return Err(e);
                    }
                }
            }
        }

        // Enable required domains (same as launch).
        // Abort on failure so the caller gets a clear error rather than a later opaque nav/eval failure (#5137).
        if let Err(e) = Self::enable_domains(&cdp).await {
            // Both creation paths leave a tab behind that no `BrowserSession` will ever track, so both have to close it — they just need different routes.
            match (cdp_created_target, target_id.as_deref()) {
                (true, Some(tid)) => Self::close_target_best_effort(&cdp, tid).await,
                (false, Some(tid)) => {
                    Self::close_http_target_best_effort(cdp_endpoint, tid, None).await
                }
                _ => {}
            }
            return Err(e);
        }

        Ok(Self {
            process: None,
            cdp,
            last_active: Instant::now(),
            _user_data_dir: None,
            attached_target_id: target_id,
            target_created_over_cdp: cdp_created_target,
        })
    }

    /// Close a tab obtained through HTTP discovery, ignoring failures.
    ///
    /// The CDP-created path uses `Target.closeTarget`, which a `ws://` endpoint understands; a tab from `/json/new` is owned by the HTTP side and is closed the way `close_session` closes it.
    /// Shared between the failed-attach cleanup path here and `close_session`'s normal teardown, which both close the same kind of tab through the same route and previously duplicated this match.
    async fn close_http_target_best_effort(cdp_endpoint: &str, tid: &str, agent_id: Option<&str>) {
        let base = cdp_endpoint.trim_end_matches('/');
        let close_url = format!("{base}/json/close/{tid}");
        let agent_id = agent_id.unwrap_or("");
        match crate::http_client::new_client()
            .get(&close_url)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                debug!(agent_id, target = %tid, "Closed HTTP-discovered tab");
            }
            Ok(resp) => {
                warn!(agent_id, target = %tid, status = %resp.status(), "Closing HTTP-discovered tab returned non-2xx; tab may leak");
            }
            Err(e) => {
                warn!(agent_id, target = %tid, error = %e, "Failed to close HTTP-discovered tab; tab may leak");
            }
        }
    }

    /// Attach to an already-created target in flattened mode and record the session on `cdp`.
    ///
    /// Split out of `attach()` so its caller can close the target on any failure here.
    async fn attach_to_target(cdp: &mut CdpConnection, tid: &str) -> Result<(), String> {
        let attached = cdp
            .send_browser(
                "Target.attachToTarget",
                serde_json::json!({ "targetId": tid, "flatten": true }),
            )
            .await
            .map_err(|e| format!("CDP Target.attachToTarget failed: {e}"))?;
        let sid = attached["sessionId"]
            .as_str()
            .ok_or("Target.attachToTarget returned no sessionId")?
            .to_string();
        debug!(target = %tid, session = %sid, "Attached to CDP target");
        cdp.session_id = Some(sid);
        Ok(())
    }

    /// Enable the CDP domains every session needs.
    async fn enable_domains(cdp: &CdpConnection) -> Result<(), String> {
        cdp.send("Page.enable", serde_json::json!({}))
            .await
            .map_err(|e| format!("CDP Page.enable failed: {e}"))?;
        cdp.send("Runtime.enable", serde_json::json!({}))
            .await
            .map_err(|e| format!("CDP Runtime.enable failed: {e}"))?;
        Ok(())
    }

    /// Close a target we created, logging rather than failing when the close itself fails.
    ///
    /// Used on setup paths that are already returning an error, where the original failure is the one worth surfacing.
    async fn close_target_best_effort(cdp: &CdpConnection, tid: &str) {
        if let Err(e) = cdp
            .send_browser("Target.closeTarget", serde_json::json!({ "targetId": tid }))
            .await
        {
            warn!(target = %tid, error = %e, "Failed to close CDP target after a failed attach; tab may leak");
        }
    }

    /// Connect with an optional bearer auth token (for CDP proxies like Browserless).
    async fn connect_with_auth(
        ws_url: &str,
        auth_token: Option<&str>,
    ) -> Result<CdpConnection, String> {
        if let Some(token) = auth_token {
            let mut req = ws_url
                .into_client_request()
                .map_err(|e| format!("Failed to build CDP auth request: {e}"))?;
            let authorization = http::HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|e| format!("Failed to build CDP auth request: {e}"))?;
            req.headers_mut()
                .insert(http::header::AUTHORIZATION, authorization);
            let (stream, _) = tokio::time::timeout(
                Duration::from_secs(CDP_CONNECT_TIMEOUT_SECS),
                tokio_tungstenite::connect_async(req),
            )
            .await
            .map_err(|_| format!("CDP WebSocket connect timed out: {ws_url}"))?
            .map_err(|e| format!("CDP WebSocket connect failed: {e}"))?;
            CdpConnection::from_stream(stream)
        } else {
            CdpConnection::connect(ws_url).await
        }
    }

    /// Read stderr until we find "DevTools listening on ws://...".
    async fn read_devtools_url(stderr: tokio::process::ChildStderr) -> Result<String, String> {
        let reader = tokio::io::BufReader::new(stderr);
        let mut lines = reader.lines();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(CDP_CONNECT_TIMEOUT_SECS);

        loop {
            let line = tokio::time::timeout_at(deadline, lines.next_line())
                .await
                .map_err(|_| {
                    "Timed out waiting for Chromium to start. Is Chrome/Chromium installed?"
                        .to_string()
                })?
                .map_err(|e| format!("Failed to read Chromium stderr: {e}"))?;

            match line {
                Some(l) if l.contains("DevTools listening on") => {
                    let url = l
                        .split("DevTools listening on ")
                        .nth(1)
                        .ok_or("Malformed DevTools URL line")?
                        .trim()
                        .to_string();
                    return Ok(url);
                }
                Some(_) => continue,
                None => {
                    return Err(
                        "Chromium exited before printing DevTools URL. Is Chrome installed?"
                            .to_string(),
                    );
                }
            }
        }
    }

    /// Fetch /json/list and find the page WebSocket URL.
    async fn find_page_ws(list_url: &str) -> Result<String, String> {
        for attempt in 0..10 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            let resp = match crate::http_client::new_client().get(list_url).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };
            let targets: Vec<serde_json::Value> = match resp.json().await {
                Ok(t) => t,
                Err(_) => continue,
            };
            for target in &targets {
                if target["type"].as_str() == Some("page") {
                    if let Some(ws) = target["webSocketDebuggerUrl"].as_str() {
                        return Ok(ws.to_string());
                    }
                }
            }
        }
        Err("No page target found in Chromium".to_string())
    }

    /// Execute a browser command via CDP.
    /// `max_content_chars` is passed per call rather than copied onto the session at construction: the value only ever reaches the extraction script, and a session lives across many commands, so caching it here would add a second freeze layer on top of the one `BrowserManager` already has.
    ///
    /// That outer freeze is why `[browser]` is classified restart-required in `config_reload.rs` — `BrowserManager` captures `BrowserConfig` by value at boot with no rebuild path, so this knob follows the same rule as `max_sessions` and `cdp_endpoint` and takes effect on daemon restart.
    async fn execute(&mut self, cmd: BrowserCommand, max_content_chars: usize) -> BrowserResponse {
        self.last_active = Instant::now();
        match cmd {
            BrowserCommand::Navigate { url } => self.cmd_navigate(&url, max_content_chars).await,
            BrowserCommand::Click { selector } => {
                self.cmd_click(&selector, max_content_chars).await
            }
            BrowserCommand::Type { selector, text } => self.cmd_type(&selector, &text).await,
            BrowserCommand::Screenshot => self.cmd_screenshot().await,
            BrowserCommand::ReadPage => self.cmd_read_page(max_content_chars).await,
            BrowserCommand::Close => BrowserResponse::ok(serde_json::json!({"closed": true})),
            BrowserCommand::Scroll { direction, amount } => {
                self.cmd_scroll(&direction, amount).await
            }
            BrowserCommand::Wait {
                selector,
                timeout_ms,
            } => self.cmd_wait(&selector, timeout_ms).await,
            BrowserCommand::RunJs { expression } => self.cmd_run_js(&expression).await,
            BrowserCommand::Back => self.cmd_back(max_content_chars).await,
        }
    }

    // ── Command implementations ────────────────────────────────────────

    async fn cmd_navigate(&self, url: &str, max_content_chars: usize) -> BrowserResponse {
        let result = self
            .cdp
            .send("Page.navigate", serde_json::json!({ "url": url }))
            .await;
        let result = match result {
            Ok(result) => result,
            Err(error) => return BrowserResponse::err(format!("Navigate failed: {error}")),
        };
        if let Some(error) = navigation_error_text(&result) {
            return BrowserResponse::err(format!("Navigate failed: {error}"));
        }

        // Wait for page load
        self.wait_for_load().await;

        match self.page_info(max_content_chars).await {
            Ok(info) => BrowserResponse::ok(info),
            Err(e) => BrowserResponse::err(format!("Navigate succeeded but page info failed: {e}")),
        }
    }

    async fn cmd_click(&self, selector: &str, max_content_chars: usize) -> BrowserResponse {
        if selector.trim().is_empty() {
            return BrowserResponse::err("Click selector cannot be empty");
        }
        // A `⟨n⟩` marker from the extracted content identifies one exact link, which the text fallback below cannot: matching on a substring of the link text resolves to the wrong element for 28% of the links on a page like Hacker News, because the first element *containing* that text wins.
        // The brackets are what the extraction emits, so a bracketed selector is unambiguous and is taken here.
        if let Some(id) = parse_link_marker(selector) {
            return self.click_link_marker(id, max_content_chars).await;
        }
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    let sel = {sel_json};
    let el = null;
    try {{ el = document.querySelector(sel); }} catch (_) {{}}
    if (!el) {{
        const all = document.querySelectorAll('a, button, [role="button"], input[type="submit"], [onclick]');
        const lower = sel.toLowerCase();
        for (const e of all) {{
            if (e.textContent.trim().toLowerCase().includes(lower)) {{ el = e; break; }}
        }}
    }}
    if (!el) return JSON.stringify({{success: false, error: 'Element not found: ' + sel}});
    el.scrollIntoView({{block: 'center'}});
    el.click();
    return JSON.stringify({{success: true, tag: el.tagName, text: el.textContent.substring(0, 100).trim()}});
}})()"#
        );

        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                if parsed["success"].as_bool() == Some(false) {
                    // A bare `12` is a marker only once nothing on the page answers to it as a selector.
                    // Taking it as a marker up front would capture a page's own numeric link text — a pagination `5`, a numbered tab — which the text path has always been able to click, and would click whichever link happens to hold that id instead, silently and with no way to tell.
                    // Trying it here keeps that path intact and still absorbs a model that copied the number out of the prose without its brackets.
                    if let Some(id) = parse_bare_marker(selector) {
                        let by_marker = self.click_link_marker(id, max_content_chars).await;
                        if by_marker.success {
                            return by_marker;
                        }
                    }
                    return BrowserResponse::err(
                        parsed["error"]
                            .as_str()
                            .unwrap_or("Click failed")
                            .to_string(),
                    );
                }
                // Wait briefly for any navigation triggered by click
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.wait_for_load().await;
                match self.page_info(max_content_chars).await {
                    Ok(info) => BrowserResponse::ok(info),
                    Err(_) => BrowserResponse::ok(parsed),
                }
            }
            Err(e) => BrowserResponse::err(format!("Click failed: {e}")),
        }
    }

    /// Click the link a `⟨n⟩` marker refers to.
    ///
    /// The marker's meaning is defined by the extraction script's numbering, so the script is re-run to resolve it rather than re-deriving the traversal here: a second copy of the walk would be free to drift from the one that produced the number.
    ///
    /// That rules out the *code* drifting, not the page.
    /// Ids are positions in a walk of the live DOM, so a page that changes between the read the model saw and this click — a lazy-loaded card, an infinite scroll, a banner unmounting — can hand the same id to a different link, and the click then lands somewhere real but unintended rather than erroring.
    /// Resolving against the table from the earlier read instead would trade that for a staler failure and needs the read's table carried on the session, which is a larger change than this one; the tool description tells the model to re-read a page it expects to have moved.
    async fn click_link_marker(&self, id: usize, max_content_chars: usize) -> BrowserResponse {
        let extracted = match self
            .cdp
            .run_js(&extract_content_js(max_content_chars))
            .await
        {
            Ok(v) => v,
            Err(e) => return BrowserResponse::err(format!("Click failed: {e}")),
        };
        let parsed: serde_json::Value = extracted
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(extracted);
        let url = parsed["links"]
            .as_array()
            .and_then(|links| {
                links
                    .iter()
                    .find(|l| l["id"].as_u64() == Some(id as u64))
                    .and_then(|l| l["url"].as_str())
            })
            .map(|s| s.to_string());
        let Some(url) = url else {
            return BrowserResponse::err(format!(
                "No link {id} on this page; re-read the page for current link markers"
            ));
        };

        let url_json = serde_json::to_string(&url).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    // The table stores same-origin links as a path, so resolve before comparing.
    const want = new URL({url_json}, location.href).href;
    const el = Array.from(document.querySelectorAll('a[href]')).find(a => a.href === want);
    if (!el) return JSON.stringify({{success: false, error: 'Link no longer on page: ' + want}});
    el.scrollIntoView({{block: 'center'}});
    el.click();
    return JSON.stringify({{success: true, tag: el.tagName, url: want, text: el.textContent.substring(0, 100).trim()}});
}})()"#
        );
        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                if parsed["success"].as_bool() == Some(false) {
                    return BrowserResponse::err(
                        parsed["error"]
                            .as_str()
                            .unwrap_or("Click failed")
                            .to_string(),
                    );
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.wait_for_load().await;
                match self.page_info(max_content_chars).await {
                    Ok(info) => BrowserResponse::ok(info),
                    Err(_) => BrowserResponse::ok(parsed),
                }
            }
            Err(e) => BrowserResponse::err(format!("Click failed: {e}")),
        }
    }

    async fn cmd_type(&self, selector: &str, text: &str) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let text_json = serde_json::to_string(text).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    let sel = {sel_json};
    let txt = {text_json};
    let el = document.querySelector(sel);
    if (!el) return JSON.stringify({{success: false, error: 'Input not found: ' + sel}});
    el.focus();
    el.value = txt;
    el.dispatchEvent(new Event('input', {{bubbles: true}}));
    el.dispatchEvent(new Event('change', {{bubbles: true}}));
    return JSON.stringify({{success: true, selector: sel, typed: txt.length + ' chars'}});
}})()"#
        );

        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                if parsed["success"].as_bool() == Some(false) {
                    BrowserResponse::err(parsed["error"].as_str().unwrap_or("Type failed"))
                } else {
                    BrowserResponse::ok(parsed)
                }
            }
            Err(e) => BrowserResponse::err(format!("Type failed: {e}")),
        }
    }

    async fn cmd_screenshot(&self) -> BrowserResponse {
        match self
            .cdp
            .send(
                "Page.captureScreenshot",
                serde_json::json!({ "format": "png" }),
            )
            .await
        {
            Ok(result) => {
                let b64 = result["data"].as_str().unwrap_or("");
                let url = self
                    .cdp
                    .run_js("location.href")
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                BrowserResponse::ok(
                    serde_json::json!({"image_base64": b64, "url": url, "format": "png"}),
                )
            }
            Err(e) => BrowserResponse::err(format!("Screenshot failed: {e}")),
        }
    }

    async fn cmd_read_page(&self, max_content_chars: usize) -> BrowserResponse {
        match self
            .cdp
            .run_js(&extract_content_js(max_content_chars))
            .await
        {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                BrowserResponse::ok(parsed)
            }
            Err(e) => BrowserResponse::err(format!("ReadPage failed: {e}")),
        }
    }

    async fn cmd_scroll(&self, direction: &str, amount: i32) -> BrowserResponse {
        let (dx, dy) = match direction {
            "up" => (0, -amount),
            "down" => (0, amount),
            "left" => (-amount, 0),
            "right" => (amount, 0),
            _ => (0, amount),
        };
        let js = format!("window.scrollBy({dx}, {dy}); JSON.stringify({{scrollX: window.scrollX, scrollY: window.scrollY}})");
        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                BrowserResponse::ok(parsed)
            }
            Err(e) => BrowserResponse::err(format!("Scroll failed: {e}")),
        }
    }

    async fn cmd_wait(&self, selector: &str, timeout_ms: u64) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let max_ms = timeout_ms.min(30_000);
        let polls = (max_ms / PAGE_LOAD_POLL_INTERVAL_MS).max(1);

        for _ in 0..polls {
            let js = format!("document.querySelector({sel_json}) ? 'found' : null");
            if let Ok(val) = self.cdp.run_js(&js).await {
                if val.as_str() == Some("found") {
                    return BrowserResponse::ok(
                        serde_json::json!({"found": true, "selector": selector}),
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(PAGE_LOAD_POLL_INTERVAL_MS)).await;
        }

        BrowserResponse::err(format!(
            "Timed out waiting for selector: {selector} ({max_ms}ms)"
        ))
    }

    async fn cmd_run_js(&self, expression: &str) -> BrowserResponse {
        match self.cdp.run_js(expression).await {
            Ok(val) => BrowserResponse::ok(serde_json::json!({"result": val})),
            Err(e) => BrowserResponse::err(format!("JS execution failed: {e}")),
        }
    }

    async fn cmd_back(&self, max_content_chars: usize) -> BrowserResponse {
        match self.cdp.run_js("history.back(); 'ok'").await {
            Ok(_) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.wait_for_load().await;
                match self.page_info(max_content_chars).await {
                    Ok(info) => BrowserResponse::ok(info),
                    Err(e) => {
                        BrowserResponse::err(format!("Back succeeded but page info failed: {e}"))
                    }
                }
            }
            Err(e) => BrowserResponse::err(format!("Back failed: {e}")),
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    /// Poll until document.readyState is 'complete' or 'interactive'.
    async fn wait_for_load(&self) {
        for _ in 0..PAGE_LOAD_MAX_POLLS {
            if let Ok(val) = self.cdp.run_js("document.readyState").await {
                let state = val.as_str().unwrap_or("");
                if state == "complete" || state == "interactive" {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(PAGE_LOAD_POLL_INTERVAL_MS)).await;
        }
    }

    /// Get current page title, URL, and readable content.
    async fn page_info(&self, max_content_chars: usize) -> Result<serde_json::Value, String> {
        let info = self
            .cdp
            .run_js("JSON.stringify({title: document.title, url: location.href})")
            .await?;
        let parsed: serde_json::Value = info
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(info);

        let content_val = self
            .cdp
            .run_js(&extract_content_js(max_content_chars))
            .await
            .unwrap_or_default();
        let content_obj: serde_json::Value = content_val
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(content_val);
        let content_text = content_obj["content"].as_str().unwrap_or("");

        Ok(serde_json::json!({
            "title": parsed["title"],
            "url": parsed["url"],
            "content": content_text,
            // Separate fields rather than a section appended to `content`: a caller that reads `content` today is unaffected, `cmd_click` gets the marker-to-URL map machine-readable instead of parsing it back out of prose, and a caller that only needs to read can ignore them.
            "links": content_obj["links"].clone(),
            "links_base": content_obj["links_base"].clone(),
        }))
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        if let Some(ref mut child) = self.process {
            let _ = child.start_kill();
        }
        // Tabs created via /json/new in attach mode are closed asynchronously
        // by BrowserManager::close_session() before drop is called.
    }
}

fn chromium_launch_args(config: &BrowserConfig, user_data_dir: &Path) -> Vec<String> {
    let mut args = vec![
        "--remote-debugging-port=0".to_string(),
        "--remote-debugging-host=127.0.0.1".to_string(),
        format!("--user-data-dir={}", user_data_dir.display()),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--disable-extensions".to_string(),
        "--disable-background-networking".to_string(),
        "--disable-sync".to_string(),
        "--disable-translate".to_string(),
        "--disable-features=TranslateUI".to_string(),
        "--metrics-recording-only".to_string(),
        format!(
            "--window-size={},{}",
            config.viewport_width, config.viewport_height
        ),
        "--user-agent=Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36".to_string(),
        "about:blank".to_string(),
    ];
    if config.headless {
        args.insert(0, "--headless=new".to_string());
        args.push("--disable-gpu".to_string());
    }
    args
}

// ── Chromium discovery ─────────────────────────────────────────────────────

/// Find a Chromium-based browser binary on this system.
fn find_chromium(config: &BrowserConfig) -> Result<PathBuf, String> {
    // 1. User-configured path
    if let Some(ref path) = config.chromium_path {
        if !path.is_empty() {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
            return Err(format!("Configured chromium_path not found: {path}"));
        }
    }

    // 2. CHROME_PATH env var
    if let Ok(path) = std::env::var("CHROME_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Ok(p);
        }
    }

    // 3. Platform-specific search
    let candidates = chromium_candidates();
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Ok(p);
        }
    }

    // 4. Try PATH lookup
    for name in &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if let Ok(output) = std::process::Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
        // Windows: use where.exe
        #[cfg(windows)]
        if let Ok(output) = std::process::Command::new("where.exe").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
    }

    Err(
        "Chromium/Chrome not found. Install Chrome or set CHROME_PATH. \
         Checked: Chrome, Chromium, Edge, Brave in standard locations."
            .to_string(),
    )
}

/// Platform-specific candidate paths for Chromium-based browsers.
fn chromium_candidates() -> Vec<String> {
    // `mut` is exercised only under the windows / macOS / linux cfg branches
    // below; on other targets (Android, iOS, …) every branch is stripped and
    // the binding is never mutated. Accept the unused-mut on those targets
    // rather than gating each platform's import — the function is meant to
    // return an empty vec on unsupported targets.
    #[allow(unused_mut)]
    let mut paths = Vec::new();

    #[cfg(windows)]
    {
        let program_files = std::env::var("ProgramFiles").unwrap_or_default();
        let program_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();
        let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();

        for pf in &[&program_files, &program_files_x86] {
            if pf.is_empty() {
                continue;
            }
            paths.push(format!("{pf}\\Google\\Chrome\\Application\\chrome.exe"));
            paths.push(format!("{pf}\\Microsoft\\Edge\\Application\\msedge.exe"));
            paths.push(format!(
                "{pf}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe"
            ));
        }
        if !local_app.is_empty() {
            paths.push(format!(
                "{local_app}\\Google\\Chrome\\Application\\chrome.exe"
            ));
            paths.push(format!(
                "{local_app}\\Microsoft\\Edge\\Application\\msedge.exe"
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        paths.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
        paths.push("/Applications/Chromium.app/Contents/MacOS/Chromium".into());
        paths.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into());
        paths.push("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".into());
    }

    #[cfg(target_os = "linux")]
    {
        paths.push("/usr/bin/google-chrome".into());
        paths.push("/usr/bin/google-chrome-stable".into());
        paths.push("/usr/bin/chromium".into());
        paths.push("/usr/bin/chromium-browser".into());
        paths.push("/snap/bin/chromium".into());
        paths.push("/usr/bin/microsoft-edge".into());
        paths.push("/usr/bin/brave-browser".into());
    }

    paths
}

// ── Browser manager ────────────────────────────────────────────────────────

/// Manages browser sessions for all agents.
pub struct BrowserManager {
    sessions: DashMap<String, Arc<Mutex<BrowserSession>>>,
    config: BrowserConfig,
}

impl BrowserManager {
    /// Create a new BrowserManager with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            sessions: DashMap::new(),
            config,
        }
    }

    /// Check whether an agent has an active browser session.
    pub fn has_session(&self, agent_id: &str) -> bool {
        self.sessions.contains_key(agent_id)
    }

    /// Send a command to an agent's browser session (creating one if needed).
    pub async fn send_command(
        &self,
        agent_id: &str,
        cmd: BrowserCommand,
    ) -> Result<BrowserResponse, String> {
        let session = self.get_or_create(agent_id).await?;
        let mut guard = session.lock().await;
        let resp = guard.execute(cmd, self.config.max_content_chars).await;

        if !resp.success {
            if let Some(ref err) = resp.error {
                warn!(agent_id, error = %err, "Browser command failed");
            }
        }

        Ok(resp)
    }

    /// Close an agent's browser session.
    pub async fn close_session(&self, agent_id: &str) {
        if let Some((_, session)) = self.sessions.remove(agent_id) {
            // For attach mode: close the tab we created before dropping the session.
            {
                let guard = session.lock().await;
                if let (Some(target_id), true) = (
                    guard.attached_target_id.as_ref(),
                    guard.target_created_over_cdp,
                ) {
                    // Created over CDP — close it the same way.
                    // A ws:// endpoint has no /json/close route to call.
                    match guard
                        .cdp
                        .send_browser(
                            "Target.closeTarget",
                            serde_json::json!({ "targetId": target_id }),
                        )
                        .await
                    {
                        Ok(_) => debug!(agent_id, target_id, "Closed CDP target"),
                        Err(e) => warn!(
                            agent_id,
                            target_id,
                            error = %e,
                            "Failed to close CDP target; tab may leak"
                        ),
                    }
                } else if let Some(ref target_id) = guard.attached_target_id {
                    let cdp_endpoint = self.config.cdp_endpoint.as_deref().unwrap_or("");
                    BrowserSession::close_http_target_best_effort(
                        cdp_endpoint,
                        target_id,
                        Some(agent_id),
                    )
                    .await;
                }
            }
            drop(session);
            info!(agent_id, "Browser session closed");
        }
    }

    /// Clean up an agent's browser session (called after agent loop ends).
    pub async fn cleanup_agent(&self, agent_id: &str) {
        self.close_session(agent_id).await;
    }

    /// Get existing session or create a new one.
    async fn get_or_create(&self, agent_id: &str) -> Result<Arc<Mutex<BrowserSession>>, String> {
        if let Some(entry) = self.sessions.get(agent_id) {
            return Ok(Arc::clone(entry.value()));
        }

        if self.sessions.len() >= self.config.max_sessions {
            return Err(format!(
                "Maximum browser sessions reached ({}). Close an existing session first.",
                self.config.max_sessions
            ));
        }

        let session = if let Some(ref endpoint) = self.config.cdp_endpoint {
            let auth_token = self
                .config
                .cdp_auth_token_env
                .as_deref()
                .and_then(|var| std::env::var(var).ok());
            let session = BrowserSession::attach(endpoint, auth_token.as_deref()).await?;
            info!(agent_id, endpoint, "Browser session attached (remote CDP)");
            session
        } else {
            let session = BrowserSession::launch(&self.config).await?;
            info!(agent_id, "Browser session created (native CDP)");
            session
        };
        let arc = Arc::new(Mutex::new(session));
        self.sessions.insert(agent_id.to_string(), Arc::clone(&arc));
        Ok(arc)
    }
}

// ── Embedded JavaScript ────────────────────────────────────────────────────

/// JavaScript to extract readable page content as markdown.
/// Page-extraction script, with `__MAX_CONTENT_CHARS__` still to be substituted.
///
/// Use [`EXTRACT_CONTENT_JS`] rather than this template.
const EXTRACT_CONTENT_JS_TEMPLATE: &str = r#"(() => {
    const title = document.title || '';
    const url = location.href || '';
    const body = document.body;
    if (!body) return JSON.stringify({title, url, content: ''});

    const clone = body.cloneNode(true);
    const remove = ['script','style','nav','aside','iframe','noscript','svg','canvas'];
    remove.forEach(tag => clone.querySelectorAll(tag).forEach(el => el.remove()));
    // A header/footer is a page landmark only when it is not scoped to sectioning content.
    // Removing every descendant drops card titles, bylines, and links from article/feed layouts.
    clone.querySelectorAll('header,footer').forEach(el => {
        if (!el.closest('article,main,section,[role="main"]')) el.remove();
    });

    // `querySelector` returns the *first* match, which on a feed or a results page is one card rather than the list of them — measured at 13.7% of the page on a DuckDuckGo results page.
    // Repeated sibling `article` elements are the signature of that shape, so climb to the ancestor that holds them, the way Readability resolves the same case by walking to the common ancestor of its close-scoring candidates.
    // The test is on a node's *direct children*: at least three of them must each carry an article of their own.
    // Merely containing three articles somewhere below is not the same shape and reaches too far — an ordinary post page whose "related posts" widget is built out of `article` clears that bar, and selection then lands on the whole document and pulls the widget in beside the post.
    // Counting article-carrying children instead sees two there, one for the post and one for the widget, so selection falls through to the single-article path that page always had.
    // Counting the articles themselves as direct children would be the narrower rule and would miss a feed whose cards each sit in a wrapper, which is the common shape.
    //
    // Searched over the whole tree rather than climbed from the first article's ancestors, because the container is not always an ancestor of the article that happens to come first in the document.
    // A featured card above a grid — `<div><article/></div>` followed by a sibling `<div>` of three more — puts the first article in a branch the grid is not under, so a climb never examines the grid at all and falls through to the very first-match query this exists to replace.
    // Between candidates, one that contains another is the wider shape and loses to it, so a feed flanked by single-article widgets selects the feed rather than the ancestor holding all three.
    // Between candidates that do not contain one another, the one with the most article-carrying children wins: they are independent clusters, and depth says nothing about which is the page's subject — a three-card widget nested deep and placed before the feed would otherwise beat a twenty-card feed for being deeper.
    // One post-order pass counts articles per subtree and collects candidates from those counts, rather than asking each child whether it contains an article, which would rescan the subtree once per child.
    let root = clone.querySelector('main, [role="main"]');
    if (!root) {
        const candidates = [];
        function scan(node) {
            let count = node.tagName === 'ARTICLE' ? 1 : 0;
            let carriers = 0;
            for (const child of node.children) {
                const sub = scan(child);
                if (sub > 0) carriers++;
                count += sub;
            }
            if (carriers >= 3) candidates.push({ node, carriers });
            return count;
        }
        if (scan(clone) >= 3 && candidates.length) {
            const innermost = candidates.filter(c => !candidates.some(o => o !== c && c.node.contains(o.node)));
            root = innermost.reduce((best, c) => (c.carriers > best.carriers ? c : best)).node;
        }
    }
    if (!root) root = clone.querySelector('article, .content, #content');
    if (!root) root = clone;

    // Links are emitted as a marker into a deduplicated table returned beside the content rather than inlined at each occurrence.
    // Measured on this page set, separating the URLs from the prose is what pays (70-84% off the link payload); deduplication alone buys 1.5-7.6% and costs 4-5% on a page with no repeats.
    const links = [];
    const linkIds = new Map();
    // An anchor used as a click hook rather than as navigation — a bare `#` href, or a `javascript:` one — is not a destination, and every one of them on a page resolves to the same string.
    // Deduplicating on that string would give two unrelated controls the same marker, so `Reply` and `Delete` would both read as ⟨5⟩ and a click on it would land on whichever comes first in the DOM.
    // They are left as plain text: the table is for links the model can navigate, and a control with no destination has nothing to put in it.
    function isNavigable(node) {
        const raw = node.getAttribute('href') || '';
        // A scheme is case-insensitive, and a browser ignores ASCII whitespace inside one, so `JavaScript:`, a leading space and a tab in the middle all reach the same place.
        // Compared with those removed, since the point is what the anchor does, not how it was spelled.
        const scheme = raw.replace(/[\u0000-\u0020]/g, '').toLowerCase();
        if (scheme.startsWith('javascript:')) return false;
        // A bare `#`, or a fragment that goes nowhere on its own.
        return !(scheme === '#' || scheme === '');
    }
    function linkMarker(href) {
        let id = linkIds.get(href);
        if (id === undefined) { id = links.length + 1; linkIds.set(href, id); links.push(href); }
        return '⟨' + id + '⟩';
    }
    // Text taken off the page, with anything shaped like one of our markers defused.
    // A marker is actionable — `browser_click` takes one — so a page that prints a literal ⟨2⟩ in its own text would be handing the model a link it never wrote, attached to whatever text it likes, and would pull that link back into the table even where the real marker was cut.
    // The page's characters are kept, since the text is still what the page says; only their power to read as a marker is removed.
    // A marker cannot form across a join either: ids are digits, and every join in this walk puts a newline or a space between two pieces.
    function plain(text) {
        return text.replace(/⟨(\d+)⟩/g, '($1)');
    }

    // Read a language hint out of a `language-rust` / `lang-rust` class, so the reader knows what it is looking at.
    function codeLanguage(cls) {
        const m = /(?:language|lang)-([A-Za-z0-9+#-]+)/.exec(cls || '');
        return m ? m[1] : '';
    }

    const lines = [];
    // Parallel to `lines`: whether that line is a bullet a nested `li` already emitted.
    // A list item folds its children onto one line, and it has to fold only its *own* text — re-folding a sub-list's bullets would put their `- ` markers mid-sentence.
    // Tracked as a flag rather than sniffed back out of the string, because a text node may legitimately begin with `- `.
    const isBullet = [];
    function emit(line, bullet) { lines.push(line); isBullet.push(bullet === true); }
    // `article` earns a blank line for the same reason `section` does, and it matters more now that root selection can resolve to a container of sibling cards: feed entries that carry no heading of their own would otherwise run together into one undifferentiated block.
    const BLOCK = ['p','div','section','tr','article'];
    function walk(node) {
        if (node.nodeType === 3) {
            const t = plain(node.textContent.trim());
            if (t) emit(t);
            return;
        }
        if (node.nodeType !== 1) return;
        const tag = node.tagName.toLowerCase();
        if (['h1','h2','h3','h4','h5','h6'].includes(tag)) {
            const level = '#'.repeat(parseInt(tag[1]));
            const start = lines.length;
            for (const child of node.childNodes) walk(child);
            const heading = lines.splice(start).join(' ').replace(/\s+/g, ' ').trim();
            isBullet.splice(start);
            if (heading) emit('\n' + level + ' ' + heading);
            return;
        }
        if (tag === 'pre') {
            // A listing is the one place where the whitespace *is* the content, so it is taken as text rather than walked.
            // Descending would trim every line in the text-node branch above and fold the block into the prose around it, which is what a compiler diagnostic or a Python session loses first.
            // The fence is emitted for any `pre`, not only `pre > code`: Python's documentation, the RFC series and diagnostics all mark listings up with no `code` element anywhere.
            const lang = codeLanguage(node.className) || codeLanguage((node.querySelector('code') || {}).className);
            const body = plain(node.textContent.replace(/^\n+/, '').replace(/\n+$/, ''));
            if (body) emit('\n```' + lang + '\n' + body + '\n```\n');
            return;
        }
        if (tag === 'a' && node.href && node.textContent.trim()) {
            emit(plain(node.textContent.trim()) + (isNavigable(node) ? linkMarker(node.href) : ''));
            return;
        }
        if (tag === 'li') {
            // Recurse rather than flattening to `textContent`: returning here dropped the destination of every link inside a list item, which on a Wikipedia article is 1100 of 1723 links and on a results page is all of them.
            // The item's own text folds back onto one line so a bullet still reads as a bullet, while bullets from a nested list stay on their own lines and gain a level of indent.
            // Source order is kept across the two: an item whose sub-list sits between two runs of its own text keeps that text where the page put it, rather than having everything before the sub-list joined to everything after it.
            const start = lines.length;
            for (const child of node.childNodes) walk(child);
            const produced = lines.splice(start);
            const flags = isBullet.splice(start);
            let run = [];
            let opened = false;
            const flush = () => {
                const own = run.join(' ').replace(/\s+/g, ' ').trim();
                run = [];
                if (!own) return;
                // The first run of the item's own text is the bullet; anything after a sub-list is a continuation of it, indented to sit under the same bullet.
                emit((opened ? '  ' : '- ') + own, true);
                opened = true;
            };
            for (let i = 0; i < produced.length; i++) {
                if (flags[i]) { flush(); emit('  ' + produced[i], true); opened = true; }
                else if (produced[i] === '') flush();
                else run.push(produced[i]);
            }
            flush();
            return;
        }
        if (tag === 'br') { emit(''); return; }
        if (BLOCK.includes(tag)) emit('');
        for (const child of node.childNodes) walk(child);
        if (BLOCK.includes(tag)) emit('');
    }
    walk(root);

    const content = lines.join('\n').replace(/\n{3,}/g, '\n\n').trim();
    const cap = __MAX_CONTENT_CHARS__;
    const contentChars = Array.from(content);
    function charLength(text) { return Array.from(text).length; }

    // Same-origin links are listed as their path alone, against the `url` already in this response.
    // Most links on a page point back into it — 1383 of 1383 on a Wikipedia article — so repeating the origin per entry is the single largest avoidable cost in the table.
    // Anything cross-origin stays absolute, since that is the part a model reading a link most needs to see.
    const origin = location.origin || '';
    function shorten(href) {
        return origin && href.startsWith(origin + '/') ? href.slice(origin.length) : href;
    }
    const maxLinkDisplayChars = Math.min(2048, Math.floor(cap / 4));
    function displayUrl(href) {
        const chars = Array.from(href);
        if (chars.length <= maxLinkDisplayChars) return href;
        if (maxLinkDisplayChars <= 0) return '';
        if (maxLinkDisplayChars === 1) return '…';
        const suffixChars = Math.min(64, Math.floor((maxLinkDisplayChars - 1) / 4));
        const prefixChars = maxLinkDisplayChars - suffixChars - 1;
        return chars.slice(0, prefixChars).join('') + '…' + chars.slice(chars.length - suffixChars).join('');
    }
    // Only the links the surviving prose can still refer to: a marker past the cut is unreachable, and listing its URL would spend context on a link the model cannot see.
    function tableFor(text) {
        // One pass collecting the ids actually present, rather than a scan of the text per link: the budget search below calls this repeatedly, and a per-link `includes` over a long article is quadratic in the size of the page.
        const present = new Set();
        const re = /⟨(\d+)⟩/g;
        let m;
        while ((m = re.exec(text)) !== null) present.add(+m[1]);
        const kept = [];
        for (let i = 0; i < links.length; i++) {
            if (!present.has(i + 1)) continue;
            const url = shorten(links[i]);
            kept.push({ id: i + 1, url, display_url: displayUrl(url) });
        }
        return kept;
    }
    // What the table costs once rendered, matching how the tool layer prints it.
    // The line that opens the table reaches the model with the entries, so it is part of the cost: counting entries alone put the payload over the operator's ceiling by the length of that line, 93 characters on a page served from a typical origin.
    // Mirrors `render_page_body` — change one and the other has to follow.
    function tableCost(kept) {
        if (!kept.length) return 0;
        let n = charLength('\n\nLinks (click with browser_click, e.g. ⟨1⟩)\n');
        if (origin) n += charLength('; paths are relative to ' + origin);
        for (const e of kept) n += charLength('⟨' + e.id + '⟩ ' + e.display_url + '\n');
        return n;
    }
    function cut(budget) {
        if (contentChars.length <= budget) return content;
        // The marker is part of what the model receives, so it counts against the cap: cut far enough back that content + marker lands within `cap`, and the operator's number is a real ceiling on what reaches the context.
        // The marker's own length varies with the total it prints, so it is measured rather than approximated by a reserved constant.
        const total = contentChars.length;
        const marker = '\n... (truncated, ' + total + ' chars total)';
        return contentChars.slice(0, Math.max(0, budget - charLength(marker))).join('') + marker;
    }

    // `cap` bounds prose *and* table together.
    // The table is a second thing the extraction sends, so budgeting only the prose would hand an operator who sized the cap to a context window a payload well past it — on a link-dense article the table alone is larger than the default cap.
    // Trimming the prose is what shrinks the table, since the table lists only surviving markers, so the two are solved together rather than by dropping entries and leaving markers in the prose that resolve to nothing.
    //
    // Searched rather than subtracted: cutting prose drops markers, which drops table entries too, so one corrective subtraction overshoots badly — 34k against a 50k cap on a Wikipedia-shaped page, a third of the operator's budget left unspent.
    // A longer cut is a prefix of a longer one still, so the surviving marker set only grows with the budget and the total is monotone, which is what makes the search valid.
    function attempt(budget) {
        const out = cut(budget);
        const kept = tableFor(out);
        return { out, kept, total: charLength(out) + tableCost(kept) };
    }
    let best = attempt(cap);
    if (best.total > cap) {
        let lo = 0, hi = cap;
        best = attempt(0);
        for (let i = 0; i < 24 && lo < hi; i++) {
            const mid = Math.floor((lo + hi + 1) / 2);
            const a = attempt(mid);
            if (a.total <= cap) { best = a; lo = mid; } else { hi = mid - 1; }
        }
    }
    return JSON.stringify({title, url, content: best.out, links: best.kept, links_base: origin});
})()"#;

/// Read a link marker in the form the extracted content writes it, `⟨12⟩`.
///
/// The brackets are what makes it unambiguous, so only the bracketed form is a marker outright.
/// A bare number is handled by `parse_bare_marker`, after the selector paths have had their turn.
fn parse_link_marker(selector: &str) -> Option<usize> {
    let s = selector.trim();
    let digits = s
        .strip_prefix('\u{27e8}')
        .and_then(|r| r.strip_suffix('\u{27e9}'))?;
    parse_marker_id(digits)
}

/// Read a bare `12` as a marker id.
///
/// Only meaningful once nothing on the page answers to it as a selector: a page's own numeric link text — a pagination `5`, a numbered tab — has always been clickable through the text path, and claiming it as a marker up front would click an unrelated link instead.
/// `12` inside a longer string is never a marker, so a class like `.col-12` stays a CSS selector.
fn parse_bare_marker(selector: &str) -> Option<usize> {
    parse_marker_id(selector.trim())
}

fn parse_marker_id(digits: &str) -> Option<usize> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Ids start at 1, so 0 is not a marker.
    digits.parse().ok().filter(|&n| n > 0)
}

/// Build the page-extraction script for a given cap.
///
/// Deliberately *not* a `LazyLock`: the cap is operator-configurable, so a process-wide singleton would serve whichever value the first extraction happened to observe to every later one — including sessions belonging to a differently-configured manager, and any future build that rebuilds the manager on reload.
/// The work is a `str::replace` over a ~2 KB template on a path that then makes a CDP WebSocket round trip, so there is nothing here worth caching around.
pub(crate) fn extract_content_js(max_content_chars: usize) -> String {
    EXTRACT_CONTENT_JS_TEMPLATE.replace("__MAX_CONTENT_CHARS__", &max_content_chars.to_string())
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// Render extracted page content together with its link table.
///
/// The extraction returns the two separately — prose carrying `⟨n⟩` markers, and a deduplicated marker-to-URL table — so that a caller reading `content` is unaffected and `browser_click` resolves a marker from data rather than by parsing prose.
/// Anything that shows the prose to a reader has to join them, though: markers with no table to resolve them against are worse than the inline links they replaced.
/// This is that join, shared so the dashboard's page preview and the tool result cannot drift apart on how a page reads.
pub fn render_page_body(data: &serde_json::Value) -> String {
    render_page_body_inner(data, data["content"].as_str().unwrap_or(""))
}

/// The same body with the prose cut to `prose_chars` first, for a surface that shows a preview rather than the page.
///
/// The cut has to come before the join, not after it: the table is 4k characters on an aggregator and 80k on a link-dense article, so a budget applied to the joined string would spend all of itself on URLs and show none of the page.
/// Only the links the shortened prose still refers to are listed, which is the same rule the extraction applies at its own cap — a marker past the cut is unreachable, so its URL is only noise.
pub fn render_page_body_within(data: &serde_json::Value, prose_chars: usize) -> String {
    let content = data["content"].as_str().unwrap_or("");
    if content.chars().count() <= prose_chars {
        return render_page_body_inner(data, content);
    }
    let cut: String = content.chars().take(prose_chars).collect();
    render_page_body_inner(data, &format!("{cut}... (truncated)"))
}

fn render_page_body_inner(data: &serde_json::Value, content: &str) -> String {
    let Some(links) = data["links"].as_array().filter(|l| !l.is_empty()) else {
        return content.to_string();
    };
    // Only the links this prose still refers to.
    // At full length that is every one of them, since the extraction already listed only its own surviving markers; on a shortened preview it is what keeps the table in proportion to the prose above it.
    let entries: Vec<String> = links
        .iter()
        .filter_map(|l| {
            Some((
                l["id"].as_u64()?,
                l["display_url"].as_str().or_else(|| l["url"].as_str())?,
            ))
        })
        .filter(|(id, _)| content.contains(&format!("\u{27e8}{id}\u{27e9}")))
        .map(|(id, url)| format!("\u{27e8}{id}\u{27e9} {url}\n"))
        .collect();
    if entries.is_empty() {
        return content.to_string();
    }

    let base = data["links_base"].as_str().unwrap_or("");
    let mut out = String::with_capacity(content.len() + entries.len() * 48);
    out.push_str(content);
    out.push_str("\n\nLinks (click with browser_click, e.g. \u{27e8}1\u{27e9})");
    if !base.is_empty() {
        out.push_str(&format!("; paths are relative to {base}"));
    }
    out.push('\n');
    for e in entries {
        out.push_str(&e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn chromium_discovery_job_does_not_block_the_async_worker() {
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();

        let job = tokio::spawn(run_chromium_discovery_job(move || {
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));

        tokio::time::timeout(Duration::from_secs(1), started_rx)
            .await
            .expect("Chromium discovery job did not start")
            .expect("Chromium discovery job dropped its start signal");
        release_tx.send(()).unwrap();
        job.await.unwrap().unwrap();
    }

    #[test]
    fn test_browser_config_defaults() {
        let config = BrowserConfig::default();
        assert!(config.headless);
        assert_eq!(config.viewport_width, 1280);
        assert_eq!(config.viewport_height, 720);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.idle_timeout_secs, 300);
        assert_eq!(config.max_sessions, 5);
        assert!(config.chromium_path.is_none());
    }

    #[test]
    fn test_chromium_launch_args_include_user_data_dir() {
        let config = BrowserConfig {
            headless: false,
            ..Default::default()
        };
        let args = chromium_launch_args(&config, Path::new("/tmp/librefang-profile"));
        assert!(args.iter().any(|a| a.starts_with("--user-data-dir=")));
        assert!(args.iter().all(|a| a != "--headless=new"));
    }

    #[test]
    fn test_browser_env_allowlist_includes_dbus_runtime_vars() {
        assert!(BROWSER_ENV_ALLOWLIST.contains(&"DBUS_SESSION_BUS_ADDRESS"));
        assert!(BROWSER_ENV_ALLOWLIST.contains(&"XDG_RUNTIME_DIR"));
    }

    #[test]
    fn test_browser_command_serialize_navigate() {
        let cmd = BrowserCommand::Navigate {
            url: "https://example.com".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Navigate\""));
        assert!(json.contains("\"url\":\"https://example.com\""));
    }

    #[test]
    fn test_browser_command_serialize_click() {
        let cmd = BrowserCommand::Click {
            selector: "#submit-btn".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Click\""));
        assert!(json.contains("\"selector\":\"#submit-btn\""));
    }

    #[test]
    fn test_browser_command_serialize_type() {
        let cmd = BrowserCommand::Type {
            selector: "input[name='email']".to_string(),
            text: "test@example.com".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Type\""));
        assert!(json.contains("test@example.com"));
    }

    #[test]
    fn test_browser_command_serialize_screenshot() {
        let cmd = BrowserCommand::Screenshot;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Screenshot\""));
    }

    #[test]
    fn test_browser_command_serialize_read_page() {
        let cmd = BrowserCommand::ReadPage;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"ReadPage\""));
    }

    #[test]
    fn test_browser_command_serialize_close() {
        let cmd = BrowserCommand::Close;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Close\""));
    }

    #[test]
    fn test_browser_command_serialize_scroll() {
        let cmd = BrowserCommand::Scroll {
            direction: "down".to_string(),
            amount: 500,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Scroll\""));
        assert!(json.contains("\"amount\":500"));
    }

    #[test]
    fn test_browser_command_serialize_run_js() {
        let cmd = BrowserCommand::RunJs {
            expression: "document.title".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"RunJs\""));
    }

    #[test]
    fn test_browser_command_serialize_back() {
        let cmd = BrowserCommand::Back;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Back\""));
    }

    #[test]
    fn test_browser_command_serialize_wait() {
        let cmd = BrowserCommand::Wait {
            selector: "#loaded".to_string(),
            timeout_ms: 3000,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Wait\""));
        assert!(json.contains("\"timeout_ms\":3000"));
    }

    #[test]
    fn test_browser_response_deserialize() {
        let json =
            r#"{"success": true, "data": {"title": "Example", "url": "https://example.com"}}"#;
        let resp: BrowserResponse = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.data.is_some());
        assert!(resp.error.is_none());
        let data = resp.data.unwrap();
        assert_eq!(data["title"], "Example");
    }

    #[test]
    fn test_browser_response_error_deserialize() {
        let json = r#"{"success": false, "error": "Element not found"}"#;
        let resp: BrowserResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert!(resp.data.is_none());
        assert_eq!(resp.error.unwrap(), "Element not found");
    }

    #[test]
    fn test_browser_manager_new() {
        let config = BrowserConfig::default();
        let mgr = BrowserManager::new(config);
        assert!(mgr.sessions.is_empty());
    }

    #[test]
    fn test_chromium_candidates_not_empty() {
        let paths = chromium_candidates();
        assert!(
            !paths.is_empty(),
            "Should have platform-specific candidates"
        );
    }

    /// The cap the model sees must be the one the caller asked for.
    ///
    /// The script used to hard-code `50000` while `MAX_CONTENT_CHARS` sat unused, so changing the constant changed nothing (#6623).
    /// Now the value is operator-configurable, so the same drift would silently ignore `[browser] max_content_chars` instead (#6624).
    #[test]
    fn test_extract_content_js_uses_max_content_chars() {
        let script = extract_content_js(12_345);
        assert!(
            !script.contains("__MAX_CONTENT_CHARS__"),
            "placeholder must be substituted before the script is sent"
        );
        assert!(
            script.contains("12345"),
            "the script must truncate at the cap it was built with"
        );
        // Guards against the placeholder being dropped from the template entirely, which would leave the two silently independent again.
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("__MAX_CONTENT_CHARS__"),
            "the template must keep the placeholder"
        );
    }

    /// A different cap must produce a different script.
    ///
    /// The previous `LazyLock` would have passed the test above and still served the first-seen value forever; this is the assertion that fails if anyone reintroduces process-wide caching over a config-dependent value.
    #[test]
    fn test_extract_content_js_varies_with_cap() {
        assert_ne!(
            extract_content_js(1_000),
            extract_content_js(50_000),
            "the script must be rebuilt per cap, not cached process-wide"
        );
        assert!(extract_content_js(1_000).contains("1000"));
        assert!(extract_content_js(50_000).contains("50000"));
    }

    async fn live_browser_session() -> Option<BrowserSession> {
        let mut config = BrowserConfig::default();
        let chromium = match find_chromium(&config) {
            Ok(path) => path,
            Err(error) => {
                eprintln!(
                    "skipping live extraction fixture because Chromium is unavailable: {error}"
                );
                return None;
            }
        };
        config.chromium_path = Some(chromium.to_string_lossy().into_owned());
        // A launch failure is the same class of environmental unavailability as a missing binary, so it skips rather than panics.
        // A CI runner routinely has Chromium installed but cannot start it — no sandbox, a missing shared library, or too little memory — and panicking there turns an environment limitation into a red `main` that every open PR then inherits (#7861).
        match BrowserSession::launch(&config).await {
            Ok(session) => Some(session),
            Err(error) => {
                eprintln!(
                    "skipping live extraction fixture because Chromium failed to launch: {error}"
                );
                None
            }
        }
    }

    async fn load_html_fixture(session: &BrowserSession, html: &str) {
        let html = serde_json::to_string(html).unwrap();
        session
            .cdp
            .run_js(&format!(
                "document.open(); document.write({html}); document.close(); 'ready'"
            ))
            .await
            .expect("fixture HTML must load");
    }

    async fn extract_fixture(session: &BrowserSession, cap: usize) -> serde_json::Value {
        let encoded = session
            .cdp
            .run_js(&extract_content_js(cap))
            .await
            .expect("extraction script must execute");
        serde_json::from_str(
            encoded
                .as_str()
                .expect("extraction script must return encoded JSON"),
        )
        .expect("extraction result must remain valid JSON")
    }

    /// A listing reaches the model fenced, with its whitespace and its language.
    ///
    /// The walk had no `pre` branch at all, so a listing fell through to the text-node branch, which trims every line: 15 of 15 blocks on a Rust book chapter and 35 of 35 on a Python tutorial arrived as prose with their indentation gone.
    /// The fence is emitted for any `pre`, so the two shapes that carry no `code` element — Python's highlighted spans and a plain RFC listing — are covered as well as a highlighted one.
    #[tokio::test]
    async fn live_extraction_fences_preformatted_blocks() {
        let Some(session) = live_browser_session().await else {
            return;
        };

        load_html_fixture(
            &session,
            r##"<!doctype html><html><head><title>listings</title></head><body><main><p>Before.</p><pre class="playground"><code class="language-rust">fn main() {
    let x = 1;
}</code></pre><p>Between.</p><pre><span class="gp">&gt;&gt;&gt; </span>len(x)
3</pre><p>After.</p></main></body></html>"##,
        )
        .await;
        let extracted = extract_fixture(&session, 10_000).await;
        let content = extracted["content"].as_str().unwrap();

        assert!(
            content.contains("```rust"),
            "a language hint on the inner code element must reach the fence: {content}"
        );
        assert!(
            content.contains("    let x = 1;"),
            "indentation is the content of a listing and must survive: {content}"
        );
        assert!(
            content.contains(">>> len(x)"),
            "a listing with no code element must be fenced too: {content}"
        );
        assert_eq!(
            content.matches("```").count(),
            4,
            "each listing must open and close exactly once: {content}"
        );
        for prose in ["Before.", "Between.", "After."] {
            assert!(
                content.contains(prose),
                "prose around a listing must survive: {prose} missing from {content}"
            );
        }
    }

    #[tokio::test]
    async fn live_extraction_preserves_structure_click_fallback_and_budget() {
        let Some(session) = live_browser_session().await else {
            return;
        };

        load_html_fixture(
            &session,
            r##"<!doctype html><html><head><title>structure</title></head><body><header>Global chrome</header><main><article><header><h2><a href="#card">Card</a></h2></header><p>Body</p><footer>Byline</footer></article><ul><li>Plan<table><tr><td>Price</td><td>Qty</td></tr><tr><td>5</td><td>10</td></tr></table>Done</li></ul><button onclick="window.clicked = true">7</button></main></body></html>"##,
        )
        .await;
        let structure = extract_fixture(&session, 10_000).await;
        let content = structure["content"].as_str().unwrap();
        assert!(
            content.contains("## Card⟨1⟩"),
            "a heading must retain its nested link marker: {content}"
        );
        assert!(
            content.contains("Byline"),
            "section-scoped footer content must survive pruning: {content}"
        );
        assert!(
            !content.contains("Global chrome"),
            "the page-level header must still be pruned: {content}"
        );
        assert!(
            !content.contains("- Plan Price Qty 5 10 Done"),
            "block rows inside a list item must not flatten into one line: {content}"
        );
        for line in ["- Plan", "  Price Qty", "  5 10", "  Done"] {
            assert!(
                content.lines().any(|actual| actual == line),
                "list-item block line {line:?} is missing from: {content}"
            );
        }

        let empty_click = session.cmd_click("", 10_000).await;
        assert!(
            !empty_click.success,
            "an empty selector must not click the first interactive element"
        );

        let click = session.cmd_click("7", 10_000).await;
        assert!(
            click.success,
            "an invalid CSS selector must still reach text fallback: {:?}",
            click.error
        );
        assert_eq!(
            session.cdp.run_js("window.clicked === true").await.unwrap(),
            true
        );

        let unicode_text = format!("{}😀{}", "a".repeat(67), "b".repeat(131));
        load_html_fixture(&session, &format!("<main><p>{unicode_text}</p></main>")).await;
        let unicode = extract_fixture(&session, 101).await;
        assert!(
            unicode["content"].as_str().unwrap().contains('😀'),
            "a character-budget cut must not split a surrogate pair"
        );

        let long_url = format!("https://example.com/{}", "q".repeat(28_000));
        let tail = "tail ".repeat(1_000);
        load_html_fixture(
            &session,
            &format!("<main><p>intro <a href=\"{long_url}\">huge</a> after {tail}</p></main>"),
        )
        .await;
        let budgeted = extract_fixture(&session, 2_000).await;
        let budgeted_content = budgeted["content"].as_str().unwrap();
        assert!(
            budgeted_content.chars().count() > 1_000,
            "one long href must not starve the prose budget: {} chars",
            budgeted_content.chars().count()
        );
        assert_eq!(
            budgeted["links"][0]["url"].as_str(),
            Some(long_url.as_str()),
            "the actionable URL must remain complete"
        );
        assert!(
            budgeted["links"][0]["display_url"]
                .as_str()
                .unwrap()
                .chars()
                .count()
                <= 500,
            "the rendered URL must be bounded relative to the response cap"
        );
        assert!(
            render_page_body(&budgeted).chars().count() <= 2_000,
            "rendered prose and link table must respect the cap"
        );
    }

    #[test]
    fn rendered_link_table_uses_bounded_display_url() {
        let full_url = format!("https://example.com/{}", "q".repeat(10_000));
        let page = serde_json::json!({
            "content": "Download⟨1⟩",
            "links": [{"id": 1, "url": full_url, "display_url": "https://example.com/q…qqq"}],
            "links_base": "",
        });

        let rendered = render_page_body(&page);
        assert!(rendered.contains("https://example.com/q…qqq"));
        assert!(!rendered.contains(&"q".repeat(1_000)));
    }

    /// A link inside a list item must keep its destination.
    ///
    /// The `li` branch used to flatten the item to `textContent` and return, so every nested anchor reached the output as bare text — 1,100 of 1,723 anchors on a Wikipedia article, and 11 of 11 on a search-results page.
    /// Asserted against the template because the traversal itself needs a live DOM; this pins the shape that made the flattening possible.
    #[test]
    fn test_list_items_recurse_instead_of_flattening_to_text() {
        assert!(
            !EXTRACT_CONTENT_JS_TEMPLATE.contains("lines.push('- ' + node.textContent.trim())"),
            "flattening a list item to its text drops the href of every link inside it"
        );
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("if (tag === 'li')"),
            "the list-item branch must still exist so bullets keep rendering as bullets"
        );
    }

    /// A sub-list keeps its own bullets instead of being folded into its parent's line.
    ///
    /// Folding a list item's children onto one line is what lets a bullet still read as a bullet, but a nested `li` has already emitted its own `- ` by the time the outer one folds — so folding everything produced `- Fruits - Apple - Banana`, with markers mid-sentence that read as list items or as a numeric range on text like `- Price - 5 - 10`.
    /// Ported from the JS the script runs, because the traversal itself needs a live DOM.
    #[test]
    fn test_nested_lists_keep_their_own_bullets() {
        // The port models the script; this checks the script still does what is modelled.
        // Reverting the fold to the two-filter form would leave the port validating itself and passing.
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("emit((opened ? '  ' : '- ') + own, true)"),
            "the item's own text must be emitted in runs, so a sub-list keeps its place between them"
        );

        /// A list item as the page writes it: runs of its own text and sub-lists, in source order.
        enum Part {
            Text(&'static str),
            Sub(Vec<Part>),
        }
        use Part::{Sub, Text};

        /// The `li` branch's fold: own text onto one line, a sub-list's bullets kept, indented, and left where the page put them.
        fn render(parts: &[Part]) -> Vec<String> {
            // `(line, came from a nested li)`, in the order the walk produced them.
            let mut produced: Vec<(String, bool)> = Vec::new();
            for part in parts {
                match part {
                    Text(t) => produced.push(((*t).to_string(), false)),
                    Sub(inner) => produced.extend(render(inner).into_iter().map(|l| (l, true))),
                }
            }

            let mut out: Vec<String> = Vec::new();
            let mut run: Vec<&str> = Vec::new();
            let mut opened = false;
            // The first run of the item's own text is the bullet; a run after a sub-list is a continuation of it.
            let flush = |run: &mut Vec<&str>, out: &mut Vec<String>, opened: &mut bool| {
                let own = run.join(" ");
                run.clear();
                if own.trim().is_empty() {
                    return;
                }
                out.push(format!("{}{own}", if *opened { "  " } else { "- " }));
                *opened = true;
            };
            for (line, nested) in &produced {
                if *nested {
                    flush(&mut run, &mut out, &mut opened);
                    out.push(format!("  {line}"));
                    opened = true;
                } else {
                    run.push(line);
                }
            }
            flush(&mut run, &mut out, &mut opened);
            out
        }

        assert_eq!(
            render(&[
                Text("Fruits"),
                Sub(vec![Text("Apple")]),
                Sub(vec![Text("Banana")])
            ]),
            vec!["- Fruits", "  - Apple", "  - Banana"],
            "a sub-list must stay a sub-list rather than collapsing into its parent's line"
        );
        assert_eq!(
            render(&[Text("A"), Sub(vec![Text("B"), Sub(vec![Text("C")])])]),
            vec!["- A", "  - B", "    - C"],
            "each level of nesting earns one level of indent"
        );
        // A sub-list between two runs of the item's own text: joining all the text first would move `Outro` above the sub-list it follows on the page.
        assert_eq!(
            render(&[
                Text("Intro"),
                Sub(vec![Text("Sub A")]),
                Sub(vec![Text("Sub B")]),
                Text("Outro")
            ]),
            vec!["- Intro", "  - Sub A", "  - Sub B", "  Outro"],
            "text after a sub-list must stay after it"
        );
        // An item with no text of its own: everything under it is a continuation of nothing, which is what the page wrote.
        assert_eq!(
            render(&[Sub(vec![Text("Sub")]), Text("After")]),
            vec!["  - Sub", "  After"]
        );
        // The flat case the fold exists for is unchanged.
        assert_eq!(render(&[Text("Just an item")]), vec!["- Just an item"]);
    }

    /// Feed entries must be separated from one another.
    ///
    /// Root selection can now resolve to the container of several sibling `<article>` cards, and an entry that carries no heading of its own emits no separator — so consecutive cards ran together into one undifferentiated block, which works against covering the feed in the first place.
    #[test]
    fn test_article_earns_a_block_separator() {
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("'tr','article'"),
            "an article is a block, like the section and div beside it in that list"
        );
    }

    /// The first-match query must not be what selects the root on a page with several articles.
    ///
    /// `querySelector('main, article, …')` returned the first match, which on a results page is one card — 13.7% of the page.
    /// Which container is chosen instead is pinned by behaviour in `test_root_selection_finds_a_container_of_repeated_cards`; this only guards the query from coming back.
    #[test]
    fn test_root_selection_does_not_take_the_first_article_match() {
        assert!(
            !EXTRACT_CONTENT_JS_TEMPLATE
                .contains(r#"querySelector('main, article, [role="main"], .content, #content')"#),
            "a single first-match query cannot distinguish one article from a feed of them"
        );
    }

    /// Selection finds the container of repeated cards, and does not reach past it on a page that merely has articles in more than one place.
    ///
    /// Ported from the JS the script runs, because the search needs a live DOM.
    /// Counting *contained* articles rather than article-carrying children reaches the whole document on an ordinary post page whose "related posts" widget is built out of `article`, taking the widget along with the post.
    /// Anchoring on the first article's ancestors instead of searching the tree misses a grid that sits beside a featured card rather than under it.
    #[test]
    fn test_root_selection_finds_a_container_of_repeated_cards() {
        // As above: the port is only evidence while it and the script agree on the rule.
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("candidates.push({ node, carriers })"),
            "selection must collect every qualifying node, not stop at the first branch that has one"
        );
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("c.node.contains(o.node)"),
            "a candidate containing another must lose to it, which is what keeps selection off the whole page"
        );

        struct Node {
            tag: &'static str,
            name: &'static str,
            kids: Vec<Node>,
        }
        fn el(tag: &'static str, name: &'static str, kids: Vec<Node>) -> Node {
            Node { tag, name, kids }
        }
        fn article(name: &'static str) -> Node {
            el("article", name, vec![])
        }
        fn has_article(n: &Node) -> bool {
            n.tag == "article" || n.kids.iter().any(has_article)
        }
        fn count_articles(n: &Node) -> usize {
            usize::from(n.tag == "article") + n.kids.iter().map(count_articles).sum::<usize>()
        }

        /// The chosen root's name, or `None` when selection falls through to the single-article path.
        ///
        /// The same rule the script runs: collect every node whose direct children carry at least three articles between them, drop any that contains another candidate, and take the one with the most carriers from what is left.
        fn chosen(page: &Node) -> Option<&'static str> {
            if count_articles(page) < 3 {
                return None;
            }
            /// `(name, carriers, names below it that also qualify)`, in post-order.
            fn collect(n: &Node, out: &mut Vec<(&'static str, usize, Vec<&'static str>)>) {
                let before = out.len();
                for k in &n.kids {
                    collect(k, out);
                }
                let carriers = n.kids.iter().filter(|k| has_article(k)).count();
                if carriers >= 3 {
                    let below = out[before..].iter().map(|(name, _, _)| *name).collect();
                    out.push((n.name, carriers, below));
                }
            }
            let mut candidates = Vec::new();
            collect(page, &mut candidates);
            // Folded rather than `max_by_key`, which keeps the *last* of equal maxima where the script's reduce keeps the first.
            candidates
                .iter()
                .filter(|(_, _, below)| below.is_empty())
                .fold(
                    None,
                    |best: Option<&(&str, usize, Vec<&str>)>, c| match best {
                        Some(b) if b.1 >= c.1 => Some(b),
                        _ => Some(c),
                    },
                )
                .map(|(name, _, _)| *name)
        }

        // The shape the change exists for: a results page of sibling cards.
        let results = el(
            "body",
            "body",
            vec![el(
                "div",
                "results",
                (0..11).map(|_| article("card")).collect(),
            )],
        );
        assert_eq!(chosen(&results), Some("results"));

        // The same shape with each card in a wrapper, which is at least as common.
        let feed = el(
            "body",
            "body",
            vec![el(
                "div",
                "feed",
                (0..5)
                    .map(|_| el("div", "card", vec![article("post")]))
                    .collect(),
            )],
        );
        assert_eq!(chosen(&feed), Some("feed"));

        // The over-reach case: one post plus a related-posts widget also built out of `article`.
        // Three articles are present, but only two children carry them, so this must not resolve to the document.
        let blog = el(
            "body",
            "body",
            vec![
                el("div", "content", vec![article("the post")]),
                el("div", "related", vec![article("rel 1"), article("rel 2")]),
            ],
        );
        assert_eq!(
            chosen(&blog),
            None,
            "a post page with a related-posts widget must stay on the post, not climb to the document"
        );

        // Below the threshold entirely.
        let single = el(
            "body",
            "body",
            vec![el("div", "content", vec![article("only")])],
        );
        assert_eq!(chosen(&single), None);

        // A featured card above the grid, which a search anchored on the first article cannot reach: the grid is that article's sibling subtree, not one of its ancestors.
        let featured = el(
            "body",
            "body",
            vec![
                el("div", "lone", vec![article("featured")]),
                el(
                    "div",
                    "feed",
                    vec![article("a"), article("b"), article("c")],
                ),
            ],
        );
        assert_eq!(
            chosen(&featured),
            Some("feed"),
            "a grid beside a featured card is still the grid, and returning the featured card alone is the bug this exists to fix"
        );

        // A feed flanked by unrelated single-article widgets: the deepest qualifying node is the feed, not the ancestor that happens to hold all three.
        let flanked = el(
            "body",
            "body",
            vec![
                el(
                    "div",
                    "feed",
                    vec![article("a"), article("b"), article("c")],
                ),
                el("div", "trending", vec![article("teaser")]),
                el("div", "related", vec![article("teaser")]),
            ],
        );
        assert_eq!(
            chosen(&flanked),
            Some("feed"),
            "scattered single-article widgets must not widen selection to the whole page"
        );

        // Two independent clusters at different depths: a small widget nested deep and placed *before* the feed in the document.
        // Depth is not evidence of being the page's subject, so the larger cluster wins.
        let widget_first = el(
            "body",
            "body",
            vec![
                el(
                    "div",
                    "sidebar",
                    vec![el(
                        "div",
                        "wrap",
                        vec![el(
                            "div",
                            "widget",
                            vec![article("w1"), article("w2"), article("w3")],
                        )],
                    )],
                ),
                el("div", "feed", (0..20).map(|_| article("card")).collect()),
            ],
        );
        assert_eq!(
            chosen(&widget_first),
            Some("feed"),
            "a deeper three-card widget must not beat the feed it happens to precede"
        );

        // Two clusters of equal size: the first in document order, so the choice is stable rather than arbitrary.
        let two_feeds = el(
            "body",
            "body",
            vec![
                el(
                    "div",
                    "first",
                    vec![article("a"), article("b"), article("c")],
                ),
                el(
                    "div",
                    "second",
                    vec![article("d"), article("e"), article("f")],
                ),
            ],
        );
        assert_eq!(chosen(&two_feeds), Some("first"));
    }

    /// Links are emitted as a marker into a table, not inlined into the prose.
    #[test]
    fn test_links_are_emitted_as_markers() {
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("linkMarker(node.href)"),
            "the anchor branch must emit a marker"
        );
        assert!(
            !EXTRACT_CONTENT_JS_TEMPLATE.contains("'](' + node.href + ')'"),
            "inlining the href per occurrence is what the marker table replaces"
        );
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("linkIds.get(href)"),
            "a repeated URL must reuse its id rather than being listed again"
        );
    }

    /// A repeated URL is one entry with one id, numbered in the order the walk first meets it.
    ///
    /// Ported from `linkMarker`/`linkIds`, because the numbering happens during a DOM walk.
    /// The order is the property worth pinning: the ids reach the model's context, so a table that renumbered between two runs over the same page would be a different prompt each time.
    #[test]
    fn test_repeated_urls_share_one_marker_in_first_seen_order() {
        fn assign(hrefs: &[&str]) -> (Vec<usize>, Vec<String>) {
            let mut ids: Vec<(String, usize)> = Vec::new();
            let mut table: Vec<String> = Vec::new();
            let markers = hrefs
                .iter()
                .map(|href| match ids.iter().find(|(h, _)| h == href) {
                    Some((_, id)) => *id,
                    None => {
                        table.push((*href).to_string());
                        ids.push(((*href).to_string(), table.len()));
                        table.len()
                    }
                })
                .collect();
            (markers, table)
        }

        let (markers, table) = assign(&["/a", "/b", "/a", "/a", "/c"]);
        assert_eq!(
            markers,
            vec![1, 2, 1, 1, 3],
            "a repeated href must reuse the id it was first given"
        );
        assert_eq!(
            table,
            vec!["/a", "/b", "/c"],
            "one entry per distinct href, in the order the walk first met it"
        );

        // The same input twice is the same table: ids come from walk order, not from a hash.
        assert_eq!(assign(&["/x", "/y", "/x"]), assign(&["/x", "/y", "/x"]));
    }

    /// A preview budget cuts the prose, not the joined page.
    ///
    /// The table is thousands of characters on a link-dense page — 4,028 on Hacker News, 8,274 on a Wikipedia article — so a budget applied after the join would spend all of itself on URLs and show none of the page.
    /// Cutting first also drops the links the shortened prose can no longer refer to, the same rule the extraction applies at its own cap.
    #[test]
    fn test_preview_budget_cuts_prose_before_the_table_is_joined() {
        let page = serde_json::json!({
            "content": format!("{}\u{27e8}1\u{27e9} tail \u{27e8}2\u{27e9}", "w".repeat(60)),
            "links": [
                {"id": 1, "url": "/one"},
                {"id": 2, "url": "/two"},
            ],
            "links_base": "https://example.com",
        });

        // Wide enough for everything: both entries listed.
        let full = render_page_body_within(&page, 10_000);
        assert_eq!(full, render_page_body(&page));
        assert!(full.contains("\u{27e8}1\u{27e9} /one"));
        assert!(full.contains("\u{27e8}2\u{27e9} /two"));

        // Narrow: the prose is cut before the join, and the entry the surviving prose can no longer reach goes with it.
        let short = render_page_body_within(&page, 20);
        assert!(short.starts_with(&"w".repeat(20)));
        assert!(short.contains("... (truncated)"));
        assert!(
            !short.contains("\u{27e8}2\u{27e9} /two"),
            "a marker past the cut must not spend the preview on a URL the reader cannot see"
        );
        assert!(
            !short.contains("/one"),
            "neither marker survives a 20-character cut, so the table should be empty"
        );

        // With every marker cut away there is no table to introduce.
        assert!(!short.contains("Links (click with"));
    }

    /// A page cannot write its own markers.
    ///
    /// A marker is actionable — `browser_click` takes one — so a page printing a literal `⟨2⟩` in its text would attach a link it never wrote to whatever words it likes, and would pull that link back into the table even where its real marker was cut, since the table is built by scanning the surviving prose.
    /// Ported from `plain`, which runs on every piece of text the walk takes off the page.
    #[test]
    fn test_page_text_cannot_forge_a_marker() {
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("plain(node.textContent.trim())"),
            "text taken off the page must be defused before it reaches the output"
        );

        fn plain(text: &str) -> String {
            let mut out = String::with_capacity(text.len());
            let chars: Vec<char> = text.chars().collect();
            let mut i = 0;
            while i < chars.len() {
                if chars[i] == '\u{27e8}' {
                    let mut j = i + 1;
                    while j < chars.len() && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                    if j > i + 1 && j < chars.len() && chars[j] == '\u{27e9}' {
                        out.push('(');
                        out.extend(&chars[i + 1..j]);
                        out.push(')');
                        i = j + 1;
                        continue;
                    }
                }
                out.push(chars[i]);
                i += 1;
            }
            out
        }
        fn markers(s: &str) -> Vec<usize> {
            let chars: Vec<char> = s.chars().collect();
            let mut found = Vec::new();
            for i in 0..chars.len() {
                if chars[i] != '\u{27e8}' {
                    continue;
                }
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 && j < chars.len() && chars[j] == '\u{27e9}' {
                    found.push(chars[i + 1..j].iter().collect::<String>().parse().unwrap());
                }
            }
            found
        }

        for hostile in [
            "\u{27e8}2\u{27e9}",
            "Sign in here \u{27e8}2\u{27e9} to continue",
            "\u{27e8}\u{27e8}2\u{27e9}\u{27e9}",
            "\u{27e8}1\u{27e9} and \u{27e8}2\u{27e9} and \u{27e8}33\u{27e9}",
        ] {
            assert!(
                markers(&plain(hostile)).is_empty(),
                "page text {hostile:?} still reads as a marker after defusing"
            );
        }

        // Half a marker on either side of a join cannot become a whole one: every join in the walk puts a newline or a space between the two pieces, and an id is digits only.
        for (a, b) in [("\u{27e8}5", "\u{27e9}"), ("\u{27e8}", "5\u{27e9}")] {
            for sep in ["\n", " "] {
                let joined = format!("{}{sep}{}", plain(a), plain(b));
                assert!(
                    markers(&joined).is_empty(),
                    "a marker formed across a join: {joined:?}"
                );
            }
        }

        // The extraction's own marker still lands, beside text that tried to fake one.
        assert_eq!(
            markers(&format!(
                "{}\u{27e8}2\u{27e9}",
                plain("item \u{27e8}9\u{27e9} five")
            )),
            vec![2]
        );
        // Anything that is not a marker keeps its characters, since the text is still what the page says.
        assert_eq!(plain("\u{27e8}abc\u{27e9}"), "\u{27e8}abc\u{27e9}");
        assert_eq!(
            plain("\u{27e8}5 and later \u{27e9}"),
            "\u{27e8}5 and later \u{27e9}"
        );
    }

    /// An anchor with no destination is left out of the table.
    ///
    /// A bare `#` or a `javascript:` href is a click hook, and every one of them on a page resolves to the same string, so deduplicating on it would hand two unrelated controls the same marker and a click would land on whichever came first in the DOM.
    #[test]
    fn test_placeholder_hrefs_are_not_marked() {
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("isNavigable(node) ? linkMarker(node.href) : ''"),
            "an anchor that is not a destination must reach the prose as plain text"
        );
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("scheme.startsWith('javascript:')"),
            "a javascript: href is a click hook, not a link"
        );

        // Ported from `isNavigable`, over the `href` attribute as written on the page.
        // A scheme is case-insensitive and a browser ignores ASCII whitespace inside one, so the comparison is against the attribute with those removed.
        fn navigable(raw: &str) -> bool {
            let scheme: String = raw
                .chars()
                .filter(|c| *c > ' ')
                .collect::<String>()
                .to_lowercase();
            !(scheme.starts_with("javascript:") || scheme == "#" || scheme.is_empty())
        }
        assert!(!navigable("#"));
        assert!(!navigable("javascript:void(0)"));
        // Spelling is not what makes it a link: a browser reaches the same place from all of these.
        assert!(!navigable("JavaScript:void(0)"));
        assert!(!navigable("  javascript:doThing()"));
        assert!(!navigable("java\tscript:doThing()"));
        assert!(!navigable("java\nscript:doThing()"));
        assert!(!navigable(" # "));
        assert!(!navigable(""));
        assert!(navigable("/newest"));
        assert!(navigable("https://example.com/"));
        // A fragment that names something on the page is still somewhere to go.
        assert!(navigable("#install"));
    }

    /// A same-origin link is listed as its path, and anything cross-origin keeps its origin.
    ///
    /// Ported from `shorten`, which needs `location.origin`.
    /// This is where the table's size comes from — nearly every link on a page points back into it — so the shortening is the part worth pinning, along with the case that must *not* shorten.
    #[test]
    fn test_same_origin_links_are_stored_as_a_path() {
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("shorten(links[i])"),
            "the table must store the shortened form, not the absolute URL"
        );

        fn shorten<'a>(origin: &str, href: &'a str) -> &'a str {
            if !origin.is_empty() && href.starts_with(&format!("{origin}/")) {
                &href[origin.len()..]
            } else {
                href
            }
        }
        let origin = "https://news.ycombinator.com";
        assert_eq!(
            shorten(origin, "https://news.ycombinator.com/newest"),
            "/newest"
        );
        assert_eq!(shorten(origin, "https://news.ycombinator.com/"), "/");
        // Cross-origin stays absolute: the origin is the part a model most needs to see.
        assert_eq!(
            shorten(origin, "https://example.com/newest"),
            "https://example.com/newest"
        );
        // A different origin that merely starts with the same text must not be mistaken for one of ours.
        assert_eq!(
            shorten(origin, "https://news.ycombinator.com.evil.test/x"),
            "https://news.ycombinator.com.evil.test/x"
        );
        // The origin with nothing after it is not `origin + '/'`, so it is left alone.
        assert_eq!(shorten(origin, origin), origin);
        // A page served from `about:blank` or a `file:` URL has no origin to strip.
        assert_eq!(
            shorten("", "https://example.com/x"),
            "https://example.com/x"
        );
    }

    /// The cap bounds prose *and* link table, not prose alone.
    ///
    /// The table is a second thing the extraction sends, and on a link-dense article it is larger than the default cap on its own, so budgeting only the prose would hand an operator who sized `max_content_chars` to a context window a payload well past it (#6624).
    #[test]
    fn test_link_table_is_budgeted_against_the_cap() {
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("charLength(out) + tableCost(kept)"),
            "the rendered cost of the table must be measured against the cap"
        );
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("content: best.out"),
            "the returned content must be the budgeted cut, not the pre-budget prose"
        );
    }

    /// The budget search holds the ceiling and does not leave most of it unspent.
    ///
    /// Ported from the JS the script runs, because the extraction itself needs a live browser — same shape as `test_truncation_marker_fits_inside_the_cap`.
    /// The subtract-the-overflow form this replaces stayed under the cap but landed at 34k against 50k on a Wikipedia-shaped page, because cutting prose drops markers and so drops table entries faster than the prose itself shrank.
    ///
    /// The ceiling is asserted against what `render_page_body` actually produces rather than against the ported cost function.
    /// Re-deriving the cost on both sides is self-consistent by construction, so it cannot see the budget and the renderer disagreeing — which is exactly what happened: the budget counted entries and the renderer also emitted a line introducing them, putting the payload 93 characters past the operator's number.
    #[test]
    fn test_budget_search_fills_the_cap_without_exceeding_it() {
        // The port below models the script; this checks the script still does what is modelled.
        // A port is only evidence about the script while the two agree, and nothing else here would notice if the header cost were dropped from the template — the port would go on validating a corrected budget against `render_page_body` and pass.
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("Links (click with browser_click"),
            "the script's own table cost must include the line that opens the table, or this test measures something the browser never runs"
        );
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("paths are relative to"),
            "that line carries the origin, whose length varies per page and so must be measured rather than assumed"
        );

        /// One `⟨n⟩` marker per line, so a cut drops markers the way a real page does.
        fn page(n_links: usize, prose_len: usize, url_len: usize) -> (String, Vec<String>) {
            let links: Vec<String> = (0..n_links)
                .map(|i| format!("/p/{}{i}", "x".repeat(url_len)))
                .collect();
            let filler = "w".repeat((prose_len / n_links.max(1)).max(1));
            let content = (0..n_links)
                .map(|i| format!("{filler}\u{27e8}{}\u{27e9}", i + 1))
                .collect::<Vec<_>>()
                .join("\n");
            (content, links)
        }
        fn cut(text: &str, budget: usize) -> String {
            if text.chars().count() <= budget {
                return text.to_string();
            }
            let total = text.chars().count();
            let marker = format!("\n... (truncated, {total} chars total)");
            let keep = budget.saturating_sub(marker.chars().count());
            format!("{}{marker}", text.chars().take(keep).collect::<String>())
        }
        /// The extraction's own view of what the table costs, header included.
        fn table_cost(text: &str, links: &[String], origin: &str) -> usize {
            let entries: usize = (0..links.len())
                .filter(|i| text.contains(&format!("\u{27e8}{}\u{27e9}", i + 1)))
                .map(|i| {
                    format!("\u{27e8}{}\u{27e9} {}\n", i + 1, links[i])
                        .chars()
                        .count()
                })
                .sum();
            if entries == 0 {
                return 0;
            }
            let mut header = "\n\nLinks (click with browser_click, e.g. \u{27e8}1\u{27e9})\n"
                .chars()
                .count();
            if !origin.is_empty() {
                header += format!("; paths are relative to {origin}").chars().count();
            }
            entries + header
        }
        /// The extraction's response for a given prose cut, as `render_page_body` receives it.
        fn response(content: &str, links: &[String], origin: &str) -> serde_json::Value {
            let kept: Vec<serde_json::Value> = (0..links.len())
                .filter(|i| content.contains(&format!("\u{27e8}{}\u{27e9}", i + 1)))
                .map(|i| serde_json::json!({"id": i + 1, "url": links[i]}))
                .collect();
            serde_json::json!({"content": content, "links": kept, "links_base": origin})
        }
        /// `(rendered length, was_truncated)` — a page that fits outright is not expected to fill the cap.
        fn budgeted(content: &str, links: &[String], cap: usize, origin: &str) -> (usize, bool) {
            let rendered = |budget: usize| {
                let out = cut(content, budget);
                render_page_body(&response(&out, links, origin))
                    .chars()
                    .count()
            };
            let total = |budget: usize| {
                let out = cut(content, budget);
                out.chars().count() + table_cost(&out, links, origin)
            };
            if total(cap) <= cap {
                return (rendered(cap), false);
            }
            let (mut lo, mut hi, mut best) = (0usize, cap, rendered(0));
            for _ in 0..24 {
                if lo >= hi {
                    break;
                }
                let mid = (lo + hi).div_ceil(2);
                if total(mid) <= cap {
                    best = rendered(mid);
                    lo = mid;
                } else {
                    hi = mid - 1;
                }
            }
            (best, true)
        }

        // Shapes measured on #6624: a link-dense article whose table alone exceeds the default cap, a page that fits outright, and two caps far smaller than the page.
        for (n_links, prose_len, cap, url_len) in [
            (1383, 122_173, 50_000, 40),
            (197, 14_612, 50_000, 30),
            (500, 90_000, 5_000, 50),
            (2000, 200_000, 1_000, 80),
        ] {
            let (content, links) = page(n_links, prose_len, url_len);
            // A real origin, since the table's opening line carries it and so its length counts against the cap.
            let (total, truncated) = budgeted(&content, &links, cap, "https://en.wikipedia.org");
            assert!(
                total <= cap,
                "cap {cap}: the rendered page is {total} chars, past the ceiling the operator set"
            );
            // Only where the page actually overflows: the HN-shaped row above fits whole at 23,904 and must not be padded out to the cap to satisfy this.
            if truncated {
                assert!(
                    total * 100 >= cap * 95,
                    "cap {cap}: only {total} chars used, leaving most of the operator's budget unspent"
                );
            }
        }
    }

    #[test]
    fn test_parse_link_marker_accepts_the_form_the_content_emits() {
        assert_eq!(parse_link_marker("\u{27e8}12\u{27e9}"), Some(12));
        assert_eq!(parse_link_marker("  \u{27e8}3\u{27e9} "), Some(3));
    }

    /// A bare number is not a marker on sight — it is one only after the selector paths have failed.
    ///
    /// `cmd_click` has always been able to click a page's own numeric link text, a pagination `5` or a numbered tab.
    /// Claiming a bare number as a marker before that path runs would click whichever link happens to hold that id instead, silently and with nothing to tell them apart.
    #[test]
    fn test_a_bare_number_is_only_a_marker_as_a_fallback() {
        assert_eq!(parse_link_marker("12"), None, "no brackets, no marker");
        assert_eq!(parse_bare_marker("12"), Some(12));
        assert_eq!(parse_bare_marker(" 3 "), Some(3));
    }

    #[test]
    fn test_parse_link_marker_leaves_selectors_alone() {
        // The important one: a class containing digits must stay a CSS selector.
        for selector in [".col-12", "a.link-3", "Sign in", "", "page 12"] {
            assert_eq!(parse_link_marker(selector), None);
            assert_eq!(parse_bare_marker(selector), None);
        }
        // Ids start at 1, so 0 is not a marker.
        assert_eq!(parse_bare_marker("0"), None);
        assert_eq!(parse_link_marker("\u{27e8}0\u{27e9}"), None);
        assert_eq!(parse_link_marker("\u{27e8}\u{27e9}"), None);
    }

    /// The truncation marker counts against the cap rather than overshooting it.
    ///
    /// Ported from the JS the script runs, because the extraction itself needs a live browser: an operator sizing `max_content_chars` to a context window gets a real ceiling, and the marker reports the pre-truncation length so the model can tell how much it is missing rather than only that something was lost (#6624).
    #[test]
    fn test_truncation_marker_fits_inside_the_cap() {
        // As above: this port predates the others and had nothing tying it to the script either.
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("... (truncated, ' + total + ' chars total)"),
            "the marker must report the pre-truncation length, which is what makes it worth its own characters"
        );

        fn truncate(content: &str, cap: usize) -> String {
            if content.chars().count() <= cap {
                return content.to_string();
            }
            let total = content.chars().count();
            let marker = format!("\n... (truncated, {total} chars total)");
            let keep = cap.saturating_sub(marker.chars().count());
            let head: String = content.chars().take(keep).collect();
            format!("{head}{marker}")
        }

        for cap in [200usize, 1_000, 50_000] {
            let content = "x".repeat(cap * 2);
            let out = truncate(&content, cap);
            assert!(
                out.chars().count() <= cap,
                "cap {cap}: output is {} chars, which exceeds the cap",
                out.chars().count()
            );
            assert!(
                out.contains(&format!("{} chars total", cap * 2)),
                "cap {cap}: the marker must report the pre-truncation length"
            );
        }

        // Under the cap, nothing is appended at all.
        let short = "hello";
        assert_eq!(truncate(short, 50_000), short);

        // A cap smaller than the marker degrades to marker-only rather than panicking or going negative.
        let out = truncate(&"x".repeat(100), 5);
        assert!(out.contains("100 chars total"));
    }

    #[test]
    fn test_browser_config_max_content_chars_default() {
        assert_eq!(
            BrowserConfig::default().max_content_chars,
            50_000,
            "the default must stay at the historical compile-time cap so existing installs see no change"
        );
    }

    /// The field must be `#[serde(default)]`-backed: an existing `config.toml` with a `[browser]` table that predates this field must still parse.
    #[test]
    fn test_browser_config_omitting_max_content_chars_uses_default() {
        let toml = r#"
            enabled = true
            headless = true
            viewport_width = 1280
            viewport_height = 720
            timeout_secs = 30
            idle_timeout_secs = 300
            max_sessions = 5
        "#;
        let config: BrowserConfig = toml::from_str(toml).expect("pre-#6624 [browser] table parses");
        assert_eq!(config.max_content_chars, 50_000);
    }

    #[test]
    fn test_response_helpers() {
        let ok = BrowserResponse::ok(serde_json::json!({"a": 1}));
        assert!(ok.success);
        assert!(ok.error.is_none());

        let err = BrowserResponse::err("bad");
        assert!(!err.success);
        assert_eq!(err.error.unwrap(), "bad");
    }

    #[test]
    fn test_navigation_error_text_classifies_cdp_failures() {
        let failed = serde_json::json!({
            "frameId": "frame-1",
            "errorText": "net::ERR_NAME_NOT_RESOLVED"
        });
        assert_eq!(
            navigation_error_text(&failed),
            Some("net::ERR_NAME_NOT_RESOLVED")
        );

        assert_eq!(
            navigation_error_text(&serde_json::json!({"frameId": "frame-1"})),
            None
        );
        assert_eq!(
            navigation_error_text(&serde_json::json!({"errorText": "  "})),
            None
        );
    }

    #[test]
    fn test_cdp_message_without_session_has_no_session_id() {
        let msg = build_cdp_message(7, "Runtime.enable", serde_json::json!({}), None);
        assert_eq!(msg["id"], 7);
        assert_eq!(msg["method"], "Runtime.enable");
        assert!(
            msg.get("sessionId").is_none(),
            "browser-level commands must not carry sessionId"
        );
    }

    #[test]
    fn test_cdp_message_with_session_is_routed_to_target() {
        let msg = build_cdp_message(8, "Page.enable", serde_json::json!({}), Some("SID-1"));
        assert_eq!(msg["sessionId"], "SID-1");
    }

    #[test]
    fn test_cdp_message_always_emits_params() {
        // Lightpanda rejects commands that omit `params` entirely.
        let msg = build_cdp_message(9, "Page.enable", serde_json::json!({}), None);
        assert!(msg.get("params").is_some());
        assert!(msg["params"].is_object());

        let with_params = build_cdp_message(
            10,
            "Page.navigate",
            serde_json::json!({ "url": "about:blank" }),
            None,
        );
        assert_eq!(with_params["params"]["url"], "about:blank");
    }

    // ── attach() against a scripted CDP server ─────────────────────────

    /// Which shape of CDP server the mock should imitate.
    #[derive(Clone, Copy)]
    enum MockCdp {
        /// A page-level endpoint: `Target.getTargetInfo` reports `type: "page"`.
        PageLevel,
        /// A browser-level endpoint: `Target.getTargetInfo` reports `type: "browser"`.
        BrowserLevel,
        /// A browser-level endpoint whose `Target.attachToTarget` fails after the target was created.
        AttachFails,
        /// An endpoint that does not implement `Target.getTargetInfo` at all.
        NoTargetDomain,
        /// A strict endpoint that drops the socket instead of answering an unknown method.
        DropsSocketOnProbe,
        /// A page-level endpoint whose `Page.enable` fails, after the tab already exists.
        EnableFails,
        /// A browser-level endpoint whose handshake succeeds and whose `Page.enable` then fails.
        BrowserLevelEnableFails,
    }

    fn mock_reply(kind: MockCdp, req: &serde_json::Value) -> serde_json::Value {
        let id = req["id"].as_u64().unwrap();
        let method = req["method"].as_str().unwrap();
        let has_session = req.get("sessionId").is_some();
        let err = |msg: &str| serde_json::json!({ "id": id, "error": { "message": msg } });
        let ok = |result: serde_json::Value| serde_json::json!({ "id": id, "result": result });

        match (kind, method) {
            (MockCdp::NoTargetDomain, "Target.getTargetInfo") => {
                err("'Target.getTargetInfo' wasn't found")
            }
            (MockCdp::EnableFails | MockCdp::BrowserLevelEnableFails, "Page.enable") => {
                err("Target closed")
            }
            (
                MockCdp::PageLevel | MockCdp::DropsSocketOnProbe | MockCdp::EnableFails,
                "Target.getTargetInfo",
            ) => ok(serde_json::json!({ "targetInfo": { "type": "page", "targetId": "P-1" } })),
            (_, "Target.getTargetInfo") => {
                ok(serde_json::json!({ "targetInfo": { "type": "browser", "targetId": "B-1" } }))
            }
            // A browser-level endpoint has no page yet, so session-less page commands fail.
            // `AttachFails` never reaches here: its `Target.attachToTarget` arm below fails first, and `attach()` returns before sending `Runtime.enable`.
            (MockCdp::BrowserLevel, "Runtime.enable") if !has_session => {
                err("'Runtime.enable' wasn't found")
            }
            (_, "Target.createTarget") => ok(serde_json::json!({ "targetId": "T-1" })),
            (MockCdp::AttachFails, "Target.attachToTarget") => err("TargetAlreadyLoaded"),
            (_, "Target.attachToTarget") => ok(serde_json::json!({ "sessionId": "S-1" })),
            _ => ok(serde_json::json!({})),
        }
    }

    /// Serve one scripted CDP connection, recording every method it receives.
    async fn spawn_mock_cdp(kind: MockCdp) -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&seen);

        // Serve every connection, not just the first: the page-level fallback reconnects, and a mock that accepted once would fail that reconnect for reasons unrelated to the test.
        tokio::spawn(async move {
            let connections = Arc::new(AtomicU64::new(0));
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let nth = connections.fetch_add(1, Ordering::SeqCst);
                let recorder = Arc::clone(&recorder);
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    while let Some(Ok(msg)) = ws.next().await {
                        let text = match msg {
                            WsMessage::Text(t) => t.to_string(),
                            WsMessage::Close(_) => break,
                            _ => continue,
                        };
                        let req: serde_json::Value = serde_json::from_str(&text).unwrap();
                        let method = req["method"].as_str().unwrap().to_string();
                        recorder.lock().await.push(method.clone());

                        // Hang up mid-request on the first connection, the way a strict server rejecting an unknown method would.
                        if matches!(kind, MockCdp::DropsSocketOnProbe)
                            && nth == 0
                            && method == "Target.getTargetInfo"
                        {
                            return;
                        }

                        let reply = mock_reply(kind, &req);
                        if ws
                            .send(WsMessage::Text(reply.to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            }
        });

        (format!("ws://127.0.0.1:{port}/"), seen)
    }

    #[tokio::test]
    async fn connect_with_auth_sends_a_complete_websocket_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/", listener.local_addr().unwrap());
        let (headers_tx, headers_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            #[allow(clippy::result_large_err)]
            let capture_headers =
                move |request: &http::Request<()>, response: http::Response<()>| {
                    let authorization = request
                        .headers()
                        .get(http::header::AUTHORIZATION)
                        .and_then(|value| value.to_str().ok())
                        .map(str::to_owned);
                    let websocket_key_present = request.headers().contains_key("Sec-WebSocket-Key");
                    headers_tx
                        .send((authorization, websocket_key_present))
                        .unwrap();
                    Ok(response)
                };
            tokio_tungstenite::accept_hdr_async(stream, capture_headers)
                .await
                .unwrap();
        });

        let connection = BrowserSession::connect_with_auth(&url, Some("test-token"))
            .await
            .expect("authenticated CDP WebSocket handshake should succeed");
        let (authorization, websocket_key_present) = headers_rx.await.unwrap();
        assert_eq!(authorization.as_deref(), Some("Bearer test-token"));
        assert!(websocket_key_present);
        drop(connection);
        server.await.unwrap();
    }

    /// A page-level endpoint must behave exactly as it did before the handshake existed.
    #[tokio::test]
    async fn test_attach_page_level_skips_handshake() {
        let (url, seen) = spawn_mock_cdp(MockCdp::PageLevel).await;
        let session = match BrowserSession::attach(&url, None).await {
            Ok(s) => s,
            Err(e) => panic!("attach should succeed on a page-level endpoint: {e}"),
        };

        assert!(session.attached_target_id.is_none());
        assert!(!session.target_created_over_cdp);
        let methods = seen.lock().await.clone();
        assert!(
            methods.contains(&"Target.getTargetInfo".to_string()),
            "the endpoint kind must be read off the protocol, saw: {methods:?}"
        );
        assert!(
            !methods.contains(&"Target.createTarget".to_string()),
            "no second tab may be opened on a page-level endpoint, saw: {methods:?}"
        );
    }

    /// A browser-level endpoint gets a target created and attached, and the session records it for teardown.
    #[tokio::test]
    async fn test_attach_browser_level_creates_and_attaches_target() {
        let (url, seen) = spawn_mock_cdp(MockCdp::BrowserLevel).await;
        let session = match BrowserSession::attach(&url, None).await {
            Ok(s) => s,
            Err(e) => panic!("attach should succeed on a browser-level endpoint: {e}"),
        };

        assert_eq!(session.attached_target_id.as_deref(), Some("T-1"));
        assert!(session.target_created_over_cdp);
        assert_eq!(session.cdp.session_id.as_deref(), Some("S-1"));
        let methods = seen.lock().await.clone();
        assert!(methods.contains(&"Target.createTarget".to_string()));
        assert!(methods.contains(&"Target.attachToTarget".to_string()));
    }

    /// An endpoint without the `Target` domain keeps the pre-handshake behaviour instead of failing.
    #[tokio::test]
    async fn test_attach_without_target_domain_stays_page_level() {
        let (url, seen) = spawn_mock_cdp(MockCdp::NoTargetDomain).await;
        let session = match BrowserSession::attach(&url, None).await {
            Ok(s) => s,
            Err(e) => panic!("attach should fall back to page-level behaviour: {e}"),
        };

        assert!(session.attached_target_id.is_none());
        assert!(!session.target_created_over_cdp);
        let methods = seen.lock().await.clone();
        assert!(
            !methods.contains(&"Target.createTarget".to_string()),
            "an unknown endpoint kind must not create a target, saw: {methods:?}"
        );
    }

    /// A server that hangs up on the probe must not leave the caller worse off than not probing.
    #[tokio::test]
    async fn test_attach_survives_endpoint_that_drops_socket_on_probe() {
        let (url, seen) = spawn_mock_cdp(MockCdp::DropsSocketOnProbe).await;
        let session = match BrowserSession::attach(&url, None).await {
            Ok(s) => s,
            Err(e) => panic!("attach must reconnect and fall back to page-level: {e}"),
        };

        assert!(session.attached_target_id.is_none());
        assert!(!session.target_created_over_cdp);
        let methods = seen.lock().await.clone();
        assert!(
            methods.iter().any(|m| m == "Page.enable"),
            "the page-level path must still run after the reconnect, saw: {methods:?}"
        );
        assert!(
            !methods.contains(&"Target.createTarget".to_string()),
            "a dropped probe is not evidence of a browser-level endpoint, saw: {methods:?}"
        );
    }

    /// A tab from `/json/new` must be closed too when the attach fails after discovery.
    #[tokio::test]
    async fn test_attach_closes_http_discovered_tab_when_enable_fails() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let (ws_url, _seen) = spawn_mock_cdp(MockCdp::EnableFails).await;
        let http = MockServer::start().await;

        // `http_discover_target` (#6619) tries PUT first; this mock must
        // match that or every request 404s against wiremock's unmatched-route
        // default before the attach logic under test ever runs.
        Mock::given(method("PUT"))
            .and(path("/json/new"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "TAB-9",
                "webSocketDebuggerUrl": ws_url,
            })))
            .expect(1)
            .mount(&http)
            .await;
        // The assertion is this mock's expectation: without the cleanup nothing requests it, and `MockServer` fails the test on drop.
        Mock::given(method("GET"))
            .and(path("/json/close/TAB-9"))
            .respond_with(ResponseTemplate::new(200).set_body_string("Target is closing"))
            .expect(1)
            .mount(&http)
            .await;

        let err = match BrowserSession::attach(&http.uri(), None).await {
            Ok(_) => panic!("Page.enable failure must fail the attach"),
            Err(e) => e,
        };
        assert!(
            err.contains("Target closed"),
            "the original failure must survive the cleanup, got: {err}"
        );
    }

    /// A target that survived the handshake must still be closed when enabling domains fails.
    #[tokio::test]
    async fn test_attach_closes_created_target_when_enable_fails() {
        let (url, seen) = spawn_mock_cdp(MockCdp::BrowserLevelEnableFails).await;
        let err = match BrowserSession::attach(&url, None).await {
            Ok(_) => panic!("Page.enable failure must fail the attach"),
            Err(e) => e,
        };

        assert!(
            err.contains("Target closed"),
            "the original failure must survive the cleanup, got: {err}"
        );
        let methods = seen.lock().await.clone();
        assert!(
            methods.contains(&"Target.attachToTarget".to_string()),
            "the handshake must have completed before the failure, saw: {methods:?}"
        );
        assert!(
            methods.contains(&"Target.closeTarget".to_string()),
            "a target that outlived its failed attach is a leaked tab, saw: {methods:?}"
        );
    }

    /// A handshake that fails after `Target.createTarget` must not abandon the target it just created.
    #[tokio::test]
    async fn test_attach_closes_target_when_handshake_fails() {
        let (url, seen) = spawn_mock_cdp(MockCdp::AttachFails).await;
        let err = match BrowserSession::attach(&url, None).await {
            Ok(_) => panic!("attach must fail when Target.attachToTarget is rejected"),
            Err(e) => e,
        };

        assert!(err.contains("attachToTarget"), "unexpected error: {err}");
        let methods = seen.lock().await.clone();
        assert!(
            methods.contains(&"Target.closeTarget".to_string()),
            "the created target must be closed before propagating, saw: {methods:?}"
        );
    }

    /// Chrome 111+ serves `/json/new` on PUT only, answering GET with 405.
    #[tokio::test]
    async fn test_http_discovery_uses_put() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // The counts are the assertion: a PUT-second implementation reaches the same result by trying GET first, so only the call counts tell the two orders apart.
        Mock::given(method("GET"))
            .and(path("/json/new"))
            .respond_with(ResponseTemplate::new(405))
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/json/new"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "TAB-1",
                "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/TAB-1",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (ws, id) = http_discover_target(&server.uri()).await.unwrap();
        assert_eq!(ws, "ws://127.0.0.1:9222/devtools/page/TAB-1");
        assert_eq!(id.as_deref(), Some("TAB-1"));
    }

    /// Endpoints that route only GET still work: a 405 on PUT falls back.
    #[tokio::test]
    async fn test_http_discovery_falls_back_to_get_on_405() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Both verbs must be exercised: a GET-first implementation would answer from the GET mock alone and never prove that PUT was attempted at all.
        Mock::given(method("PUT"))
            .and(path("/json/new"))
            .respond_with(ResponseTemplate::new(405))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/json/new"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "TAB-2",
                "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/TAB-2",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (ws, id) = http_discover_target(&server.uri()).await.unwrap();
        assert_eq!(ws, "ws://127.0.0.1:9222/devtools/page/TAB-2");
        assert_eq!(id.as_deref(), Some("TAB-2"));
    }

    /// A non-2xx that is not 405 surfaces as an error instead of a JSON parse failure further down.
    #[tokio::test]
    async fn test_http_discovery_reports_non_success_status() {
        use wiremock::matchers::path;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(path("/json/new"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let err = http_discover_target(&server.uri()).await.unwrap_err();
        assert!(err.contains("404"), "unexpected error: {err}");
    }
}

// ── Tool handler functions ─────────────────────────────────────────────────
