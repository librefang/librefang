//! HTTP contract coverage for video task polling.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use librefang_api::routes;
use librefang_testing::TestAppState;
use tower::ServiceExt;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test(flavor = "multi_thread")]
async fn video_poll_flattens_active_and_failed_driver_statuses() {
    // This integration target contains one test, so no peer test can observe
    // this process-local credential while the mock MiniMax driver uses it.
    unsafe {
        std::env::set_var("MINIMAX_API_KEY", "integration-test-key");
    }

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/query/video_generation"))
        .and(query_param("task_id", "processing-task"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "Processing",
            "base_resp": { "status_code": 0 }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/query/video_generation"))
        .and(query_param("task_id", "failed-task"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "status": "Fail",
            "base_resp": { "status_code": 0 }
        })))
        .mount(&server)
        .await;

    let test = TestAppState::new();
    test.state
        .media_drivers
        .update_provider_urls([("minimax".to_string(), server.uri())]);
    let app = Router::new()
        .nest("/api", routes::media::router())
        .with_state(test.state.clone());

    let processing = get_json(&app, "/api/media/video/processing-task?provider=minimax").await;
    assert_eq!(processing.0, StatusCode::OK);
    assert_eq!(
        processing.1,
        serde_json::json!({
            "status": "processing",
            "task_id": "processing-task"
        })
    );

    let failed = get_json(&app, "/api/media/video/failed-task?provider=minimax").await;
    assert_eq!(failed.0, StatusCode::OK);
    assert_eq!(
        failed.1,
        serde_json::json!({
            "status": "failed",
            "task_id": "failed-task",
            "error": "Video generation failed"
        })
    );
}

async fn get_json(app: &Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, serde_json::from_slice(&body).unwrap())
}
