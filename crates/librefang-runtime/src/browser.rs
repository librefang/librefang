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
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::io::AsyncBufReadExt;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info, warn};

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ── Constants ──────────────────────────────────────────────────────────────

const CDP_CONNECT_TIMEOUT_SECS: u64 = 15;
const CDP_COMMAND_TIMEOUT_SECS: u64 = 30;
const PAGE_LOAD_POLL_INTERVAL_MS: u64 = 200;
const PAGE_LOAD_MAX_POLLS: u32 = 150; // 30 seconds
/// Cap on the extracted page text handed back to the model.
const MAX_CONTENT_CHARS: usize = 50_000;
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

impl BrowserSession {
    /// Launch Chromium and establish a CDP connection.
    async fn launch(config: &BrowserConfig) -> Result<Self, String> {
        let chrome_path = find_chromium(config)?;
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
            let req = http::Request::get(ws_url)
                .header("Authorization", format!("Bearer {token}"))
                .body(())
                .map_err(|e| format!("Failed to build CDP auth request: {e}"))?;
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
    async fn execute(&mut self, cmd: BrowserCommand) -> BrowserResponse {
        self.last_active = Instant::now();
        match cmd {
            BrowserCommand::Navigate { url } => self.cmd_navigate(&url).await,
            BrowserCommand::Click { selector } => self.cmd_click(&selector).await,
            BrowserCommand::Type { selector, text } => self.cmd_type(&selector, &text).await,
            BrowserCommand::Screenshot => self.cmd_screenshot().await,
            BrowserCommand::ReadPage => self.cmd_read_page().await,
            BrowserCommand::Close => BrowserResponse::ok(serde_json::json!({"closed": true})),
            BrowserCommand::Scroll { direction, amount } => {
                self.cmd_scroll(&direction, amount).await
            }
            BrowserCommand::Wait {
                selector,
                timeout_ms,
            } => self.cmd_wait(&selector, timeout_ms).await,
            BrowserCommand::RunJs { expression } => self.cmd_run_js(&expression).await,
            BrowserCommand::Back => self.cmd_back().await,
        }
    }

    // ── Command implementations ────────────────────────────────────────

    async fn cmd_navigate(&self, url: &str) -> BrowserResponse {
        let result = self
            .cdp
            .send("Page.navigate", serde_json::json!({ "url": url }))
            .await;

        if let Err(e) = result {
            return BrowserResponse::err(format!("Navigate failed: {e}"));
        }

        // Wait for page load
        self.wait_for_load().await;

        match self.page_info().await {
            Ok(info) => BrowserResponse::ok(info),
            Err(e) => BrowserResponse::err(format!("Navigate succeeded but page info failed: {e}")),
        }
    }

    async fn cmd_click(&self, selector: &str) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    let sel = {sel_json};
    let el = document.querySelector(sel);
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
                match self.page_info().await {
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

    async fn cmd_read_page(&self) -> BrowserResponse {
        match self.cdp.run_js(&EXTRACT_CONTENT_JS).await {
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

    async fn cmd_back(&self) -> BrowserResponse {
        match self.cdp.run_js("history.back(); 'ok'").await {
            Ok(_) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.wait_for_load().await;
                match self.page_info().await {
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
    async fn page_info(&self) -> Result<serde_json::Value, String> {
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
            .run_js(&EXTRACT_CONTENT_JS)
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
        let resp = guard.execute(cmd).await;

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
    const remove = ['script','style','nav','footer','header','aside','iframe','noscript','svg','canvas'];
    remove.forEach(tag => clone.querySelectorAll(tag).forEach(el => el.remove()));

    let root = clone.querySelector('main, article, [role="main"], .content, #content');
    if (!root) root = clone;

    const lines = [];
    function walk(node) {
        if (node.nodeType === 3) {
            const t = node.textContent.trim();
            if (t) lines.push(t);
            return;
        }
        if (node.nodeType !== 1) return;
        const tag = node.tagName.toLowerCase();
        if (['h1','h2','h3','h4','h5','h6'].includes(tag)) {
            const level = '#'.repeat(parseInt(tag[1]));
            lines.push('\n' + level + ' ' + node.textContent.trim());
            return;
        }
        if (tag === 'a' && node.href && node.textContent.trim()) {
            lines.push('[' + node.textContent.trim() + '](' + node.href + ')');
            return;
        }
        if (tag === 'li') {
            lines.push('- ' + node.textContent.trim());
            return;
        }
        if (tag === 'br') { lines.push(''); return; }
        if (['p','div','section','tr'].includes(tag)) lines.push('');
        for (const child of node.childNodes) walk(child);
        if (['p','div','section','tr'].includes(tag)) lines.push('');
    }
    walk(root);

    let content = lines.join('\n').replace(/\n{3,}/g, '\n\n').trim();
    if (content.length > __MAX_CONTENT_CHARS__) content = content.substring(0, __MAX_CONTENT_CHARS__) + '\n... (truncated)';
    return JSON.stringify({title, url, content});
})()"#;

/// Page-extraction script with the cap substituted in, built once.
pub(crate) static EXTRACT_CONTENT_JS: LazyLock<String> = LazyLock::new(|| {
    EXTRACT_CONTENT_JS_TEMPLATE.replace("__MAX_CONTENT_CHARS__", &MAX_CONTENT_CHARS.to_string())
});

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The cap the model sees must be the one the constant declares.
    ///
    /// The script used to hard-code `50000` while `MAX_CONTENT_CHARS` sat unused, so changing the constant changed nothing.
    #[test]
    fn test_extract_content_js_uses_max_content_chars() {
        assert!(
            !EXTRACT_CONTENT_JS.contains("__MAX_CONTENT_CHARS__"),
            "placeholder must be substituted before the script is sent"
        );
        assert!(
            EXTRACT_CONTENT_JS.contains(&MAX_CONTENT_CHARS.to_string()),
            "the script must truncate at MAX_CONTENT_CHARS"
        );
        // Guards against the placeholder being dropped from the template entirely, which would leave the two silently independent again.
        assert!(
            EXTRACT_CONTENT_JS_TEMPLATE.contains("__MAX_CONTENT_CHARS__"),
            "the template must keep the placeholder"
        );
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
