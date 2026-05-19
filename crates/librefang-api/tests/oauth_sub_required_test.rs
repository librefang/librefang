//! Integration tests for the OIDC `sub` claim enforcement (#5128).
//!
//! Background: `IdTokenClaims.sub` used to be `#[serde(default)]`, and
//! `validate_jwt_cached` only required `aud` + `exp`. A JWT missing the
//! `sub` claim — or one with an explicit empty `"sub": ""` — would
//! deserialise with `sub = ""`, and the OAuth callback would then call
//! `TOKEN_STORE.store(&claims.sub, …)` keyed on the empty string. Every
//! token-less login collided on the same slot, so a fresh sign-in could
//! silently inherit another user's refresh token.
//!
//! These tests drive the `/api/auth/introspect` route end-to-end with
//! real JWTs signed by an RSA-2048 keypair we generate in-process. The
//! corresponding public key is served as a JWKS document from a local
//! axum listener bound to `127.0.0.1:0`, so the daemon's
//! `validate_jwt_cached` path runs exactly as in production (JWKS fetch,
//! RS256 signature verification, and claims validation) without needing
//! a live identity provider.
//!
//! Three cases cover the three security layers added in the fix:
//!   1. `sub` claim absent → JWT-layer required-claims check fires.
//!   2. `sub = ""` (explicit empty) → defence-in-depth empty-string
//!      check in `validate_jwt_cached` fires after deserialisation.
//!   3. `sub = "alice"` (well-formed) → introspection succeeds.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use librefang_api::routes::AppState;
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::{ExternalAuthConfig, OidcProvider};
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use std::sync::Arc;
use tower::ServiceExt;

// `rand_core` is not a direct workspace dep, but the `rsa` crate pulls in
// `rand_core = 0.6.4` for its `CryptoRngCore` bound and `argon2` (already
// a dependency of `librefang-api`) re-exports the same `OsRng`. Pulling
// `OsRng` from the `argon2::password_hash::rand_core` re-export avoids
// having to add another direct `rand_core` dev-dep just to satisfy
// `RsaPrivateKey::new`.
use argon2::password_hash::rand_core::OsRng;

// ─── JWKS-server harness ────────────────────────────────────────────────

/// A pair of (private encoding key, JWKS document) for one RSA-2048 key.
struct TestKey {
    encoding_key: EncodingKey,
    /// Pre-serialised JWKS JSON the local axum listener returns.
    jwks_body: String,
    /// Stable `kid` baked into the JWKS entry and the JWT header.
    kid: String,
}

/// Generate a fresh RSA-2048 keypair, serialise the public half as a
/// JWKS document, and return the EncodingKey we use to sign test JWTs.
///
/// We rebuild the key per test rather than sharing one across the
/// module: `fetch_jwks_cached` caches by URI, and each test binds a
/// unique `127.0.0.1:0` listener, so cache entries are partitioned and
/// can't bleed.
fn generate_test_key(kid: &str) -> TestKey {
    let private_key = RsaPrivateKey::new(&mut OsRng, 2048).expect("RSA keygen");
    let pem = private_key
        .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
        .expect("PKCS#8 encode");
    let encoding_key =
        EncodingKey::from_rsa_pem(pem.as_bytes()).expect("EncodingKey::from_rsa_pem");

    let public = private_key.to_public_key();
    let n_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public.n().to_bytes_be());
    let e_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(public.e().to_bytes_be());
    let jwks_body = serde_json::json!({
        "keys": [{
            "kty": "RSA",
            "kid": kid,
            "use": "sig",
            "alg": "RS256",
            "n": n_b64,
            "e": e_b64,
        }]
    })
    .to_string();

    TestKey {
        encoding_key,
        jwks_body,
        kid: kid.to_string(),
    }
}

/// Spin up a local axum server that serves the JWKS at
/// `/.well-known/jwks.json` and return the absolute URI for the caller
/// to register on the provider config. The server is dropped together
/// with the returned `_handle`.
///
/// The JWKS body is wrapped in `Arc<String>` and shared into the
/// handler via `axum::extract::State`, which is the idiomatic
/// per-request shared-state path in axum 0.8 and avoids the
/// `Fn() + Clone` capture dance that `axum::routing::get(closure)`
/// otherwise demands.
async fn spawn_jwks_server(jwks_body: String) -> (String, tokio::task::JoinHandle<()>) {
    async fn jwks_handler(State(body): State<Arc<String>>) -> impl IntoResponse {
        ([("content-type", "application/json")], (*body).clone())
    }

    let body = Arc::new(jwks_body);
    let app: Router = Router::new()
        .route("/.well-known/jwks.json", axum::routing::get(jwks_handler))
        .with_state(body);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (format!("http://{addr}/.well-known/jwks.json"), handle)
}

// ─── Token construction ─────────────────────────────────────────────────

/// Build a JWT header that carries the provider's `kid` so the daemon's
/// JWKS lookup hits a deterministic entry.
fn rs256_header(kid: &str) -> Header {
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(kid.to_string());
    header
}

/// Sign a JWT whose claims are an arbitrary JSON object. We hand-roll
/// the claim map rather than constructing `IdTokenClaims` directly so
/// each test can exercise a different `sub` shape — present + non-empty,
/// present + empty, absent entirely.
fn sign_jwt(encoding_key: &EncodingKey, kid: &str, claims: serde_json::Value) -> String {
    encode(&rs256_header(kid), &claims, encoding_key).expect("encode JWT")
}

/// `iat` / `exp` window that's safely in the future for the test run.
fn timestamps_now_plus(secs: u64) -> (u64, u64) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    (now, now + secs)
}

// ─── Daemon harness ─────────────────────────────────────────────────────

/// Build an `ExternalAuthConfig` with one provider pointing at the
/// local JWKS URL. `audience` is left empty so the daemon's audience
/// check is disabled and we can focus on the `sub` enforcement path.
fn ext_auth_with_jwks(jwks_uri: String) -> ExternalAuthConfig {
    ExternalAuthConfig {
        enabled: true,
        // The introspect handler does not consult `require_email_verified`
        // (that gate runs in `auth_callback_post`), but disabling it here
        // keeps the harness focused on the `sub` enforcement path and
        // makes intent explicit for future readers.
        require_email_verified: false,
        providers: vec![OidcProvider {
            id: "test".into(),
            display_name: "Test".into(),
            issuer_url: String::new(),
            auth_url: "https://example.invalid/authorize".into(),
            token_url: "https://example.invalid/token".into(),
            userinfo_url: String::new(),
            jwks_uri,
            client_id: "client-id".into(),
            client_secret_env: "LIBREFANG_SUB_REQUIRED_TEST_DOES_NOT_EXIST".into(),
            redirect_url: "http://127.0.0.1:4545/api/auth/callback".into(),
            scopes: vec!["openid".into()],
            allowed_domains: vec![],
            audience: String::new(),
            require_email_verified: None,
        }],
        ..Default::default()
    }
}

/// Hand-rolled router around the real `auth_introspect` handler — same
/// shape as `oauth_routes_test.rs::oauth_router`. The full
/// `api_v1_routes()` stack would only add irrelevant middleware noise
/// to assertions that focus on JWT-validation semantics.
fn introspect_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route(
            "/api/auth/introspect",
            axum::routing::post(librefang_api::oauth::auth_introspect),
        )
        .with_state(state)
}

async fn boot(ext: ExternalAuthConfig) -> (Router, TestAppState) {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(move |cfg| {
        cfg.external_auth = ext;
    }));
    let state = test.state.clone();
    let app = introspect_router(state);
    (app, test)
}

async fn introspect(app: &Router, token: &str) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({"token": token}).to_string();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/auth/introspect")
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

// ─── Tests ──────────────────────────────────────────────────────────────

/// JWT missing the `sub` claim entirely → required-claims check must
/// fire and `introspect` must report `active: false`. Pre-fix, the
/// `#[serde(default)]` attribute let this token deserialise with
/// `sub = ""` and validation would *succeed*.
#[tokio::test(flavor = "multi_thread")]
async fn introspect_rejects_jwt_with_missing_sub_claim() {
    let key = generate_test_key("test-kid-missing");
    let (jwks_uri, _jwks_handle) = spawn_jwks_server(key.jwks_body.clone()).await;
    let (app, _state) = boot(ext_auth_with_jwks(jwks_uri)).await;

    let (iat, exp) = timestamps_now_plus(300);
    // Deliberately omit `sub` from the claims object.
    let claims = serde_json::json!({
        "iss": "test",
        "aud": "client-id",
        "iat": iat,
        "exp": exp,
    });
    let token = sign_jwt(&key.encoding_key, &key.kid, claims);

    let (status, body) = introspect(&app, &token).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "introspect returns 200 by RFC 7662 conventions"
    );
    assert_eq!(
        body["active"], false,
        "JWT missing `sub` must be reported inactive — body was {body:?}"
    );
}

/// JWT with an explicit empty `sub = ""` → the defence-in-depth empty
/// check inside `validate_jwt_cached` must fire. Removing
/// `#[serde(default)]` alone does NOT catch this case: the field is
/// structurally present in the JSON, so serde happily deserialises it
/// to the empty string. The third gate (the `claims.sub.is_empty()`
/// rejection) is what closes the regression here.
#[tokio::test(flavor = "multi_thread")]
async fn introspect_rejects_jwt_with_empty_sub_claim() {
    let key = generate_test_key("test-kid-empty");
    let (jwks_uri, _jwks_handle) = spawn_jwks_server(key.jwks_body.clone()).await;
    let (app, _state) = boot(ext_auth_with_jwks(jwks_uri)).await;

    let (iat, exp) = timestamps_now_plus(300);
    let claims = serde_json::json!({
        "sub": "",
        "iss": "test",
        "aud": "client-id",
        "iat": iat,
        "exp": exp,
    });
    let token = sign_jwt(&key.encoding_key, &key.kid, claims);

    let (status, body) = introspect(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["active"], false,
        "JWT with empty `sub` must be reported inactive — body was {body:?}"
    );
}

/// Happy-path control: a well-formed JWT with `sub = "alice"` must
/// successfully introspect and surface the subject claim. This pins
/// that the tighter validation in the fix does NOT regress the legit
/// path — required-claims enforcement should be transparent when every
/// required claim is present.
#[tokio::test(flavor = "multi_thread")]
async fn introspect_accepts_jwt_with_well_formed_sub_claim() {
    let key = generate_test_key("test-kid-ok");
    let (jwks_uri, _jwks_handle) = spawn_jwks_server(key.jwks_body.clone()).await;
    let (app, _state) = boot(ext_auth_with_jwks(jwks_uri)).await;

    let (iat, exp) = timestamps_now_plus(300);
    let claims = serde_json::json!({
        "sub": "alice",
        "iss": "test",
        "aud": "client-id",
        "iat": iat,
        "exp": exp,
    });
    let token = sign_jwt(&key.encoding_key, &key.kid, claims);

    let (status, body) = introspect(&app, &token).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["active"], true,
        "well-formed JWT must introspect active — body was {body:?}"
    );
    assert_eq!(
        body["sub"], "alice",
        "introspect response must echo the subject — body was {body:?}"
    );
    assert_eq!(body["provider"], "test");
}
