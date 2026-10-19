//! The WebSocket connection gauge, in a test binary of its own.
//!
//! `eink_ws_connections_active` is process-global, so a leak check is only meaningful in
//! a process where the only connections are the ones this test makes. Tests in one file
//! share a process but run on threads, so this file holds exactly one test.
//!
//! The runbook tells the reader to check this gauge when feedback stops arriving. A gauge
//! that never returns to zero would send them looking for connections that are not there.
use eink_bridge::app::{AppState, build_app};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;

async fn start_server() -> String {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.keep(), 5);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, build_app(state)).into_future());
    tokio::time::sleep(Duration::from_millis(50)).await;
    url
}

async fn create_session(url: &str) -> String {
    reqwest::Client::new()
        .post(format!("{url}/api/sessions"))
        .body("# Test")
        .header("Content-Type", "text/plain")
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

fn ws_url(http_url: &str, path: &str) -> String {
    http_url.replacen("http://", "ws://", 1) + path
}

async fn gauge(url: &str) -> i64 {
    let text = reqwest::get(format!("{url}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    text.lines()
        .find_map(|line| line.strip_prefix("eink_ws_connections_active "))
        .expect("the gauge is exported")
        .trim()
        .parse()
        .unwrap()
}

/// Wait for the gauge to reach `want`, so the assertion is about the steady state rather
/// than about how quickly a task was scheduled.
async fn wait_for(url: &str, want: i64) -> i64 {
    let mut seen = gauge(url).await;
    for _ in 0..60 {
        if seen == want {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        seen = gauge(url).await;
    }
    seen
}

#[tokio::test]
async fn the_connection_gauge_counts_up_and_back_down_to_zero() {
    // The server binary registers the metrics at startup; a connection made before that
    // would export nothing, so the test starts the same way the process does.
    eink_bridge::metrics::init();
    let url = start_server().await;
    let id = create_session(&url).await;
    assert_eq!(gauge(&url).await, 0, "nothing has connected yet");

    // A refused upgrade must not count at all.
    let refused = connect_async(ws_url(&url, "/ws/does-not-exist")).await;
    assert!(
        refused.is_err() || refused.unwrap().0.close(None).await.is_err(),
        "an unknown session has no socket"
    );
    assert_eq!(
        gauge(&url).await,
        0,
        "a rejected upgrade is not a connection"
    );

    let (mut clean, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();
    let (dropped, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();
    assert_eq!(wait_for(&url, 2).await, 2, "both clients count");

    // A polite close, and a client that just vanishes (the tablet asleep, the CLI killed).
    clean.close(None).await.unwrap();
    drop(dropped);

    assert_eq!(
        wait_for(&url, 0).await,
        0,
        "every connection must be given back, or the runbook's gauge lies"
    );

    // The server is still healthy afterwards.
    let health = reqwest::get(format!("{url}/api/health")).await.unwrap();
    assert!(health.status().is_success());
    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();
    assert_eq!(wait_for(&url, 1).await, 1, "a later client still counts");
    ws.close(None).await.unwrap();
    assert_eq!(wait_for(&url, 0).await, 0);
}
