//! ChatGPT OAuth 2.0 authentication helpers for browser and device login.
//!
//! Uses OpenAI's official Codex OAuth endpoints to authenticate. The flow
//! can either open the user's browser and wait for the localhost callback or
//! use the device auth flow for headless environments, then exchange the
//! resulting authorization code for access and refresh tokens.

use std::net::TcpListener;
use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Default ChatGPT API base URL (ChatGPT backend, used with OAuth tokens).
/// OAuth tokens with `api.connectors` scopes work with the Responses API
/// at this endpoint, NOT the standard `/v1/chat/completions` endpoint.
pub const CHATGPT_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// OAuth client ID (OpenAI Codex CLI).
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

/// OpenAI OAuth authorization endpoint.
const AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";

/// OpenAI OAuth token endpoint.
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// Device auth endpoint for requesting a one-time user code.
const DEVICE_AUTH_USERCODE_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/usercode";

/// Device auth endpoint for polling authorization completion.
const DEVICE_AUTH_TOKEN_URL: &str = "https://auth.openai.com/api/accounts/deviceauth/token";

/// Device auth verification page shown to the user.
pub const DEVICE_AUTH_URL: &str = "https://auth.openai.com/codex/device";

/// Device auth redirect URI used for the final token exchange.
pub const DEVICE_AUTH_REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";

/// OAuth scopes.
const SCOPE: &str = "openid profile email offline_access api.connectors.read api.connectors.invoke";

/// Local callback server bind address (port 1455 matches OpenAI's registered redirect_uri).
const CALLBACK_BIND: &str = "127.0.0.1:1455";

/// Local callback server timeout (seconds).
const AUTH_TIMEOUT_SECS: u64 = 300;

/// Device auth poll timeout (seconds).
const DEVICE_AUTH_TIMEOUT_SECS: u64 = 15 * 60;

/// Default server poll interval when the response omits a usable value.
const DEFAULT_DEVICE_AUTH_POLL_INTERVAL_SECS: u64 = 5;

/// Result of a successful OAuth flow.
pub struct ChatGptAuthResult {
    /// The bearer access token.
    pub access_token: Zeroizing<String>,
    /// The refresh token for obtaining new access tokens.
    pub refresh_token: Option<Zeroizing<String>>,
    /// Seconds until the access token expires (from server response).
    pub expires_in: Option<u64>,
}

/// Device auth prompt details that must be shown to the user before polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAuthPrompt {
    /// Server-issued device auth identifier used for polling.
    pub device_auth_id: String,
    /// One-time code the user must enter on the verification page.
    pub user_code: String,
    /// Recommended poll interval from the server, in seconds.
    pub interval_secs: u64,
}

/// Errors returned by the device auth flow.
#[derive(Debug, thiserror::Error)]
pub enum DeviceAuthFlowError {
    /// Device auth is not enabled for the current OpenAI account or workspace.
    #[error("{message}")]
    BrowserFallback { message: String },
    /// Device auth failed and should not silently fall back.
    #[error("{0}")]
    Fatal(String),
}

impl DeviceAuthFlowError {
    /// Return true when the caller should fall back to browser auth.
    pub fn should_fallback_to_browser(&self) -> bool {
        matches!(self, Self::BrowserFallback { .. })
    }

    /// Return the user-facing message for logging or CLI output.
    pub fn message(&self) -> &str {
        match self {
            Self::BrowserFallback { message } => message,
            Self::Fatal(message) => message,
        }
    }
}

#[derive(Debug, Deserialize)]
struct DeviceAuthPromptEnvelope {
    device_auth_id: String,
    #[serde(alias = "usercode")]
    user_code: Option<String>,
    interval: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct DeviceAuthPollEnvelope {
    authorization_code: String,
    code_verifier: String,
}

/// PKCE code verifier and challenge pair.
pub struct PkceChallenge {
    /// The code_verifier (random 32 bytes, base64url-encoded).
    pub verifier: String,
    /// The code_challenge (SHA-256 of verifier, base64url-encoded).
    pub challenge: String,
}

/// Generate a PKCE code verifier and S256 challenge.
pub fn generate_pkce() -> PkceChallenge {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut bytes = [0u8; 64];
    rng.fill(&mut bytes);

    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        URL_SAFE_NO_PAD.encode(hasher.finalize())
    };

    PkceChallenge {
        verifier,
        challenge,
    }
}

/// Generate a random state parameter (16 bytes, hex-encoded).
pub fn create_state() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    let mut bytes = [0u8; 16];
    rng.fill(&mut bytes);
    hex::encode(bytes)
}

/// Build the full authorization URL with all required parameters.
pub fn build_authorization_url(port: u16, code_challenge: &str, state: &str) -> String {
    let redirect_uri = browser_redirect_uri(port);

    // Build query parameters manually to keep full control of encoding.
    let params = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("redirect_uri", &redirect_uri),
        ("scope", SCOPE),
        ("state", state),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "codex_cli_rs"),
    ];

    let query: String = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencod(v)))
        .collect::<Vec<_>>()
        .join("&");

    format!("{AUTHORIZE_URL}?{query}")
}

/// Start the OAuth flow: bind a local server, generate PKCE, build auth URL.
///
/// Returns `(auth_url, port, pkce_verifier, state)`.
pub async fn start_oauth_flow() -> Result<(String, u16, String, String), String> {
    let listener = TcpListener::bind(CALLBACK_BIND)
        .map_err(|e| format!("Failed to bind local server: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local address: {e}"))?
        .port();

    // Drop the std listener so we can re-bind with tokio later.
    drop(listener);

    let pkce = generate_pkce();
    let state = create_state();
    let auth_url = build_authorization_url(port, &pkce.challenge, &state);

    info!("OAuth flow started on port {port}");
    debug!("Authorization URL: {auth_url}");

    Ok((auth_url, port, pkce.verifier, state))
}

/// Request a one-time device auth code from OpenAI.
pub async fn start_device_auth_flow() -> Result<DeviceAuthPrompt, DeviceAuthFlowError> {
    let client = crate::http_client::proxied_client();
    let resp = client
        .post(DEVICE_AUTH_USERCODE_URL)
        .json(&serde_json::json!({ "client_id": CLIENT_ID }))
        .send()
        .await
        .map_err(|e| DeviceAuthFlowError::Fatal(format!("Device auth request failed: {e}")))?;

    let status = resp.status();
    let body = resp.text().await.map_err(|e| {
        DeviceAuthFlowError::Fatal(format!("Failed to read device auth response: {e}"))
    })?;

    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(DeviceAuthFlowError::BrowserFallback {
            message: format!(
                "Device code login does not appear to be enabled for this OpenAI account or workspace (HTTP {status}). Check OpenAI security settings or workspace permissions, then retry. Falling back to the standard browser login flow."
            ),
        });
    }

    if !status.is_success() {
        return Err(DeviceAuthFlowError::Fatal(format!(
            "Device auth request failed (HTTP {status}): {body}"
        )));
    }

    parse_device_auth_prompt_response(&body).map_err(DeviceAuthFlowError::Fatal)
}

/// Poll the device auth endpoint until the user completes verification.
pub async fn poll_device_auth_flow(prompt: &DeviceAuthPrompt) -> Result<ChatGptAuthResult, String> {
    let client = crate::http_client::proxied_client();
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_secs(DEVICE_AUTH_TIMEOUT_SECS);

    loop {
        let resp = client
            .post(DEVICE_AUTH_TOKEN_URL)
            .json(&serde_json::json!({
                "device_auth_id": prompt.device_auth_id,
                "user_code": prompt.user_code,
            }))
            .send()
            .await
            .map_err(|e| format!("Device auth poll request failed: {e}"))?;

        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read device auth poll response: {e}"))?;

        match status {
            reqwest::StatusCode::OK => {
                let poll = parse_device_auth_poll_response(&body)?;
                return exchange_code_for_tokens_with_redirect_uri(
                    &poll.authorization_code,
                    &poll.code_verifier,
                    DEVICE_AUTH_REDIRECT_URI,
                )
                .await;
            }
            reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND => {}
            _ => {
                return Err(format!(
                    "Device auth polling failed (HTTP {status}): {body}"
                ));
            }
        }

        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Device auth timed out after {} minutes",
                DEVICE_AUTH_TIMEOUT_SECS / 60
            ));
        }

        tokio::time::sleep(std::time::Duration::from_secs(prompt.interval_secs.max(1))).await;
    }
}

/// Run the full device auth flow without any browser automation.
pub async fn run_device_auth_flow() -> Result<ChatGptAuthResult, DeviceAuthFlowError> {
    let prompt = start_device_auth_flow().await?;
    poll_device_auth_flow(&prompt)
        .await
        .map_err(DeviceAuthFlowError::Fatal)
}

/// Run the local callback server, waiting for the OAuth redirect.
///
/// Listens for `GET /auth/callback?code=...&state=...`, validates the state
/// parameter, and returns the authorization code. A success HTML page is
/// served to the browser.
pub async fn run_oauth_callback_server(port: u16, expected_state: &str) -> Result<String, String> {
    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));
    let expected_state = expected_state.to_string();

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .map_err(|e| format!("Failed to bind async listener on port {port}: {e}"))?;

    debug!("OAuth callback server listening on port {port}");

    let server_handle = tokio::spawn({
        let tx = tx.clone();
        let expected_state = expected_state.clone();
        async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        warn!("Accept error: {e}");
                        continue;
                    }
                };

                let tx = tx.clone();
                let expected_state = expected_state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_oauth_callback(stream, tx, &expected_state).await {
                        debug!("Callback handler error: {e}");
                    }
                });
            }
        }
    });

    let code = tokio::time::timeout(std::time::Duration::from_secs(AUTH_TIMEOUT_SECS), rx)
        .await
        .map_err(|_| {
            "Authentication timed out -- no callback received within 5 minutes".to_string()
        })?
        .map_err(|_| "Auth channel closed unexpectedly".to_string())?;

    server_handle.abort();

    if code.is_empty() {
        return Err("Received empty authorization code".to_string());
    }

    info!("OAuth authorization code received");
    Ok(code)
}

/// Exchange the authorization code for access and refresh tokens.
pub async fn exchange_code_for_tokens(
    code: &str,
    code_verifier: &str,
    port: u16,
) -> Result<ChatGptAuthResult, String> {
    exchange_code_for_tokens_with_redirect_uri(code, code_verifier, &browser_redirect_uri(port))
        .await
}

/// Exchange an authorization code for tokens using an explicit redirect URI.
pub async fn exchange_code_for_tokens_with_redirect_uri(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Result<ChatGptAuthResult, String> {
    let params = build_token_exchange_form(code, code_verifier, redirect_uri);

    let client = crate::http_client::proxied_client();
    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read token response: {e}"))?;

    if !status.is_success() {
        return Err(format!("Token exchange failed (HTTP {status}): {body}"));
    }

    parse_chatgpt_auth_result(&body, "token response")
}

/// Refresh an expired access token using a refresh token.
pub async fn refresh_access_token(refresh_token: &str) -> Result<ChatGptAuthResult, String> {
    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", CLIENT_ID),
        ("refresh_token", refresh_token),
    ];

    let client = crate::http_client::proxied_client();
    let resp = client
        .post(TOKEN_URL)
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Token refresh request failed: {e}"))?;

    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read refresh response: {e}"))?;

    if !status.is_success() {
        return Err(format!("Token refresh failed (HTTP {status}): {body}"));
    }

    parse_chatgpt_auth_result(&body, "refresh response")
}

/// Fetch the best available Codex model from the ChatGPT backend API.
///
/// Calls `GET {base_url}/codex/models?client_version={VERSION}` with the given
/// access token, sorts by priority (highest first), and returns the model slug.
/// Falls back to `gpt-5.1-codex-mini` if the API call fails.
pub async fn fetch_best_codex_model(access_token: &str) -> String {
    const FALLBACK_MODEL: &str = "gpt-5.1-codex-mini";

    let url = format!(
        "{CHATGPT_BASE_URL}/codex/models?client_version={}",
        librefang_types::VERSION
    );
    let client = crate::http_client::proxied_client();
    let resp = match client.get(&url).bearer_auth(access_token).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to fetch Codex models: {e}");
            return FALLBACK_MODEL.to_string();
        }
    };

    if !resp.status().is_success() {
        warn!("Codex models API returned HTTP {}", resp.status());
        return FALLBACK_MODEL.to_string();
    }

    let body = match resp.text().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to read Codex models response: {e}");
            return FALLBACK_MODEL.to_string();
        }
    };

    let json: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to parse Codex models JSON: {e}");
            return FALLBACK_MODEL.to_string();
        }
    };

    // Response format: { "models": [ { "slug": "gpt-5.2-codex", "priority": 100 }, ... ] }
    let models = match json.get("models").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => {
            warn!("Codex models response missing 'models' array");
            return FALLBACK_MODEL.to_string();
        }
    };

    // Sort by priority descending; pick the first (highest priority).
    let mut sorted: Vec<(&str, i64)> = models
        .iter()
        .filter_map(|m| {
            let slug = m.get("slug")?.as_str()?;
            let priority = m.get("priority").and_then(|p| p.as_i64()).unwrap_or(0);
            Some((slug, priority))
        })
        .collect();

    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    if let Some((best_slug, priority)) = sorted.first() {
        info!("Best Codex model: {best_slug} (priority {priority})");
        best_slug.to_string()
    } else {
        warn!("No models found in Codex API response");
        FALLBACK_MODEL.to_string()
    }
}

/// Check if ChatGPT session auth is available (CHATGPT_SESSION_TOKEN env var is set).
pub fn chatgpt_session_available() -> bool {
    std::env::var("CHATGPT_SESSION_TOKEN").is_ok()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn browser_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/auth/callback")
}

fn build_token_exchange_form(
    code: &str,
    code_verifier: &str,
    redirect_uri: &str,
) -> Vec<(String, String)> {
    vec![
        ("grant_type".to_string(), "authorization_code".to_string()),
        ("client_id".to_string(), CLIENT_ID.to_string()),
        ("code".to_string(), code.to_string()),
        ("code_verifier".to_string(), code_verifier.to_string()),
        ("redirect_uri".to_string(), redirect_uri.to_string()),
    ]
}

fn parse_chatgpt_auth_result(
    body: &str,
    response_label: &str,
) -> Result<ChatGptAuthResult, String> {
    let json: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse {response_label} JSON: {e}"))?;

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Missing access_token in {response_label}"))?
        .to_string();

    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| Zeroizing::new(s.to_string()));

    let expires_in = json.get("expires_in").and_then(|v| v.as_u64());

    info!("OAuth tokens obtained successfully");

    Ok(ChatGptAuthResult {
        access_token: Zeroizing::new(access_token),
        refresh_token,
        expires_in,
    })
}

fn parse_device_auth_prompt_response(body: &str) -> Result<DeviceAuthPrompt, String> {
    let parsed: DeviceAuthPromptEnvelope = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse device auth response JSON: {e}"))?;

    let user_code = parsed
        .user_code
        .filter(|code| !code.trim().is_empty())
        .ok_or("Missing user_code in device auth response")?;

    if parsed.device_auth_id.trim().is_empty() {
        return Err("Missing device_auth_id in device auth response".to_string());
    }

    Ok(DeviceAuthPrompt {
        device_auth_id: parsed.device_auth_id,
        user_code,
        interval_secs: parse_poll_interval_secs(parsed.interval.as_ref()),
    })
}

fn parse_device_auth_poll_response(body: &str) -> Result<DeviceAuthPollEnvelope, String> {
    let parsed: DeviceAuthPollEnvelope = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse device auth poll response JSON: {e}"))?;

    if parsed.authorization_code.trim().is_empty() {
        return Err("Missing authorization_code in device auth poll response".to_string());
    }

    if parsed.code_verifier.trim().is_empty() {
        return Err("Missing code_verifier in device auth poll response".to_string());
    }

    Ok(parsed)
}

fn parse_poll_interval_secs(value: Option<&serde_json::Value>) -> u64 {
    let parsed = match value {
        Some(serde_json::Value::Number(n)) => n.as_u64(),
        Some(serde_json::Value::String(s)) => s.parse::<u64>().ok(),
        _ => None,
    };

    parsed
        .filter(|secs| *secs > 0)
        .unwrap_or(DEFAULT_DEVICE_AUTH_POLL_INTERVAL_SECS)
}

/// Handle a single HTTP connection on the OAuth callback server.
async fn handle_oauth_callback(
    mut stream: tokio::net::TcpStream,
    tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<String>>>>,
    expected_state: &str,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = vec![0u8; 8192];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("Read error: {e}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let first_line = request.lines().next().unwrap_or("");

    if first_line.starts_with("GET /auth/callback") {
        // Parse query parameters from the request path.
        let path = first_line
            .split_whitespace()
            .nth(1)
            .unwrap_or("/auth/callback");
        let params = parse_query_params(path);

        let code = params.get("code").cloned().unwrap_or_default();
        let state = params.get("state").cloned().unwrap_or_default();
        let error = params.get("error").cloned();

        if let Some(err) = error {
            let desc = params.get("error_description").cloned().unwrap_or_default();
            let error_html = error_html(&err, &desc);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                error_html.len(),
                error_html
            );
            stream
                .write_all(response.as_bytes())
                .await
                .map_err(|e| format!("Write error: {e}"))?;
            return Err(format!("OAuth error: {err}: {desc}"));
        }

        if state != expected_state {
            let msg = "State parameter mismatch -- possible CSRF attack";
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                msg.len(),
                msg
            );
            stream
                .write_all(response.as_bytes())
                .await
                .map_err(|e| format!("Write error: {e}"))?;
            return Err(msg.to_string());
        }

        if code.is_empty() {
            let msg = "Missing authorization code in callback";
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                msg.len(),
                msg
            );
            stream
                .write_all(response.as_bytes())
                .await
                .map_err(|e| format!("Write error: {e}"))?;
            return Err(msg.to_string());
        }

        // Send the code to the waiting channel.
        let mut guard = tx.lock().await;
        if let Some(sender) = guard.take() {
            let _ = sender.send(code);
        }

        let html = success_html();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|e| format!("Write error: {e}"))?;
    } else {
        let body = "Not Found";
        let response = format!(
            "HTTP/1.1 404 Not Found\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|e| format!("Write error: {e}"))?;
    }

    Ok(())
}

/// Parse query parameters from a URL path (e.g. `/auth/callback?code=abc&state=xyz`).
fn parse_query_params(path: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    if let Some(query) = path.split('?').nth(1) {
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(urldecode(k), urldecode(v));
            }
        }
    }
    map
}

/// Simple URL-decode (%XX and + handling).
fn urldecode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
            }
        } else if c == '+' {
            result.push(' ');
        } else {
            result.push(c);
        }
    }
    result
}

/// Percent-encode a string for use in a URL query parameter.
fn urlencod(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// Success page shown after OAuth callback is received.
fn success_html() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>LibreFang -- Authentication Complete</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; max-width: 600px; margin: 60px auto; padding: 0 20px; background: #f5f5f5; }
  .card { background: white; border-radius: 12px; padding: 24px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); text-align: center; }
  .check { font-size: 48px; margin: 16px 0; }
  h1 { color: #10a37f; }
</style>
</head>
<body>
<div class="card">
  <div class="check">&#10003;</div>
  <h1>Authentication Successful</h1>
  <p>Your ChatGPT session has been authenticated. You can close this tab.</p>
</div>
</body>
</html>"#
        .to_string()
}

/// Error page shown when OAuth returns an error.
fn error_html(error: &str, description: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>LibreFang -- Authentication Error</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, sans-serif; max-width: 600px; margin: 60px auto; padding: 0 20px; background: #f5f5f5; }}
  .card {{ background: white; border-radius: 12px; padding: 24px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); text-align: center; }}
  .icon {{ font-size: 48px; margin: 16px 0; }}
  h1 {{ color: #e74c3c; }}
</style>
</head>
<body>
<div class="card">
  <div class="icon">&#10007;</div>
  <h1>Authentication Failed</h1>
  <p><strong>Error:</strong> {error}</p>
  <p>{description}</p>
  <p>Please close this tab and try again.</p>
</div>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_pkce_verifier_length() {
        let pkce = generate_pkce();
        // 64 random bytes base64url-encoded = 86 chars (no padding).
        assert_eq!(pkce.verifier.len(), 86);
        assert!(!pkce.challenge.is_empty());
        // Verifier and challenge must differ.
        assert_ne!(pkce.verifier, pkce.challenge);
    }

    #[test]
    fn test_generate_pkce_challenge_is_sha256() {
        let pkce = generate_pkce();
        // Manually compute challenge from verifier.
        let mut hasher = Sha256::new();
        hasher.update(pkce.verifier.as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(hasher.finalize());
        assert_eq!(pkce.challenge, expected);
    }

    #[test]
    fn test_create_state_length() {
        let state = create_state();
        // 16 random bytes hex-encoded = 32 chars.
        assert_eq!(state.len(), 32);
    }

    #[test]
    fn test_create_state_is_hex() {
        let state = create_state();
        assert!(state.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_build_authorization_url_contains_params() {
        let url = build_authorization_url(12345, "test_challenge", "test_state");
        assert!(url.starts_with(AUTHORIZE_URL));
        assert!(url.contains("response_type=code"));
        assert!(url.contains(&format!("client_id={CLIENT_ID}")));
        assert!(url.contains("redirect_uri="));
        assert!(url.contains("12345"));
        assert!(url.contains("code_challenge=test_challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=test_state"));
        assert!(url.contains("codex_cli_simplified_flow=true"));
        assert!(url.contains("originator=codex_cli_rs"));
    }

    #[test]
    fn test_build_token_exchange_form_uses_explicit_redirect_uri() {
        let form =
            build_token_exchange_form("auth-code", "code-verifier", DEVICE_AUTH_REDIRECT_URI);

        assert!(form.iter().any(|(k, v)| k == "code" && v == "auth-code"));
        assert!(form
            .iter()
            .any(|(k, v)| k == "code_verifier" && v == "code-verifier"));
        assert!(form
            .iter()
            .any(|(k, v)| k == "redirect_uri" && v == DEVICE_AUTH_REDIRECT_URI));
    }

    #[test]
    fn test_parse_device_auth_prompt_response_supports_user_code() {
        let prompt = parse_device_auth_prompt_response(
            r#"{"device_auth_id":"device-1","user_code":"ABCD-EFGH","interval":7}"#,
        )
        .unwrap();

        assert_eq!(prompt.device_auth_id, "device-1");
        assert_eq!(prompt.user_code, "ABCD-EFGH");
        assert_eq!(prompt.interval_secs, 7);
    }

    #[test]
    fn test_parse_device_auth_prompt_response_supports_usercode_alias() {
        let prompt = parse_device_auth_prompt_response(
            r#"{"device_auth_id":"device-2","usercode":"WXYZ-1234","interval":"11"}"#,
        )
        .unwrap();

        assert_eq!(prompt.user_code, "WXYZ-1234");
        assert_eq!(prompt.interval_secs, 11);
    }

    #[test]
    fn test_parse_device_auth_prompt_response_defaults_interval_defensively() {
        let prompt = parse_device_auth_prompt_response(
            r#"{"device_auth_id":"device-3","user_code":"SAFE-0001","interval":"invalid"}"#,
        )
        .unwrap();

        assert_eq!(prompt.interval_secs, DEFAULT_DEVICE_AUTH_POLL_INTERVAL_SECS);
    }

    #[test]
    fn test_parse_device_auth_poll_response_requires_fields() {
        let parsed = parse_device_auth_poll_response(
            r#"{"authorization_code":"code-123","code_verifier":"verifier-456"}"#,
        )
        .unwrap();

        assert_eq!(parsed.authorization_code, "code-123");
        assert_eq!(parsed.code_verifier, "verifier-456");
    }

    #[test]
    fn test_parse_query_params_basic() {
        let params = parse_query_params("/auth/callback?code=abc123&state=xyz789");
        assert_eq!(params.get("code"), Some(&"abc123".to_string()));
        assert_eq!(params.get("state"), Some(&"xyz789".to_string()));
    }

    #[test]
    fn test_parse_query_params_empty() {
        let params = parse_query_params("/auth/callback");
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_query_params_encoded() {
        let params = parse_query_params("/cb?key=hello%20world&b=a+b");
        assert_eq!(params.get("key"), Some(&"hello world".to_string()));
        assert_eq!(params.get("b"), Some(&"a b".to_string()));
    }

    #[test]
    fn test_urldecode() {
        assert_eq!(urldecode("hello%20world"), "hello world");
        assert_eq!(urldecode("a+b"), "a b");
        assert_eq!(urldecode("no%2Fslash"), "no/slash");
    }

    #[test]
    fn test_urlencod_passthrough() {
        assert_eq!(urlencod("hello"), "hello");
        assert_eq!(urlencod("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn test_urlencod_special_chars() {
        let encoded = urlencod("hello world");
        assert_eq!(encoded, "hello%20world");
        let encoded = urlencod("a&b=c");
        assert!(encoded.contains("%26"));
        assert!(encoded.contains("%3D"));
    }

    #[test]
    fn test_chatgpt_base_url() {
        assert_eq!(CHATGPT_BASE_URL, "https://chatgpt.com/backend-api");
    }

    #[test]
    fn test_success_html_not_empty() {
        let html = success_html();
        assert!(html.contains("Authentication Successful"));
    }

    #[test]
    fn test_error_html_contains_message() {
        let html = error_html("access_denied", "User cancelled");
        assert!(html.contains("access_denied"));
        assert!(html.contains("User cancelled"));
    }
}
