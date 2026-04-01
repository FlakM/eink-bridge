use eink_bridge::app::{AppState, build_app};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};

async fn start_server() -> (String, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.keep(), 5);
    let app = build_app(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (url, handle)
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

async fn next_text(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> serde_json::Value {
    loop {
        match tokio::time::timeout(Duration::from_secs(3), ws.next())
            .await
            .expect("timeout waiting for WS message")
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected WS message: {other:?}"),
        }
    }
}

#[tokio::test]
async fn ws_connect_unknown_session_gets_404() {
    let (url, _handle) = start_server().await;
    let result = connect_async(ws_url(&url, "/ws/doesnotexist")).await;
    assert!(result.is_err(), "expected WS connect to fail with 404");
}

#[tokio::test]
async fn ws_receives_session_submitted() {
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();

    reqwest::Client::new()
        .post(format!("{url}/api/sessions/{id}/submit"))
        .json(&json!({"typed_notes": "looks good"}))
        .send()
        .await
        .unwrap();

    let msg = next_text(&mut ws).await;
    assert_eq!(msg["type"].as_str().unwrap(), "session_submitted");
    assert_eq!(msg["result"]["status"].as_str().unwrap(), "Submitted");
}

#[tokio::test]
async fn ws_cancelled_session_receives_error() {
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();

    reqwest::Client::new()
        .delete(format!("{url}/api/sessions/{id}"))
        .send()
        .await
        .unwrap();

    let msg = next_text(&mut ws).await;
    assert_eq!(msg["type"].as_str().unwrap(), "error");
    assert!(msg["message"].as_str().unwrap().contains("cancelled"));
}

#[tokio::test]
async fn ws_request_update_returns_annotation_result() {
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();

    ws.send(Message::Text(
        json!({"type": "request_update", "annotations": [], "typed_notes": ""}).to_string(),
    ))
    .await
    .unwrap();

    // Skip any processing_status messages (OCR engine may or may not be available)
    let msg = loop {
        let m = next_text(&mut ws).await;
        if m["type"].as_str() != Some("processing_status") {
            break m;
        }
    };
    assert_eq!(msg["type"].as_str().unwrap(), "annotation_result");
    assert_eq!(msg["version"].as_u64().unwrap(), 1);
}

#[tokio::test]
async fn ws_update_content_increments_version() {
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();

    // Transition to Processing first
    ws.send(Message::Text(
        json!({"type": "request_update", "annotations": [], "typed_notes": ""}).to_string(),
    ))
    .await
    .unwrap();
    // Consume processing_status (if OCR present) and annotation_result
    loop {
        let m = next_text(&mut ws).await;
        if m["type"].as_str() != Some("processing_status") {
            break; // annotation_result consumed
        }
    }

    // Now update content
    ws.send(Message::Text(
        json!({"type": "update_content", "content": "# Updated\n\nv2"}).to_string(),
    ))
    .await
    .unwrap();

    let msg = next_text(&mut ws).await;
    assert_eq!(msg["type"].as_str().unwrap(), "version_updated");
    assert_eq!(msg["version"].as_u64().unwrap(), 2);
}
