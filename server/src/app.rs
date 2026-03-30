use axum::{
    Router,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
};
use serde::Deserialize;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Notify, RwLock};

use crate::render;
use crate::session::{SessionManager, SessionStatus};

#[derive(Clone)]
pub struct AppState {
    pub sessions: Arc<RwLock<SessionManager>>,
    pub notifiers: Arc<RwLock<HashMap<String, Arc<Notify>>>>,
    pub long_poll_seconds: u64,
}

impl AppState {
    pub fn new(state_dir: PathBuf) -> Self {
        Self::with_config(state_dir, 30)
    }

    pub fn with_config(state_dir: PathBuf, long_poll_seconds: u64) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(SessionManager::new(state_dir))),
            notifiers: Arc::new(RwLock::new(HashMap::new())),
            long_poll_seconds,
        }
    }

    async fn get_or_create_notify(&self, id: &str) -> Arc<Notify> {
        let mut map = self.notifiers.write().await;
        map.entry(id.to_string())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    async fn notify_session(&self, id: &str) {
        let map = self.notifiers.read().await;
        if let Some(n) = map.get(id) {
            n.notify_waiters();
        }
    }
}

pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/sessions/{id}",
            get(get_session).delete(cancel_session),
        )
        .route("/api/sessions/{id}/result", get(get_result))
        .route("/api/sessions/{id}/submit", post(submit_review))
        .route("/session/{id}", get(render_session))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Deserialize, Default)]
struct CreateParams {
    title: Option<String>,
}

async fn create_session(
    State(state): State<AppState>,
    Query(params): Query<CreateParams>,
    body: String,
) -> impl IntoResponse {
    let mut mgr = state.sessions.write().await;
    let session = mgr.create(body, params.title);
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": session.id,
            "url": format!("/session/{}", session.id),
        })),
    )
}

#[derive(Deserialize, Default)]
struct ListParams {
    status: Option<String>,
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> impl IntoResponse {
    let mgr = state.sessions.read().await;
    let all = mgr.list();
    let filtered: Vec<_> = all
        .into_iter()
        .filter(|s| match &params.status {
            Some(st) => format!("{:?}", s.status).to_lowercase() == st.to_lowercase(),
            None => true,
        })
        .map(|s| {
            serde_json::json!({
                "id": s.id,
                "title": s.title,
                "status": format!("{:?}", s.status),
                "created_at": s.created_at,
            })
        })
        .collect();
    Json(filtered)
}

async fn get_session(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        Some(s) => Ok(Json(serde_json::json!({
            "id": s.id,
            "title": s.title,
            "status": format!("{:?}", s.status),
            "created_at": s.created_at,
        }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn cancel_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mut mgr = state.sessions.write().await;
    match mgr.cancel(&id) {
        true => {
            drop(mgr);
            state.notify_session(&id).await;
            StatusCode::OK
        }
        false => StatusCode::NOT_FOUND,
    }
}

async fn get_result(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    // Check if session exists and is already resolved
    {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return Err(StatusCode::NOT_FOUND),
            Some(s) if s.status == SessionStatus::Submitted => {
                return Ok(Json(serde_json::json!({
                    "id": s.id,
                    "status": "submitted",
                    "typed_notes": s.typed_notes,
                    "annotation_images": s.annotation_images,
                })));
            }
            Some(s) if s.status == SessionStatus::Cancelled => {
                return Err(StatusCode::GONE);
            }
            Some(s) if s.status == SessionStatus::Expired => {
                return Err(StatusCode::GONE);
            }
            _ => {}
        }
    }

    let notify = state.get_or_create_notify(&id).await;
    let timeout = Duration::from_secs(state.long_poll_seconds);

    tokio::select! {
        _ = notify.notified() => {}
        _ = tokio::time::sleep(timeout) => {}
    }

    // Re-check after wake
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        Some(s) if s.status == SessionStatus::Submitted => Ok(Json(serde_json::json!({
            "id": s.id,
            "status": "submitted",
            "typed_notes": s.typed_notes,
            "annotation_images": s.annotation_images,
        }))),
        Some(s) if s.status == SessionStatus::Cancelled || s.status == SessionStatus::Expired => {
            Err(StatusCode::GONE)
        }
        Some(_) => Err(StatusCode::NO_CONTENT), // still active, timeout
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn submit_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let mut typed_notes = String::new();
    let mut images: Vec<String> = Vec::new();

    while let Ok(Some(field)) = multipart.next_field().await {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "typed_notes" => {
                typed_notes = field.text().await.unwrap_or_default();
            }
            "annotation" => {
                let data = field.bytes().await.unwrap_or_default();
                let mgr = state.sessions.read().await;
                if let Some(session) = mgr.get(&id) {
                    let img_path = session.save_annotation(&data);
                    images.push(img_path);
                }
            }
            _ => {}
        }
    }

    let mut mgr = state.sessions.write().await;
    match mgr.submit(&id, typed_notes, images) {
        true => {
            drop(mgr);
            state.notify_session(&id).await;
            StatusCode::OK
        }
        false => StatusCode::NOT_FOUND,
    }
}

async fn render_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        Some(s) => Ok(Html(render::to_eink_html(&s.content, &s.id))),
        None => Err(StatusCode::NOT_FOUND),
    }
}
