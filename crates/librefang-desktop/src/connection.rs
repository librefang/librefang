//! Connection screen and remote/local mode switching for the desktop app.
//!
//! Provides a self-contained HTML connection page and Tauri IPC commands
//! for testing connections, connecting to remote servers, and starting local
//! servers on demand.

use librefang_kernel::config::librefang_home;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use tauri::Manager;
use tracing::{info, warn};

/// Persisted connection preference stored in `~/.librefang/desktop.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionPreference {
    pub mode: String,
    #[serde(default)]
    pub server_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct DesktopConfig {
    #[serde(default)]
    connection: Option<ConnectionPreference>,
}

/// Load saved connection preference from `~/.librefang/desktop.toml`.
pub fn load_saved_preference() -> Option<ConnectionPreference> {
    let path = librefang_home().join("desktop.toml");
    let content = std::fs::read_to_string(path).ok()?;
    let config: DesktopConfig = toml::from_str(&content).ok()?;
    config.connection
}

/// Save connection preference to `~/.librefang/desktop.toml`.
pub fn save_preference(pref: &ConnectionPreference) {
    let path = librefang_home().join("desktop.toml");
    let config = DesktopConfig {
        connection: Some(pref.clone()),
    };
    if let Ok(content) = toml::to_string_pretty(&config) {
        let _ = std::fs::create_dir_all(librefang_home());
        if let Err(e) = std::fs::write(&path, content) {
            warn!("Failed to save desktop preference: {e}");
        }
    }
}

/// Test connectivity to a remote LibreFang server by hitting `/api/health`.
#[tauri::command]
pub async fn test_connection(url: String) -> Result<serde_json::Value, String> {
    let url = url.trim_end_matches('/').to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    let health_url = format!("{url}/api/health");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    let resp = client
        .get(&health_url)
        .send()
        .await
        .map_err(|e| format!("Connection failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("Server returned status {}", resp.status()));
    }

    resp.json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Invalid response: {e}"))
}

/// Connect to a remote LibreFang server. Validates the URL, optionally saves
/// the preference, and navigates the WebView to the remote dashboard.
#[tauri::command]
pub async fn connect_remote(
    url: String,
    remember: bool,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    let url = url.trim_end_matches('/').to_string();
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    if remember {
        save_preference(&ConnectionPreference {
            mode: "remote".to_string(),
            server_url: Some(url.clone()),
        });
    }

    // Store the server URL in managed state
    let app = window.app_handle();
    app.manage(crate::ServerUrlState(url.clone()));
    app.manage(crate::RemoteMode(true));

    info!("Connecting to remote server: {url}");

    // Navigate WebView to the remote dashboard
    let js = format!(
        "window.location.href = {};",
        serde_json::to_string(&url).unwrap_or_default()
    );
    window
        .eval(&js)
        .map_err(|e| format!("Navigation failed: {e}"))?;

    Ok(())
}

/// Start a local LibreFang server, store state, optionally save preference,
/// and navigate the WebView to the local dashboard.
#[tauri::command]
pub async fn start_local(
    remember: bool,
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), String> {
    info!("Starting local LibreFang server...");

    // Boot kernel + server on a blocking thread.
    // Wrap in a closure to convert the non-Send error into a String.
    let handle =
        tokio::task::spawn_blocking(|| crate::server::start_server().map_err(|e| e.to_string()))
            .await
            .map_err(|e| format!("Server task panicked: {e}"))?
            .map_err(|e| format!("Failed to start server: {e}"))?;

    let port = handle.port;
    let url = format!("http://127.0.0.1:{port}");

    // Store state for tray and commands
    app.manage(crate::PortState(port));
    app.manage(crate::KernelState {
        kernel: handle.kernel.clone(),
        started_at: Instant::now(),
    });
    app.manage(crate::ServerUrlState(url.clone()));
    app.manage(crate::RemoteMode(false));

    // Store the ServerHandle for shutdown
    if let Some(holder) = app.try_state::<crate::ServerHandleHolder>() {
        let mut guard = holder.0.lock().expect("ServerHandleHolder lock poisoned");
        *guard = Some(handle);
    }

    // Start event forwarding for native notifications
    if let Some(ks) = app.try_state::<crate::KernelState>() {
        let app_handle = app.clone();
        let mut event_rx = ks.kernel.event_bus_ref().subscribe_all();
        tauri::async_runtime::spawn(async move {
            crate::forward_kernel_events(app_handle, &mut event_rx).await;
        });
    }

    if remember {
        save_preference(&ConnectionPreference {
            mode: "local".to_string(),
            server_url: None,
        });
    }

    info!("Local server running on port {port}");

    // Navigate WebView to the local dashboard
    let js = format!(
        "window.location.href = {};",
        serde_json::to_string(&url).unwrap_or_default()
    );
    window
        .eval(&js)
        .map_err(|e| format!("Navigation failed: {e}"))?;

    Ok(())
}

/// Returns self-contained HTML/CSS/JS for the connection screen.
pub fn connection_html() -> String {
    r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>LibreFang — Connect</title>
<style>
  *, *::before, *::after { box-sizing: border-box; margin: 0; padding: 0; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, sans-serif;
    background: #0f1117;
    color: #e4e4e7;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100vh;
  }
  .container {
    width: 420px;
    max-width: 90vw;
    background: #1a1b23;
    border: 1px solid #2a2b35;
    border-radius: 16px;
    padding: 40px 36px 36px;
  }
  .logo {
    text-align: center;
    margin-bottom: 32px;
  }
  .logo h1 {
    font-size: 28px;
    font-weight: 700;
    color: #f4f4f5;
    letter-spacing: -0.5px;
  }
  .logo p {
    font-size: 14px;
    color: #71717a;
    margin-top: 6px;
  }
  label {
    display: block;
    font-size: 13px;
    font-weight: 500;
    color: #a1a1aa;
    margin-bottom: 6px;
  }
  input[type="text"] {
    width: 100%;
    padding: 10px 14px;
    font-size: 14px;
    background: #0f1117;
    border: 1px solid #2a2b35;
    border-radius: 8px;
    color: #f4f4f5;
    outline: none;
    transition: border-color 0.15s;
  }
  input[type="text"]:focus {
    border-color: #6366f1;
  }
  input[type="text"]::placeholder {
    color: #52525b;
  }
  .btn-row {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
  .btn {
    flex: 1;
    padding: 10px 16px;
    font-size: 14px;
    font-weight: 500;
    border: none;
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.15s, opacity 0.15s;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn-test {
    background: #27272a;
    color: #d4d4d8;
  }
  .btn-test:hover:not(:disabled) {
    background: #3f3f46;
  }
  .btn-connect {
    background: #6366f1;
    color: #fff;
  }
  .btn-connect:hover:not(:disabled) {
    background: #4f46e5;
  }
  .divider {
    display: flex;
    align-items: center;
    gap: 16px;
    margin: 24px 0;
    color: #52525b;
    font-size: 13px;
  }
  .divider::before, .divider::after {
    content: '';
    flex: 1;
    height: 1px;
    background: #27272a;
  }
  .btn-local {
    width: 100%;
    padding: 10px 16px;
    font-size: 14px;
    font-weight: 500;
    background: #27272a;
    color: #d4d4d8;
    border: 1px solid #3f3f46;
    border-radius: 8px;
    cursor: pointer;
    transition: background 0.15s;
  }
  .btn-local:hover:not(:disabled) {
    background: #3f3f46;
  }
  .remember-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 20px;
  }
  .remember-row input[type="checkbox"] {
    accent-color: #6366f1;
  }
  .remember-row label {
    margin: 0;
    font-size: 13px;
    color: #a1a1aa;
    cursor: pointer;
  }
  .status {
    margin-top: 16px;
    min-height: 20px;
    font-size: 13px;
    text-align: center;
    transition: color 0.2s;
  }
  .status.error { color: #ef4444; }
  .status.success { color: #22c55e; }
  .status.info { color: #6366f1; }
</style>
</head>
<body>
<div class="container">
  <div class="logo">
    <h1>LibreFang</h1>
    <p>Agent Operating System</p>
  </div>

  <label for="url-input">Server URL</label>
  <input type="text" id="url-input" placeholder="http://192.168.1.100:4545" spellcheck="false">

  <div class="btn-row">
    <button class="btn btn-test" id="btn-test" onclick="testConn()">Test Connection</button>
    <button class="btn btn-connect" id="btn-connect" onclick="connectRemote()">Connect</button>
  </div>

  <div class="divider">or</div>

  <button class="btn-local" id="btn-local" onclick="startLocal()">Start Local Server</button>

  <div class="remember-row">
    <input type="checkbox" id="remember" checked>
    <label for="remember">Remember this choice</label>
  </div>

  <div class="status" id="status"></div>
</div>

<script>
  const { invoke } = window.__TAURI__.core;

  function setStatus(msg, cls) {
    const el = document.getElementById('status');
    el.textContent = msg;
    el.className = 'status ' + (cls || '');
  }

  function setAllDisabled(disabled) {
    document.getElementById('btn-test').disabled = disabled;
    document.getElementById('btn-connect').disabled = disabled;
    document.getElementById('btn-local').disabled = disabled;
    document.getElementById('url-input').disabled = disabled;
  }

  async function testConn() {
    const url = document.getElementById('url-input').value.trim();
    if (!url) { setStatus('Please enter a server URL.', 'error'); return; }
    setStatus('Testing connection...', 'info');
    setAllDisabled(true);
    try {
      const result = await invoke('test_connection', { url });
      setStatus('Connected! Server is healthy.', 'success');
    } catch (e) {
      setStatus(String(e), 'error');
    } finally {
      setAllDisabled(false);
    }
  }

  async function connectRemote() {
    const url = document.getElementById('url-input').value.trim();
    if (!url) { setStatus('Please enter a server URL.', 'error'); return; }
    const remember = document.getElementById('remember').checked;
    setStatus('Connecting...', 'info');
    setAllDisabled(true);
    try {
      await invoke('connect_remote', { url, remember });
    } catch (e) {
      setStatus(String(e), 'error');
      setAllDisabled(false);
    }
  }

  async function startLocal() {
    const remember = document.getElementById('remember').checked;
    setStatus('Starting local server...', 'info');
    setAllDisabled(true);
    try {
      await invoke('start_local', { remember });
    } catch (e) {
      setStatus(String(e), 'error');
      setAllDisabled(false);
    }
  }
</script>
</body>
</html>"##
        .to_string()
}
