mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn health_returns_ok() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "ok");
    assert!(body["exe"].is_string());
}

#[tokio::test]
async fn create_session_returns_id_and_url() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Test\n\nHello world"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["schema_version"], 1);
    assert!(body["id"].is_string());
    assert!(body["url"].as_str().unwrap().starts_with("/session/"));
}

#[tokio::test]
async fn openapi_endpoint_returns_contract() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/api/openapi.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["openapi"], "3.1.0");
    assert!(body["paths"]["/api/sessions"].is_object());
}

#[tokio::test]
async fn get_nonexistent_session_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/api/sessions/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn render_session_returns_html() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // create a session first
    let create_resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Heading\n\nParagraph"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&create_resp.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    let id = body["id"].as_str().unwrap();

    let resp = app
        .oneshot(
            Request::get(format!("/session/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("Heading"));
    assert!(html.contains("Paragraph"));
}

#[tokio::test]
async fn render_session_preserves_diagram_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());
    let markdown = r#"# Review

```mermaid
flowchart LR
  A[Start] --> B[Done]
```

```mindmap
root: Review
nodes:
  - id: parser
    label: Parser
```

```graph
layout:
  algorithm: layered
nodes:
  - id: cli
    label: CLI
  - id: server
    label: Server
edges:
  - from: cli
    to: server
    label: push
```
"#;

    let create_resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from(markdown))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&create_resp.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    let id = body["id"].as_str().unwrap();

    let resp = app
        .oneshot(
            Request::get(format!("/session/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();

    assert!(html.contains("class=\"diagram-block\" data-kind=\"mermaid\""));
    assert!(html.contains("class=\"diagram-block\" data-kind=\"mindmap\""));
    assert!(html.contains("class=\"diagram-block\" data-kind=\"graph\""));
    assert!(html.contains("Rendering Mind Map..."));
    assert!(html.contains("Rendering Diagram..."));
    assert!(html.contains("Rendering Graph..."));
    assert!(html.contains("/assets/diagram/mermaid.min.js"));
    assert!(html.contains("/assets/diagram/elk.bundled.js"));
}

#[tokio::test]
async fn vendored_diagram_assets_are_served() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/assets/diagram/mermaid.min.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn render_nonexistent_session_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::get("/session/doesnotexist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
