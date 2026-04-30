//! OAuth2 PKCE flows — localhost callback for Google/GitHub/Microsoft/Slack.
//!
//! Launches a temporary localhost HTTP server, opens the browser to the auth URL,
//! receives the callback with the authorization code, and exchanges it for tokens.
//! All tokens are stored in the credential vault with `Zeroizing<String>`.

use crate::{ExtensionError, ExtensionResult, OAuthTemplate};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

// Canonical OAuth token type lives in `librefang-types`. Re-export so existing
// callers can keep their `extensions::oauth::OAuthTokens` import path.
pub use librefang_types::oauth::OAuthTokens;

/// Default OAuth client IDs for public PKCE flows.
/// These are safe to embed — PKCE doesn't require a client_secret.
pub fn default_client_ids() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    // Placeholder IDs — users should configure their own via config
    m.insert("google", "librefang-google-client-id");
    m.insert("github", "librefang-github-client-id");
    m.insert("microsoft", "librefang-microsoft-client-id");
    m.insert("slack", "librefang-slack-client-id");
    m
}

/// Resolve OAuth client IDs with config overrides applied on top of defaults.
pub fn resolve_client_ids(
    config: &librefang_types::config::OAuthConfig,
) -> HashMap<String, String> {
    let defaults = default_client_ids();
    let mut resolved: HashMap<String, String> = defaults
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();

    if let Some(ref id) = config.google_client_id {
        resolved.insert("google".into(), id.clone());
    }
    if let Some(ref id) = config.github_client_id {
        resolved.insert("github".into(), id.clone());
    }
    if let Some(ref id) = config.microsoft_client_id {
        resolved.insert("microsoft".into(), id.clone());
    }
    if let Some(ref id) = config.slack_client_id {
        resolved.insert("slack".into(), id.clone());
    }

    resolved
}

/// PKCE code verifier and challenge pair.
struct PkcePair {
    verifier: Zeroizing<String>,
    challenge: String,
}

/// Generate a PKCE code_verifier and code_challenge (S256).
fn generate_pkce() -> PkcePair {
    let bytes: [u8; 32] = rand::random();
    let verifier = Zeroizing::new(base64_url_encode(&bytes));
    let challenge = {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        base64_url_encode(&digest)
    };
    PkcePair {
        verifier,
        challenge,
    }
}

/// URL-safe base64 encoding (no padding).
fn base64_url_encode(data: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, data)
}

/// State token TTL — login flows must complete within 10 minutes.
const STATE_TOKEN_TTL_SECS: u64 = 600;

/// HMAC-signed state payload binding the in-flight flow to its
/// `(provider, client_id, redirect_uri)` tuple plus a random nonce and
/// an absolute expiry. Prevents cross-flow code injection (#3791).
#[derive(Serialize, Deserialize)]
struct StatePayload {
    /// Auth URL of the OAuth template — pins state to one provider.
    provider: String,
    /// OAuth client ID — pins state to one app registration.
    client_id: String,
    /// Loopback redirect URI — pins state to the listener that issued it.
    redirect_uri: String,
    /// 16-byte random nonce, also kept in-process to anchor callback equality.
    nonce: String,
    /// Absolute UNIX-second expiry (now + STATE_TOKEN_TTL_SECS).
    exp: u64,
}

/// Per-process random HMAC key used to sign state payloads.
/// Re-seeded on every daemon restart, which invalidates any in-flight flows
/// from a prior process — fail-closed for credential flows.
fn state_signing_key() -> &'static [u8] {
    static KEY: std::sync::OnceLock<[u8; 32]> = std::sync::OnceLock::new();
    KEY.get_or_init(rand::random)
}

/// Build an HMAC-signed state token bound to this specific flow.
///
/// Returns `(token, nonce)` so the caller can keep the nonce in-process
/// without round-tripping through `verify_signed_state` to recover it.
///
/// Format: `base64url(payload_json).base64url(hmac)`. Both halves are
/// URL-safe (no padding) so the token survives an unmodified pass through
/// the OAuth provider's `state` parameter.
fn build_signed_state(
    provider: &str,
    client_id: &str,
    redirect_uri: &str,
) -> (String, String) {
    let nonce_bytes: [u8; 16] = rand::random();
    let nonce = base64_url_encode(&nonce_bytes);
    let exp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        + STATE_TOKEN_TTL_SECS;
    let payload = StatePayload {
        provider: provider.to_string(),
        client_id: client_id.to_string(),
        redirect_uri: redirect_uri.to_string(),
        nonce: nonce.clone(),
        exp,
    };
    let payload_json = serde_json::to_vec(&payload).unwrap_or_default();
    let payload_b64 = base64_url_encode(&payload_json);
    let mut mac = HmacSha256::new_from_slice(state_signing_key()).expect("HMAC accepts any key");
    mac.update(payload_b64.as_bytes());
    let sig = mac.finalize().into_bytes();
    (format!("{payload_b64}.{}", base64_url_encode(&sig)), nonce)
}

/// Verify a state token and return the embedded payload on success.
///
/// Rejects malformed tokens, bad HMAC, expired payloads, or any field
/// mismatch against the expected `(provider, client_id, redirect_uri)`.
fn verify_signed_state(
    state: &str,
    expected_provider: &str,
    expected_client_id: &str,
    expected_redirect_uri: &str,
) -> Result<StatePayload, &'static str> {
    let (payload_b64, sig_b64) = state.split_once('.').ok_or("malformed state")?;
    let mut mac = HmacSha256::new_from_slice(state_signing_key()).expect("HMAC accepts any key");
    mac.update(payload_b64.as_bytes());
    let expected_sig = mac.finalize().into_bytes();
    let provided_sig =
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, sig_b64)
            .map_err(|_| "bad sig encoding")?;
    if expected_sig.len() != provided_sig.len()
        || !bool::from(expected_sig.as_slice().ct_eq(&provided_sig))
    {
        return Err("state signature mismatch");
    }
    let payload_bytes = base64::Engine::decode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        payload_b64,
    )
    .map_err(|_| "bad payload encoding")?;
    let payload: StatePayload =
        serde_json::from_slice(&payload_bytes).map_err(|_| "bad payload json")?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if payload.exp < now {
        return Err("state expired");
    }
    if payload.provider != expected_provider {
        return Err("state provider mismatch");
    }
    if payload.client_id != expected_client_id {
        return Err("state client_id mismatch");
    }
    if payload.redirect_uri != expected_redirect_uri {
        return Err("state redirect_uri mismatch");
    }
    Ok(payload)
}

/// Run the complete OAuth2 PKCE flow for a given template.
///
/// 1. Start localhost callback server on a random port.
/// 2. Open browser to authorization URL.
/// 3. Wait for callback with authorization code.
/// 4. Exchange code for tokens.
/// 5. Return tokens.
pub async fn run_pkce_flow(oauth: &OAuthTemplate, client_id: &str) -> ExtensionResult<OAuthTokens> {
    let pkce = generate_pkce();

    // Find an available port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| ExtensionError::OAuth(format!("Failed to bind localhost: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| ExtensionError::OAuth(format!("Failed to get port: {e}")))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    // #3791: state is HMAC-signed and bound to (provider, client_id,
    // redirect_uri, nonce, exp) so a leaked random nonce alone is not
    // enough — a sibling local process trying to feed a stolen state to
    // a different listener / client_id is rejected at verify time.
    let (state, expected_nonce) =
        build_signed_state(&oauth.auth_url, client_id, &redirect_uri);
    let expected_provider = oauth.auth_url.clone();
    let expected_client_id = client_id.to_string();
    let expected_redirect_uri = redirect_uri.clone();

    info!("OAuth callback server listening on port {port}");

    // Build authorization URL
    let scopes = oauth.scopes.join(" ");
    let auth_url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256",
        oauth.auth_url,
        urlencoding_encode(client_id),
        urlencoding_encode(&redirect_uri),
        urlencoding_encode(&scopes),
        urlencoding_encode(&state),
        urlencoding_encode(&pkce.challenge),
    );

    // Open browser
    info!("Opening browser for OAuth authorization...");
    if let Err(e) = open_browser(&auth_url) {
        // Tracing is already initialized when we reach here (the daemon is
        // running), so route through the structured logger instead of stderr.
        // The URL is embedded in the log line so it reaches log files,
        // OpenTelemetry, and Loki rather than being silently discarded in
        // headless / daemon deployments.
        warn!(
            url = %auth_url,
            "Could not open browser ({e}); open this URL manually to complete OAuth authorization"
        );
    }

    // Wait for callback
    let (code_tx, code_rx) = oneshot::channel::<String>();
    let code_tx = Arc::new(Mutex::new(Some(code_tx)));

    // Spawn callback handler
    let server = axum::Router::new().route(
        "/callback",
        axum::routing::get({
            let code_tx = code_tx.clone();
            let expected_provider = expected_provider.clone();
            let expected_client_id = expected_client_id.clone();
            let expected_redirect_uri = expected_redirect_uri.clone();
            let expected_nonce = expected_nonce.clone();
            move |query: axum::extract::Query<CallbackParams>| {
                let code_tx = code_tx.clone();
                let expected_provider = expected_provider.clone();
                let expected_client_id = expected_client_id.clone();
                let expected_redirect_uri = expected_redirect_uri.clone();
                let expected_nonce = expected_nonce.clone();
                async move {
                    // #3791: verify the HMAC-signed state binds back to this
                    // exact flow's (provider, client_id, redirect_uri) tuple
                    // and a non-expired payload.
                    let payload = match verify_signed_state(
                        &query.state,
                        &expected_provider,
                        &expected_client_id,
                        &expected_redirect_uri,
                    ) {
                        Ok(p) => p,
                        Err(reason) => {
                            warn!(reason, "OAuth callback state verification failed");
                            return axum::response::Html(
                                "<h1>Error</h1><p>Invalid state parameter. Possible CSRF attack.</p>"
                                    .to_string(),
                            );
                        }
                    };
                    // Redundant w/ HMAC (covers `nonce`); kept in case a future refactor drops fields.
                    if !bool::from(payload.nonce.as_bytes().ct_eq(expected_nonce.as_bytes())) {
                        warn!("OAuth callback nonce mismatch");
                        return axum::response::Html(
                            "<h1>Error</h1><p>Invalid state parameter. Possible CSRF attack.</p>"
                                .to_string(),
                        );
                    }
                    if let Some(ref error) = query.error {
                        return axum::response::Html(format!(
                            "<h1>Error</h1><p>OAuth error: {error}</p>"
                        ));
                    }
                    if let Some(ref code) = query.code {
                        // #3791: only the first valid callback wins; subsequent
                        // hits on the same listener are rejected explicitly so
                        // a replay against the live channel cannot succeed.
                        let mut guard = code_tx.lock().await;
                        if let Some(tx) = guard.take() {
                            let _ = tx.send(code.clone());
                            axum::response::Html(
                                "<h1>Success!</h1><p>Authorization complete. You can close this tab.</p><script>window.close()</script>"
                                    .to_string(),
                            )
                        } else {
                            axum::response::Html(
                                "<h1>Gone</h1><p>This callback was already redeemed.</p>"
                                    .to_string(),
                            )
                        }
                    } else {
                        axum::response::Html(
                            "<h1>Error</h1><p>No authorization code received.</p>".to_string(),
                        )
                    }
                }
            }
        }),
    );

    // Serve with timeout
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, server).await.ok();
    });

    // Wait for auth code with 5-minute timeout
    let code = tokio::time::timeout(std::time::Duration::from_secs(300), code_rx)
        .await
        .map_err(|_| ExtensionError::OAuth("OAuth flow timed out after 5 minutes".to_string()))?
        .map_err(|_| ExtensionError::OAuth("Callback channel closed".to_string()))?;

    // Shut down callback server
    server_handle.abort();

    debug!("Received authorization code, exchanging for tokens...");

    // Exchange code for tokens
    let client = crate::http_client::new_client();
    let mut params = HashMap::new();
    params.insert("grant_type", "authorization_code");
    params.insert("code", &code);
    params.insert("redirect_uri", &redirect_uri);
    params.insert("client_id", client_id);
    let verifier_str = pkce.verifier.as_str().to_string();
    params.insert("code_verifier", &verifier_str);

    let resp = client
        .post(&oauth.token_url)
        .form(&params)
        .send()
        .await
        .map_err(|e| ExtensionError::OAuth(format!("Token exchange request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(ExtensionError::OAuth(format!(
            "Token exchange failed ({}): {}",
            status, body
        )));
    }

    let tokens: OAuthTokens = resp
        .json()
        .await
        .map_err(|e| ExtensionError::OAuth(format!("Token response parse failed: {e}")))?;

    info!(
        "OAuth tokens obtained (expires_in: {}s, scopes: {})",
        tokens.expires_in, tokens.scope
    );
    Ok(tokens)
}

/// Callback query parameters.
#[derive(Deserialize)]
struct CallbackParams {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    error: Option<String>,
}

/// Simple percent-encoding for URL parameters.
fn urlencoding_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            _ => {
                result.push('%');
                result.push_str(&format!("{:02X}", byte));
            }
        }
    }
    result
}

/// Open a URL in the default browser.
fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(url)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_generation() {
        let pkce = generate_pkce();
        assert!(!pkce.verifier.is_empty());
        assert!(!pkce.challenge.is_empty());
        // Verifier and challenge should be different
        assert_ne!(pkce.verifier.as_str(), &pkce.challenge);
    }

    #[test]
    fn pkce_challenge_is_sha256() {
        let pkce = generate_pkce();
        // Verify: challenge = base64url(sha256(verifier))
        let mut hasher = Sha256::new();
        hasher.update(pkce.verifier.as_bytes());
        let digest = hasher.finalize();
        let expected = base64_url_encode(&digest);
        assert_eq!(pkce.challenge, expected);
    }

    #[test]
    fn signed_state_round_trip_succeeds() {
        let (state, nonce) =
            build_signed_state("https://idp/auth", "client-1", "http://127.0.0.1:1/cb");
        let payload = verify_signed_state(
            &state,
            "https://idp/auth",
            "client-1",
            "http://127.0.0.1:1/cb",
        )
        .expect("valid state must verify");
        assert_eq!(payload.nonce, nonce, "verify must echo the build-time nonce");
    }

    #[test]
    fn signed_state_rejects_provider_swap() {
        let (state, _) =
            build_signed_state("https://idp/auth", "client-1", "http://127.0.0.1:1/cb");
        assert!(verify_signed_state(
            &state,
            "https://other/auth",
            "client-1",
            "http://127.0.0.1:1/cb",
        )
        .is_err());
    }

    #[test]
    fn signed_state_rejects_redirect_swap() {
        let (state, _) =
            build_signed_state("https://idp/auth", "client-1", "http://127.0.0.1:1/cb");
        assert!(verify_signed_state(
            &state,
            "https://idp/auth",
            "client-1",
            "http://127.0.0.1:2/cb",
        )
        .is_err());
    }

    #[test]
    fn signed_state_rejects_client_id_swap() {
        let (state, _) =
            build_signed_state("https://idp/auth", "client-1", "http://127.0.0.1:1/cb");
        assert!(verify_signed_state(
            &state,
            "https://idp/auth",
            "client-2",
            "http://127.0.0.1:1/cb",
        )
        .is_err());
    }

    #[test]
    fn signed_state_rejects_truncated_signature() {
        let (state, _) =
            build_signed_state("https://idp/auth", "client-1", "http://127.0.0.1:1/cb");
        let half = &state[..state.len() - 4];
        assert!(verify_signed_state(
            half,
            "https://idp/auth",
            "client-1",
            "http://127.0.0.1:1/cb",
        )
        .is_err());
    }

    #[test]
    fn signed_state_uniqueness_across_calls() {
        let (s1, n1) = build_signed_state("https://idp/auth", "client-1", "http://127.0.0.1:1/cb");
        let (s2, n2) = build_signed_state("https://idp/auth", "client-1", "http://127.0.0.1:1/cb");
        assert_ne!(s1, s2, "nonce must randomize each token");
        assert_ne!(n1, n2, "returned nonces must differ across builds");
    }

    #[test]
    fn urlencoding_basic() {
        assert_eq!(urlencoding_encode("hello"), "hello");
        assert_eq!(urlencoding_encode("hello world"), "hello%20world");
        assert_eq!(urlencoding_encode("a=b&c=d"), "a%3Db%26c%3Dd");
    }

    #[test]
    fn default_client_ids_populated() {
        let ids = default_client_ids();
        assert!(ids.contains_key("google"));
        assert!(ids.contains_key("github"));
        assert!(ids.contains_key("microsoft"));
        assert!(ids.contains_key("slack"));
    }

    #[test]
    fn resolve_client_ids_uses_defaults() {
        let config = librefang_types::config::OAuthConfig::default();
        let ids = resolve_client_ids(&config);
        assert_eq!(ids["google"], "librefang-google-client-id");
        assert_eq!(ids["github"], "librefang-github-client-id");
    }

    #[test]
    fn resolve_client_ids_applies_overrides() {
        let config = librefang_types::config::OAuthConfig {
            google_client_id: Some("my-real-google-id".into()),
            github_client_id: None,
            microsoft_client_id: Some("my-msft-id".into()),
            slack_client_id: None,
        };
        let ids = resolve_client_ids(&config);
        assert_eq!(ids["google"], "my-real-google-id");
        assert_eq!(ids["github"], "librefang-github-client-id"); // default
        assert_eq!(ids["microsoft"], "my-msft-id");
        assert_eq!(ids["slack"], "librefang-slack-client-id"); // default
    }
}
