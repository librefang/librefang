//! ChatGPT session authentication — browser-based login flow for ChatGPT Plus/Ultra subscribers.
//!
//! Opens a local HTTP server, redirects the user to ChatGPT's web login,
//! and captures the session token via a callback. The session token can then
//! be used with the ChatGPT backend API (which is OpenAI-compatible).

use std::net::TcpListener;
use std::sync::Arc;
use tokio::sync::oneshot;
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

/// Default ChatGPT API base URL (same as OpenAI).
pub const CHATGPT_BASE_URL: &str = "https://api.openai.com/v1";

/// ChatGPT web session URL — the page where users paste their session token.
const CHATGPT_SESSION_URL: &str = "https://chatgpt.com/api/auth/session";

/// Local callback server timeout — how long to wait for the user to complete auth.
const AUTH_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// Local callback server bind address.
const CALLBACK_BIND: &str = "127.0.0.1:0";

/// Result of a browser-based auth flow.
pub struct ChatGptAuthResult {
    /// The bearer token extracted from the ChatGPT session.
    pub access_token: Zeroizing<String>,
}

/// Start a browser-based authentication flow for ChatGPT.
///
/// 1. Opens a local HTTP server on a random port
/// 2. Instructs the user to visit ChatGPT, log in, and copy their session token
/// 3. Provides a local callback URL where they can paste the token
/// 4. Returns the captured access token
///
/// This approach doesn't require intercepting cookies or running a headless browser —
/// users simply paste their bearer token from the ChatGPT session API endpoint.
pub async fn start_browser_auth() -> Result<(String, u16), String> {
    let listener = TcpListener::bind(CALLBACK_BIND)
        .map_err(|e| format!("Failed to bind local server: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed to get local address: {e}"))?
        .port();

    let callback_url = format!("http://127.0.0.1:{port}");
    let instructions = format!(
        "\n\
        ╔══════════════════════════════════════════════════════════╗\n\
        ║         ChatGPT Session Authentication                  ║\n\
        ╠══════════════════════════════════════════════════════════╣\n\
        ║                                                          ║\n\
        ║  1. Open: {CHATGPT_SESSION_URL}  ║\n\
        ║     (Log in to ChatGPT if needed)                        ║\n\
        ║                                                          ║\n\
        ║  2. Copy the \"accessToken\" value from the JSON response  ║\n\
        ║                                                          ║\n\
        ║  3. Paste it at: {callback_url}/auth                ║\n\
        ║     Or POST to:  {callback_url}/callback             ║\n\
        ║     with body:   {{\"token\": \"your_token_here\"}}         ║\n\
        ║                                                          ║\n\
        ║  Waiting for token (timeout: {AUTH_TIMEOUT_SECS}s)...               ║\n\
        ╚══════════════════════════════════════════════════════════╝\n"
    );

    info!("{}", instructions);

    Ok((callback_url, port))
}

/// Run the local callback server and wait for the user to submit their token.
///
/// Serves two endpoints:
/// - GET /auth — shows an HTML form where the user can paste their token
/// - POST /callback — accepts JSON `{"token": "..."}` or form data `token=...`
pub async fn run_callback_server(port: u16) -> Result<ChatGptAuthResult, String> {
    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
        .await
        .map_err(|e| format!("Failed to bind async listener: {e}"))?;

    debug!("ChatGPT auth callback server listening on port {port}");

    let server_handle = tokio::spawn({
        let tx = tx.clone();
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
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, tx).await {
                        debug!("Connection handler error: {e}");
                    }
                });
            }
        }
    });

    // Wait for token with timeout
    let token = tokio::time::timeout(std::time::Duration::from_secs(AUTH_TIMEOUT_SECS), rx)
        .await
        .map_err(|_| "Authentication timed out — no token received".to_string())?
        .map_err(|_| "Auth channel closed unexpectedly".to_string())?;

    server_handle.abort();

    if token.is_empty() {
        return Err("Received empty token".to_string());
    }

    info!("ChatGPT session token received successfully");

    Ok(ChatGptAuthResult {
        access_token: Zeroizing::new(token),
    })
}

/// Handle a single HTTP connection on the callback server.
async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<String>>>>,
) -> Result<(), String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut buf = vec![0u8; 8192];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("Read error: {e}"))?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let first_line = request.lines().next().unwrap_or("");

    if first_line.starts_with("GET /auth") {
        // Serve the HTML form
        let html = auth_form_html();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            html.len(),
            html
        );
        stream
            .write_all(response.as_bytes())
            .await
            .map_err(|e| format!("Write error: {e}"))?;
    } else if first_line.starts_with("POST /callback") {
        // Extract token from body
        let body = extract_body(&request);
        let token = extract_token(&body);

        if let Some(token) = token {
            // Send token to the waiting channel
            let mut guard = tx.lock().await;
            if let Some(sender) = guard.take() {
                let _ = sender.send(token);
            }

            let success_html = success_html();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                success_html.len(),
                success_html
            );
            stream
                .write_all(response.as_bytes())
                .await
                .map_err(|e| format!("Write error: {e}"))?;
        } else {
            let error = "Missing 'token' in request body";
            let response = format!(
                "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                error.len(),
                error
            );
            stream
                .write_all(response.as_bytes())
                .await
                .map_err(|e| format!("Write error: {e}"))?;
        }
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

/// Extract the HTTP body from a raw request string.
fn extract_body(request: &str) -> String {
    // HTTP body comes after \r\n\r\n
    if let Some(idx) = request.find("\r\n\r\n") {
        request[idx + 4..].to_string()
    } else if let Some(idx) = request.find("\n\n") {
        request[idx + 2..].to_string()
    } else {
        String::new()
    }
}

/// Extract the token from either JSON or form-encoded body.
fn extract_token(body: &str) -> Option<String> {
    let body = body.trim();
    if body.is_empty() {
        return None;
    }

    // Try JSON: {"token": "..."}
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
        if let Some(token) = json.get("token").and_then(|v| v.as_str()) {
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
        // Also try "accessToken" (ChatGPT session API format)
        if let Some(token) = json.get("accessToken").and_then(|v| v.as_str()) {
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }

    // Try form-encoded: token=...
    for pair in body.split('&') {
        if let Some(value) = pair.strip_prefix("token=") {
            let decoded = urldecode(value);
            if !decoded.is_empty() {
                return Some(decoded);
            }
        }
    }

    // Last resort: if the body looks like a bare JWT/token, use it directly
    if body.len() > 20 && !body.contains(' ') && !body.contains('<') {
        return Some(body.to_string());
    }

    None
}

/// Simple URL decode (handles %XX sequences).
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

/// HTML form for pasting the ChatGPT session token.
fn auth_form_html() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>LibreFang — ChatGPT Authentication</title>
<style>
  body { font-family: -apple-system, BlinkMacSystemFont, sans-serif; max-width: 600px; margin: 60px auto; padding: 0 20px; background: #f5f5f5; }
  h1 { color: #333; }
  .card { background: white; border-radius: 12px; padding: 24px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); }
  textarea { width: 100%; height: 120px; margin: 12px 0; padding: 8px; border: 1px solid #ddd; border-radius: 6px; font-family: monospace; font-size: 13px; }
  button { background: #10a37f; color: white; border: none; padding: 12px 24px; border-radius: 6px; cursor: pointer; font-size: 16px; }
  button:hover { background: #0d8c6d; }
  .steps { margin: 16px 0; padding-left: 20px; }
  .steps li { margin: 8px 0; }
  code { background: #f0f0f0; padding: 2px 6px; border-radius: 3px; }
  a { color: #10a37f; }
</style>
</head>
<body>
<div class="card">
  <h1>ChatGPT Authentication</h1>
  <ol class="steps">
    <li>Open <a href="https://chatgpt.com/api/auth/session" target="_blank">chatgpt.com/api/auth/session</a> in a new tab</li>
    <li>Log in to ChatGPT if prompted</li>
    <li>Copy the <code>accessToken</code> value from the JSON response</li>
    <li>Paste it below and click Submit</li>
  </ol>
  <form method="POST" action="/callback">
    <textarea name="token" placeholder="Paste your accessToken here..." required></textarea>
    <button type="submit">Submit Token</button>
  </form>
</div>
</body>
</html>"#.to_string()
}

/// Success page shown after token is received.
fn success_html() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>LibreFang — Authentication Complete</title>
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
  <p>Your ChatGPT session token has been captured. You can close this tab.</p>
</div>
</body>
</html>"#.to_string()
}

/// Check if ChatGPT session auth is available (CHATGPT_SESSION_TOKEN env var is set).
pub fn chatgpt_session_available() -> bool {
    std::env::var("CHATGPT_SESSION_TOKEN").is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_token_json() {
        let body = r#"{"token": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9"}"#;
        assert_eq!(
            extract_token(body),
            Some("eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9".to_string())
        );
    }

    #[test]
    fn test_extract_token_access_token_field() {
        let body = r#"{"accessToken": "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9"}"#;
        assert_eq!(
            extract_token(body),
            Some("eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9".to_string())
        );
    }

    #[test]
    fn test_extract_token_form_encoded() {
        let body = "token=my-session-token-12345";
        assert_eq!(
            extract_token(body),
            Some("my-session-token-12345".to_string())
        );
    }

    #[test]
    fn test_extract_token_bare_jwt() {
        let body = "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0";
        assert_eq!(extract_token(body), Some(body.to_string()));
    }

    #[test]
    fn test_extract_token_empty() {
        assert_eq!(extract_token(""), None);
        assert_eq!(extract_token("   "), None);
    }

    #[test]
    fn test_extract_token_json_empty_value() {
        let body = r#"{"token": ""}"#;
        assert_eq!(extract_token(body), None);
    }

    #[test]
    fn test_extract_body() {
        let req = "POST /callback HTTP/1.1\r\nHost: localhost\r\n\r\n{\"token\": \"abc\"}";
        assert_eq!(extract_body(req), "{\"token\": \"abc\"}");
    }

    #[test]
    fn test_urldecode() {
        assert_eq!(urldecode("hello%20world"), "hello world");
        assert_eq!(urldecode("a+b"), "a b");
        assert_eq!(urldecode("no%2Fslash"), "no/slash");
    }

    #[test]
    fn test_chatgpt_base_url() {
        assert_eq!(CHATGPT_BASE_URL, "https://api.openai.com/v1");
    }
}
