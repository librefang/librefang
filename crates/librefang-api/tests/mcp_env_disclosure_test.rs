//! #6630 — `GET /api/mcp/servers` (and the `{name}` detail route) must never serialize MCP environment *values*.
//!
//! `McpServerConfigEntry::env` is documented as a list of variable names to pass through, but the supported representation also accepts an inline `KEY=VALUE`, so an operator can put a live credential there.
//! Both read routes used to return the raw list.
//! The report covers the Viewer-role case; it is worse than that — `/api/mcp/servers` sits in `PUBLIC_ROUTES_DASHBOARD_READS`, so with `require_auth_for_reads` unset (the default) an unauthenticated caller reads it too.
//!
//! Redacting the read side alone would have been a data-loss bug worse than the disclosure, because `McpServersPage` hydrates its edit form from the list response and submits every field back on save.
//! So the write path merges a bare `NAME` against what is stored, and that round-trip is pinned here too.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use librefang_api::routes::{self, AppState};
use librefang_testing::{MockKernelBuilder, TestAppState};
use librefang_types::config::McpRuntimeStore;
use std::sync::Arc;
use tower::ServiceExt;

/// Distinctive enough that a substring search over a whole response body is a meaningful assertion — no chance of a coincidental match.
const SENTINEL: &str = "ghp_S3nt1nelMustNeverAppearInAnyResponseBody";
const ENV_NAME: &str = "GITHUB_PERSONAL_ACCESS_TOKEN";
/// A second variable with no inline value: the name-only form must survive untouched, which is what the dashboard's notice ("referenced by name only") describes.
const PLAIN_ENV_NAME: &str = "GITHUB_API_URL";
/// The `http_compat` counterpart of [`SENTINEL`]: a static header `value` is a credential the read route must not disclose and the write path must not destroy (#6612).
const HEADER_SENTINEL: &str = "sk_h3aderMustNeverAppearInAnyResponseBody";

struct Harness {
    app: axum::Router,
    state: Arc<AppState>,
    _test: TestAppState,
}

/// DB-backed store so the test can read the persisted entry directly and prove the inline value is still there after a round-trip.
fn boot() -> Harness {
    let test = TestAppState::with_builder(MockKernelBuilder::new().with_config(|cfg| {
        cfg.mcp_runtime_store = McpRuntimeStore::Db;
    }));
    let state = test.state.clone();
    let app = routes::skills::router().with_state(state.clone());
    Harness {
        app,
        state,
        _test: test,
    }
}

async fn send(app: axum::Router, req: Request<Body>) -> (StatusCode, String) {
    let resp = app.oneshot(req).await.expect("router response");
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("read body");
    (status, String::from_utf8_lossy(&bytes).to_string())
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap()
}

fn put_json(uri: &str, body: serde_json::Value) -> Request<Body> {
    Request::builder()
        .method(Method::PUT)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

/// `false` exits immediately, so the connect attempt fails fast while the entry is still persisted and added to the effective set.
fn add_server_with_inline_secret(name: &str) -> Request<Body> {
    let body = serde_json::json!({
        "name": name,
        "transport": { "type": "stdio", "command": "false", "args": [] },
        "env": [format!("{ENV_NAME}={SENTINEL}"), PLAIN_ENV_NAME],
    });
    Request::builder()
        .method(Method::POST)
        .uri("/mcp/servers")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn stored_entry(
    state: &Arc<AppState>,
    name: &str,
) -> librefang_types::config::McpServerConfigEntry {
    let store = librefang_memory::McpConfigStore::new(state.kernel.memory_substrate().pool());
    store
        .get(name)
        .expect("store read")
        .expect("entry persisted")
}

fn stored_env(state: &Arc<AppState>, name: &str) -> Vec<String> {
    stored_entry(state, name).env
}

fn stored_http_compat_headers(
    state: &Arc<AppState>,
    name: &str,
) -> Vec<librefang_types::config::HttpCompatHeaderConfig> {
    match stored_entry(state, name)
        .transport
        .expect("transport persisted")
    {
        librefang_types::config::McpTransportEntry::HttpCompat { headers, .. } => headers,
        other => panic!("expected an http_compat transport, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_and_detail_never_disclose_inline_env_values() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("leaky")).await;
    assert_eq!(status, StatusCode::CREATED, "add should succeed");

    // Precondition: the inline value really is stored, so the assertions below are about the response and not about the value never existing.
    assert!(
        stored_env(&h.state, "leaky").contains(&format!("{ENV_NAME}={SENTINEL}")),
        "precondition: the inline value must be persisted for this test to mean anything"
    );

    for uri in ["/mcp/servers", "/mcp/servers/leaky"] {
        let (status, body) = send(h.app.clone(), get(uri)).await;
        assert_eq!(status, StatusCode::OK, "GET {uri} failed: {body}");
        assert!(
            !body.contains(SENTINEL),
            "GET {uri} disclosed the inline env value:\n{body}"
        );
        // The name is the useful, non-secret half and must survive — the dashboard renders it and an operator needs to know what the server expects.
        assert!(
            body.contains(ENV_NAME),
            "GET {uri} must still report the variable NAME:\n{body}"
        );
        assert!(
            body.contains(PLAIN_ENV_NAME),
            "GET {uri} must still report name-only entries:\n{body}"
        );
        // Guard the specific shape a naive fix produces: `KEY=***` still round-trips into the stored config and destroys the real value.
        assert!(
            !body.contains(&format!("{ENV_NAME}=")),
            "GET {uri} must return the bare name, not a masked `KEY=...` form \
             (a masked value round-trips back into the config):\n{body}"
        );
    }
}

/// The other half of the contract: a client that hydrates its form from the redacted list response and submits every field back must not wipe the inline value it was never shown.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn round_tripping_the_redacted_env_preserves_the_stored_inline_value() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("roundtrip")).await;
    assert_eq!(status, StatusCode::CREATED);

    // Read back exactly what a client sees, then submit it verbatim with one unrelated field changed — the dashboard's save flow.
    let (_, list) = send(h.app.clone(), get("/mcp/servers/roundtrip")).await;
    let detail: serde_json::Value = serde_json::from_str(&list).expect("detail is JSON");
    let env_from_response = detail
        .get("env")
        .and_then(|e| e.as_array())
        .expect("env array in detail response")
        .clone();
    assert_eq!(
        env_from_response,
        serde_json::json!([ENV_NAME, PLAIN_ENV_NAME])
            .as_array()
            .unwrap()
            .clone(),
        "the client sees names only"
    );

    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/roundtrip",
            serde_json::json!({
                "name": "roundtrip",
                "transport": { "type": "stdio", "command": "false", "args": [] },
                "timeout_secs": 99,
                "env": env_from_response,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");

    let stored = stored_env(&h.state, "roundtrip");
    assert!(
        stored.contains(&format!("{ENV_NAME}={SENTINEL}")),
        "the inline value must survive a round-trip through the redacted \
         response — otherwise redaction silently destroys the credential and \
         the server stops connecting for a reason the UI cannot explain. \
         stored: {stored:?}"
    );
    assert!(
        stored.iter().any(|e| e == PLAIN_ENV_NAME),
        "name-only entries must survive unchanged. stored: {stored:?}"
    );
}

/// #6612 — the round-trip above hardcodes a stdio transport, so it only ever proved the property for one of the four variants.
///
/// `McpTransportEntry` now carries `deny_unknown_fields`, which makes `serialize_mcp_transport` load-bearing rather than cosmetic: any key the read route synthesises into the `transport` object that is not a real field of the variant is a 400 on the way back in.
/// Serde short-circuits at the first unknown key, so a single stray key hides every other one — the test has to cover each variant rather than sampling.
///
/// The `http_compat` case is the one that regressed, and status alone does not pin it: a serializer can be accepted-but-lossy, which is what omitting `input_schema` and a static header `value` was.
/// So the fixture is seeded with a hand-authored `input_schema` and a static-`value` header, and the assertions read the persisted entry back rather than trusting the 200.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_transport_variant_survives_a_get_then_put_round_trip() {
    let h = boot();

    // One server per variant. `command = "false"` exits immediately so the connect attempt fails fast while the entry stays persisted; the URLs and `base_url` are `.invalid`, which can never resolve.
    let fixtures = [
        (
            "rt-stdio",
            serde_json::json!({ "type": "stdio", "command": "false", "args": ["--flag"] }),
        ),
        (
            "rt-sse",
            serde_json::json!({ "type": "sse", "url": "https://example.invalid/sse" }),
        ),
        (
            "rt-http",
            serde_json::json!({ "type": "http", "url": "https://example.invalid/mcp" }),
        ),
        (
            "rt-http-compat",
            serde_json::json!({
                "type": "http_compat",
                "base_url": "https://example.invalid",
                "headers": [
                    // A static value the read route must not disclose (#6630) and must not destroy either.
                    { "name": "X-Api-Key", "value": HEADER_SENTINEL },
                    // An env-sourced header: the name is not secret, so it round-trips verbatim.
                    { "name": "Authorization", "value_env": "COMPAT_TOKEN" },
                ],
                "tools": [{
                    "name": "forecast",
                    "description": "Fetch a forecast",
                    "path": "/forecast/{city}",
                    "method": "get",
                    "request_mode": "query",
                    "response_mode": "text",
                    // Deliberately not the `{"type":"object"}` default, so an omitted `input_schema` on the read side shows up as a diff rather than coinciding with the default.
                    "input_schema": {
                        "type": "object",
                        "properties": { "city": { "type": "string", "description": "City to forecast" } },
                        "required": ["city"],
                    },
                }],
            }),
        ),
    ];

    for (name, transport) in &fixtures {
        let body = serde_json::json!({
            "name": name,
            "transport": transport,
            "timeout_secs": 30,
        });
        let (status, resp) = send(
            h.app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/mcp/servers")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "adding {name} failed: {resp}");
    }

    for (name, _) in &fixtures {
        let (status, detail_body) = send(h.app.clone(), get(&format!("/mcp/servers/{name}"))).await;
        assert_eq!(status, StatusCode::OK, "GET {name} failed: {detail_body}");
        let detail: serde_json::Value = serde_json::from_str(&detail_body).expect("detail is JSON");
        let transport_from_response = detail
            .get("transport")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        assert!(
            transport_from_response.is_object(),
            "GET {name} returned no transport object: {detail_body}"
        );
        // The whole reason the static header `value` is omitted rather than round-tripped: this route is in `PUBLIC_ROUTES_DASHBOARD_READS` (#6630).
        // Asserted inside the loop so the merge below can never be "fixed" by simply disclosing the credential.
        assert!(
            !detail_body.contains(HEADER_SENTINEL),
            "GET {name} disclosed a static http_compat header value:\n{detail_body}"
        );

        // The read-modify-write a client actually performs: take the `transport` sub-object verbatim, change one unrelated field, submit.
        let (status, body) = send(
            h.app.clone(),
            put_json(
                &format!("/mcp/servers/{name}"),
                serde_json::json!({
                    "name": name,
                    "transport": transport_from_response,
                    "timeout_secs": 45,
                }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "PUT of the transport object GET returned for {name} was rejected — \
             the read route emits a key that is not a field of the variant, so \
             read-modify-write is broken for every external client: {body}"
        );

        // Status alone does not prove losslessness: the entry has to still be the variant it was, with its fields intact.
        let stored = stored_entry(&h.state, name);
        assert_eq!(
            stored.timeout_secs, 45,
            "{name}: the PUT must have taken effect, or the assertions below are vacuous"
        );
        let stored_transport = stored
            .transport
            .as_ref()
            .unwrap_or_else(|| panic!("{name}: transport must survive the round-trip"));
        assert_eq!(
            serde_json::to_value(stored_transport)
                .expect("stored transport serializes")
                .get("type")
                .and_then(|t| t.as_str()),
            transport_from_response.get("type").and_then(|t| t.as_str()),
            "{name}: the round-trip must not change which variant is stored"
        );
    }

    // The `http_compat` specifics, which are what a status-only assertion misses.
    let compat = stored_entry(&h.state, "rt-http-compat");
    match compat.transport.expect("http_compat transport persisted") {
        librefang_types::config::McpTransportEntry::HttpCompat {
            base_url,
            headers,
            tools,
        } => {
            assert_eq!(base_url, "https://example.invalid");

            assert_eq!(headers.len(), 2, "both headers must survive: {headers:?}");
            let static_header = headers
                .iter()
                .find(|h| h.name == "X-Api-Key")
                .expect("static header must survive");
            assert_eq!(
                static_header.value.as_deref(),
                Some(HEADER_SENTINEL),
                "a static header `value` is never shown to the caller (#6630), so a round-trip \
                 that submits the header back without it must restore the stored value rather \
                 than blank the credential"
            );
            let env_header = headers
                .iter()
                .find(|h| h.name == "Authorization")
                .expect("env-sourced header must survive");
            assert_eq!(env_header.value_env.as_deref(), Some("COMPAT_TOKEN"));
            assert!(
                env_header.value.is_none(),
                "the merge must not invent a static value for an env-sourced header: {env_header:?}"
            );

            assert_eq!(tools.len(), 1, "the tool must survive: {tools:?}");
            let tool = &tools[0];
            assert_eq!(tool.name, "forecast");
            assert_eq!(tool.description, "Fetch a forecast");
            assert_eq!(tool.path, "/forecast/{city}");
            assert!(matches!(
                tool.method,
                librefang_types::config::HttpCompatMethod::Get
            ));
            assert!(matches!(
                tool.request_mode,
                librefang_types::config::HttpCompatRequestMode::Query
            ));
            assert!(matches!(
                tool.response_mode,
                librefang_types::config::HttpCompatResponseMode::Text
            ));
            assert_eq!(
                tool.input_schema["properties"]["city"]["description"], "City to forecast",
                "a hand-authored input_schema must survive the round-trip rather than being \
                 reset to the `{{\"type\":\"object\"}}` default"
            );
            assert_eq!(
                tool.input_schema["required"],
                serde_json::json!(["city"]),
                "every part of the schema must survive, not just the properties block"
            );
        }
        other => panic!("expected the stored transport to still be http_compat, got {other:?}"),
    }
}

/// The three write-side cases the header merge has to get right, beyond the "restore what was redacted" case the round-trip test covers (#6612).
///
/// The rotation and removal cases are the ones that would make the merge a data trap rather than a safety net: if a value-less submission always won, an operator could never rotate a static header value, and if an absent header were restored they could never remove one.
/// The transport-switch case is the boundary — credentials must not survive a change of transport type and reappear when the operator switches back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_compat_header_values_rotate_remove_and_do_not_cross_a_transport_switch() {
    let h = boot();
    let compat_transport = |headers: serde_json::Value| {
        serde_json::json!({
            "type": "http_compat",
            "base_url": "https://example.invalid",
            "headers": headers,
            "tools": [{ "name": "ping", "path": "/ping" }],
        })
    };

    let (status, resp) = send(
        h.app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/mcp/servers")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "name": "hdr",
                    "transport": compat_transport(serde_json::json!([
                        { "name": "X-Api-Key", "value": HEADER_SENTINEL },
                        { "name": "X-Doomed", "value": "will-be-removed" },
                    ])),
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "add failed: {resp}");

    // Rotation: an explicit value wins over the stored one, and the header dropped from the submission goes away rather than being merged back.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/hdr",
            serde_json::json!({
                "name": "hdr",
                "transport": compat_transport(serde_json::json!([
                    { "name": "X-Api-Key", "value": "sk_rotated" },
                ])),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rotate failed: {body}");

    let headers = stored_http_compat_headers(&h.state, "hdr");
    assert_eq!(
        headers.len(),
        1,
        "the omitted header must be gone: {headers:?}"
    );
    assert_eq!(
        headers[0].value.as_deref(),
        Some("sk_rotated"),
        "an explicitly submitted value must replace the stored one, or a static header value \
         could never be rotated: {headers:?}"
    );

    // Switching transport type must not carry the credential across. `false` exits immediately, so the connect attempt fails fast.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/hdr",
            serde_json::json!({
                "name": "hdr",
                "transport": { "type": "stdio", "command": "false", "args": [] },
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "switch to stdio failed: {body}");

    // Switching back with a value-less header: there is no stored `http_compat` transport to restore from any more, so the header must stay value-less rather than resurrecting the old secret.
    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/hdr",
            serde_json::json!({
                "name": "hdr",
                "transport": compat_transport(serde_json::json!([
                    { "name": "X-Api-Key" },
                ])),
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "switch back failed: {body}");

    let headers = stored_http_compat_headers(&h.state, "hdr");
    assert_eq!(headers.len(), 1);
    assert!(
        headers[0].value.is_none(),
        "a credential must not survive a transport-type change and reappear on switch back: \
         {headers:?}"
    );
    assert!(
        !headers[0]
            .value
            .as_deref()
            .unwrap_or_default()
            .contains(HEADER_SENTINEL),
        "the original secret in particular must be gone: {headers:?}"
    );
}

/// A header may carry *both* a static `value` and a `value_env`, and `apply_http_compat_headers` resolves `value` first — so the static one is the live credential and the variable is a fallback that only takes effect once the static value is removed (#6612).
///
/// The read route redacts `value` and emits `value_env`, so on submit-back such a header arrives with only its `value_env`.
/// A merge that treated the presence of `value_env` as "this header needs nothing restored" would therefore drop the static value on every read-modify-write, silently flipping the header from "send this secret" to "resolve this variable" — a 200 with no error, no log, and a different request on the wire.
/// The other two tests cannot reach this: their fixtures set exactly one of the two fields per header.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_header_carrying_both_value_and_value_env_keeps_its_static_value_across_a_round_trip() {
    let h = boot();

    let (status, resp) = send(
        h.app.clone(),
        Request::builder()
            .method(Method::POST)
            .uri("/mcp/servers")
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&serde_json::json!({
                    "name": "both",
                    "transport": {
                        "type": "http_compat",
                        "base_url": "https://example.invalid",
                        "headers": [
                            { "name": "X-Api-Key", "value": HEADER_SENTINEL, "value_env": "COMPAT_TOKEN" },
                        ],
                        "tools": [{ "name": "ping", "path": "/ping" }],
                    },
                }))
                .unwrap(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "add failed: {resp}");

    let (status, detail_body) = send(h.app.clone(), get("/mcp/servers/both")).await;
    assert_eq!(status, StatusCode::OK, "GET failed: {detail_body}");
    assert!(
        !detail_body.contains(HEADER_SENTINEL),
        "the static value is a credential and must stay redacted even when a value_env sits beside it:\n{detail_body}"
    );
    let detail: serde_json::Value = serde_json::from_str(&detail_body).expect("detail is JSON");
    let transport_from_response = detail
        .get("transport")
        .cloned()
        .expect("detail carries a transport object");

    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/both",
            serde_json::json!({
                "name": "both",
                "transport": transport_from_response,
                "timeout_secs": 45,
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "PUT failed: {body}");

    let headers = stored_http_compat_headers(&h.state, "both");
    assert_eq!(headers.len(), 1, "the header must survive: {headers:?}");
    assert_eq!(
        headers[0].value.as_deref(),
        Some(HEADER_SENTINEL),
        "the static value must survive a round-trip that never showed it to the caller — dropping \
         it here silently changes which credential the transport sends: {headers:?}"
    );
    assert_eq!(
        headers[0].value_env.as_deref(),
        Some("COMPAT_TOKEN"),
        "restoring the static value must not disturb the variable name beside it: {headers:?}"
    );
}

/// A submitted `NAME=value` is an explicit change and must win over what is stored, otherwise an operator could never rotate an inline credential.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_explicitly_submitted_value_overrides_the_stored_one() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("rotate")).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/rotate",
            serde_json::json!({
                "name": "rotate",
                "transport": { "type": "stdio", "command": "false", "args": [] },
                "env": [format!("{ENV_NAME}=ghp_rotated_value"), PLAIN_ENV_NAME],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");

    let stored = stored_env(&h.state, "rotate");
    assert!(
        stored.contains(&format!("{ENV_NAME}=ghp_rotated_value")),
        "an explicit new value must replace the stored one. stored: {stored:?}"
    );
    assert!(
        !stored.iter().any(|e| e.contains(SENTINEL)),
        "the old value must be gone after an explicit rotation. stored: {stored:?}"
    );
}

/// A name dropped from the submission is an explicit removal, not something to restore from the stored entry.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn omitting_a_name_removes_it_rather_than_restoring_it() {
    let h = boot();
    let (status, _) = send(h.app.clone(), add_server_with_inline_secret("remove")).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, body) = send(
        h.app.clone(),
        put_json(
            "/mcp/servers/remove",
            serde_json::json!({
                "name": "remove",
                "transport": { "type": "stdio", "command": "false", "args": [] },
                "env": [PLAIN_ENV_NAME],
            }),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "update failed: {body}");

    let stored = stored_env(&h.state, "remove");
    assert!(
        !stored.iter().any(|e| e.starts_with(ENV_NAME)),
        "a name absent from the submission must be removed, not merged back. \
         stored: {stored:?}"
    );
    assert_eq!(stored, vec![PLAIN_ENV_NAME.to_string()]);
}
