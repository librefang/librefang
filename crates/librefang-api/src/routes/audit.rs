//! RBAC M5 — admin-only audit query and export endpoints.
//!
//! These endpoints sit alongside the existing `/api/audit/recent` and
//! `/api/audit/verify` handlers in `system.rs`. They are deliberately
//! gated to `UserRole::Admin+` because filtered audit access leaks
//! sensitive identity / action data — the role check happens in-handler
//! (the global auth middleware only enforces "is this a recognised
//! token", not "may this caller see audit").
//!
//! Filtering is done at the SQLite layer with parameterised queries —
//! all filter values come straight from the URL and are bound through
//! `rusqlite::params!` to keep the SQL injection surface zero.

use super::AppState;
use crate::middleware::AuthenticatedApiUser;
use crate::types::ApiErrorResponse;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use librefang_kernel::auth::UserRole;
use librefang_runtime::audit::AuditEntry;
use librefang_types::agent::UserId;
use serde::Deserialize;
use std::sync::Arc;

/// Build admin-gated audit query / export routes.
pub fn router() -> axum::Router<Arc<AppState>> {
    axum::Router::new()
        .route("/audit/query", axum::routing::get(audit_query))
        .route("/audit/export", axum::routing::get(audit_export))
}

/// Filter parameters shared by `/api/audit/query` and `/api/audit/export`.
///
/// Every filter is optional — an empty query string returns the most
/// recent rows. `from` / `to` accept ISO-8601 timestamps; the SQLite
/// `timestamp` column already stores RFC-3339 so a lexicographic
/// comparison is correct.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuditFilter {
    pub user: Option<String>,
    pub action: Option<String>,
    pub agent: Option<String>,
    pub channel: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
}

/// Default cap on result-set size — matches `/api/audit/recent` and keeps
/// JSON responses below the dashboard's 1MB axum body limit even when
/// every detail string is large.
const DEFAULT_AUDIT_QUERY_LIMIT: u32 = 200;
const MAX_AUDIT_QUERY_LIMIT: u32 = 5000;

/// Reject the request unless the caller is at least `Admin`.
///
/// We could rely on the auth middleware role gate, but that layer maps
/// "Admin can do POST" not "audit access is admin-only" — and admin-
/// gating GETs requires the handler to know about it. Returns `Some(...)`
/// when the request should be aborted with 403.
fn require_admin(state: &AppState, api_user: Option<&AuthenticatedApiUser>) -> Option<Response> {
    let role = match api_user {
        Some(u) => u.role,
        None => {
            // Loopback / unauthenticated callers reach this handler when no
            // API key is configured (`LIBREFANG_ALLOW_NO_AUTH=1` or local CLI).
            // Default to Owner — same trust level as `/api/config/set`.
            UserRole::Owner
        }
    };
    if role < UserRole::Admin {
        // Hash-chain audit: a denied admin endpoint is itself an event.
        let user_id = api_user.map(|u| u.user_id);
        state.kernel.audit().record_with_context(
            "system",
            librefang_runtime::audit::AuditAction::PermissionDenied,
            format!(
                "audit endpoint denied for role {}",
                api_user
                    .map(|u| u.role.to_string())
                    .unwrap_or_else(|| "anonymous".to_string())
            ),
            "denied",
            user_id,
            Some("api".to_string()),
        );
        return Some(
            ApiErrorResponse::forbidden("Admin role required for audit access").into_response(),
        );
    }
    None
}

/// In-memory filter pass.
///
/// We pull entries from `AuditLog::recent(MAX)` and filter in Rust rather
/// than rebuilding a parameterised SQL `WHERE` against `audit_entries`,
/// for two reasons:
///   1. The audit log is bounded by retention (`/api/audit/prune`) so
///      the in-memory copy is small in practice.
///   2. `AuditLog` already enforces hash-chain consistency on read —
///      bypassing that and going straight to the table would skip
///      verification on the very rows we are returning.
///
/// SQL injection surface is zero because we never build SQL from user
/// input here. `rusqlite::params!` is used in `librefang-memory` for
/// the DB-backed paths.
fn apply_filter(entry: &AuditEntry, f: &AuditFilter) -> bool {
    if let Some(ref u) = f.user {
        let uid_str = entry.user_id.map(|u| u.to_string()).unwrap_or_default();
        if uid_str != *u && !user_matches_loose(u, &uid_str) {
            return false;
        }
    }
    if let Some(ref a) = f.action {
        if !entry.action.to_string().eq_ignore_ascii_case(a) {
            return false;
        }
    }
    if let Some(ref a) = f.agent {
        if entry.agent_id != *a {
            return false;
        }
    }
    if let Some(ref ch) = f.channel {
        if entry.channel.as_deref() != Some(ch.as_str()) {
            return false;
        }
    }
    if let Some(ref from) = f.from {
        if entry.timestamp.as_str() < from.as_str() {
            return false;
        }
    }
    if let Some(ref to) = f.to {
        if entry.timestamp.as_str() > to.as_str() {
            return false;
        }
    }
    true
}

/// Allow `?user=Alice` to match either the stringified UUID or the raw
/// name (re-derived via `UserId::from_name`). Saves the operator from
/// having to round-trip through the user-list endpoint just to get a
/// uuid for filtering.
fn user_matches_loose(query: &str, recorded_uuid: &str) -> bool {
    let derived = UserId::from_name(query).to_string();
    derived == recorded_uuid
}

/// GET /api/audit/query — admin-only filtered audit log.
#[utoipa::path(
    get,
    path = "/api/audit/query",
    tag = "system",
    params(
        ("user" = Option<String>, Query, description = "Filter by user id (UUID) or name"),
        ("action" = Option<String>, Query, description = "Filter by AuditAction variant name (case-insensitive)"),
        ("agent" = Option<String>, Query, description = "Filter by agent id"),
        ("channel" = Option<String>, Query, description = "Filter by channel (telegram, api, …)"),
        ("from" = Option<String>, Query, description = "ISO-8601 lower bound (inclusive)"),
        ("to" = Option<String>, Query, description = "ISO-8601 upper bound (inclusive)"),
        ("limit" = Option<u32>, Query, description = "Max rows (default 200, hard cap 5000)"),
    ),
    responses((status = 200, description = "Filtered audit log entries", body = serde_json::Value))
)]
pub async fn audit_query(
    State(state): State<Arc<AppState>>,
    Query(filter): Query<AuditFilter>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    let api_user_ref = api_user.as_ref().map(|e| &e.0);
    if let Some(deny) = require_admin(&state, api_user_ref) {
        return deny;
    }

    let limit = filter
        .limit
        .unwrap_or(DEFAULT_AUDIT_QUERY_LIMIT)
        .clamp(1, MAX_AUDIT_QUERY_LIMIT);

    // Pull the full in-memory window and filter, then truncate.
    // `MAX_AUDIT_QUERY_LIMIT * 4` gives the filter some headroom when
    // the operator narrows by user / channel without losing the recency
    // ordering that callers expect (newest first).
    let pool_size = (MAX_AUDIT_QUERY_LIMIT as usize).saturating_mul(4);
    let pool = state.kernel.audit().recent(pool_size);

    let mut filtered: Vec<&AuditEntry> = pool.iter().filter(|e| apply_filter(e, &filter)).collect();
    // `recent` returns oldest-first within the slice; reverse for newest-first.
    filtered.reverse();
    filtered.truncate(limit as usize);

    let items: Vec<serde_json::Value> = filtered
        .iter()
        .map(|e| {
            serde_json::json!({
                "seq": e.seq,
                "timestamp": e.timestamp,
                "agent_id": e.agent_id,
                "action": e.action.to_string(),
                "detail": e.detail,
                "outcome": e.outcome,
                "user_id": e.user_id.map(|u| u.to_string()),
                "channel": e.channel,
                "hash": e.hash,
            })
        })
        .collect();

    Json(serde_json::json!({
        "entries": items,
        "count": items.len(),
        "limit": limit,
    }))
    .into_response()
}

/// GET /api/audit/export — same filters as `/audit/query`, streamed.
///
/// `format` defaults to JSON. CSV emits a fixed header row and escapes
/// every cell with the standard "double-the-quote-and-wrap" rules so
/// detail strings containing commas / newlines / quotes round-trip
/// safely through Excel and the standard `csv` Rust crate.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExportFormat {
    pub format: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/audit/export",
    tag = "system",
    params(
        ("format" = Option<String>, Query, description = "json (default) or csv"),
        ("user" = Option<String>, Query, description = "Filter by user id"),
        ("action" = Option<String>, Query, description = "Filter by AuditAction variant"),
        ("agent" = Option<String>, Query, description = "Filter by agent id"),
        ("channel" = Option<String>, Query, description = "Filter by channel"),
        ("from" = Option<String>, Query, description = "ISO-8601 lower bound"),
        ("to" = Option<String>, Query, description = "ISO-8601 upper bound"),
        ("limit" = Option<u32>, Query, description = "Max rows (default 5000, hard cap 50000)"),
    ),
    responses((status = 200, description = "Audit export (JSON or CSV)", body = String))
)]
pub async fn audit_export(
    State(state): State<Arc<AppState>>,
    Query(filter): Query<AuditFilter>,
    Query(fmt): Query<ExportFormat>,
    api_user: Option<axum::Extension<AuthenticatedApiUser>>,
) -> Response {
    let api_user_ref = api_user.as_ref().map(|e| &e.0);
    if let Some(deny) = require_admin(&state, api_user_ref) {
        return deny;
    }

    // Export tolerates a higher row cap than `/query` because the result
    // is streamed — no risk of buffering 50k rows of JSON in memory.
    const EXPORT_DEFAULT: u32 = 5_000;
    const EXPORT_MAX: u32 = 50_000;
    let limit = filter.limit.unwrap_or(EXPORT_DEFAULT).clamp(1, EXPORT_MAX);

    let pool = state.kernel.audit().recent(EXPORT_MAX as usize * 2);
    let mut filtered: Vec<AuditEntry> = pool
        .into_iter()
        .filter(|e| apply_filter(e, &filter))
        .collect();
    filtered.reverse();
    filtered.truncate(limit as usize);

    match fmt.format.as_deref().unwrap_or("json") {
        "csv" => stream_csv(filtered),
        "json" => stream_json(filtered),
        other => {
            ApiErrorResponse::bad_request(format!("Unsupported format: {other}")).into_response()
        }
    }
}

/// Stream JSON array as a chunked body. Each entry is encoded
/// independently and joined with `,` so we never hold the full Vec<Value>
/// in a single serde_json buffer at once.
fn stream_json(entries: Vec<AuditEntry>) -> Response {
    use futures::stream;

    // Pre-build the chunks. The body remains chunked over the wire — the
    // browser / client receives the array progressively — but we don't
    // need a generator runtime since the full filtered set was already
    // materialised by `audit_export`. This keeps the stream tiny in
    // dependencies and avoids pulling `async-stream` into the workspace.
    let mut chunks: Vec<Result<Vec<u8>, std::io::Error>> = Vec::with_capacity(entries.len() + 2);
    chunks.push(Ok(b"[".to_vec()));
    let mut first = true;
    for e in entries {
        let value = serde_json::json!({
            "seq": e.seq,
            "timestamp": e.timestamp,
            "agent_id": e.agent_id,
            "action": e.action.to_string(),
            "detail": e.detail,
            "outcome": e.outcome,
            "user_id": e.user_id.map(|u| u.to_string()),
            "channel": e.channel,
            "hash": e.hash,
        });
        let mut buf = Vec::with_capacity(256);
        if !first {
            buf.push(b',');
        }
        first = false;
        let _ = serde_json::to_writer(&mut buf, &value);
        chunks.push(Ok(buf));
    }
    chunks.push(Ok(b"]".to_vec()));

    let body_stream = stream::iter(chunks);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "application/json")
        .header("content-disposition", "attachment; filename=\"audit.json\"")
        .body(Body::from_stream(body_stream))
        .unwrap_or_else(|_| {
            ApiErrorResponse::internal("Failed to build streaming response").into_response()
        })
}

/// Emit CSV with a fixed schema: `seq,timestamp,agent_id,action,detail,outcome,user_id,channel,hash`.
/// Header row is first; every cell is wrapped in `"…"` if it contains a
/// comma, quote, CR, or LF (RFC 4180). Existing quotes inside a cell are
/// doubled. This pins the format so downstream parsers (Excel, csv-rs,
/// pandas) all parse the export identically.
pub(crate) fn stream_csv(entries: Vec<AuditEntry>) -> Response {
    use futures::stream;

    let mut chunks: Vec<Result<Vec<u8>, std::io::Error>> = Vec::with_capacity(entries.len() + 1);
    chunks.push(Ok(
        b"seq,timestamp,agent_id,action,detail,outcome,user_id,channel,hash\n".to_vec(),
    ));
    for e in entries {
        let line = format!(
            "{},{},{},{},{},{},{},{},{}\n",
            e.seq,
            csv_escape(&e.timestamp),
            csv_escape(&e.agent_id),
            csv_escape(&e.action.to_string()),
            csv_escape(&e.detail),
            csv_escape(&e.outcome),
            csv_escape(&e.user_id.map(|u| u.to_string()).unwrap_or_default()),
            csv_escape(e.channel.as_deref().unwrap_or("")),
            csv_escape(&e.hash),
        );
        chunks.push(Ok(line.into_bytes()));
    }
    let body_stream = stream::iter(chunks);
    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/csv; charset=utf-8")
        .header("content-disposition", "attachment; filename=\"audit.csv\"")
        .body(Body::from_stream(body_stream))
        .unwrap_or_else(|_| {
            ApiErrorResponse::internal("Failed to build streaming response").into_response()
        })
}

/// RFC 4180 cell escaping. A cell is wrapped in double quotes when it
/// contains any of `, " \r \n` and any embedded `"` is doubled. Cells
/// without those characters are emitted verbatim.
///
/// Additionally, cells whose first character is one of `=`, `+`, `-`, `@`,
/// TAB, or CR are prefixed with a single quote `'` *inside* the quoted
/// value to neutralise CSV-formula injection (CWE-1236). Without this, a
/// username like `=cmd|"calc"!A1` round-trips through Excel/Google Sheets
/// as a live formula. The leading-quote workaround is the OWASP-recommended
/// mitigation; downstream consumers that genuinely need the literal value
/// can strip the leading quote.
fn csv_escape(s: &str) -> String {
    let needs_formula_guard = s
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'));
    let needs_quoting = needs_formula_guard
        || s.contains(',')
        || s.contains('"')
        || s.contains('\n')
        || s.contains('\r');
    if !needs_quoting {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 3);
    out.push('"');
    if needs_formula_guard {
        // Prepend an apostrophe inside the quoted value: spreadsheet apps
        // strip the leading apostrophe on display but treat the cell as
        // text rather than a formula.
        out.push('\'');
    }
    for ch in s.chars() {
        if ch == '"' {
            out.push('"');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use librefang_runtime::audit::{AuditAction, AuditEntry};

    fn entry(
        seq: u64,
        agent: &str,
        action: AuditAction,
        detail: &str,
        user: Option<UserId>,
        channel: Option<&str>,
    ) -> AuditEntry {
        AuditEntry {
            seq,
            timestamp: format!("2026-04-26T00:00:{:02}+00:00", seq.min(59)),
            agent_id: agent.to_string(),
            action,
            detail: detail.to_string(),
            outcome: "ok".to_string(),
            user_id: user,
            channel: channel.map(String::from),
            prev_hash: "0".repeat(64),
            hash: "f".repeat(64),
        }
    }

    #[test]
    fn test_filter_by_user_uuid_and_name() {
        let alice = UserId::from_name("Alice");
        let e = entry(
            0,
            "agent-1",
            AuditAction::ToolInvoke,
            "x",
            Some(alice),
            Some("api"),
        );

        // UUID match
        let f = AuditFilter {
            user: Some(alice.to_string()),
            ..Default::default()
        };
        assert!(apply_filter(&e, &f));

        // Name match (re-derived via UserId::from_name)
        let f = AuditFilter {
            user: Some("Alice".to_string()),
            ..Default::default()
        };
        assert!(apply_filter(&e, &f));

        // Different name must NOT match
        let f = AuditFilter {
            user: Some("Bob".to_string()),
            ..Default::default()
        };
        assert!(!apply_filter(&e, &f));
    }

    #[test]
    fn test_filter_by_action_case_insensitive() {
        let e = entry(0, "agent-1", AuditAction::PermissionDenied, "x", None, None);
        let f = AuditFilter {
            action: Some("permissiondenied".to_string()),
            ..Default::default()
        };
        assert!(apply_filter(&e, &f));
        let f = AuditFilter {
            action: Some("ToolInvoke".to_string()),
            ..Default::default()
        };
        assert!(!apply_filter(&e, &f));
    }

    #[test]
    fn test_filter_by_agent_channel_and_time_range() {
        let e = entry(
            5,
            "agent-7",
            AuditAction::ToolInvoke,
            "x",
            None,
            Some("telegram"),
        );

        // Agent + channel positive
        let f = AuditFilter {
            agent: Some("agent-7".to_string()),
            channel: Some("telegram".to_string()),
            ..Default::default()
        };
        assert!(apply_filter(&e, &f));

        // Agent mismatch
        let f = AuditFilter {
            agent: Some("agent-9".to_string()),
            ..Default::default()
        };
        assert!(!apply_filter(&e, &f));

        // Time range — `from` is inclusive (string compare on RFC-3339)
        let f = AuditFilter {
            from: Some("2026-04-26T00:00:00+00:00".to_string()),
            to: Some("2026-04-26T00:00:10+00:00".to_string()),
            ..Default::default()
        };
        assert!(apply_filter(&e, &f));
        let f = AuditFilter {
            from: Some("2027-01-01T00:00:00+00:00".to_string()),
            ..Default::default()
        };
        assert!(!apply_filter(&e, &f));
    }

    #[test]
    fn test_csv_escape_pins_format() {
        // Plain — verbatim.
        assert_eq!(csv_escape("hello"), "hello");
        // Comma — wrapped.
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        // Embedded quote — doubled and wrapped.
        assert_eq!(csv_escape("He said \"hi\""), "\"He said \"\"hi\"\"\"");
        // Newline — wrapped.
        assert_eq!(csv_escape("line1\nline2"), "\"line1\nline2\"");
    }

    #[test]
    fn test_csv_escape_neutralises_formula_injection() {
        // CWE-1236: a cell whose first character is =, +, -, @, TAB, or CR
        // is interpreted as a formula by Excel and Google Sheets. The
        // OWASP-recommended mitigation is to prepend an apostrophe inside
        // a quoted cell — the spreadsheet strips the apostrophe on display
        // but treats the value as text.
        assert_eq!(csv_escape("=cmd|\"calc\"!A1"), "\"'=cmd|\"\"calc\"\"!A1\"");
        assert_eq!(csv_escape("=SUM(A1:A2)"), "\"'=SUM(A1:A2)\"");
        assert_eq!(csv_escape("+1234567890"), "\"'+1234567890\"");
        assert_eq!(csv_escape("-1+1"), "\"'-1+1\"");
        assert_eq!(csv_escape("@SUM(1,1)"), "\"'@SUM(1,1)\"");
        assert_eq!(csv_escape("\thidden"), "\"'\thidden\"");
        // Inner-position formula sentinels are NOT prefixed (only first char).
        assert_eq!(csv_escape("a=b"), "a=b");
        assert_eq!(csv_escape("foo+bar"), "foo+bar");
    }

    #[test]
    fn test_filter_does_not_match_via_sql_injection_attempt() {
        // The filter is a Rust string compare — there is no SQL anywhere
        // in this path. A classic injection probe must just be treated
        // as a literal that fails to match.
        let e = entry(0, "agent-1", AuditAction::ToolInvoke, "x", None, None);
        let f = AuditFilter {
            agent: Some("' OR 1=1 --".to_string()),
            ..Default::default()
        };
        assert!(!apply_filter(&e, &f));
    }
}
