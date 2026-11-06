#![allow(dead_code)]

pub mod eval;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use eink_bridge::app::{AppState, build_app};
use http_body_util::BodyExt;
use std::path::PathBuf;
use tower::ServiceExt;

/// Serve `state` on a random loopback port and wait until it answers.
///
/// With `ConnectInfo`, so a loopback caller is seen as one: without it every caller is
/// "unknown" and treated as remote, which quietly disables `/source`.
pub async fn serve(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let app = build_app(state).into_make_service_with_connect_info::<std::net::SocketAddr>();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    for _ in 0..500 {
        if reqwest::get(format!("{url}/api/health")).await.is_ok() {
            return (url, handle);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the test server never answered on {url}");
}

pub fn test_app(state_dir: PathBuf) -> Router {
    let mut state = AppState::new(state_dir);
    // Retries are real behaviour, but a test that waits 5.25 s to observe one is a test
    // that gets deleted. Keep the retry, drop the wait.
    state.webhook_backoff = [std::time::Duration::ZERO; 3];
    build_app(state)
}

pub async fn render_html(app: Router, markdown: &str) -> String {
    let create_resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from(markdown.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&create_resp.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    let id = body["id"].as_str().unwrap();

    let resp = app
        .oneshot(
            Request::get(format!("/session/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}
