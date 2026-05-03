//! Axum extractors shared by the route handlers.
//!
//! At the moment this module only hosts the [`AgentIdPath`] path extractor,
//! which collapses the parsing boilerplate that was duplicated across many
//! handlers (see #3603). Add new extractors here whenever a parsing pattern
//! starts to repeat between route modules.

use axum::extract::{FromRequestParts, Path};
use axum::http::request::Parts;
use librefang_types::agent::AgentId;
use librefang_types::i18n::{self, ErrorTranslator};

use crate::middleware::RequestLanguage;
use crate::types::ApiErrorResponse;

/// Translation key used by handlers for "invalid agent id". Kept as a constant
/// so the extractor and any remaining hand-rolled parse sites agree on the key.
const INVALID_AGENT_ID_KEY: &str = "api-error-agent-invalid-id";

/// Build the localized "invalid agent id" message using the resolved request
/// language, falling back to the English bundle when the
/// [`RequestLanguage`] extension is missing (e.g. in tests that bypass the
/// `accept_language` middleware).
fn invalid_agent_id_message(parts: &Parts) -> String {
    let lang = parts
        .extensions
        .get::<RequestLanguage>()
        .map(|rl| rl.0)
        .unwrap_or(i18n::DEFAULT_LANGUAGE);
    ErrorTranslator::new(lang).t(INVALID_AGENT_ID_KEY)
}

/// Newtype extractor that parses an [`AgentId`] from a single-segment path
/// parameter.
///
/// Lets handlers replace the repeated
///
/// ```ignore
/// let agent_id: AgentId = match id.parse() {
///     Ok(aid) => aid,
///     Err(_) => {
///         return ApiErrorResponse::bad_request(t.t("api-error-agent-invalid-id"))
///             .into_json_tuple();
///     }
/// };
/// ```
///
/// boilerplate with a direct `AgentIdPath(agent_id): AgentIdPath` argument.
///
/// On a malformed path segment, returns a 400 [`ApiErrorResponse`] whose
/// message is translated via the request's `Accept-Language` header just like
/// the handlers used to do explicitly.
///
/// `Path` cannot be implemented for foreign types and the route modules also
/// use `Path<(String, String)>` tuples (e.g. `/agents/:id/kv/:key`), so a
/// newtype wrapper is preferable to overriding `Path<AgentId>` itself — it
/// keeps the existing tuple extractors working without coercion.
#[derive(Debug, Clone, Copy)]
pub struct AgentIdPath(pub AgentId);

impl AgentIdPath {
    /// Consume the wrapper and return the inner [`AgentId`].
    #[allow(dead_code)]
    pub fn into_inner(self) -> AgentId {
        self.0
    }
}

impl std::ops::Deref for AgentIdPath {
    type Target = AgentId;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> FromRequestParts<S> for AgentIdPath
where
    S: Send + Sync,
{
    type Rejection = ApiErrorResponse;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| ApiErrorResponse::bad_request(invalid_agent_id_message(parts)))?;
        let id: AgentId = raw
            .parse()
            .map_err(|_| ApiErrorResponse::bad_request(invalid_agent_id_message(parts)))?;
        Ok(AgentIdPath(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::get;
    use axum::Router;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn echo(AgentIdPath(id): AgentIdPath) -> String {
        id.to_string()
    }

    fn app() -> Router {
        Router::new().route("/agents/{id}", get(echo))
    }

    #[tokio::test]
    async fn extracts_valid_agent_id() {
        let aid = AgentId::new();
        let uri = format!("/agents/{aid}");
        let response = app()
            .oneshot(Request::builder().uri(&uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], aid.to_string().as_bytes());
    }

    #[tokio::test]
    async fn rejects_invalid_agent_id_with_400_and_localized_message() {
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/agents/not-a-uuid")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        // Without the `accept_language` middleware the extractor falls back to
        // the English bundle, which is what `ErrorTranslator::new("en")` returns
        // for the well-known key. The test asserts on that exact text so a
        // translation typo would surface here.
        let expected = ErrorTranslator::new("en").t(INVALID_AGENT_ID_KEY);
        assert_eq!(body["error"].as_str(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn into_response_has_400_status() {
        // Sanity-check that the rejection type renders as 400 outside of the
        // routing layer, since `ApiErrorResponse::bad_request` is used in many
        // other places too.
        let resp = ApiErrorResponse::bad_request("invalid agent id").into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
