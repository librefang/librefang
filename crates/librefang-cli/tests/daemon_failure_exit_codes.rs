//! Exit-code contract for CLI commands whose daemon call fails.
//!
//! Two related holes, both of which made a failed command indistinguishable from a successful one for anything reading `$?`:
//!
//! - `librefang health` printed `Daemon is healthy` and exited 0 whenever the daemon answered at all, because `/api/health` is a liveness probe that deliberately returns HTTP 200 with `status = "degraded"` when a subsystem check fails.
//!   The human output was self-contradicting (a green check directly above `Status: degraded`) and `--json` never inspected the payload at all.
//! - `daemon_json_checked` calls `std::process::exit` only for a transport error; on an HTTP error status it returns to the caller, and the mutating handlers printed their message and fell off the end of the function, leaving the exit code at 0.
//!
//! Both fixes are `std::process::exit` calls, so they are only observable from outside the process — hence driving the compiled binary rather than calling the handlers.
//! The daemon is a stub `TcpListener` speaking just enough HTTP/1.1 to answer the requests each command makes (the `/api/health` probe in `find_daemon`, then the command's own calls), so no real daemon, database or port 4545 is involved.
//!
//! Every command is covered in **both** directions, and the success direction is the load-bearing half.
//! These handlers do not share one success predicate: `group create` reads the HTTP status, `webhooks create` reads a payload field that must be present, `webhooks test` reads a payload field that must hold a particular value, and `models set` reads a payload field that must be absent.
//! A failure-only test passes just as happily against a predicate that can never be true, which is exactly how an `exit(1)` guarded by a nonexistent `success` field turned every delivered webhook into a non-zero exit.
//! Only an exit-0 assertion against the payload the daemon really sends pins that.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Command, Output};
use std::time::Duration;

const BIN: &str = env!("CARGO_BIN_EXE_librefang");

/// Stub routes, keyed by `"METHOD /path"`, valued by `(status line, JSON body)`.
type Routes = HashMap<&'static str, (&'static str, &'static str)>;

/// A healthy liveness payload, as `routes/config/system.rs::health` builds it.
const HEALTH_OK: &str = r#"{"status":"ok","version":"0.0.0-test","checks":[{"name":"database","status":"ok"},{"name":"embedding","status":"ok"}]}"#;

/// The same endpoint with the database probe failing: still HTTP 200, because `/api/health` reports liveness and `/api/ready` is the readiness probe that answers 503.
const HEALTH_DEGRADED: &str = r#"{"status":"degraded","version":"0.0.0-test","checks":[{"name":"database","status":"error"},{"name":"embedding","status":"warn"}]}"#;

/// `POST /api/groups` on a duplicate name, in the daemon's real `ApiErrorResponse` shape — `error` is a nested object, not a bare string.
const GROUP_CONFLICT: &str = r#"{"error":{"code":"conflict","message":"group 'dup' already exists"},"message":"group 'dup' already exists"}"#;

/// `GET /api/agents` with nothing registered, which is all `resolve_agent_id` needs — an unmatched name falls through unchanged.
const AGENTS_EMPTY: &str = r#"{"items":[]}"#;

/// The webhook id `webhooks test` is driven with; the daemon parses this path segment as a `Uuid`, so it has to be well-formed.
///
/// The stub's route keys are `&'static str`, so the same value is spelled out again in each `POST /api/webhooks/.../test` key below.
const WEBHOOK_ID: &str = "11111111-1111-4111-8111-111111111111";

/// `POST /api/webhooks` accepting the create, as `create_webhook_inner` serialises a `WebhookSubscription` — the CLI's success predicate is the presence of `id`, and `WebhookId` is a newtype over `Uuid` so it lands as a bare string.
const WEBHOOK_CREATED: &str = r#"{"id":"11111111-1111-4111-8111-111111111111","name":"coder-example.com","url":"https://example.com/hook","events":["all"],"enabled":true,"created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}"#;

/// The same route rejecting the create, in the flat shape `create_webhook_inner` builds for an invalid request.
const WEBHOOK_CREATE_REJECTED: &str =
    r#"{"error":"url is not allowed","code":"invalid_request","type":"invalid_request"}"#;

/// `POST /api/webhooks/{id}/test` after the payload reached the endpoint, exactly as `test_webhook` builds it.
///
/// There is deliberately no `success` field here, because the route has never emitted one; the CLI must read `status` instead.
const WEBHOOK_TEST_SENT: &str = r#"{"status":"sent","response_status":200,"webhook_id":"11111111-1111-4111-8111-111111111111"}"#;

/// The same route when the endpoint could not be reached — HTTP 502, and `status` is `"error"` rather than `"sent"`.
const WEBHOOK_TEST_FAILED: &str = r#"{"status":"error","error":"connection refused","webhook_id":"11111111-1111-4111-8111-111111111111"}"#;

/// `POST /api/config/set` accepting the write; the CLI treats the *absence* of `error` as success, and a partial reload adds `reload_error`, never `error`.
const CONFIG_SET_OK: &str = r#"{"status":"reloaded","path":"default_model.model"}"#;

/// The same route rejecting the write, with the `error` field the CLI keys on.
const CONFIG_SET_REJECTED: &str = r#"{"error":"path 'default_model.model' is not writable"}"#;

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Read one HTTP request in full — headers plus any `Content-Length` body.
///
/// The body has to be drained before responding, or the client is left writing into a socket we already closed and reports a transport error instead of reading our status line.
fn read_request(stream: &mut TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    // The stub serves connections one at a time, so a socket that opens without ever sending a request would park this thread forever and hang the whole test rather than failing it.
    // The timeout turns that into an `Err` from `read`, which falls through to the unmatched-route 404 and lets the test fail on its exit-code assertion instead.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(_) => break,
        }
        if let Some(end) = find_subslice(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]);
            let content_length = head
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|line| line.split(':').nth(1))
                .and_then(|value| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// `"METHOD /path"` for the request line, with any query string dropped.
fn route_key(request: &str) -> String {
    let Some(line) = request.lines().next() else {
        return String::new();
    };
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    let path = target.split('?').next().unwrap_or(target);
    format!("{method} {path}")
}

/// Bind an ephemeral port and serve `routes` until the test process exits.
///
/// The serving thread is deliberately detached: every test binds its own port, and the listener dies with the process.
fn spawn_stub_daemon(routes: Routes) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub daemon");
    let addr = listener.local_addr().expect("stub daemon address");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let key = route_key(&read_request(&mut stream));
            let (status, body) = routes
                .get(key.as_str())
                .copied()
                .unwrap_or(("404 Not Found", "{}"));
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.flush();
        }
    });
    addr
}

/// A `LIBREFANG_HOME` holding only a `daemon.json` that points at `addr`, which is all `find_daemon` reads before probing.
fn stub_home(addr: SocketAddr) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("daemon.json"),
        format!(
            r#"{{"pid":1,"listen_addr":"{addr}","started_at":"2026-01-01T00:00:00Z","version":"0.0.0-test","platform":"test"}}"#
        ),
    )
    .expect("write daemon.json");
    dir
}

fn run_cli(home: &std::path::Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env("LIBREFANG_HOME", home)
        // Keep the invocation on the stub: an inherited config path or proxy would send the request somewhere else entirely.
        .env_remove("LIBREFANG_CONFIG_PATH")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("HTTPS_PROXY")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .env("NO_PROXY", "127.0.0.1,localhost")
        .output()
        .expect("run librefang")
}

fn routes(entries: &[(&'static str, (&'static str, &'static str))]) -> Routes {
    entries.iter().copied().collect()
}

#[test]
fn health_exits_non_zero_when_the_daemon_reports_degraded() {
    let addr = spawn_stub_daemon(routes(&[("GET /api/health", ("200 OK", HEALTH_DEGRADED))]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["health"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // Before the fix the command printed the green "Daemon is healthy" line and returned normally, so this was `Some(0)`.
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("degraded"),
        "the degraded status must still be shown; stdout: {stdout}"
    );
    assert!(
        stdout.contains("database"),
        "the failing subsystem must be named; stdout: {stdout}"
    );
}

#[test]
fn health_json_exits_non_zero_when_the_daemon_reports_degraded() {
    let addr = spawn_stub_daemon(routes(&[("GET /api/health", ("200 OK", HEALTH_DEGRADED))]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["health", "--json"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // The `--json` arm used to print the payload and return before looking at it at all, which is the form a monitor is most likely to call.
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("\"degraded\""),
        "the payload must still be printed verbatim; stdout: {stdout}"
    );
}

#[test]
fn health_exits_zero_when_the_daemon_reports_ok() {
    let addr = spawn_stub_daemon(routes(&[("GET /api/health", ("200 OK", HEALTH_OK))]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["health"]);
    assert!(
        out.status.success(),
        "a healthy daemon must still exit 0; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn group_create_exits_non_zero_when_the_daemon_rejects_it() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        ("POST /api/groups", ("409 Conflict", GROUP_CONFLICT)),
    ]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["group", "create", "dup"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    // `ui::error` is a `println!`, so before the fix `librefang group create dup >/dev/null || fail` saw a clean success for a group that was never created.
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn group_create_exits_zero_when_the_daemon_accepts_it() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        (
            "POST /api/groups",
            ("200 OK", r#"{"name":"oncall","members":[],"roles":[]}"#),
        ),
    ]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["group", "create", "oncall"]);
    assert!(
        out.status.success(),
        "an accepted create must still exit 0; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn webhooks_test_exits_zero_when_the_daemon_reports_the_payload_was_sent() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        (
            "POST /api/webhooks/11111111-1111-4111-8111-111111111111/test",
            ("200 OK", WEBHOOK_TEST_SENT),
        ),
    ]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["webhooks", "test", WEBHOOK_ID]);

    // The regression this pins: the command gated success on a `success` field the route never emits, so adding `exit(1)` to the failure branch made a perfectly delivered webhook exit 1.
    assert!(
        out.status.success(),
        "a delivered webhook must exit 0; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn webhooks_test_exits_non_zero_when_the_delivery_fails() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        (
            "POST /api/webhooks/11111111-1111-4111-8111-111111111111/test",
            ("502 Bad Gateway", WEBHOOK_TEST_FAILED),
        ),
    ]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["webhooks", "test", WEBHOOK_ID]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn webhooks_create_exits_zero_when_the_daemon_accepts_it() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        ("GET /api/agents", ("200 OK", AGENTS_EMPTY)),
        ("POST /api/webhooks", ("201 Created", WEBHOOK_CREATED)),
    ]));
    let home = stub_home(addr);

    let out = run_cli(
        home.path(),
        &["webhooks", "create", "coder", "https://example.com/hook"],
    );

    assert!(
        out.status.success(),
        "an accepted create must exit 0; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn webhooks_create_exits_non_zero_when_the_daemon_rejects_it() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        ("GET /api/agents", ("200 OK", AGENTS_EMPTY)),
        (
            "POST /api/webhooks",
            ("400 Bad Request", WEBHOOK_CREATE_REJECTED),
        ),
    ]));
    let home = stub_home(addr);

    let out = run_cli(
        home.path(),
        &["webhooks", "create", "coder", "https://example.com/hook"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn models_set_exits_zero_when_the_daemon_accepts_the_write() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        ("POST /api/config/set", ("200 OK", CONFIG_SET_OK)),
    ]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["models", "set", "gpt-4o"]);

    assert!(
        out.status.success(),
        "an accepted write must exit 0; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn models_set_exits_non_zero_when_the_daemon_rejects_the_write() {
    let addr = spawn_stub_daemon(routes(&[
        ("GET /api/health", ("200 OK", HEALTH_OK)),
        (
            "POST /api/config/set",
            ("400 Bad Request", CONFIG_SET_REJECTED),
        ),
    ]));
    let home = stub_home(addr);

    let out = run_cli(home.path(), &["models", "set", "gpt-4o"]);
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
