//! Integration tests for identity-provider group claims reaching local `[[groups]]` membership (#7746, the fourth and last step of the identity programme).
//!
//! #7906 made an IdP *role* claim able to confer RBAC privilege through `[external_auth.role_map]`.
//! The group half stayed unreachable: `IdTokenClaims` had no `groups` field at all, `[[groups]]` membership (#7913) was whatever an operator had typed into `config.toml` by hand, and `Principal::Group` (#7928) named a set whose contents drifted from the directory that actually knows who is on which team.
//! `[external_auth.group_map]` closes that, and these tests drive the **real** router — `server::build_router`, which layers `oidc_auth_middleware` and `middleware::auth` in production order — against RSA-2048-signed JWTs served from an in-process JWKS endpoint.
//!
//! `GET /api/authz/whoami` is the probe for "what did the daemon decide this caller is".
//! It reports the resolved name, the RBAC role, the effective group set, the subset of that set which came from this request's token, and the group-conferred role strings — which is exactly the five-way split the decisions under test have to be read against.
//! `GET /api/config/export` is the companion probe for privilege: `min_role_for_privileged_get` gates it at `Owner`, so it separates 401 (no credential at all) from 403 (a credential below `Owner`) from 200.
//!
//! The security property every test here exists to protect is one sentence: **a grant is something an operator wrote down, never something the identity provider can mint by inventing a claim value.**
//! `forged_group_claims_naming_local_groups_confer_nothing` is the negative control for it.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::Router;
use base64::Engine;
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use librefang_api::server;
use librefang_kernel::LibreFangKernel;
use librefang_types::config::{
    DefaultModelConfig, ExternalAuthConfig, GroupConfig, KernelConfig, OidcProvider,
};
use rsa::pkcs8::EncodePrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use std::collections::BTreeMap;
use std::sync::Arc;
use tower::ServiceExt;

// `rand_core` is not a direct workspace dep; `argon2` (already a dependency of
// `librefang-api`) re-exports the same `OsRng` the `rsa` crate's
// `CryptoRngCore` bound wants. Same trick as `oidc_role_claim_authorization_test.rs`.
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

/// Serve the JWKS from a local listener so `validate_jwt_cached` runs its real
/// fetch + RS256 verification path without a live identity provider.
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
const SSO_EMAIL: &str = "sso-user@corp.example";

/// Sign an ID token whose identity-attribute claims are given verbatim.
///
/// `extra` is merged into the top-level claim object so a test can pin a nested
/// Keycloak shape (`realm_access.roles`) as easily as a flat `groups` array —
/// the claim *shape* is the thing several of these tests are about.
fn sign_token(key: &TestKey, extra: serde_json::Value, email_verified: bool) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut claims = serde_json::json!({
        "sub": "idp-subject-0001",
        "email": SSO_EMAIL,
        "email_verified": email_verified,
        "name": "SSO User",
        "iss": "test",
        "aud": TEST_AUDIENCE,
        "iat": now,
        "exp": now + 300,
    });
    if let (Some(target), Some(source)) = (claims.as_object_mut(), extra.as_object()) {
        for (k, v) in source {
            target.insert(k.clone(), v.clone());
        }
    }
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid.clone());
    encode(&header, &claims, &key.encoding_key).expect("encode JWT")
}

/// Flat `groups` claim, the spelling Okta / Authentik / Google Workspace use.
fn groups_claim(groups: &[&str]) -> serde_json::Value {
    serde_json::json!({ "groups": groups })
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

/// Everything a test varies about the deployment under test.
///
/// A struct rather than eight positional parameters, because half of these are
/// maps and booleans and a call site reading `(map, map, "", true, true)` says
/// nothing about which gate it is pinning.
#[derive(Default)]
struct Deployment {
    role_map: BTreeMap<String, String>,
    group_map: BTreeMap<String, String>,
    groups: Vec<GroupConfig>,
    claim_paths: Option<Vec<String>>,
    /// Doubles as `client_id`: `resolve_single_provider` falls back to the
    /// client id when `audience` is unset, so a provider is only genuinely
    /// audience-unbound when neither is configured.
    audience: String,
    require_email_verified: bool,
}

impl Deployment {
    /// The ordinary case: audience-bound, verification required.
    fn bound() -> Self {
        Self {
            audience: TEST_AUDIENCE.to_string(),
            require_email_verified: true,
            ..Default::default()
        }
    }

    fn with_role_map(mut self, pairs: &[(&str, &str)]) -> Self {
        self.role_map = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self
    }

    fn with_group_map(mut self, pairs: &[(&str, &str)]) -> Self {
        self.group_map = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        self
    }

    fn with_group(mut self, name: &str, members: &[&str], roles: &[&str]) -> Self {
        self.groups.push(GroupConfig {
            name: name.to_string(),
            description: String::new(),
            members: members.iter().map(|m| m.to_string()).collect(),
            roles: roles.iter().map(|r| r.to_string()).collect(),
        });
        self
    }

    fn with_claim_paths(mut self, paths: &[&str]) -> Self {
        self.claim_paths = Some(paths.iter().map(|p| p.to_string()).collect());
        self
    }
}

const MASTER_KEY: &str = "operator-master-key";

/// Boot the full production router with `[external_auth]` pointed at a local JWKS endpoint.
async fn boot(dep: Deployment) -> (RouterHarness, TestKey) {
    // `external_auth.enabled = true` makes the kernel require a well-formed
    // `LIBREFANG_STATE_SECRET` at boot. 32 zero bytes, base64. Only ever set,
    // never cleared, and valid for any concurrent boot, so parallel tests in
    // this binary are unaffected. These tests never exercise the state HMAC.
    std::env::set_var(
        "LIBREFANG_STATE_SECRET",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
    );

    let key = generate_test_key("oidc-group-kid");
    let (jwks_uri, jwks_handle) = spawn_jwks_server(key.jwks_body.clone()).await;

    let tmp = tempfile::tempdir().expect("tempdir");
    librefang_kernel::registry_sync::seed_registry_fixture_for_tests(tmp.path());

    let external_auth = ExternalAuthConfig {
        enabled: true,
        require_email_verified: dep.require_email_verified,
        role_map: dep.role_map,
        group_map: dep.group_map,
        claim_paths: dep
            .claim_paths
            .unwrap_or_else(|| ExternalAuthConfig::default().claim_paths),
        providers: vec![OidcProvider {
            id: "test".into(),
            display_name: "Test".into(),
            issuer_url: String::new(),
            auth_url: "https://example.invalid/authorize".into(),
            token_url: "https://example.invalid/token".into(),
            userinfo_url: String::new(),
            jwks_uri,
            client_id: dep.audience.clone(),
            client_secret_env: "LIBREFANG_OIDC_GROUP_TEST_SECRET_DOES_NOT_EXIST".into(),
            redirect_url: "http://127.0.0.1:4545/api/auth/callback".into(),
            scopes: vec!["openid".into()],
            allowed_domains: vec![],
            audience: dep.audience.clone(),
            require_email_verified: None,
        }],
        ..Default::default()
    };

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: MASTER_KEY.to_string(),
        groups: dep.groups,
        external_auth,
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

/// `oneshot` leaves `ConnectInfo` unset, so `auth` treats the caller as non-loopback and takes the same path a LAN client would.
/// That is deliberate — the loopback fast-path would attribute the request before the OIDC branch is ever reached.
async fn send(app: &Router, uri: &str, bearer: Option<&str>) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder().method(Method::GET).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap_or_default();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn status(app: &Router, uri: &str, bearer: Option<&str>) -> StatusCode {
    send(app, uri, bearer).await.0
}

/// Fetch `/api/authz/whoami`, asserting it was reachable, and return the body.
async fn whoami(app: &Router, bearer: Option<&str>) -> serde_json::Value {
    let (st, body) = send(app, WHOAMI, bearer).await;
    assert_eq!(
        st,
        StatusCode::OK,
        "whoami must be reachable by any authenticated caller; got {st} with {body}"
    );
    body
}

fn strings(value: &serde_json::Value, field: &str) -> Vec<String> {
    value[field]
        .as_array()
        .unwrap_or_else(|| panic!("`{field}` must be an array, got {value}"))
        .iter()
        .map(|v| v.as_str().unwrap_or_default().to_string())
        .collect()
}

/// The caller's own resolved identity. Authenticated, not Admin-gated.
const WHOAMI: &str = "/api/authz/whoami";
/// Owner-gated (`min_role_for_privileged_get`). 200 only for `Owner`.
const OWNER_PROBE: &str = "/api/config/export";

// ─── Tests ──────────────────────────────────────────────────────────────

/// The happy path, end to end: an IdP group claim becomes membership in the local group the operator mapped it to, and that membership confers the group's role strings.
///
/// The caller is authenticated at `viewer` through `role_map` so there is a credential to attach the membership to; `groups`/`idp_groups`/`roles` are then the three separable answers.
#[tokio::test(flavor = "multi_thread")]
async fn idp_group_claim_confers_membership_in_the_mapped_local_group() {
    let (h, key) = boot(
        Deployment::bound()
            .with_role_map(&[("everyone", "viewer")])
            .with_group_map(&[("platform-oncall", "oncall")])
            .with_group("oncall", &[], &["approver"]),
    )
    .await;
    let token = sign_token(&key, groups_claim(&["everyone", "platform-oncall"]), true);

    let me = whoami(&h.app, Some(&token)).await;
    assert_eq!(me["name"], SSO_EMAIL);
    assert_eq!(me["role"], "viewer");
    assert_eq!(
        strings(&me, "groups"),
        vec!["oncall".to_string()],
        "the mapped IdP group must appear in the effective group set"
    );
    assert_eq!(
        strings(&me, "idp_groups"),
        vec!["oncall".to_string()],
        "and must be reported as token-derived, since nothing declared this caller a member"
    );
    assert_eq!(
        strings(&me, "roles"),
        vec!["approver".to_string(), "oncall".to_string()],
        "membership confers the group's own name plus its declared roles — the \
         channel-binding vocabulary `KernelConfig::effective_roles_for` returns"
    );
    // The membership is a principal the ownership model can name (#7928).
    assert_eq!(
        me["principal"],
        librefang_types::principal::Principal::user_named(SSO_EMAIL).to_string(),
        "the caller is still their own user principal; the group is membership, not identity"
    );
}

/// SECURITY NEGATIVE CONTROL — an identity provider cannot mint a grant by naming a claim after a local group.
///
/// The token asserts membership in three groups whose names are *exactly* the local `[[groups]]` names, plus a claim value matching a `role_map` key that was never declared here.
/// None of them appears in `group_map`, so all four must confer nothing: an unmapped claim value falls through to the same outcome as no claim at all, never to a lower-but-nonzero grant.
///
/// This is the property the whole design rests on. If `group_map` were replaced by name-matching against `[[groups]]`, anyone who can create a group in the identity provider — in a self-service tenant, every employee — could name one `oncall` and acquire whatever `oncall` owns and confers.
///
/// The negative control was verified by deletion: removing the `group_map.get(claim)` lookup in `translate_oidc_groups` and inserting the claim value directly makes this test fail with `groups` = `["compliance", "oncall"]`.
#[tokio::test(flavor = "multi_thread")]
async fn forged_group_claims_naming_local_groups_confer_nothing() {
    let (h, key) = boot(
        Deployment::bound()
            .with_role_map(&[("everyone", "viewer")])
            // Only one entry, and it names a claim value the token does not carry.
            .with_group_map(&[("platform-oncall", "oncall")])
            .with_group("oncall", &[], &["approver"])
            .with_group("compliance", &[], &["auditor"]),
    )
    .await;
    let token = sign_token(
        &key,
        groups_claim(&[
            "everyone",
            // Named after the local groups, which is the whole attack.
            "oncall",
            "compliance",
            // And after a privilege-shaped value, for good measure.
            "librefang-owners",
        ]),
        true,
    );

    let me = whoami(&h.app, Some(&token)).await;
    assert!(
        strings(&me, "groups").is_empty(),
        "a claim value the operator never mapped must confer no membership; got {:?}",
        strings(&me, "groups")
    );
    assert!(
        strings(&me, "idp_groups").is_empty(),
        "and must not be reported as token-derived membership either"
    );
    assert!(
        strings(&me, "roles").is_empty(),
        "no membership means no group-conferred role strings"
    );
    assert_eq!(
        me["role"], "viewer",
        "the caller keeps exactly the privilege `role_map` granted — the forged \
         group claims neither promote nor demote"
    );
    assert_eq!(
        status(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::FORBIDDEN,
        "and the Owner gate is still closed: 200 would mean a group claim reached the privilege ladder"
    );
}

/// SECURITY — group membership contributes no RBAC privilege, however the group is named.
///
/// The worst case an identity provider can construct: a claim the operator *did* map, onto a local group *named* `owner`, whose declared `roles` also say `owner`.
/// Both are role-shaped strings in the channel-binding vocabulary and neither may touch the `viewer < user < admin < owner` ladder.
///
/// This is the assertion that would invert if a later change decided a group could carry a `UserRole`, which is why it pins the 403 as well as the reported role.
#[tokio::test(flavor = "multi_thread")]
async fn group_membership_named_owner_confers_no_owner_privilege() {
    let (h, key) = boot(
        Deployment::bound()
            .with_role_map(&[("everyone", "viewer")])
            .with_group_map(&[("corp-admins", "owner")])
            .with_group("owner", &[], &["owner", "admin"]),
    )
    .await;
    let token = sign_token(&key, groups_claim(&["everyone", "corp-admins"]), true);

    let me = whoami(&h.app, Some(&token)).await;
    assert_eq!(
        strings(&me, "groups"),
        vec!["owner".to_string()],
        "the mapping is legitimate — the operator wrote it — so membership is conferred"
    );
    assert_eq!(
        me["role"], "viewer",
        "…and confers no privilege: `role` still reflects `role_map` alone"
    );
    assert_eq!(
        status(&h.app, OWNER_PROBE, Some(&token)).await,
        StatusCode::FORBIDDEN,
        "a group called `owner` is a team called `owner`; the Owner gate stays closed"
    );
}

/// A `group_map` target that names no `[[groups]]` entry confers nothing.
///
/// A typo or a rename that missed the map must be a no-op, not a phantom group.
/// A group that existed only inside `group_map` would be a live `Principal` — able to own artifacts and appear in audit entries — while being invisible in `[[groups]]` and therefore unmanageable by the operator nominally responsible for it.
#[tokio::test(flavor = "multi_thread")]
async fn a_map_target_naming_no_declared_group_confers_nothing() {
    let (h, key) = boot(
        Deployment::bound()
            .with_role_map(&[("everyone", "viewer")])
            .with_group_map(&[
                ("platform-oncall", "oncal"), // typo
                ("sox-reviewers", "compliance"),
            ])
            .with_group("compliance", &[], &["auditor"]),
    )
    .await;
    let token = sign_token(
        &key,
        groups_claim(&["everyone", "platform-oncall", "sox-reviewers"]),
        true,
    );

    let me = whoami(&h.app, Some(&token)).await;
    assert_eq!(
        strings(&me, "groups"),
        vec!["compliance".to_string()],
        "the dangling target drops out and the sound half of the map still works"
    );
    assert!(!strings(&me, "roles").contains(&"oncal".to_string()));
}

/// Declared `[[groups]]` membership and IdP-derived membership are unioned, and the union collapses the overlap.
///
/// Membership is a set, not a ladder, so there is no precedence rule to apply — and in particular the identity provider not asserting `billing` does not retract the `members` entry an operator typed.
#[tokio::test(flavor = "multi_thread")]
async fn declared_and_idp_membership_are_unioned() {
    let (h, key) = boot(
        Deployment::bound()
            .with_role_map(&[("everyone", "viewer")])
            .with_group_map(&[
                ("platform-oncall", "oncall"),
                // Maps onto a group the caller is *also* a declared member of.
                ("finance", "billing"),
            ])
            // The caller is declared here under the name their token resolves to.
            .with_group("billing", &[SSO_EMAIL], &["invoicer"])
            .with_group("oncall", &[], &["approver"])
            .with_group("archive", &[SSO_EMAIL], &[]),
    )
    .await;
    let token = sign_token(
        &key,
        groups_claim(&["everyone", "platform-oncall", "finance"]),
        true,
    );

    let me = whoami(&h.app, Some(&token)).await;
    assert_eq!(
        strings(&me, "groups"),
        vec![
            "archive".to_string(),
            "billing".to_string(),
            "oncall".to_string()
        ],
        "declared ∪ claimed, ordered by name, with `billing` appearing once despite both sources naming it"
    );
    assert_eq!(
        strings(&me, "idp_groups"),
        vec!["billing".to_string(), "oncall".to_string()],
        "`idp_groups` reports what the token asserted, including the overlap — the \
         operator needs to see which memberships would survive removing the claim"
    );
    assert_eq!(
        strings(&me, "roles"),
        vec![
            "approver".to_string(),
            "archive".to_string(),
            "billing".to_string(),
            "invoicer".to_string(),
            "oncall".to_string()
        ],
    );
    // `archive` is declared-only: the identity provider asserts nothing about it
    // and cannot retract it.
    assert!(!strings(&me, "idp_groups").contains(&"archive".to_string()));
}

/// Keycloak's nested claim shapes resolve through dotted `claim_paths`, with `<client>` substituted from the provider's client id.
///
/// `realm_access.roles` and `resource_access.<client>.roles` have no flat spelling, so a `String` claim *name* cannot address either; this is the reason `claim_paths` is a path list rather than a name list.
/// The token also carries another client's roles under `resource_access`, which must not be picked up.
#[tokio::test(flavor = "multi_thread")]
async fn keycloak_nested_claim_paths_resolve_to_membership() {
    let (h, key) = boot(
        Deployment::bound()
            .with_claim_paths(&["realm_access.roles", "resource_access.<client>.roles"])
            .with_role_map(&[("librefang-users", "viewer")])
            .with_group_map(&[
                ("platform-oncall", "oncall"),
                ("workflow-author", "authors"),
                // Present in the map, but only asserted for a *different* client.
                ("finance-admin", "billing"),
            ])
            .with_group("oncall", &[], &[])
            .with_group("authors", &[], &[])
            .with_group("billing", &[], &[]),
    )
    .await;
    let token = sign_token(
        &key,
        serde_json::json!({
            "realm_access": { "roles": ["librefang-users", "platform-oncall"] },
            "resource_access": {
                TEST_AUDIENCE: { "roles": ["workflow-author"] },
                "some-other-client": { "roles": ["finance-admin"] },
            },
        }),
        true,
    );

    let me = whoami(&h.app, Some(&token)).await;
    assert_eq!(
        me["role"], "viewer",
        "the realm role reached `role_map`, so both maps read the same resolved claim set"
    );
    assert_eq!(
        strings(&me, "groups"),
        vec!["authors".to_string(), "oncall".to_string()],
    );
    assert!(
        !strings(&me, "groups").contains(&"billing".to_string()),
        "`<client>` must bind to this provider's client id — another client's roles in the same token are not ours"
    );
}

/// SECURITY — the two provider-level gates withhold membership exactly as they withhold privilege.
///
/// An unverified email (with `require_email_verified` on) is the #3703 mitigation: an address inside an `allowed_domains` domain that nobody proved they own must not become a team membership either.
/// The second half is the control that the refusal is about verification rather than the harness — the identical token with a verified address is admitted.
#[tokio::test(flavor = "multi_thread")]
async fn an_unverified_email_confers_no_membership() {
    let (h, key) = boot(
        Deployment::bound()
            .with_role_map(&[("everyone", "viewer")])
            .with_group_map(&[("platform-oncall", "oncall")])
            .with_group("oncall", &[], &[]),
    )
    .await;

    let unverified = sign_token(&key, groups_claim(&["everyone", "platform-oncall"]), false);
    assert_eq!(
        status(&h.app, WHOAMI, Some(&unverified)).await,
        StatusCode::UNAUTHORIZED,
        "an unverified address carries neither a role nor a membership, so no credential is established at all"
    );

    let verified = sign_token(&key, groups_claim(&["everyone", "platform-oncall"]), true);
    let me = whoami(&h.app, Some(&verified)).await;
    assert_eq!(strings(&me, "groups"), vec!["oncall".to_string()]);
}

/// SECURITY — a provider with nothing to bind tokens to confers no membership.
///
/// `validate_jwt_cached` sets `validation.validate_aud = false` when the expected audience is empty, so any token signed by that issuer's JWKS validates, including one minted for an unrelated OAuth client in the same tenant.
/// The grant is withheld rather than the request rejected, so a caller authenticating by some other means and merely carrying a JWT is undisturbed — which the master-key assertion pins.
#[tokio::test(flavor = "multi_thread")]
async fn an_audience_unbound_provider_confers_no_membership() {
    let mut dep = Deployment::bound()
        .with_role_map(&[("everyone", "viewer")])
        .with_group_map(&[("platform-oncall", "oncall")])
        .with_group("oncall", &[], &[]);
    dep.audience = String::new();
    let (h, key) = boot(dep).await;
    let token = sign_token(&key, groups_claim(&["everyone", "platform-oncall"]), true);

    assert_eq!(
        status(&h.app, WHOAMI, Some(&token)).await,
        StatusCode::UNAUTHORIZED,
        "an unbound provider must mint nothing from an unbound token"
    );
    let me = whoami(&h.app, Some(MASTER_KEY)).await;
    assert_eq!(
        me["role"], "owner",
        "withholding the grant must not disturb any other credential"
    );
}

/// With no `group_map` declared — the default — an OIDC bearer confers no membership, and every other credential path is untouched.
///
/// The control has to run on a server where external auth is enabled and working: the token below validates against JWKS and its `groups` claim lands in request extensions, and it still must confer nothing.
#[tokio::test(flavor = "multi_thread")]
async fn group_mapping_is_inert_until_an_operator_opts_in() {
    let (h, key) = boot(
        Deployment::bound()
            .with_role_map(&[("everyone", "viewer")])
            .with_group("oncall", &[], &["approver"]),
    )
    .await;
    let token = sign_token(&key, groups_claim(&["everyone", "oncall"]), true);

    let me = whoami(&h.app, Some(&token)).await;
    assert_eq!(me["role"], "viewer", "the role mapping is unaffected");
    assert!(
        strings(&me, "groups").is_empty(),
        "an empty group_map leaves the group claim inert"
    );

    let root = whoami(&h.app, Some(MASTER_KEY)).await;
    assert_eq!(root["name"], "root");
    assert_eq!(root["role"], "owner");
    assert_eq!(
        root["principal"],
        serde_json::Value::Null,
        "the synthetic root credential names no `[[users]]` entry and is not an owning principal (#7928)"
    );
    assert!(
        strings(&root, "idp_groups").is_empty(),
        "a local credential has no token-derived membership"
    );
}

/// `scope` is not read unless an operator names it in `claim_paths`.
///
/// `roles` and `groups` are assertions the identity provider makes about the user; `scope` is an assertion about what a client application asked for and was granted, which is a weaker statement with a different attacker.
/// Nobody inherits it by accident.
#[tokio::test(flavor = "multi_thread")]
async fn scope_confers_nothing_until_it_is_named_in_claim_paths() {
    let scoped = serde_json::json!({ "scope": "openid email librefang-oncall" });

    let (default_paths, key) = boot(
        Deployment::bound()
            .with_role_map(&[("openid", "viewer")])
            .with_group_map(&[("librefang-oncall", "oncall")])
            .with_group("oncall", &[], &[]),
    )
    .await;
    let token = sign_token(&key, scoped.clone(), true);
    assert_eq!(
        status(&default_paths.app, WHOAMI, Some(&token)).await,
        StatusCode::UNAUTHORIZED,
        "with the default claim_paths nothing in `scope` is read, so the token establishes no credential"
    );

    let (opted_in, key2) = boot(
        Deployment::bound()
            .with_claim_paths(&["roles", "groups", "scope"])
            .with_role_map(&[("openid", "viewer")])
            .with_group_map(&[("librefang-oncall", "oncall")])
            .with_group("oncall", &[], &[]),
    )
    .await;
    let token2 = sign_token(&key2, scoped, true);
    let me = whoami(&opted_in.app, Some(&token2)).await;
    assert_eq!(
        me["role"], "viewer",
        "opting in splits the space-delimited scope into claim values both maps see"
    );
    assert_eq!(strings(&me, "groups"), vec!["oncall".to_string()]);
}
