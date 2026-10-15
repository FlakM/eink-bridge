mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use eink_bridge::app::{AppState, build_app};
use http_body_util::BodyExt;
use std::future::{Future, poll_fn};
use std::task::Poll;
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
                Request::get(format!("/api/sessions/{id2}/result"))
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
        Request::post(format!("/api/sessions/{id}/submit"))
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
            Request::get(format!("/api/sessions/{id}/result"))
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
            Request::get(format!("/api/sessions/{id2}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    });

    tokio::time::sleep(Duration::from_millis(100)).await;
    app.oneshot(
        Request::delete(format!("/api/sessions/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .unwrap();

    let resp = poll_handle.await.unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

#[tokio::test]
async fn expiration_wakes_all_pollers_and_cleans_up_channels() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.path().to_path_buf(), 30);
    let app = build_app(state.clone());
    let (session, pinned) = {
        let mut sessions = state.sessions.write().await;
        (
            sessions.create("expire".into(), None, None, Default::default(), false, None),
            sessions.create("keep".into(), None, None, Default::default(), true, None),
        )
    };
    let mut receiver = state.ws_subscribe(&session.id).await;
    let mut pinned_receiver = state.ws_subscribe(&pinned.id).await;
    let request = || {
        Request::get(format!("/api/sessions/{}/result", session.id))
            .body(Body::empty())
            .unwrap()
    };
    let mut first = std::pin::pin!(app.clone().oneshot(request()));
    let mut second = std::pin::pin!(app.clone().oneshot(request()));
    poll_fn(|cx| {
        assert!(first.as_mut().poll(cx).is_pending());
        assert!(second.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;

    assert_eq!(state.expire_stale(Duration::ZERO).await, 1);
    assert!(state.notifiers.read().await.is_empty());
    let senders = state.ws_senders.read().await;
    assert!(!senders.contains_key(&session.id));
    assert!(senders.contains_key(&pinned.id));
    drop(senders);
    assert!(matches!(
        receiver.recv().await.unwrap(),
        eink_bridge::ws::ServerMessage::Error { message } if message == "session expired"
    ));
    assert!(matches!(
        receiver.recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    ));
    assert!(matches!(
        pinned_receiver.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));

    let (first, second) = tokio::time::timeout(Duration::from_secs(1), async {
        tokio::join!(first, second)
    })
    .await
    .expect("expiration should wake every poller immediately");
    assert_eq!(first.unwrap().status(), StatusCode::GONE);
    assert_eq!(second.unwrap().status(), StatusCode::GONE);
    assert_eq!(state.expire_stale(Duration::ZERO).await, 0);

    let persisted: eink_bridge::session::Session =
        serde_json::from_slice(&std::fs::read(session.state_dir.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(
        persisted.status,
        eink_bridge::session::SessionStatus::Expired
    );
    assert!(persisted.updated_at > session.updated_at);
}

#[tokio::test]
async fn long_poll_registers_before_allowing_a_terminal_transition() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.path().to_path_buf(), 30);
    let app = build_app(state.clone());
    let session = state.sessions.write().await.create(
        "review".into(),
        None,
        None,
        Default::default(),
        false,
        None,
    );
    let notifiers = state.notifiers.write().await;
    let mut poll = std::pin::pin!(
        app.clone().oneshot(
            Request::get(format!("/api/sessions/{}/result", session.id))
                .body(Body::empty())
                .unwrap(),
        )
    );
    poll_fn(|cx| {
        assert!(poll.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(
        state.sessions.try_write().is_err(),
        "transitions must wait until the poller has registered"
    );
    drop(notifiers);

    let cancel = app.oneshot(
        Request::delete(format!("/api/sessions/{}", session.id))
            .body(Body::empty())
            .unwrap(),
    );
    let (result, cancelled) =
        tokio::time::timeout(Duration::from_secs(1), async { tokio::join!(poll, cancel) })
            .await
            .expect("cancellation should wake the registered poller");
    assert_eq!(result.unwrap().status(), StatusCode::GONE);
    assert_eq!(cancelled.unwrap().status(), StatusCode::OK);
}
