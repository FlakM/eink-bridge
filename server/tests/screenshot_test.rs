//! The agent's window onto the tablet.
//!
//! The element map says what is on the page; only the device knows where the reader has
//! scrolled it to, how far in they are zoomed, and what they have drawn on it. Asking for
//! a picture is the one way to see that, so the pieces that can go wrong are worth pinning:
//! `delivered` means a tablet heard it and not merely the agent's own observer, a retry
//! while the tablet is still working reuses the request but "show me again" does not, a
//! capture the tablet could not take is reported rather than waited out, the request is
//! refused for a session that is over, the id cannot be used to read or write outside the
//! session, and a caller polling before the answer arrives gets "not yet" rather than an
//! error.

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use eink_bridge::app::{AppState, build_app};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tower::ServiceExt;

/// The smallest thing `bytes_are_image` will accept as a PNG.
fn png() -> Vec<u8> {
    let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    bytes.extend_from_slice(b"not really a picture, but the magic is right");
    bytes
}

async fn create_session(app: &axum::Router) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# A page worth photographing"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn ask(app: &axum::Router, id: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/screenshot"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, body)
}

async fn put(
    app: &axum::Router,
    id: &str,
    request_id: &str,
    content_type: &str,
    body: Vec<u8>,
) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::put(format!("/api/sessions/{id}/screenshot/{request_id}"))
                .header(header::CONTENT_TYPE, content_type)
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn get(app: &axum::Router, id: &str, request_id: &str) -> axum::response::Response {
    app.clone()
        .oneshot(
            Request::get(format!("/api/sessions/{id}/screenshot/{request_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
}

fn no_screenshot_stored(state_dir: &std::path::Path, id: &str) -> bool {
    let screenshots = state_dir.join("sessions").join(id).join("screenshots");
    !screenshots.exists() || std::fs::read_dir(&screenshots).unwrap().next().is_none()
}

#[tokio::test]
async fn asking_names_a_request_the_tablet_can_answer() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;

    let (status, body) = ask(&app, &id).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let request_id = body["id"].as_str().unwrap();
    assert_eq!(request_id.len(), 32);
    assert!(request_id.bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(
        body["url"].as_str().unwrap(),
        format!("/api/sessions/{id}/screenshot/{request_id}")
    );
    // No WebSocket client in this test, so the honest answer is "nobody heard it".
    assert_eq!(body["delivered"], serde_json::Value::Bool(false));
}

#[tokio::test]
async fn a_second_ask_while_the_first_is_unanswered_reuses_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;

    let (_, first) = ask(&app, &id).await;
    let (_, second) = ask(&app, &id).await;
    assert_eq!(
        first["id"], second["id"],
        "a retry must not flash the tablet twice"
    );
}

#[tokio::test]
async fn an_ask_after_the_answer_is_a_new_picture() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;

    let (_, first) = ask(&app, &id).await;
    let request_id = first["id"].as_str().unwrap();
    let uploaded = put(&app, &id, request_id, "image/png", png()).await;
    assert_eq!(uploaded.status(), StatusCode::NO_CONTENT);

    let (_, second) = ask(&app, &id).await;
    assert_ne!(
        first["id"], second["id"],
        "\"show me again\" must not be answered with the picture already taken"
    );
}

#[tokio::test]
async fn a_capture_the_tablet_could_not_take_is_reported_not_waited_for() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;
    let (_, body) = ask(&app, &id).await;
    let request_id = body["id"].as_str().unwrap().to_string();

    let reported = put(
        &app,
        &id,
        &request_id,
        "application/json",
        br#"{"error":"the page has not drawn yet"}"#.to_vec(),
    )
    .await;
    assert_eq!(reported.status(), StatusCode::NO_CONTENT);

    let fetched = get(&app, &id, &request_id).await;
    assert_eq!(
        fetched.status(),
        StatusCode::GONE,
        "not \"not yet\": it will never arrive"
    );
    let text = String::from_utf8(
        fetched
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(text.contains("the page has not drawn yet"), "{text}");

    // A failed request is not reused: the next ask gives the tablet another go.
    let (_, again) = ask(&app, &id).await;
    assert_ne!(again["id"].as_str().unwrap(), request_id);

    // A failure report without a reason is a malformed request, not a silent success.
    let (_, body) = ask(&app, &id).await;
    let empty = put(
        &app,
        &id,
        body["id"].as_str().unwrap(),
        "application/json",
        b"{}".to_vec(),
    )
    .await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn an_answer_to_a_request_nobody_made_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;

    let well_formed_but_unasked = "0123456789abcdef0123456789abcdef";
    let resp = put(&app, &id, well_formed_but_unasked, "image/png", png()).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(no_screenshot_stored(dir.path(), &id));

    // Nor can a stale one be answered twice: once stored, the request is over.
    let (_, body) = ask(&app, &id).await;
    let request_id = body["id"].as_str().unwrap();
    assert_eq!(
        put(&app, &id, request_id, "image/png", png())
            .await
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        put(&app, &id, request_id, "image/png", png())
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_finished_session_has_no_page_to_photograph() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;

    let resp = app
        .clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let (status, _) = ask(&app, &id).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn an_unknown_session_is_a_404_not_a_wait() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let (status, _) = ask(&app, "nope").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_answer_waits_for_the_tablet_then_appears() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;
    let (_, body) = ask(&app, &id).await;
    let request_id = body["id"].as_str().unwrap().to_string();
    let url = format!("/api/sessions/{id}/screenshot/{request_id}");

    // Polling before the tablet has answered is "not yet", never an error.
    let early = app
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(early.status(), StatusCode::NOT_FOUND);

    let image = png();
    let uploaded = app
        .clone()
        .oneshot(
            Request::put(&url)
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from(image.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(uploaded.status(), StatusCode::NO_CONTENT);

    let fetched = app
        .clone()
        .oneshot(Request::get(&url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        fetched.headers().get(header::CONTENT_TYPE).unwrap(),
        "image/png"
    );
    assert_eq!(
        fetched
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .unwrap(),
        "nosniff"
    );
    let served = fetched.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(served.as_ref(), image.as_slice(), "byte for byte");
}

#[tokio::test]
async fn a_request_id_cannot_name_a_file_outside_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;

    // A picture that is not a screenshot, one directory up from where they live. The
    // screenshots directory exists, as it would after one answered request: `..` only
    // resolves through a directory that is there.
    let session_dir = dir.path().join("sessions").join(&id);
    std::fs::create_dir_all(session_dir.join("screenshots")).unwrap();
    std::fs::write(session_dir.join("escape.png"), png()).unwrap();

    for bad in ["..%2Fescape", "ABCDEF0123456789ABCDEF0123456789", "short"] {
        let resp = put(&app, &id, bad, "image/png", png()).await;
        assert!(
            matches!(
                resp.status(),
                StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND
            ),
            "{bad} was accepted for writing: {}",
            resp.status()
        );
        let resp = get(&app, &id, bad).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{bad} was served");
    }
    assert!(no_screenshot_stored(dir.path(), &id));
}

#[tokio::test]
async fn bytes_that_are_not_a_png_are_refused_before_they_are_stored() {
    let dir = tempfile::tempdir().unwrap();
    let app = build_app(AppState::with_config(dir.path().to_path_buf(), 30));
    let id = create_session(&app).await;
    let (_, body) = ask(&app, &id).await;
    let url = format!(
        "/api/sessions/{id}/screenshot/{}",
        body["id"].as_str().unwrap()
    );

    let resp = app
        .clone()
        .oneshot(
            Request::put(&url)
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from("<html>not a picture</html>"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// --- With real sockets: who counts as listening ---------------------------------------

async fn start_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let (url, _) = common::serve(AppState::with_config(dir.path().to_path_buf(), 5)).await;
    (url, dir)
}

async fn create_session_over_http(url: &str) -> String {
    reqwest::Client::new()
        .post(format!("{url}/api/sessions"))
        .body("# A page worth photographing")
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

async fn ask_over_http(url: &str, id: &str) -> serde_json::Value {
    reqwest::Client::new()
        .post(format!("{url}/api/sessions/{id}/screenshot"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn connect(
    url: &str,
    id: &str,
    query: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let ws_url = format!("{}/ws/{id}{query}", url.replacen("http://", "ws://", 1));
    tokio_tungstenite::connect_async(&ws_url).await.unwrap().0
}

/// Wait for `delivered` to settle at `want`: presence is kept by the socket task, which
/// notices a connection or a hang-up a few scheduler ticks after the client does.
async fn delivered_becomes(url: &str, id: &str, want: bool) -> serde_json::Value {
    for _ in 0..100 {
        let body = ask_over_http(url, id).await;
        if body["delivered"] == serde_json::Value::Bool(want) {
            return body;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("delivered never became {want} for {id}");
}

#[tokio::test]
async fn delivered_means_a_tablet_heard_it_not_the_agents_own_observer() {
    let (url, _dir) = start_server().await;
    let id = create_session_over_http(&url).await;

    // The CLI's observer subscribes to the same socket. It cannot take a picture.
    let mut observer = connect(&url, &id, "").await;
    observer
        .send(WsMessage::Text(r#"{"type":"subscribe"}"#.into()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let body = ask_over_http(&url, &id).await;
    assert_eq!(
        body["delivered"],
        serde_json::Value::Bool(false),
        "an observer is not a tablet"
    );

    let mut tablet = connect(&url, &id, "?client=tablet").await;
    let body = delivered_becomes(&url, &id, true).await;
    let request_id = body["id"].as_str().unwrap().to_string();

    // The tablet is told which request to answer; so is the observer, harmlessly.
    let heard = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Ok(WsMessage::Text(text))) = tablet.next().await {
                let msg: serde_json::Value = serde_json::from_str(&text).unwrap();
                if msg["type"] == "screenshot_requested" {
                    break msg;
                }
            }
        }
    })
    .await
    .expect("the tablet hears the request");
    assert_eq!(heard["request_id"].as_str().unwrap(), request_id);

    // When the tablet goes, the honest answer goes back to "nobody can take this".
    tablet.close(None).await.unwrap();
    drop(tablet);
    delivered_becomes(&url, &id, false).await;
    drop(observer);
}
