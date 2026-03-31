mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

struct HookReceiver {
    value: Mutex<Option<serde_json::Value>>,
    signal: tokio::sync::Notify,
}

impl HookReceiver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            value: Mutex::new(None),
            signal: tokio::sync::Notify::new(),
        })
    }

    async fn store(&self, v: serde_json::Value) {
        *self.value.lock().await = Some(v);
        self.signal.notify_waiters();
    }

    async fn wait(&self, timeout_ms: u64) -> Option<serde_json::Value> {
        tokio::select! {
            _ = self.signal.notified() => self.value.lock().await.clone(),
            _ = tokio::time::sleep(std::time::Duration::from_millis(timeout_ms)) => {
                self.value.lock().await.clone()
            }
        }
    }
}

async fn hook_server() -> (String, Arc<HookReceiver>) {
    let receiver = HookReceiver::new();
    let recv_clone = receiver.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move |body: String| {
                let store = recv_clone.clone();
                async move {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                        store.store(v).await;
                    }
                    StatusCode::OK
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    (format!("http://127.0.0.1:{port}/hook"), receiver)
}

#[tokio::test]
async fn webhook_fires_on_submit() {
    let dir = tempfile::tempdir().unwrap();
    let (hook_url, received) = hook_server().await;

    let app = common::test_app(dir.path().to_path_buf());

    // Create session with callback_url
    let resp = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/sessions?callback_url={}",
                urlencoding::encode(&hook_url)
            ))
            .body(Body::from("# Webhook Test"))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    // Submit the session
    let boundary = "b";
    let mp = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nLGTM: nice work\r\n--{boundary}--\r\n"
    );
    app.oneshot(
        Request::post(format!("/api/sessions/{id}/submit"))
            .header(
                "content-type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(mp))
            .unwrap(),
    )
    .await
    .unwrap();

    let payload = received.wait(2000).await.expect("webhook was not called");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["status"], "Submitted");
    assert_eq!(payload["typed_notes"], "LGTM: nice work");
    assert_eq!(payload["verdict"], "lgtm");
}

#[tokio::test]
async fn webhook_fires_on_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let (hook_url, received) = hook_server().await;

    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/sessions?callback_url={}",
                urlencoding::encode(&hook_url)
            ))
            .body(Body::from("# Cancel Webhook"))
            .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    app.oneshot(
        Request::delete(format!("/api/sessions/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    let payload = received
        .wait(2000)
        .await
        .expect("webhook was not called on cancel");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["status"], "Cancelled");
}

#[tokio::test]
async fn no_webhook_without_callback_url() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create without callback_url — should not panic or fail
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# No Hook"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    let boundary = "b";
    let mp = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nnotes\r\n--{boundary}--\r\n"
    );
    let resp = app
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(mp))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn result_includes_verdict_field() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Verdict Test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let boundary = "b";
    let mp = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nCHANGES: needs refactor\r\n--{boundary}--\r\n"
    );
    app.clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(mp))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let result: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(result["verdict"], "changes");
    assert_eq!(result["typed_notes"], "CHANGES: needs refactor");
}

#[tokio::test]
async fn no_verdict_for_plain_notes() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Plain"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let boundary = "b";
    let mp = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nlooks good to me\r\n--{boundary}--\r\n"
    );
    app.clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(mp))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let result: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        result["verdict"].is_null(),
        "expected no verdict for plain notes"
    );
}
