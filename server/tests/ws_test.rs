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
    wait_until_ready(&url).await;
    (url, handle)
}

/// Wait until the server is accepting connections instead of guessing how long it needs:
/// a loaded machine can take much longer than the usual 50 ms, and a test that fails
/// because of that teaches people to ignore failures.
async fn wait_until_ready(url: &str) {
    for _ in 0..500 {
        if reqwest::get(format!("{url}/api/health")).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the test server never answered on {url}");
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

#[tokio::test]
async fn late_watcher_recovers_pending_feedback_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.path().to_path_buf(), 5);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(axum::serve(listener, build_app(state)).into_future());
    let id = create_session(&url).await;
    let client = reqwest::Client::new();
    assert!(client.post(format!("{url}/api/sessions/{id}/request_update"))
        .json(&json!({"annotations": [{"anchor":null,"strokes":[],"recognized_text":"make it bigger"}]}))
        .send().await.unwrap().status().is_success());
    handle.abort();
    let state = AppState::with_config(dir.path().to_path_buf(), 5);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(axum::serve(listener, build_app(state)).into_future());
    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();
    let msg = next_text(&mut ws).await;
    assert_eq!(msg["type"], "annotation_result");
    assert_eq!(msg["annotations"][0]["recognized_text"], "make it bigger");
    handle.abort();
}

#[tokio::test]
async fn server_reanchors_changed_text_and_rejects_deleted_or_ambiguous_targets() {
    let (url, handle) = start_server().await;
    let id = create_session(&url).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/api/sessions/{id}/reanchor"))
        .json(&json!({
            "previous": [
                {"i":0,"tag":"P","text":"Local recognition is fast and private"},
                {"i":1,"tag":"P","text":"Deleted sentence"},
                {"i":2,"tag":"P","text":"Repeated sentence"}
            ],
            "current": [
                {"i":7,"tag":"P","text":"Local recognition is very fast and private"},
                {"i":8,"tag":"P","text":"Repeated sentence"},
                {"i":9,"tag":"P","text":"Repeated sentence"}
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let body = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(body["targets"], json!([7, null, null]));
    handle.abort();
}

#[tokio::test]
async fn feedback_is_only_acknowledged_by_matching_id_and_consumes_array_indices() {
    let (url, handle) = start_server().await;
    let id = create_session(&url).await;
    let client = reqwest::Client::new();
    client.post(format!("{url}/api/sessions/{id}/request_update"))
        .json(&json!({"annotations":[{"anchor":null,"strokes":[[[1,2],[3,4]]],"recognized_text":"larger"}]}))
        .send().await.unwrap().error_for_status().unwrap();
    let snapshot = client
        .get(format!("{url}/api/sessions/{id}/feedback"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let feedback_id = snapshot["pending"]["feedback_id"].clone();
    // Unsolicited content changes preserve pending requests.
    client
        .put(format!("{url}/api/sessions/{id}/content"))
        .body("# A manual edit")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let snapshot = client
        .get(format!("{url}/api/sessions/{id}/feedback"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(snapshot["pending"]["feedback_id"], feedback_id);
    let stale = client
        .put(format!("{url}/api/sessions/{id}/content"))
        .json(&json!({"content":"stale","feedback_id":feedback_id,"expected_version":1}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), 409);
    let stale_feedback = client
        .post(format!("{url}/api/sessions/{id}/request_update"))
        .json(&json!({"document_version":1,"annotations":[]}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale_feedback.status(), 409);
    client.put(format!("{url}/api/sessions/{id}/content"))
        .json(&json!({"content":"# Fixed","feedback_id":feedback_id,"expected_version":2,"consumed_annotations":[0]}))
        .send().await.unwrap().error_for_status().unwrap();
    let snapshot = client
        .get(format!("{url}/api/sessions/{id}/feedback"))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(snapshot["pending"].is_null());
    assert_eq!(
        snapshot["last_content_update"]["consumed_strokes"],
        json!([[[1.0, 2.0], [3.0, 4.0]]])
    );
    handle.abort();
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
async fn resume_fetches_current_markdown_and_version_in_one_request() {
    let (url, handle) = start_server().await;
    let id = create_session(&url).await;
    let response = reqwest::get(format!("{url}/api/sessions/{id}/content"))
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap(),
        json!({
            "content":"# Test", "version":1, "status":"Active"
        })
    );
    handle.abort();
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
async fn ws_content_update_via_http_notifies_version() {
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();

    reqwest::Client::new()
        .put(format!("{url}/api/sessions/{id}/content"))
        .header("Content-Type", "text/plain")
        .body("# Updated\n\nv2")
        .send()
        .await
        .unwrap();

    let msg = next_text(&mut ws).await;
    assert_eq!(msg["type"].as_str().unwrap(), "version_updated");
    assert_eq!(msg["version"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn a_client_that_connects_after_an_update_still_receives_it() {
    // The observer reconnects constantly; an update written while nobody was attached
    // must still arrive, or the note the reader wrote is lost to the reconnecting CLI.
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    reqwest::Client::new()
        .put(format!("{url}/api/sessions/{id}/content"))
        .header("Content-Type", "text/plain")
        .body("# Updated before anyone watched\n\nv2")
        .send()
        .await
        .unwrap();

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();
    let msg = next_text(&mut ws).await;
    assert_eq!(msg["type"].as_str().unwrap(), "version_updated");
    assert_eq!(msg["version"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn an_unreadable_message_does_not_break_the_connection() {
    // The CLI and the app both send freely; a frame the server does not understand must
    // not drop the socket, because that socket is the way feedback comes back.
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();
    ws.send(Message::Text("not json at all".into()))
        .await
        .unwrap();
    ws.send(Message::Text(r#"{"type":"something_new"}"#.into()))
        .await
        .unwrap();

    reqwest::Client::new()
        .put(format!("{url}/api/sessions/{id}/content"))
        .header("Content-Type", "text/plain")
        .body("# Still connected\n\nv2")
        .send()
        .await
        .unwrap();

    let msg = next_text(&mut ws).await;
    assert_eq!(
        msg["type"].as_str().unwrap(),
        "version_updated",
        "the connection survived the junk frames: {msg}"
    );
}

#[tokio::test]
async fn a_submitted_session_replays_nothing_to_a_late_watcher() {
    // Replaying an update to a session that is already finished would look like the
    // review was reopened.
    let (url, _handle) = start_server().await;
    let id = create_session(&url).await;

    reqwest::Client::new()
        .put(format!("{url}/api/sessions/{id}/content"))
        .header("Content-Type", "text/plain")
        .body("# left behind\n\nv2")
        .send()
        .await
        .unwrap();
    reqwest::Client::new()
        .post(format!("{url}/api/sessions/{id}/submit"))
        .json(&json!({"typed_notes": "done"}))
        .send()
        .await
        .unwrap();

    let (mut ws, _) = connect_async(ws_url(&url, &format!("/ws/{id}")))
        .await
        .unwrap();
    let quiet = tokio::time::timeout(Duration::from_millis(400), ws.next()).await;
    assert!(
        quiet.is_err() || quiet.unwrap().is_none(),
        "a finished session should not replay a stale update"
    );
}
