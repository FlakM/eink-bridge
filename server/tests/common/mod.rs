#![allow(dead_code)]

pub mod eval;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use eink_bridge::app::{AppState, build_app};
use http_body_util::BodyExt;
use std::path::PathBuf;
use tower::ServiceExt;

pub fn test_app(state_dir: PathBuf) -> Router {
    let state = AppState::new(state_dir);
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
