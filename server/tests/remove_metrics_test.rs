//! The active-session gauge across a forced removal, in a test binary of its own.
//!
//! `eink_sessions_active` is process-global, so a check that it comes back down is only
//! meaningful in a process where this is the only session being counted. Like
//! `ws_metrics_test`, this file holds exactly one test.
//!
//! Removing a live review with `force=true` used to skip the cancel path: the gauge
//! stayed one too high for good, and the runbook's first question -- "how many reviews
//! are open?" -- had a wrong answer.
use axum::body::Body;
use axum::http::{Request, StatusCode};
use eink_bridge::app::{AppState, build_app};
use eink_bridge::metrics::{SESSIONS_ACTIVE, SESSIONS_CANCELLED};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn a_forced_removal_gives_the_active_gauge_back() {
    eink_bridge::metrics::init();
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 5));
    let active_before = SESSIONS_ACTIVE.get();
    let cancelled_before = SESSIONS_CANCELLED.get();

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Abandoned"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();
    assert_eq!(SESSIONS_ACTIVE.get(), active_before + 1);

    let resp = app
        .oneshot(
            Request::delete(format!("/api/sessions/{id}?remove=true&force=true"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        SESSIONS_ACTIVE.get(),
        active_before,
        "the review is no longer open"
    );
    assert_eq!(
        SESSIONS_CANCELLED.get(),
        cancelled_before + 1,
        "and it counts as cancelled, which to the reader it was"
    );
}
