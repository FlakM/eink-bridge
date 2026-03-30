mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn create_with_title() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::post("/api/sessions?title=My+Doc")
                .body(Body::from("# Test"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    let app2 = common::test_app(dir.path().to_path_buf());
    let resp = app2
        .oneshot(
            Request::get(&format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["title"], "My Doc");
}

#[tokio::test]
async fn session_persists_across_restart() {
    let dir = tempfile::tempdir().unwrap();

    // Create session with first app instance
    let app = common::test_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Persist test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    // Create new app instance (simulates restart) from same state_dir
    let app2 = common::test_app(dir.path().to_path_buf());
    let resp = app2
        .oneshot(
            Request::get(&format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "Active");
}

#[tokio::test]
async fn session_expiry() {
    use eink_bridge::session::SessionManager;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let mut mgr = SessionManager::new(dir.path().to_path_buf());
    mgr.create("# Will expire".into(), None);

    // Expire with zero timeout (everything is stale)
    mgr.expire_stale(Duration::ZERO);

    let sessions = mgr.list();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].status,
        eink_bridge::session::SessionStatus::Expired
    );
}

#[tokio::test]
async fn list_filters_by_status() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create two sessions
    let app_clone = app.clone();
    app_clone
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# One"))
                .unwrap(),
        )
        .await
        .unwrap();

    let app_clone = app.clone();
    let resp = app_clone
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Two"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id2 = body["id"].as_str().unwrap().to_string();

    // Cancel the second one
    let app_clone = app.clone();
    app_clone
        .oneshot(
            Request::delete(&format!("/api/sessions/{id2}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // List only active
    let resp = app
        .oneshot(
            Request::get("/api/sessions?status=active")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.len(), 1);
}

#[tokio::test]
async fn submit_with_image() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create session
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Image test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    // Submit with multipart containing typed_notes and a fake image
    let boundary = "----boundary123";
    let multipart_body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"typed_notes\"\r\n\r\n\
         Great work!\r\n\
         --{boundary}\r\n\
         Content-Disposition: form-data; name=\"annotation\"; filename=\"test.png\"\r\n\
         Content-Type: image/png\r\n\r\n\
         FAKEPNG\r\n\
         --{boundary}--\r\n"
    );

    let resp = app
        .clone()
        .oneshot(
            Request::post(&format!("/api/sessions/{id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(multipart_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify result
    let resp = app
        .oneshot(
            Request::get(&format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["typed_notes"], "Great work!");
    assert_eq!(body["annotation_images"].as_array().unwrap().len(), 1);
}
