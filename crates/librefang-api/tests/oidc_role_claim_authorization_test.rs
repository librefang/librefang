//! Integration tests for OIDC role claims reaching an authorization decision (#7744, step 2 of five).
//!
//! `IdTokenClaims` has parsed a `roles: Vec<String>` claim since the type was written (`crates/librefang-api/src/oauth.rs`), and `oidc_auth_middleware` injected the whole struct into request extensions.
//! Nothing read it.
//! `grep -rn IdTokenClaims crates/ --include='*.rs'` on the pre-change tree returns the definition, three construction sites inside `oauth.rs`, and two test files — no handler, no middleware, no authorization check.
//! So a cryptographically validated statement by the identity provider about what the caller may do was fetched, verified against JWKS, and dropped.
//!
//! These tests drive the **real** router — `server::build_router`, which layers `oidc_auth_middleware` and `middleware::auth` in the production order — against RSA-2048-signed JWTs served from an in-process JWKS endpoint.
//! They assert over HTTP status codes on routes whose required role differs, which is what makes them regression tests rather than tautologies: the file compiles identically before and after the change and fails at runtime, because before it the grant does not exist and every OIDC caller is simply unauthenticated.
//!
//! `GET /api/config/export` is the probe for "which role reached the decision".
//! `min_role_for_privileged_get` (`crates/librefang-api/src/middleware.rs`) gates it at `Owner`, so it separates three outcomes that a coarser route collapses into one: 401 means no credential was established at all, 403 means a credential was established at a role below `Owner`, and 200 means the caller reached the top of the ladder.
//! `GET /api/health/detail` is the companion probe for "was the caller authenticated at all" — it requires auth unconditionally and is on no public allowlist, so a `Viewer` reaches it while an unauthenticated caller does not.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{DefaultModelConfig, ExternalAuthConfig, KernelConfig, OidcProvider};
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use std::collections::BTreeMap;
use std::sync::Arc;
use tower::ServiceExt;

// `rand_core` is not a direct workspace dep; `argon2` (already a dependency of `librefang-api`) re-exports the same `OsRng` the `rsa` crate's `CryptoRngCore` bound wants.
// Same trick as `oauth_sub_required_test.rs`.
use argon2::password_hash::rand_core::OsRng;

// ─── JWKS harness ───────────────────────────────────────────────────────

/// A fresh RSA-2048 keypair plus the JWKS document for its public half.
struct TestKey {
    encoding_key: EncodingKey,
    jwks_body: String,
    kid: String,
}

/// Generate a keypair per test rather than sharing one.
/// `fetch_jwks_cached` caches by URI and each test binds its own `127.0.0.1:0` listener, so cache entries stay partitioned and cannot bleed between tests.
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

/// Serve the JWKS from a local listener so `validate_jwt_cached` runs its real fetch + RS256 verification path without a live identity provider.
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

const TEST_AUDIENCE: &str = "librefang-test-client";

fn sign_id_token(key: &TestKey, roles: &[&str], email_verified: bool) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let claims = serde_json::json!({
        "sub": "idp-subject-0001",
        "email": "sso-user@corp.example",
        "email_verified": email_verified,
        "name": "SSO User",
        "roles": roles,
        "iss": "test",
        "aud": TEST_AUDIENCE,
        "iat": now,
        "exp": now + 300,
    });
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key).expect("encode JWT")
}

// ─── Daemon harness ─────────────────────────────────────────────────────

struct RouterHarness {
    app: axum::Router,
    _tmp: tempfile::TempDir,
    _state: Arc<librefang_api::routes::AppState>,
    _jwks: tokio::task::JoinHandle<()>,
}

impl Drop for RouterHarness {
    fn drop(&mut self) {
        self._state.kernel.shutdown();
    }
}

/// Boot the full production router with `[external_auth]` pointed at a local JWKS endpoint.
///
/// `api_key` is a parameter because the two admission points added in #7744 sit in different branches of `middleware::auth`: a deployment with a local credential configured reaches the final fallthrough after every local check missed, and a deployment whose only credential is the identity provider reaches the earlier fail-closed branch that used to answer "API key required for non-loopback requests".
/// `audience` is a parameter so one test can pin the audience-unbound refusal; it doubles as `client_id`, because `resolve_single_provider` falls back to the client id when no explicit audience is configured and a provider is therefore only unbound when both are empty.
async fn boot(
    api_key: &str,
    role_map: BTreeMap<String, String>,
    audience: &str,
    require_email_verified: bool,
) -> (RouterHarness, TestKey) {
    // `external_auth.enabled = true` makes the kernel require a well-formed `LIBREFANG_STATE_SECRET` at boot.
    // 32 zero bytes, base64.
    // Only ever set, never cleared, and valid for any concurrent boot, so parallel tests in this binary are unaffected.
    // These tests never exercise the state HMAC.
    std::env::set_var(
        "LIBREFANG_STATE_SECRET",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
    );

    let key = generate_test_key("oidc-role-kid");
    let (jwks_uri, jwks_handle) = spawn_jwks_server(key.jwks_body.clone()).await;

    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        external_auth: ExternalAuthConfig {
            enabled: true,
            require_email_verified,
            role_map,
            providers: vec![OidcProvider {
                id: "test".into(),
                display_name: "Test".into(),
                issuer_url: String::new(),
                auth_url: "https://example.invalid/authorize".into(),
                token_url: "https://example.invalid/token".into(),
                userinfo_url: String::new(),
                jwks_uri,
                // Empty when the test is pinning the audience-unbound refusal.
                // `resolve_single_provider` falls back to `client_id` when `audience` is unset, so a provider is only genuinely unbound when neither is configured.
                client_id: audience.into(),
                client_secret_env: "LIBREFANG_OIDC_ROLE_TEST_SECRET_DOES_NOT_EXIST".into(),
                redirect_url: "http://127.0.0.1:4545/api/auth/callback".into(),
                scopes: vec!["openid".into()],
                allowed_domains: vec![],
                audience: audience.into(),
                require_email_verified: None,
            }],
            ..Default::default()
        },
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
            message_timeout_secs: 300,
            extra_params: BTreeMap::new(),
            cli_profile_dirs: Vec::new(),
        },
        ..KernelConfig::default()
    };

    let kernel = Arc::new(LibreFangKernel::boot_with_config(config).expect("kernel boot"));
    kernel.set_self_handle();
    let (app, state) = server::build_router(kernel, "127.0.0.1:0".parse().expect("addr")).await;
    (
        RouterHarness {
            app,
            _tmp: tmp,
            _state: state,
            _jwks: jwks_handle,
        },
        key,
    )
}

fn role_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// `oneshot` leaves `ConnectInfo` unset, so `auth` treats the caller as non-loopback and takes the same path a LAN client would. That is deliberate — the loopback fast-path would attribute the request before the OIDC branch is ever reached.
async fn get(app: &Router, uri: &str, bearer: Option<&str>) -> StatusCode {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    resp.status()
}

/// Owner-gated (`min_role_for_privileged_get`). 200 only for `Owner`.
const OWNER_PROBE: &str = "/api/config/export";
/// Authenticated but role-agnostic: any established credential reaches it, no credential does not.
const AUTHED_PROBE: &str = "/api/health/detail";

// ─── Tests ──────────────────────────────────────────────────────────────

/// The claim reaches the decision, and the decision is the one the operator mapped.
///
/// `roles: ["librefang-owners"]` with `[external_auth.role_map] "librefang-owners" = "owner"` must clear the `Owner` gate on `GET /api/config/export`.
/// Before this change the same request is 401: the claims were validated, injected, and ignored, and no other credential matched a JWT.
#[tokio::test(flavor = "multi_thread")]
async fn oidc_role_claim_reaches_the_authorization_decision() {
    let (h, key) = boot(
        "operator-master-key",
        role_map(&[("librefang-owners", "owner")]),
        TEST_AUDIENCE,
        true,
    )
    .await;
    let token = sign_id_token(&key, &["librefang-owners"], true);

    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::OK,
        "a mapped `owner` claim must clear the Owner gate on {OWNER_PROBE}; \
         401 means the grant never reached `middleware::auth`, 403 means it \
         arrived at the wrong role"
    );
}

/// The highest-privilege match wins, mirroring `[channel_role_mapping.discord] role_map`.
///
/// The caller holds `viewer` and `owner` mappings simultaneously and the token lists the lower one first, so a first-match implementation resolves `Viewer` and 403s.
/// Claim ordering is the identity provider's business and must not decide LibreFang privilege.
#[tokio::test(flavor = "multi_thread")]
async fn highest_privilege_mapping_wins_regardless_of_claim_order() {
    let (h, key) = boot(
        "operator-master-key",
        role_map(&[("everyone", "viewer"), ("librefang-owners", "owner")]),
        TEST_AUDIENCE,
        true,
    )
    .await;
    let token = sign_id_token(&key, &["everyone", "librefang-owners"], true);

    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::OK,
        "`everyone` listed first must not cap the caller at Viewer"
    );
}

/// A mapped role below the gate is denied *as that role*, not waved through and not promoted.
///
/// The pair of probes is the point: 200 on the authenticated probe proves a credential was genuinely established from the claim, and 403 on the Owner probe proves the level that credential carried was the mapped `viewer` rather than a blanket admission.
#[tokio::test(flavor = "multi_thread")]
async fn mapped_role_below_the_gate_is_denied_at_that_role() {
    let (h, key) = boot(
        "operator-master-key",
        role_map(&[("librefang-readers", "viewer")]),
        TEST_AUDIENCE,
        true,
    )
    .await;
    let token = sign_id_token(&key, &["librefang-readers"], true);

    assert_eq!(
        get(&h.app, AUTHED_PROBE, Some(&token)).await,
        StatusCode::OK,
        "a mapped `viewer` must be authenticated for an ordinary read"
    );
    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::FORBIDDEN,
        "a mapped `viewer` must be refused the Owner-gated export — 200 here \
         would mean the claim admitted the caller without carrying a role"
    );
}

/// An unrecognised LibreFang role string on the right-hand side grants nothing.
///
/// `UserRole::from_str_role` resolves any unknown string to `User`, which is exactly the silent promotion this must not do; the strict `try_from_str_role` behind `translate_oidc_roles` is what makes `"ownr"` a no-op instead.
/// Both probes must reject: no escalation to Owner, and no quiet demotion to an authenticated `User` either.
#[tokio::test(flavor = "multi_thread")]
async fn unrecognised_role_string_grants_nothing() {
    let (h, key) = boot(
        "operator-master-key",
        role_map(&[("librefang-owners", "ownr")]),
        TEST_AUDIENCE,
        true,
    )
    .await;
    let token = sign_id_token(&key, &["librefang-owners"], true);

    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::UNAUTHORIZED,
        "a typo'd target role must not escalate"
    );
    assert_eq!(
        get(&h.app, AUTHED_PROBE, Some(&token)).await,
        StatusCode::UNAUTHORIZED,
        "a typo'd target role must not quietly authenticate the caller at `User` either"
    );
}

/// A claim value the operator never mapped grants nothing.
///
/// This is the common case in a real tenant: an identity provider hands out dozens of group memberships, and only the ones written into `role_map` may carry privilege.
#[tokio::test(flavor = "multi_thread")]
async fn unmapped_claim_value_grants_nothing() {
    let (h, key) = boot(
        "operator-master-key",
        role_map(&[("librefang-owners", "owner")]),
        TEST_AUDIENCE,
        true,
    )
    .await;
    let token = sign_id_token(&key, &["some-other-corporate-group"], true);

    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::UNAUTHORIZED,
        "an unmapped group must not authorize anything"
    );
}

/// With no `role_map` declared — the default — an OIDC bearer authorizes exactly nothing, and every other credential path is untouched.
///
/// This is the "behaves exactly as today" control, and it has to be checked on a server where external auth is *enabled and working*: the token below validates against JWKS and its claims land in request extensions, and it still must not authenticate anything.
#[tokio::test(flavor = "multi_thread")]
async fn caller_with_no_configured_mapping_behaves_exactly_as_before() {
    let (h, key) = boot("operator-master-key", BTreeMap::new(), TEST_AUDIENCE, true).await;
    let token = sign_id_token(&key, &["librefang-owners"], true);

    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::UNAUTHORIZED,
        "an empty role_map must leave the OIDC bearer inert"
    );
    assert_eq!(
        get(&h.app, OWNER_PROBE, None).await,
        StatusCode::UNAUTHORIZED,
        "a caller with no credential at all is unchanged"
    );
    assert_eq!(
        get(&h.app, OWNER_PROBE, Some("operator-master-key")).await,
        StatusCode::OK,
        "the master api_key must still authenticate as Owner — the OIDC branch \
         runs only where `auth` was about to reject"
    );
}

/// A deployment whose only configured credential is the identity provider.
///
/// `api_key` empty, no `[[users]]`, no dashboard password: `middleware::auth` takes its fail-closed branch and used to answer "API key required for non-loopback requests" even to a caller holding a token this daemon had just verified against the provider's JWKS.
/// That branch is the second of the two admission points, and it is the one an SSO-only install actually hits.
#[tokio::test(flavor = "multi_thread")]
async fn oidc_only_deployment_is_admitted_at_the_fail_closed_branch() {
    let (h, key) = boot(
        "",
        role_map(&[("librefang-owners", "owner")]),
        TEST_AUDIENCE,
        true,
    )
    .await;
    let token = sign_id_token(&key, &["librefang-owners"], true);

    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::OK,
        "an SSO-only deployment must admit a mapped owner"
    );
    assert_eq!(
        get(&h.app, OWNER_PROBE, None).await,
        StatusCode::UNAUTHORIZED,
        "and must still fail closed for a caller with no token"
    );
}

/// A provider with no audience to bind tokens to grants nothing.
///
/// `validate_jwt_cached` sets `validation.validate_aud = false` when the expected audience is empty, so any token signed by that issuer's JWKS validates — including one minted for a different OAuth client in the same tenant.
/// That is survivable while the claims are inert and is not survivable as a grant of API privilege, so the grant is withheld rather than the request rejected: a caller authenticating by some other means and merely carrying a JWT must not start getting 403s.
#[tokio::test(flavor = "multi_thread")]
async fn audience_unbound_provider_grants_nothing() {
    let (h, key) = boot(
        "operator-master-key",
        role_map(&[("librefang-owners", "owner")]),
        "",
        true,
    )
    .await;
    let token = sign_id_token(&key, &["librefang-owners"], true);

    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::UNAUTHORIZED,
        "an unbound provider must not mint privilege from an unbound token"
    );
    assert_eq!(
        get(&h.app, OWNER_PROBE, Some("operator-master-key")).await,
        StatusCode::OK,
        "withholding the grant must not disturb any other credential"
    );
}

/// An unverified email grants nothing while `require_email_verified` is on.
///
/// That flag is the #3703 mitigation against claiming an address inside an `allowed_domains` domain without owning it, and the callback route has enforced it since; the middleware path never did, because it had nothing to mint.
/// The second assertion is the control that the refusal is about verification and not about the harness: the identical token with `email_verified = true` is admitted.
#[tokio::test(flavor = "multi_thread")]
async fn unverified_email_grants_nothing_when_verification_is_required() {
    let (h, key) = boot(
        "operator-master-key",
        role_map(&[("librefang-owners", "owner")]),
        TEST_AUDIENCE,
        true,
    )
    .await;

    let unverified = sign_id_token(&key, &["librefang-owners"], false);
    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&unverified)).await,
        StatusCode::UNAUTHORIZED,
        "an unverified address must not carry a role"
    );

    let verified = sign_id_token(&key, &["librefang-owners"], true);
    assert_eq!(
        get(&h.app, OWNER_PROBE, Some(&verified)).await,
        StatusCode::OK,
        "the same token with a verified address must be admitted — otherwise \
         the assertion above proves nothing about the verification gate"
    );
}
