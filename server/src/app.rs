use axum::{
    Router,
    body::Bytes,
    extract::{MatchedPath, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
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
    SessionResultResponse, SessionSummaryResponse, SetStarredRequest, SubmitReviewRequest,
    openapi_spec,
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
    /// Seconds between WebSocket pings; 0 disables the heartbeat. A client that says
    /// nothing for three intervals is dropped, which is what makes a half-open socket
    /// visible instead of silently dead.
    pub ws_heartbeat_seconds: u64,
}

impl AppState {
    pub fn new(state_dir: PathBuf) -> Self {
        Self::with_config(state_dir, 30)
    }

    pub fn with_config(state_dir: PathBuf, long_poll_seconds: u64) -> Self {
        let ws_heartbeat_seconds = 30;
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
        let mgr = SessionManager::new(state_dir);
        let active = mgr
            .list()
            .iter()
            .filter(|s| s.status == SessionStatus::Active)
            .count() as i64;
        crate::metrics::SESSIONS_ACTIVE.set(active);
        crate::metrics::SESSIONS_STORED.set(mgr.count() as i64);
        crate::metrics::SESSIONS_STARRED.set(mgr.starred_count() as i64);
        Self {
            sessions: Arc::new(RwLock::new(mgr)),
            notifiers: Arc::new(RwLock::new(HashMap::new())),
            ws_senders: Arc::new(RwLock::new(HashMap::new())),
            long_poll_seconds,
            assets_dir: default_assets_dir(),
            ocr_engine,
            ws_heartbeat_seconds,
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

    pub async fn expire_stale(&self, timeout: Duration) -> usize {
        let expired = self.sessions.write().await.expire_stale_ids(timeout);
        for id in &expired {
            self.notify_and_cleanup(id).await;
            self.ws_send(
                id,
                crate::ws::ServerMessage::Error {
                    message: "session expired".into(),
                },
            )
            .await;
            self.ws_cleanup(id).await;
        }
        expired.len()
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
    // Export the metrics from the first scrape rather than from the first event: a
    // labelled series has no children until something sets a label, so a quiet server
    // would answer /metrics with an empty document.
    crate::metrics::init();
    let assets_dir = state.assets_dir.clone();
    Router::new()
        .route("/metrics", get(metrics_handler))
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
            get(get_session).delete(delete_session),
        )
        .route("/api/sessions/{id}/result", get(get_result))
        .route(
            "/api/sessions/{id}/content",
            get(get_content).put(update_content),
        )
        .route("/api/sessions/{id}/reanchor", post(reanchor))
        .route("/api/sessions/{id}/feedback", get(pending_feedback))
        .route("/api/sessions/{id}/star", put(set_starred))
        .route(
            "/api/sessions/{id}/annotation/{index}",
            get(annotation_image),
        )
        .route("/api/sessions/{id}/submit", post(submit_review))
        .route("/api/sessions/{id}/request_update", post(request_update))
        .route("/ws/{id}", get(crate::ws::ws_handler))
        .route("/session/{id}", get(render_session))
        .route("/review/{id}", get(render_review))
        .route_layer(axum::middleware::from_fn(track_metrics))
        .nest_service("/assets", ServeDir::new(assets_dir))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY_SIZE))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn metrics_handler() -> impl IntoResponse {
    let body = crate::metrics::render();
    (
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        body,
    )
}

async fn track_metrics(req: Request, next: Next) -> Response {
    let method = req.method().as_str().to_string();
    let path = req
        .extensions()
        .get::<MatchedPath>()
        .map(|mp| mp.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let start = std::time::Instant::now();
    let resp = next.run(req).await;
    let status = resp.status().as_u16().to_string();
    let dur = start.elapsed().as_secs_f64();
    crate::metrics::HTTP_REQUESTS
        .with_label_values(&[&method, &path, &status])
        .inc();
    crate::metrics::HTTP_DURATION
        .with_label_values(&[&method, &path])
        .observe(dur);
    resp
}

/// Whether the server can actually write where it keeps sessions.
///
/// A permission bit is not the answer: a path that is a regular file looks writable by
/// its mode while nothing can be created inside it, and a read-only mount keeps the
/// mode of the directory underneath. Creating a file is the only honest probe.
fn state_dir_is_writable(state_dir: &std::path::Path) -> bool {
    let probe = state_dir.join(format!(".write-probe-{}", std::process::id()));
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "unknown".to_string());

    // "ok" only says the process is up. What actually fails is the OCR engine, the disk,
    // and the shape of the session store, so say those too rather than making the reader
    // guess from symptoms.
    let (stored, active, state_dir) = {
        let mgr = state.sessions.read().await;
        let stored = mgr.count();
        let active = mgr
            .list()
            .iter()
            .filter(|s| s.status == crate::session::SessionStatus::Active)
            .count();
        (stored, active, mgr.state_dir().to_path_buf())
    };
    let writable = state_dir_is_writable(&state_dir);

    Json(serde_json::json!({
        "status": "ok",
        "exe": exe,
        "ready": {
            "version": env!("CARGO_PKG_VERSION"),
            "ocr": if state.ocr_engine.is_some() { "available" } else { "unavailable" },
            "state_dir": state_dir.to_string_lossy(),
            "state_dir_writable": writable,
            "sessions": stored,
            "active": active,
            "ws_heartbeat_seconds": state.ws_heartbeat_seconds,
        },
    }))
}

async fn openapi_json() -> Json<serde_json::Value> {
    Json(openapi_spec())
}

#[derive(Deserialize)]
struct OcrRequest {
    strokes: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    pressures: Vec<Vec<f64>>,
}

async fn ocr_strokes(State(state): State<AppState>, Json(body): Json<OcrRequest>) -> Response {
    let Some(engine) = &state.ocr_engine else {
        return (StatusCode::SERVICE_UNAVAILABLE, "OCR engine not available").into_response();
    };
    let strokes = body.strokes.len();
    let points: usize = body.strokes.iter().map(|s| s.len()).sum();
    tracing::info!(strokes, points, "OCR request");
    match engine
        .recognize_strokes(&body.strokes, &body.pressures)
        .await
    {
        Ok(text) => {
            tracing::info!(strokes, text_bytes = text.len(), "OCR done");
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
            starred: false,
            origin: None,
        }
    };

    let content_len = request.content.len();
    let mut mgr = state.sessions.write().await;
    let session = mgr.create(
        request.content,
        request.title,
        request.callback_url,
        request.tags,
        request.starred,
        request.origin,
    );
    tracing::info!(
        id = %session.id,
        title = ?session.title,
        content_bytes = content_len,
        "session created"
    );
    crate::metrics::SESSIONS_CREATED.inc();
    crate::metrics::SESSIONS_ACTIVE.inc();
    crate::metrics::SESSIONS_STORED.set(mgr.count() as i64);
    crate::metrics::SESSIONS_STARRED.set(mgr.starred_count() as i64);
    crate::metrics::SESSION_CONTENT_BYTES.observe(content_len as f64);

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
    starred: Option<bool>,
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
        .filter(|s| match params.starred {
            Some(want) => s.starred == want,
            None => true,
        })
        .collect();
    filtered.sort_by(|a, b| {
        b.starred
            .cmp(&a.starred)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
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

#[derive(Deserialize)]
struct DeleteParams {
    /// `?remove=true` forgets the session and deletes its files; the default only
    /// cancels it. `eink-review remove` sends this, and without it the command silently
    /// cancelled instead, leaving the review and its images on disk for good.
    #[serde(default)]
    remove: bool,
    /// Removing a session that is still being read destroys work in progress, so it
    /// takes a second, deliberate word.
    #[serde(default)]
    force: bool,
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteParams>,
) -> Response {
    if params.remove {
        if !params.force {
            let live = {
                let mgr = state.sessions.read().await;
                mgr.get(&id)
                    .is_some_and(|s| s.status == crate::session::SessionStatus::Active)
            };
            if live {
                return (
                    StatusCode::CONFLICT,
                    "this review is still being read; cancel it first, or pass force=true",
                )
                    .into_response();
            }
        }
        let removed = state.sessions.write().await.delete(&id);
        if !removed {
            return StatusCode::NOT_FOUND.into_response();
        }
        // A tablet or observer still holding the id must stop waiting on it.
        state.notify_and_cleanup(&id).await;
        state.ws_cleanup(&id).await;
        let mgr = state.sessions.read().await;
        crate::metrics::SESSIONS_STORED.set(mgr.count() as i64);
        crate::metrics::SESSIONS_STARRED.set(mgr.starred_count() as i64);
        tracing::info!(id = %id, "session removed");
        return StatusCode::OK.into_response();
    }
    let (was_active, webhook) = {
        let mut mgr = state.sessions.write().await;
        if mgr.get(&id).is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }
        let was_active = mgr.cancel(&id);
        if was_active {
            crate::metrics::SESSIONS_CANCELLED.inc();
            crate::metrics::SESSIONS_ACTIVE.dec();
        }
        let webhook = was_active
            .then(|| mgr.get(&id).and_then(webhook_payload))
            .flatten();
        tracing::info!(id = %id, was_active, "session cancelled");
        (was_active, webhook)
    };
    state.notify_and_cleanup(&id).await;
    if was_active {
        state
            .ws_send(
                &id,
                crate::ws::ServerMessage::Error {
                    message: "session cancelled".into(),
                },
            )
            .await;
        if let Some((url, body)) = webhook {
            fire_webhook(url, body);
        }
    }
    state.ws_cleanup(&id).await;
    StatusCode::OK.into_response()
}

async fn purge_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (count, stored, starred) = {
        let mut mgr = state.sessions.write().await;
        let n = mgr.purge_finished();
        (n, mgr.count(), mgr.starred_count())
    };
    crate::metrics::SESSIONS_STORED.set(stored as i64);
    crate::metrics::SESSIONS_STARRED.set(starred as i64);
    tracing::info!(count, "sessions purged");
    Json(serde_json::json!({ "purged": count }))
}

#[derive(Deserialize)]
struct UpdateContentBody {
    content: String,
    #[serde(default)]
    consumed_annotations: Vec<i32>,
    #[serde(default)]
    expected_version: Option<u32>,
    #[serde(default)]
    feedback_id: Option<String>,
}

#[derive(Deserialize)]
struct ReanchorBody {
    previous: Vec<crate::reanchor::Element>,
    current: Vec<crate::reanchor::Element>,
}

async fn pending_feedback(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(s) => Json(serde_json::json!({"status":s.status,"version":s.version,"pending":s.pending_feedback,"last_content_update":s.last_content_update})).into_response(),
    }
}

/// Resolve a recorded annotation path, refusing anything outside that session's own
/// annotations directory.
///
/// The names are written by `save_annotation`, so a value that escapes means the session
/// file was edited by hand -- and this is the one place a client-supplied index reaches
/// the filesystem. Canonicalising resolves symlinks before the containment check, which
/// is why the check is on the resolved path and not on the string.
pub fn resolve_annotation_path(
    state_dir: &std::path::Path,
    recorded: &str,
) -> Option<std::path::PathBuf> {
    let root = state_dir.join("annotations").canonicalize().ok()?;
    let candidate = std::path::Path::new(recorded).canonicalize().ok()?;
    candidate.starts_with(&root).then_some(candidate)
}

async fn annotation_image(
    State(state): State<AppState>,
    Path((id, index)): Path<(String, usize)>,
) -> Response {
    let session = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            Some(session) => session.clone(),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let Some(recorded) = session.annotation_images.get(index) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(path) = resolve_annotation_path(&session.state_dir, recorded) else {
        tracing::warn!(id = %id, index, "refused an annotation path outside the session");
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&path).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn get_content(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(s) => {
            Json(serde_json::json!({"status":s.status,"version":s.version,"content":s.content}))
                .into_response()
        }
    }
}

async fn reanchor(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ReanchorBody>,
) -> Response {
    if state.sessions.read().await.get(&id).is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Bound the quadratic edit-distance workload independently of the HTTP body limit.
    if body.previous.len() > 1000
        || body.current.len() > 1000
        || body
            .previous
            .iter()
            .chain(&body.current)
            .any(|e| e.text.chars().count() > 150)
    {
        return bad_request("element map exceeds reanchor limits".into());
    }
    let targets = tokio::task::spawn_blocking(move || {
        crate::reanchor::targets(&body.previous, &body.current)
    })
    .await;
    match targets {
        Ok(targets) => Json(serde_json::json!({"targets":targets})).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn update_content(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let (content, consumed_annotations, expected_version, feedback_id) = if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("application/json"))
    {
        match serde_json::from_slice::<UpdateContentBody>(&body) {
            Ok(b) => (
                b.content,
                b.consumed_annotations,
                b.expected_version,
                b.feedback_id,
            ),
            Err(e) => return bad_request(format!("invalid JSON: {e}")),
        }
    } else {
        match String::from_utf8(body.to_vec()) {
            Ok(s) => (s, vec![], None, None),
            Err(_) => return bad_request("request body must be valid UTF-8".into()),
        }
    };
    let event = {
        let mut mgr = state.sessions.write().await;
        let Some(s) = mgr.get(&id) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        if expected_version.is_some_and(|v| v != s.version) {
            return StatusCode::CONFLICT.into_response();
        }
        let pending = s.pending_feedback.as_ref();
        let acknowledge = feedback_id
            .as_ref()
            .is_some_and(|id| pending.and_then(|p| p["feedback_id"].as_str()) == Some(id.as_str()));
        if (feedback_id.is_some() || !consumed_annotations.is_empty()) && !acknowledge {
            return bad_request("consuming annotations requires the matching feedback_id".into());
        }
        let annotations = pending.and_then(|p| p["annotations"].as_array());
        let mut consumed_strokes = Vec::new();
        for &index in &consumed_annotations {
            let Some(annotation) = usize::try_from(index)
                .ok()
                .and_then(|i| annotations.and_then(|a| a.get(i)))
            else {
                return bad_request("consumed annotation index is out of range".into());
            };
            if let Some(strokes) = annotation.get("strokes") {
                consumed_strokes.extend(
                    serde_json::from_value::<Vec<Vec<[f64; 2]>>>(strokes.clone())
                        .unwrap_or_default(),
                );
            }
        }
        mgr.update_content(&id, content).map(|version| {
            let event = crate::ws::ServerMessage::VersionUpdated {
                version,
                consumed_annotations,
                consumed_strokes,
            };
            mgr.record_content_update(&id, serde_json::to_value(&event).unwrap(), acknowledge);
            event
        })
    };
    match event {
        Some(event) => {
            let crate::ws::ServerMessage::VersionUpdated { version: v, .. } = &event else {
                unreachable!()
            };
            let v = *v;
            state.ws_send(&id, event).await;
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

async fn set_starred(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SetStarredRequest>,
) -> Response {
    let mut mgr = state.sessions.write().await;
    if !mgr.set_starred(&id, body.starred) {
        return StatusCode::NOT_FOUND.into_response();
    }
    crate::metrics::SESSIONS_STARRED.set(mgr.starred_count() as i64);
    let response = mgr
        .get(&id)
        .map(SessionDetailResponse::from_session)
        .expect("session must exist after set_starred returned true");
    tracing::info!(id = %id, starred = body.starred, "session star toggled");
    Json(response).into_response()
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
        match client.post(&callback_url).json(&body).send().await {
            Ok(resp) if resp.status().is_success() => {
                crate::metrics::WEBHOOK_TOTAL
                    .with_label_values(&["ok"])
                    .inc();
            }
            Ok(resp) => {
                tracing::warn!("webhook POST to {callback_url} returned {}", resp.status());
                crate::metrics::WEBHOOK_TOTAL
                    .with_label_values(&["error"])
                    .inc();
            }
            Err(error) => {
                tracing::warn!("webhook POST to {callback_url} failed: {error}");
                crate::metrics::WEBHOOK_TOTAL
                    .with_label_values(&["error"])
                    .inc();
            }
        }
    });
}

async fn get_result(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionResultResponse>, StatusCode> {
    let notified = {
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
        let notify = state.get_or_create_notify(&id).await;
        let mut notified = Box::pin(notify.notified_owned());
        notified.as_mut().enable();
        notified
    };

    tracing::debug!(id = %id, "long-poll waiting");
    let poll_outcome = tokio::select! {
        _ = notified => {
            tracing::debug!(id = %id, "long-poll: notified");
            "notified"
        }
        _ = tokio::time::sleep(Duration::from_secs(state.long_poll_seconds)) => {
            tracing::debug!(id = %id, "long-poll: timeout");
            "timeout"
        }
    };
    crate::metrics::LONG_POLL_TOTAL
        .with_label_values(&[poll_outcome])
        .inc();

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
            crate::metrics::SESSIONS_SUBMITTED.inc();
            crate::metrics::SESSIONS_ACTIVE.dec();
            crate::metrics::SESSION_ANNOTATION_IMAGES.observe(image_count as f64);
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

async fn request_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SubmitReviewRequest>,
) -> Response {
    let version = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(s) if s.status != crate::session::SessionStatus::Active => {
                return StatusCode::CONFLICT.into_response();
            }
            Some(s) => {
                if body.document_version.is_some_and(|v| v != s.version) {
                    return StatusCode::CONFLICT.into_response();
                }
                s.version
            }
        }
    };

    let mut annotations = body.annotations;

    if let Some(engine) = &state.ocr_engine {
        let ocr_start = std::time::Instant::now();
        let needs_ocr_count = annotations
            .iter()
            .filter(|g| g.recognized_text.is_none() && !g.strokes.is_empty())
            .count();
        tracing::debug!(id = %id, total = annotations.len(), needs_ocr = needs_ocr_count, "request_update: starting OCR");
        let futs: Vec<_> = annotations
            .iter()
            .map(|g| {
                let engine = Arc::clone(engine);
                let strokes = g.strokes.clone();
                let pressures = g.pressures.clone();
                let needs_ocr = g.recognized_text.is_none() && !strokes.is_empty();
                async move {
                    if needs_ocr {
                        engine
                            .recognize_strokes_within(
                                &strokes,
                                &pressures,
                                engine.interactive_timeout(),
                            )
                            .await
                            .ok()
                    } else {
                        None
                    }
                }
            })
            .collect();
        let results = futures_util::future::join_all(futs).await;
        for (ann, text) in annotations.iter_mut().zip(results) {
            if let Some(t) = text {
                ann.recognized_text = Some(t);
            }
        }
        let ocr_ms = ocr_start.elapsed().as_millis();
        tracing::debug!(id = %id, ocr_ms = ocr_ms, "request_update: OCR complete");
    }

    tracing::info!(id = %id, version, count = annotations.len(), "annotation_result broadcast");
    let msg = crate::ws::ServerMessage::AnnotationResult {
        feedback_id: uuid::Uuid::new_v4().to_string(),
        version,
        annotations,
    };
    // Persist before acknowledging or broadcasting, including when no agent is
    // connected. Reject OCR that finished after the document changed.
    if !state.sessions.write().await.record_feedback(
        &id,
        version,
        serde_json::to_value(&msg).unwrap(),
    ) {
        return StatusCode::CONFLICT.into_response();
    }
    let callback_url = {
        let mgr = state.sessions.read().await;
        mgr.get(&id).and_then(|s| s.callback_url.clone())
    };
    state.ws_send(&id, msg.clone()).await;
    if let Some(url) = callback_url {
        let (feedback_id, version, annotations) = match msg {
            crate::ws::ServerMessage::AnnotationResult {
                feedback_id,
                version,
                annotations,
            } => (feedback_id, version, annotations),
            _ => unreachable!(),
        };
        tokio::spawn(async move {
            #[derive(serde::Serialize)]
            struct AnnotationWebhook {
                #[serde(rename = "type")]
                r#type: &'static str,
                id: String,
                version: u32,
                feedback_id: String,
                annotations: Vec<crate::api::AnnotationGroup>,
            }
            let client = reqwest::Client::new();
            let _ = client
                .post(&url)
                .json(&AnnotationWebhook {
                    r#type: "annotation_result",
                    id,
                    version,
                    feedback_id,
                    annotations,
                })
                .send()
                .await;
        });
    }
    StatusCode::OK.into_response()
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
    let mut verdict_text: Option<String> = None;

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
            // The JSON path already accepts an explicit verdict; a multipart post that
            // carries one must not have it silently dropped, and an unusable tag must be
            // reported rather than ignored.
            "verdict" => {
                let text = field.text().await.map_err(|error| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("failed to read verdict: {error}"),
                    )
                })?;
                verdict_text = Some(text);
            }
            _ => {}
        }
    }

    let verdict = parse_verdict_override(verdict_text)?;
    Ok((typed_notes, images, verdict, stroke_data, annotations))
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
            Ok(Html(render::to_eink_html(&s.content, &s.id).replace(
                "</head>",
                &format!(
                    "<meta name=\"eink-version\" content=\"{}\"></head>",
                    s.version
                ),
            )))
        }
        None => {
            tracing::debug!(id = %id, "render: session not found");
            Err(StatusCode::NOT_FOUND)
        }
    }
}

async fn render_review(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        Some(s) => {
            tracing::debug!(id = %id, "rendering browser review HTML");
            Ok(Html(render::to_browser_html(&s.content, &s.id).replace(
                "</head>",
                &format!(
                    "<meta name=\"eink-version\" content=\"{}\"></head>",
                    s.version
                ),
            )))
        }
        None => {
            tracing::debug!(id = %id, "review: session not found");
            Err(StatusCode::NOT_FOUND)
        }
    }
}
