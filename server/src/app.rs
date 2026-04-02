use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post, put},
};
use futures_util::stream;
use multer::{Multipart as MulterMultipart, parse_boundary};
use serde::Deserialize;
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Notify, RwLock, broadcast};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::api::{
    CreateSessionRequest, CreateSessionResponse, SCHEMA_VERSION, SessionDetailResponse,
    SessionResultResponse, SessionSummaryResponse, SubmitReviewRequest, openapi_spec,
};
use crate::ocr::OcrEngine;
use crate::render;
use crate::session::{Session, SessionManager, SessionStatus, SubmitResult};
use crate::verdict::{Verdict, parse_verdict};

#[derive(Clone)]
pub struct AppState {
    pub sessions: Arc<RwLock<SessionManager>>,
    pub notifiers: Arc<RwLock<HashMap<String, Arc<Notify>>>>,
    pub ws_senders: Arc<RwLock<HashMap<String, broadcast::Sender<crate::ws::ServerMessage>>>>,
    pub long_poll_seconds: u64,
    pub assets_dir: PathBuf,
    pub ocr_engine: Option<Arc<OcrEngine>>,
}

impl AppState {
    pub fn new(state_dir: PathBuf) -> Self {
        Self::with_config(state_dir, 30)
    }

    pub fn with_config(state_dir: PathBuf, long_poll_seconds: u64) -> Self {
        let ocr_engine = match OcrEngine::new() {
            Ok(engine) => {
                tracing::info!("OCR engine initialized");
                Some(Arc::new(engine))
            }
            Err(e) => {
                tracing::warn!(error = %e, "OCR engine unavailable, recognized_text will not be populated");
                None
            }
        };
        Self {
            sessions: Arc::new(RwLock::new(SessionManager::new(state_dir))),
            notifiers: Arc::new(RwLock::new(HashMap::new())),
            ws_senders: Arc::new(RwLock::new(HashMap::new())),
            long_poll_seconds,
            assets_dir: default_assets_dir(),
            ocr_engine,
        }
    }

    async fn get_or_create_notify(&self, id: &str) -> Arc<Notify> {
        let mut map = self.notifiers.write().await;
        map.entry(id.to_string())
            .or_insert_with(|| Arc::new(Notify::new()))
            .clone()
    }

    async fn notify_and_cleanup(&self, id: &str) {
        let mut map = self.notifiers.write().await;
        if let Some(n) = map.remove(id) {
            n.notify_waiters();
        }
    }

    pub async fn ws_subscribe(&self, id: &str) -> broadcast::Receiver<crate::ws::ServerMessage> {
        type Map = std::collections::HashMap<String, broadcast::Sender<crate::ws::ServerMessage>>;
        let mut map: tokio::sync::RwLockWriteGuard<'_, Map> = self.ws_senders.write().await;
        map.entry(id.to_string())
            .or_insert_with(|| broadcast::channel::<crate::ws::ServerMessage>(16).0)
            .subscribe()
    }

    pub async fn ws_send(&self, id: &str, msg: crate::ws::ServerMessage) {
        type Map = std::collections::HashMap<String, broadcast::Sender<crate::ws::ServerMessage>>;
        let map: tokio::sync::RwLockReadGuard<'_, Map> = self.ws_senders.read().await;
        if let Some(sender) = map.get(id) {
            let _ = sender.send(msg as crate::ws::ServerMessage);
        }
    }

    pub async fn ws_cleanup(&self, id: &str) {
        type Map = std::collections::HashMap<String, broadcast::Sender<crate::ws::ServerMessage>>;
        let mut map: tokio::sync::RwLockWriteGuard<'_, Map> = self.ws_senders.write().await;
        map.remove(id);
    }
}

fn default_assets_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let share_assets = exe
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join("../share/eink-bridge/assets");
        if share_assets.exists() {
            return share_assets;
        }
    }
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/assets"))
}

const MAX_BODY_SIZE: usize = 10 * 1024 * 1024;

use crate::api::AnnotationGroup;

type SubmitPayload = (
    String,
    Vec<String>,
    Option<Verdict>,
    Option<serde_json::Value>,
    Vec<AnnotationGroup>,
);
type SubmitError = (StatusCode, String);

pub fn build_app(state: AppState) -> Router {
    let assets_dir = state.assets_dir.clone();
    Router::new()
        .route("/api/health", get(health))
        .route("/api/openapi.json", get(openapi_json))
        .route("/api/ocr", post(ocr_strokes))
        .route(
            "/api/sessions",
            get(list_sessions)
                .post(create_session)
                .delete(purge_sessions),
        )
        .route(
            "/api/sessions/{id}",
            get(get_session).delete(cancel_session),
        )
        .route("/api/sessions/{id}/result", get(get_result))
        .route("/api/sessions/{id}/content", put(update_content))
        .route("/api/sessions/{id}/submit", post(submit_review))
        .route("/ws/{id}", get(crate::ws::ws_handler))
        .route("/session/{id}", get(render_session))
        .nest_service("/assets", ServeDir::new(assets_dir))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

async fn openapi_json() -> Json<serde_json::Value> {
    Json(openapi_spec())
}

#[derive(Deserialize)]
struct OcrRequest {
    strokes: Vec<Vec<[f64; 2]>>,
}

async fn ocr_strokes(
    State(state): State<AppState>,
    Json(body): Json<OcrRequest>,
) -> Response {
    let Some(engine) = &state.ocr_engine else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OCR engine not available").into_response();
    };
    let strokes = body.strokes.len();
    let points: usize = body.strokes.iter().map(|s| s.len()).sum();
    tracing::info!(strokes, points, "OCR request");
    match engine.recognize_strokes(&body.strokes).await {
        Ok(text) => {
            tracing::info!(strokes, text = %text, "OCR done");
            Json(serde_json::json!({ "text": text })).into_response()
        }
        Err(e) => {
            tracing::warn!(strokes, error = %e, "OCR failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}

#[derive(Deserialize, Default)]
struct CreateParams {
    title: Option<String>,
    callback_url: Option<String>,
}

async fn create_session(
    State(state): State<AppState>,
    Query(params): Query<CreateParams>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let content_type = content_type(&headers);
    let request = if content_type.starts_with("application/json") {
        match serde_json::from_slice::<CreateSessionRequest>(&body) {
            Ok(mut request) => {
                if request.schema_version.is_none() {
                    request.schema_version = Some(SCHEMA_VERSION);
                }
                if request.title.is_none() {
                    request.title = params.title.clone();
                }
                if request.callback_url.is_none() {
                    request.callback_url = params.callback_url.clone();
                }
                request
            }
            Err(error) => return bad_request(format!("invalid create-session JSON: {error}")),
        }
    } else {
        let content = match String::from_utf8(body.to_vec()) {
            Ok(content) => content,
            Err(error) => return bad_request(format!("request body must be valid UTF-8: {error}")),
        };
        CreateSessionRequest {
            schema_version: Some(SCHEMA_VERSION),
            title: params.title,
            content,
            callback_url: params.callback_url,
            tags: HashMap::new(),
        }
    };

    let content_len = request.content.len();
    let mut mgr = state.sessions.write().await;
    let session = mgr.create(
        request.content,
        request.title,
        request.callback_url,
        request.tags,
    );
    tracing::info!(
        id = %session.id,
        title = ?session.title,
        content_bytes = content_len,
        "session created"
    );

    (
        StatusCode::CREATED,
        Json(CreateSessionResponse {
            schema_version: SCHEMA_VERSION,
            id: session.id.clone(),
            url: format!("/session/{}", session.id),
        }),
    )
        .into_response()
}

#[derive(Deserialize, Default)]
struct ListParams {
    status: Option<String>,
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Json<Vec<SessionSummaryResponse>> {
    let mgr = state.sessions.read().await;
    let all = mgr.list();
    let mut filtered: Vec<_> = all
        .into_iter()
        .filter(|s| match &params.status {
            Some(st) => s.status.as_str().eq_ignore_ascii_case(st),
            None => true,
        })
        .collect();
    filtered.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Json(
        filtered
            .iter()
            .map(|s| SessionSummaryResponse::from_session(s))
            .collect(),
    )
}

async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionDetailResponse>, StatusCode> {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        Some(s) => Ok(Json(SessionDetailResponse::from_session(s))),
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
            tracing::info!(id = %id, "session cancelled");
            let webhook = mgr.get(&id).and_then(webhook_payload);
            drop(mgr);
            state.notify_and_cleanup(&id).await;
            state
                .ws_send(
                    &id,
                    crate::ws::ServerMessage::Error {
                        message: "session cancelled".into(),
                    },
                )
                .await;
            state.ws_cleanup(&id).await;
            if let Some((url, body)) = webhook {
                fire_webhook(url, body);
            }
            StatusCode::OK
        }
        false => StatusCode::NOT_FOUND,
    }
}

async fn purge_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    let count = {
        let mut mgr = state.sessions.write().await;
        mgr.purge_finished()
    };
    tracing::info!(count, "sessions purged");
    Json(serde_json::json!({ "purged": count }))
}

async fn update_content(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let content = match String::from_utf8(body.to_vec()) {
        Ok(s) => s,
        Err(_) => return bad_request("request body must be valid UTF-8".into()),
    };
    let version = {
        let mut mgr = state.sessions.write().await;
        mgr.update_content(&id, content)
    };
    match version {
        Some(v) => {
            tracing::info!(id = %id, version = v, "content updated via HTTP PUT");
            state
                .ws_send(&id, crate::ws::ServerMessage::VersionUpdated { version: v })
                .await;
            Json(serde_json::json!({ "version": v })).into_response()
        }
        None => {
            let mgr = state.sessions.read().await;
            if mgr.get(&id).is_none() {
                StatusCode::NOT_FOUND.into_response()
            } else {
                StatusCode::CONFLICT.into_response()
            }
        }
    }
}

fn webhook_payload(session: &Session) -> Option<(String, SessionResultResponse)> {
    session
        .callback_url
        .clone()
        .map(|url| (url, SessionResultResponse::from_session(session)))
}

fn fire_webhook(callback_url: String, body: SessionResultResponse) {
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        if let Err(error) = client.post(&callback_url).json(&body).send().await {
            tracing::warn!("webhook POST to {callback_url} failed: {error}");
        }
    });
}

async fn get_result(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionResultResponse>, StatusCode> {
    {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return Err(StatusCode::NOT_FOUND),
            Some(s) if s.status == SessionStatus::Submitted => {
                return Ok(Json(SessionResultResponse::from_session(s)));
            }
            Some(s) if matches!(s.status, SessionStatus::Cancelled | SessionStatus::Expired) => {
                return Err(StatusCode::GONE);
            }
            _ => {}
        }
    }

    tracing::debug!(id = %id, "long-poll waiting");
    let notify = state.get_or_create_notify(&id).await;
    tokio::select! {
        _ = notify.notified() => {
            tracing::debug!(id = %id, "long-poll: notified");
        }
        _ = tokio::time::sleep(Duration::from_secs(state.long_poll_seconds)) => {
            tracing::debug!(id = %id, "long-poll: timeout");
        }
    }

    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        Some(s) if s.status == SessionStatus::Submitted => {
            Ok(Json(SessionResultResponse::from_session(s)))
        }
        Some(s) if matches!(s.status, SessionStatus::Cancelled | SessionStatus::Expired) => {
            Err(StatusCode::GONE)
        }
        Some(_) => Err(StatusCode::NO_CONTENT),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn submit_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(session) = ({
        let mgr = state.sessions.read().await;
        mgr.get(&id).cloned()
    }) else {
        tracing::debug!(id = %id, "submit: session not found");
        return StatusCode::NOT_FOUND.into_response();
    };

    let content_type = content_type(&headers);
    let parsed = if content_type.starts_with("application/json") {
        parse_json_submit(&body)
    } else if content_type.starts_with("multipart/form-data") {
        parse_multipart_submit(&session, &headers, body).await
    } else {
        Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("unsupported content-type: {content_type}"),
        ))
    };

    let (typed_notes, images, verdict_override, stroke_data, annotations) = match parsed {
        Ok(parsed) => parsed,
        Err((status, message)) => return (status, message).into_response(),
    };

    let mut mgr = state.sessions.write().await;
    match mgr.submit(
        &id,
        typed_notes,
        images,
        verdict_override,
        stroke_data,
        annotations,
    ) {
        SubmitResult::Ok => {
            let webhook = mgr.get(&id).and_then(webhook_payload);
            let verdict = mgr
                .get(&id)
                .and_then(|s| s.verdict.as_ref().map(|v| v.as_str().to_string()));
            let image_count = mgr.get(&id).map(|s| s.annotation_images.len()).unwrap_or(0);
            let ws_result = mgr.get(&id).map(SessionResultResponse::from_session);
            drop(mgr);
            tracing::info!(id = %id, ?verdict, images = image_count, "review submitted");
            state.notify_and_cleanup(&id).await;
            if let Some(result) = ws_result {
                state
                    .ws_send(
                        &id,
                        crate::ws::ServerMessage::SessionSubmitted {
                            result: Box::new(result),
                        },
                    )
                    .await;
                state.ws_cleanup(&id).await;
            }
            if let Some((url, body)) = webhook {
                fire_webhook(url, body);
            }
            StatusCode::OK.into_response()
        }
        SubmitResult::NotFound => {
            tracing::debug!(id = %id, "submit: session not found");
            StatusCode::NOT_FOUND.into_response()
        }
        SubmitResult::NotActive => {
            tracing::debug!(id = %id, "submit: session not active");
            StatusCode::CONFLICT.into_response()
        }
    }
}

fn parse_json_submit(body: &[u8]) -> Result<SubmitPayload, SubmitError> {
    let request: SubmitReviewRequest = serde_json::from_slice(body).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid submit-review JSON: {error}"),
        )
    })?;
    let verdict_override = parse_verdict_override(request.verdict)?;
    Ok((
        request.typed_notes,
        Vec::new(),
        verdict_override,
        None,
        request.annotations,
    ))
}

async fn parse_multipart_submit(
    session: &Session,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<SubmitPayload, SubmitError> {
    let content_type = content_type(headers);
    let boundary = parse_boundary(content_type).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid multipart boundary: {error}"),
        )
    })?;
    let stream = stream::once(async move { Ok::<Bytes, std::io::Error>(body) });
    let mut multipart = MulterMultipart::new(stream, boundary);
    let mut typed_notes = String::new();
    let mut images = Vec::new();
    let mut stroke_data: Option<serde_json::Value> = None;
    let mut annotations: Vec<AnnotationGroup> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("failed to parse multipart body: {error}"),
        )
    })? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "typed_notes" => {
                typed_notes = field.text().await.map_err(|error| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("failed to read typed_notes: {error}"),
                    )
                })?;
            }
            "annotation" => {
                let data = field.bytes().await.map_err(|error| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("failed to read annotation: {error}"),
                    )
                })?;
                images.push(session.save_annotation(&data));
            }
            "stroke_data" => {
                let text = field.text().await.map_err(|error| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("failed to read stroke_data: {error}"),
                    )
                })?;
                if !text.is_empty() {
                    stroke_data = serde_json::from_str(&text).ok();
                }
            }
            "annotations" => {
                let text = field.text().await.map_err(|error| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("failed to read annotations: {error}"),
                    )
                })?;
                if !text.is_empty() {
                    annotations = serde_json::from_str(&text).map_err(|error| {
                        (
                            StatusCode::BAD_REQUEST,
                            format!("invalid annotations JSON: {error}"),
                        )
                    })?;
                }
            }
            _ => {}
        }
    }

    Ok((typed_notes, images, None, stroke_data, annotations))
}

fn parse_verdict_override(verdict: Option<String>) -> Result<Option<Verdict>, SubmitError> {
    match verdict {
        Some(verdict_text) if verdict_text.trim().is_empty() => Ok(None),
        Some(verdict_text) => parse_verdict(&verdict_text)
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("invalid verdict: {verdict_text}"),
                )
            })
            .map(Some),
        None => Ok(None),
    }
}

fn content_type(headers: &HeaderMap) -> &str {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("text/plain")
}

fn bad_request(message: String) -> Response {
    (StatusCode::BAD_REQUEST, message).into_response()
}

async fn render_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        Some(s) => {
            tracing::debug!(id = %id, "rendering session HTML");
            Ok(Html(render::to_eink_html(&s.content, &s.id)))
        }
        None => {
            tracing::debug!(id = %id, "render: session not found");
            Err(StatusCode::NOT_FOUND)
        }
    }
}
