//! The WebSocket heartbeat, in its own test binary so the connection gauge is
//! deterministic: a half-open socket must become visible rather than silently dead.
use eink_bridge::app::{AppState, build_app};
use futures_util::StreamExt;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;

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

async fn wait_for(url: &str, want: i64) -> i64 {
    let mut seen = gauge(url).await;
    for _ in 0..80 {
        if seen == want {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        seen = gauge(url).await;
    }
    seen
}

#[tokio::test]
async fn a_client_that_stops_answering_is_dropped() {
    eink_bridge::metrics::init();
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::with_config(dir.path().to_path_buf(), 5);
    // One second between pings: a client silent for three intervals is gone.
    state.ws_heartbeat_seconds = 1;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(axum::serve(listener, build_app(state)).into_future());
    tokio::time::sleep(Duration::from_millis(50)).await;

    let id = reqwest::Client::new()
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
        .to_string();

    let ws_url = url.replacen("http://", "ws://", 1) + &format!("/ws/{id}");
    let (mut socket, _) = connect_async(&ws_url).await.unwrap();
    assert_eq!(wait_for(&url, 1).await, 1, "the client connects");

    // Now stop reading: the handshake is done, so no Pong is ever sent back. The server
    // must notice and let the connection go rather than holding a socket nobody is on.
    let quiet = tokio::time::timeout(Duration::from_millis(300), socket.next()).await;
    assert!(quiet.is_err(), "nothing should arrive on a quiet session");
    let dropped = wait_for(&url, 0).await;
    assert_eq!(
        dropped, 0,
        "a client that never answers a ping must be dropped, or the gauge reports a \
         reader that is not there"
    );

    // And the server is still healthy for a client that does answer: the same socket
    // was closed, so connect again and this time keep reading.
    let (mut live, _) = connect_async(&ws_url).await.unwrap();
    assert_eq!(wait_for(&url, 1).await, 1);
    let mut beat = tokio::time::Instant::now();
    // Read for longer than the drop threshold: as long as we poll, tungstenite answers
    // each Ping and the connection survives.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), live.next()).await {
            Ok(Some(Ok(_))) => beat = tokio::time::Instant::now(),
            Ok(Some(Err(_))) | Ok(None) => break,
            Ok(_) => {}
            Err(_) => {}
        }
    }
    let _ = beat;
    assert_eq!(
        gauge(&url).await,
        1,
        "a client that keeps answering stays connected"
    );
    live.close(None).await.unwrap();
    assert_eq!(wait_for(&url, 0).await, 0);
}
