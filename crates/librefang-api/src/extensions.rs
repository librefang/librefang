//! Typed wrappers stashed in `Response::extensions` so the HTTP middleware
//! can lift handler-resolved identifiers into structured access-log fields.
//!
//! ## Why response extensions?
//!
//! The access-log middleware in [`crate::middleware::request_logging`] only
//! sees the raw URI path (e.g. `/api/agents/{uuid}/suspend`). To attach a
//! structured `agent_id` or `session_id` field to every log line — without
//! forcing every handler to take a `tracing::Span` argument or duplicating
//! UUID parsing in the middleware itself — handlers that already extract an
//! [`AgentId`] / [`SessionId`] can drop a marker into
//! `response.extensions_mut()`. The middleware reads it back after
//! `next.run().await` and emits the field on the access-log event.
//!
//! Closes #3511 (HTTP access log lacks `agent_id` / `session_id`).

use axum::response::{IntoResponse, Response};
use librefang_types::agent::{AgentId, SessionId};

/// Marker placed in `Response::extensions` by handlers that have resolved
/// an [`AgentId`] from the request path. Read back by the access-log
/// middleware to add `agent_id=<uuid>` to the structured log line.
///
/// Not serialized — extensions are an in-process channel between the
/// handler and middleware layers, never crossing the wire.
#[derive(Debug, Clone)]
pub struct AgentIdField(pub AgentId);

/// Wrap any [`IntoResponse`] body and attach an [`AgentIdField`] marker.
///
/// Handlers that already parse an `AgentId` from the path use this to opt
/// into structured access-log enrichment without rewriting their return
/// types. Example:
///
/// ```ignore
/// pub async fn kill_agent(
///     State(state): State<Arc<AppState>>,
///     Path(id): Path<String>,
/// ) -> impl IntoResponse {
///     let agent_id: AgentId = id.parse()?;
///     // ... handler body ...
///     with_agent_id(
///         agent_id,
///         (StatusCode::OK, Json(json!({"status": "killed"}))),
///     )
/// }
/// ```
pub fn with_agent_id<R: IntoResponse>(agent_id: AgentId, body: R) -> Response {
    let mut response = body.into_response();
    response.extensions_mut().insert(AgentIdField(agent_id));
    response
}

/// Marker placed in `Response::extensions` by handlers that have resolved
/// a [`SessionId`] from the request path or kernel registry. Read back by
/// the access-log middleware to add `session_id=<uuid>` to the structured
/// log line.
///
/// Not serialized — extensions are an in-process channel between the
/// handler and middleware layers, never crossing the wire.
#[derive(Debug, Clone)]
pub struct SessionIdField(pub SessionId);

/// Wrap any [`IntoResponse`] body and attach a [`SessionIdField`] marker.
///
/// Handlers that already have a [`SessionId`] (from the path, registry entry,
/// or kernel return value) use this to opt into structured access-log
/// enrichment. It composes with [`with_agent_id`]: call one first and pass
/// the resulting [`Response`] as the body of the other. Example:
///
/// ```ignore
/// with_session_id(session_id, with_agent_id(agent_id, (StatusCode::OK, Json(payload))))
/// ```
pub fn with_session_id<R: IntoResponse>(session_id: SessionId, body: R) -> Response {
    let mut response = body.into_response();
    response.extensions_mut().insert(SessionIdField(session_id));
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use axum::Json;
    use librefang_types::agent::{AgentId, SessionId};
    use serde_json::json;

    #[test]
    fn with_agent_id_attaches_marker_to_response_extensions() {
        let agent_id = AgentId::new();
        let resp = with_agent_id(agent_id, (StatusCode::OK, Json(json!({"status": "ok"}))));

        let field = resp
            .extensions()
            .get::<AgentIdField>()
            .expect("AgentIdField must be present in response extensions");
        assert_eq!(field.0, agent_id);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn with_agent_id_preserves_status_and_body() {
        let agent_id = AgentId::new();
        let resp = with_agent_id(
            agent_id,
            (StatusCode::NOT_FOUND, Json(json!({"error": "missing"}))),
        );

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(resp.extensions().get::<AgentIdField>().is_some());
    }

    #[test]
    fn with_session_id_attaches_marker_to_response_extensions() {
        let session_id = SessionId::new();
        let resp = with_session_id(session_id, (StatusCode::OK, Json(json!({"status": "ok"}))));

        let field = resp
            .extensions()
            .get::<SessionIdField>()
            .expect("SessionIdField must be present in response extensions");
        assert_eq!(field.0, session_id);
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn with_session_id_preserves_status_code() {
        let session_id = SessionId::new();
        let resp = with_session_id(
            session_id,
            (StatusCode::NOT_FOUND, Json(json!({"error": "missing"}))),
        );

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert!(resp.extensions().get::<SessionIdField>().is_some());
    }

    #[test]
    fn with_agent_id_and_session_id_compose() {
        // Both markers can live on the same response simultaneously.
        let agent_id = AgentId::new();
        let session_id = SessionId::new();
        let resp = with_session_id(
            session_id,
            with_agent_id(agent_id, (StatusCode::OK, Json(json!({})))),
        );

        assert!(resp.extensions().get::<AgentIdField>().is_some());
        assert!(resp.extensions().get::<SessionIdField>().is_some());
    }
}
