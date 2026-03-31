mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn contract_scenarios_match_goldens() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let scenarios = vec![
        ("create_session", create_session_response(app.clone()).await),
        (
            "create_session_plaintext",
            create_session_plaintext(app.clone()).await,
        ),
        ("list_sessions", list_sessions_response(app.clone()).await),
        (
            "list_sessions_filtered",
            list_sessions_filtered(app.clone()).await,
        ),
        ("get_session_detail", get_session_detail(app.clone()).await),
        (
            "submitted_result",
            submitted_result_response(app.clone()).await,
        ),
        (
            "submitted_with_verdict_changes",
            submitted_with_verdict_changes(app.clone()).await,
        ),
        ("cancelled_result", cancelled_result(app.clone()).await),
        ("openapi", openapi_response(app.clone()).await),
    ];

    for (name, value) in scenarios {
        let normalized = common::eval::normalize_json(value);
        let pretty = serde_json::to_string_pretty(&normalized).unwrap();
        let golden = common::eval::repo_server_dir()
            .join("tests/golden/contracts")
            .join(name)
            .with_extension("json");
        common::eval::assert_or_update(&golden, &pretty);
    }
}

async fn create_session_response(app: axum::Router) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "title": "Contract Doc",
                        "content": "# Contract Doc\n\nHello",
                        "callback_url": "http://localhost/hook",
                        "tags": {"type": "contract", "repo": "eink-bridge"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn list_sessions_response(app: axum::Router) -> serde_json::Value {
    app.clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "title": "List Doc",
                        "content": "# List Doc",
                        "tags": {"type": "list"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn submitted_result_response(app: axum::Router) -> serde_json::Value {
    let create = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "title": "Result Doc",
                        "content": "# Result Doc",
                        "tags": {"type": "review"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let create_body: serde_json::Value =
        serde_json::from_slice(&create.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = create_body["id"].as_str().unwrap();

    let submit = app
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "typed_notes": "LGTM: ship it",
                        "verdict": "lgtm"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(submit.status(), StatusCode::OK);

    let result = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    serde_json::from_slice(&result.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn create_session_plaintext(app: axum::Router) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::post("/api/sessions?title=Plain%20Text")
                .body(Body::from("# Plain Text\n\nCreated with plain body"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn list_sessions_filtered(app: axum::Router) -> serde_json::Value {
    // Create one, submit it, then filter for submitted only
    let create = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "content": "# Filter Test",
                        "tags": {"purpose": "filter"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&create.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    app.clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"typed_notes": "done"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::get("/api/sessions?status=submitted")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn get_session_detail(app: axum::Router) -> serde_json::Value {
    let create = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "title": "Detail Test",
                        "content": "# Detail",
                        "callback_url": "http://localhost/hook",
                        "tags": {"env": "test"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&create.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn submitted_with_verdict_changes(app: axum::Router) -> serde_json::Value {
    let create = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "title": "Changes Doc",
                        "content": "# Needs Work"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&create.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    app.clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "typed_notes": "CHANGES: needs refactor in session.rs",
                        "verdict": "changes"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn cancelled_result(app: axum::Router) -> serde_json::Value {
    let create = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "title": "Cancel Doc",
                        "content": "# Will Cancel"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&create.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    let cancel = app
        .clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);

    // Result for cancelled session should be 410
    let response = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    // Return status code as JSON since the body is empty for 410
    serde_json::json!({
        "status_code": response.status().as_u16(),
        "description": "cancelled session returns 410 GONE"
    })
}

async fn openapi_response(app: axum::Router) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::get("/api/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
