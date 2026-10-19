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

#[tokio::test]
async fn webhook_fires_on_expiry() {
    // Expiry is the third terminal transition, and the agent waiting on the callback
    // used to hear nothing: the two it knew about fired, this one did not.
    let dir = tempfile::tempdir().unwrap();
    let (hook_url, received) = hook_server().await;
    let state = eink_bridge::app::AppState::with_config(dir.path().to_path_buf(), 5);
    let app = eink_bridge::app::build_app(state.clone());

    let resp = app
        .oneshot(
            Request::post(format!(
                "/api/sessions?callback_url={}",
                urlencoding::encode(&hook_url)
            ))
            .body(Body::from("# Left open"))
            .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    assert_eq!(state.expire_stale(std::time::Duration::ZERO).await, 1);
    let payload = received
        .wait(2000)
        .await
        .expect("webhook was not called on expiry");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["status"], "Expired");
}

#[tokio::test]
async fn webhook_fires_when_a_live_review_is_force_removed() {
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
            .body(Body::from("# Taken away"))
            .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(
            Request::delete(format!("/api/sessions/{id}?remove=true&force=true"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let payload = received
        .wait(2000)
        .await
        .expect("webhook was not called on a forced removal");
    assert_eq!(payload["id"], id);
    assert_eq!(
        payload["status"], "Cancelled",
        "to the agent, it was cancelled"
    );
}

#[tokio::test]
async fn handwritten_feedback_reaches_the_callback_and_is_counted() {
    // The mid-review callback used to be sent and forgotten: no metric, no log, so a
    // receiver that was down looked exactly like a reader who had written nothing.
    eink_bridge::metrics::init();
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
            .body(Body::from("# Annotated"))
            .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/request_update"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"annotations": [{"anchor": null, "strokes": [[[1.0, 2.0]]], "recognized_text": "tighten this"}]})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let payload = received
        .wait(2000)
        .await
        .expect("the feedback webhook was not called");
    assert_eq!(payload["type"], "annotation_result");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["version"], 1);
    assert!(
        payload["feedback_id"]
            .as_str()
            .is_some_and(|f| !f.is_empty())
    );
    assert_eq!(payload["annotations"][0]["recognized_text"], "tighten this");

    let mut counted = false;
    for _ in 0..40 {
        let resp = app
            .clone()
            .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
            .to_string();
        if text
            .lines()
            .any(|l| l.starts_with("eink_webhook_total{result=\"ok\"}") && !l.ends_with(" 0"))
        {
            counted = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(counted, "the delivery is counted like every other webhook");
}

#[tokio::test]
async fn a_callback_url_must_be_somewhere_the_server_should_post_to() {
    // The server posts review bodies wherever it is told. Not to a scheme it cannot
    // speak, and not to the link-local range where cloud metadata services answer.
    // Loopback stays allowed: the local relay listens on 127.0.0.1.
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let create = |url: &str| {
        let app = app.clone();
        let uri = format!("/api/sessions?callback_url={}", urlencoding::encode(url));
        async move {
            let resp = app
                .oneshot(Request::post(uri).body(Body::from("# Hook")).unwrap())
                .await
                .unwrap();
            let status = resp.status();
            let body =
                String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes())
                    .to_string();
            (status, body)
        }
    };
    for refused in [
        "ftp://files.example/hook",
        "not a url at all",
        "http://169.254.169.254/latest/meta-data/",
        "http://[fe80::1]/hook",
    ] {
        let (status, body) = create(refused).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "{refused} should be refused"
        );
        assert!(body.contains("callback_url"), "{refused}: {body}");
    }
    for allowed in [
        "http://127.0.0.1:8789/hook",
        "https://hooks.example.com/eink",
    ] {
        let (status, body) = create(allowed).await;
        assert_eq!(status, StatusCode::CREATED, "{allowed}: {body}");
    }
    // The same check covers a callback carried in the JSON body.
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"content": "# Hook", "callback_url": "gopher://old/hook"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_webhook_that_fails_twice_is_retried_until_it_lands() {
    // A channel that is restarting is deaf for a second or two, not gone. The feedback is
    // durable on the session, so the delivery has to be retried rather than dropped.
    use std::sync::atomic::{AtomicUsize, Ordering};

    let attempts = Arc::new(AtomicUsize::new(0));
    let received = HookReceiver::new();
    let counter = attempts.clone();
    let receiver = received.clone();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move |body: String| {
                let counter = counter.clone();
                let receiver = receiver.clone();
                async move {
                    let attempt = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    if attempt < 3 {
                        return StatusCode::INTERNAL_SERVER_ERROR;
                    }
                    if let Ok(value) = serde_json::from_str(&body) {
                        receiver.store(value).await;
                    }
                    StatusCode::OK
                }
            }),
        );
        axum::serve(listener, app).await.unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());
    let hook = format!("http://127.0.0.1:{port}/hook");
    let resp = app
        .clone()
        .oneshot(
            Request::post(format!(
                "/api/sessions?callback_url={}",
                urlencoding::encode(&hook)
            ))
            .body(Body::from("# Flaky receiver"))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Cancelling also fires the callback, which is faster than filling in a review.
    let resp = app
        .oneshot(
            Request::delete(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let payload = received.wait(3000).await.expect("the third attempt was not delivered");
    assert_eq!(payload["id"], id);
    assert_eq!(
        attempts.load(Ordering::SeqCst),
        3,
        "two refusals, then the delivery"
    );
}
