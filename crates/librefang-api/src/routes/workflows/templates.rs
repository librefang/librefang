use super::*;

// ---------------------------------------------------------------------------
// Workflow template routes
// ---------------------------------------------------------------------------
/// Query parameters for listing workflow templates.
#[derive(Debug, Deserialize)]
pub struct TemplateListParams {
    /// Free-text search across name, description, and tags.
    pub q: Option<String>,
    /// Filter by category (exact match).
    pub category: Option<String>,
}

/// GET /api/workflow-templates — List all workflow templates with optional search/filter.
#[utoipa::path(
    get,
    path = "/api/workflow-templates",
    tag = "workflows",
    params(
        ("q" = Option<String>, Query, description = "Search name, description, tags"),
        ("category" = Option<String>, Query, description = "Filter by category"),
    ),
    responses(
        (status = 200, description = "List of workflow templates", body = Vec<serde_json::Value>)
    )
)]
pub async fn list_workflow_templates(
    State(state): State<Arc<AppState>>,
    Query(params): Query<TemplateListParams>,
) -> impl IntoResponse {
    let all = state.kernel.templates().list().await;

    let filtered: Vec<_> = all
        .into_iter()
        .filter(|t| {
            // Category filter (exact match).
            if let Some(ref cat) = params.category {
                match &t.category {
                    Some(tc) if tc == cat => {}
                    _ => return false,
                }
            }
            // Free-text search across name, description, tags.
            if let Some(ref q) = params.q {
                let q_lower = q.to_lowercase();
                let matches_name = t.name.to_lowercase().contains(&q_lower);
                let matches_desc = t.description.to_lowercase().contains(&q_lower);
                let matches_tags = t
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&q_lower));
                if !matches_name && !matches_desc && !matches_tags {
                    return false;
                }
            }
            true
        })
        .collect();

    let list: Vec<serde_json::Value> = filtered
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();

    Json(serde_json::json!({ "templates": list }))
}

/// GET /api/workflow-templates/:id — Get full template details.
#[utoipa::path(
    get,
    path = "/api/workflow-templates/{id}",
    tag = "workflows",
    params(("id" = String, Path, description = "Template ID")),
    responses(
        (status = 200, description = "Template details", body = crate::types::JsonObject),
        (status = 404, description = "Template not found")
    )
)]
pub async fn get_workflow_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.kernel.templates().get(&id).await {
        Some(t) => (
            StatusCode::OK,
            Json(serde_json::to_value(&t).unwrap_or_default()),
        ),
        None => {
            ApiErrorResponse::not_found(format!("Template '{}' not found", id)).into_json_tuple()
        }
    }
}

/// POST /api/workflow-templates/:id/instantiate — Create a live workflow from a template.
#[utoipa::path(
    post,
    path = "/api/workflow-templates/{id}/instantiate",
    tag = "workflows",
    params(("id" = String, Path, description = "Template ID")),
    request_body = HashMap<String, serde_json::Value>,
    responses(
        (status = 201, description = "Workflow created from template", body = crate::types::JsonObject),
        (status = 400, description = "Invalid parameters"),
        (status = 404, description = "Template not found")
    )
)]
pub async fn instantiate_template(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(params): Json<HashMap<String, serde_json::Value>>,
) -> impl IntoResponse {
    let template = match state.kernel.templates().get(&id).await {
        Some(t) => t,
        None => {
            return ApiErrorResponse::not_found(format!("Template '{}' not found", id))
                .into_json_tuple();
        }
    };

    let workflow = match state.kernel.templates().instantiate(&template, &params) {
        Ok(w) => w,
        Err(e) => {
            return ApiErrorResponse::bad_request(e).into_json_tuple();
        }
    };

    // Same pre-flight validation as the direct /workflows endpoints —
    // an instantiated template can produce a workflow whose Transform
    // code / Wait duration / etc. is invalid (template-author error),
    // surface that here rather than at run time.
    let validation_errs = workflow.validate();
    if !validation_errs.is_empty() {
        let detail = validation_errs
            .iter()
            .map(|(step, reason)| format!("step '{step}': {reason}"))
            .collect::<Vec<_>>()
            .join("; ");
        return ApiErrorResponse::bad_request(format!(
            "template '{id}' instantiated to an invalid workflow: {detail}"
        ))
        .into_json_tuple();
    }

    let workflow_id = state.kernel.register_workflow(workflow).await;
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "workflow_id": workflow_id.to_string(),
            "template_id": id,
            "status": "instantiated",
        })),
    )
}
