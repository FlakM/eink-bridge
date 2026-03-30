mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use eink_bridge::app::{AppState, build_app};
use http_body_util::BodyExt;
use std::time::{Duration, Instant};
use tower::ServiceExt;

fn test_app_with_poll(dir: &std::path::Path, poll_secs: u64) -> axum::Router {
    let state = AppState::with_config(dir.to_path_buf(), poll_secs);
    build_app(state)
}

#[tokio::test]
async fn long_poll_returns_on_submit() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app_with_poll(dir.path(), 30);

    // Create session
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let app2 = app.clone();
    let id2 = id.clone();

    // Start long-poll in background
    let poll_handle = tokio::spawn(async move {
        let start = Instant::now();
        let resp = app2
            .oneshot(
                Request::get(&format!("/api/sessions/{id2}/result"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        (resp, start.elapsed())
    });

    // Wait a bit, then submit
    tokio::time::sleep(Duration::from_millis(100)).await;
    let boundary = "----test";
    let multipart_body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"typed_notes\"\r\n\r\n\
         feedback\r\n\
         --{boundary}--\r\n"
    );
    app.oneshot(
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

    let (resp, elapsed) = poll_handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["typed_notes"], "feedback");
}

#[tokio::test]
async fn long_poll_timeout_returns_204() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app_with_poll(dir.path(), 1); // 1-second poll timeout

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    let start = Instant::now();
    let resp = app
        .oneshot(
            Request::get(&format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert!(
        elapsed >= Duration::from_millis(900),
        "too fast: {elapsed:?}"
    );
    assert!(elapsed < Duration::from_secs(3), "too slow: {elapsed:?}");
}

#[tokio::test]
async fn long_poll_cancel_returns_410() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app_with_poll(dir.path(), 30);

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let app2 = app.clone();
    let id2 = id.clone();

    let poll_handle = tokio::spawn(async move {
        app2.oneshot(
            Request::get(&format!("/api/sessions/{id2}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    app.oneshot(
        Request::delete(&format!("/api/sessions/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    let resp = poll_handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}
