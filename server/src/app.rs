use axum::{
    Router,
    body::Bytes,
    extract::{ConnectInfo, FromRequestParts, MatchedPath, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header, request::Parts},
    middleware::Next,
    response::{Html, IntoResponse, Json, Response},
    routing::{get, post, put},
};
use futures_util::stream;
use multer::{Multipart as MulterMultipart, parse_boundary};
use serde::Deserialize;
use std::{
    collections::HashMap,
    convert::Infallible,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Notify, RwLock, broadcast};
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;

use crate::api::{
    CreateSessionRequest, CreateSessionResponse, SCHEMA_VERSION, SessionDetailResponse,
    SessionResultResponse, SessionSummaryResponse, SetStarredRequest, SubmitReviewRequest,
    openapi_spec,
};
use crate::images::{alt_from_name, bytes_are_image, document_image_type, format_label};
use crate::ocr::OcrEngine;
use crate::render;
use crate::session::{NewSession, Session, SessionManager, SessionStatus, SubmitResult};
use crate::verdict::{Verdict, parse_verdict};
use crate::ws::ServerMessage;

mod auth;
mod feedback;
mod media;
mod multipart;
mod sessions;
mod source;

use auth::*;
use feedback::*;
use media::*;
use multipart::*;
use sessions::*;
use source::*;

pub use media::resolve_annotation_path;
pub use sessions::validate_callback_url;

type WsSenders = HashMap<String, broadcast::Sender<ServerMessage>>;

#[derive(Clone)]
pub struct AppState {
    pub sessions: Arc<RwLock<SessionManager>>,
    /// The queue the session writes go through, so the session lock is never held while
    /// the disk is being written.
    write_tx: tokio::sync::mpsc::UnboundedSender<crate::session::WriteRequest>,
    pub notifiers: Arc<RwLock<HashMap<String, Arc<Notify>>>>,
    pub ws_senders: Arc<RwLock<WsSenders>>,
    pub long_poll_seconds: u64,
    pub assets_dir: PathBuf,
    pub ocr_engine: Option<Arc<OcrEngine>>,
    /// How often a WebSocket is pinged; zero disables the heartbeat. A client that says
    /// nothing for three intervals is dropped, which is what makes a half-open socket
    /// visible instead of silently dead. A duration rather than whole seconds so a test
    /// can watch the same decision without spending seven seconds on it.
    pub ws_heartbeat: Duration,
    /// The one client every webhook goes out through, with a bound: a receiver that
    /// never answers used to hold a task forever.
    pub http: reqwest::Client,
    /// Where session directories live, for the retention sweep and the disk gauge.
    pub state_dir: PathBuf,
    /// Days a terminal, unstarred session is kept before its directory is removed.
    pub retention_days: i64,
    /// When set, a caller that is not on this machine must present it. Loopback stays
    /// open so the CLI and the tablet on the same host need no configuration.
    pub token: Option<String>,
    /// Waits after a failed webhook attempt. Injectable so a test can prove the retry
    /// happens without spending five seconds doing it.
    pub webhook_backoff: [Duration; 3],
    /// Directories `/source` may read from, from `EINK_SOURCE_ROOTS`. `None` means any
    /// directory a local creator names; a non-local creator never gets one.
    pub source_roots: Option<Vec<PathBuf>>,
    /// How long a reanchor may compute before the page is told to keep its notes put.
    pub reanchor_budget: Duration,
    /// The newest screenshot request per session. A second ask while one is pending
    /// reuses the id and re-broadcasts it, so an agent that retries (or a tablet that
    /// reconnected) does not flash the reader twice; an answered or failed request is
    /// never reused, because "show me again" wants a new picture.
    pub screenshot_requests: Arc<RwLock<HashMap<String, ScreenshotRequest>>>,
    /// Tablets connected per session, kept by the WebSocket handler. An observer (the CLI,
    /// the channel) subscribes to the same socket, so the broadcast's receiver count
    /// cannot say whether anyone can answer a screenshot request; this can.
    pub tablets: Arc<std::sync::Mutex<HashMap<String, usize>>>,
    /// Speech: the recogniser the tablet streams to, the cleaner, and the voice that
    /// reads a note back. None when no recogniser is configured.
    pub voice: Option<Arc<crate::voice::Voice>>,
    /// Sessions with a voice capture open right now; a second ask is a 409.
    pub captures: Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
}

/// A screenshot the agent asked for, from the ask until the tablet answers.
#[derive(Debug, Clone)]
pub struct ScreenshotRequest {
    pub id: String,
    pub asked_at: tokio::time::Instant,
    /// Why the tablet could not take the picture, when it said so.
    pub failure: Option<String>,
}

impl ScreenshotRequest {
    /// A repeat ask reuses a request that is still waiting for its answer, and only for
    /// [`SCREENSHOT_COALESCE`]: past that, the tablet is not going to answer it.
    fn is_pending(&self) -> bool {
        self.failure.is_none() && self.asked_at.elapsed() < SCREENSHOT_COALESCE
    }
}

/// How long a webhook receiver has to answer.
const WEBHOOK_TIMEOUT: Duration = Duration::from_secs(10);

/// The most strokes, and the most points over all of them, one request may ask to be
/// rasterised. Beyond this the request is refused rather than rendered.
pub const MAX_OCR_STROKES: usize = 5_000;
pub const MAX_OCR_POINTS: usize = 500_000;

/// Where a request came from, when the server knows: the connection's peer address, or
/// nothing when the app is driven without one (in-process tests). Unknown is treated as
/// remote, so a path that is only for this machine fails closed.
#[derive(Clone, Copy, Debug)]
pub struct Caller(pub Option<IpAddr>);

impl Caller {
    pub fn local(ip: IpAddr) -> Self {
        Self(Some(ip))
    }

    pub fn is_local(&self) -> bool {
        self.0.is_some_and(|ip| ip.is_loopback())
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        // An injected `Caller` wins, which is how a test speaks for a local client; the
        // server itself only ever sets `ConnectInfo`.
        let ip = parts
            .extensions
            .get::<Caller>()
            .map(|c| c.0)
            .unwrap_or_else(|| {
                parts
                    .extensions
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|c| c.0.ip())
            });
        Ok(Caller(ip))
    }
}

/// How long finished work stays on disk. A day at least, so a mistake in the environment
/// cannot sweep everything on the next start; ten years at most, so a typo is clamped
/// rather than immortal.
fn retention_days_from_env() -> i64 {
    std::env::var("EINK_RETENTION_DAYS")
        .ok()
        .and_then(|value| value.parse::<i64>().ok())
        .filter(|days| (1..=3650).contains(days))
        .unwrap_or(30)
}

/// The directories `/source` may read from, colon-separated, or nothing when unset.
fn source_roots_from_env() -> Option<Vec<PathBuf>> {
    let value = std::env::var("EINK_SOURCE_ROOTS").ok()?;
    let roots: Vec<PathBuf> = value
        .split(':')
        .filter(|root| !root.trim().is_empty())
        .map(PathBuf::from)
        .collect();
    Some(roots)
}

impl AppState {
    pub fn new(state_dir: PathBuf) -> Self {
        Self::with_config(state_dir, 30)
    }

    pub fn with_config(state_dir: PathBuf, long_poll_seconds: u64) -> Self {
        // Writes are queued so that no mutation holds the session lock while the disk is
        // being written; callers that must be durable before they answer call
        // `flush_writes`.
        let (write_tx, write_rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(crate::session::drain_writes(write_rx));
        let ws_heartbeat = Duration::from_secs(30);
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
        let http = reqwest::Client::builder()
            .timeout(WEBHOOK_TIMEOUT)
            .build()
            .expect("a default HTTP client can be built");
        let mgr = SessionManager::with_writer(state_dir.clone(), write_tx.clone());
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
            write_tx,
            notifiers: Arc::new(RwLock::new(HashMap::new())),
            ws_senders: Arc::new(RwLock::new(HashMap::new())),
            long_poll_seconds,
            assets_dir: default_assets_dir(),
            ocr_engine,
            ws_heartbeat,
            http,
            state_dir: state_dir.clone(),
            retention_days: retention_days_from_env(),
            token: std::env::var("EINK_TOKEN")
                .ok()
                .map(|token| token.trim().to_string())
                .filter(|token| !token.is_empty()),
            webhook_backoff: [
                Duration::from_millis(250),
                Duration::from_secs(1),
                Duration::from_secs(4),
            ],
            source_roots: source_roots_from_env(),
            reanchor_budget: Duration::from_secs(10),
            screenshot_requests: Arc::new(RwLock::new(HashMap::new())),
            tablets: Arc::new(std::sync::Mutex::new(HashMap::new())),
            voice: crate::voice::Voice::from_env(),
            captures: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// How many tablets have this session open on the WebSocket right now.
    pub fn tablets_listening(&self, id: &str) -> usize {
        self.tablets.lock().unwrap().get(id).copied().unwrap_or(0)
    }

    /// Wait until every queued write has landed, and learn whether one of them failed.
    ///
    /// Used on the paths where a response promises durability: the review that was just
    /// submitted, the cancellation, the content update a client will rely on.
    pub async fn flush_writes(&self) -> Result<(), String> {
        let (ack, waited) = tokio::sync::oneshot::channel();
        if self
            .write_tx
            .send(crate::session::WriteRequest::Barrier { ack })
            .is_err()
        {
            return Err("the write queue is gone".into());
        }
        waited
            .await
            .unwrap_or_else(|_| Err("the write queue stopped".into()))
    }

    /// The checkout `/source` may read for a session, if this creator may name one.
    ///
    /// Only a client on this machine may: the CLI runs beside the server, the tablet never
    /// does, and a LAN client naming `/home` would otherwise be reading it. With
    /// `EINK_SOURCE_ROOTS` set, the directory must also be inside one of those.
    pub fn source_root_for(&self, caller: Caller, cwd: Option<&str>) -> Option<PathBuf> {
        let cwd = cwd?;
        if !caller.is_local() {
            tracing::info!(
                cwd,
                "checkout not recorded: the session was not created from this machine"
            );
            return None;
        }
        let Ok(root) = std::path::Path::new(cwd).canonicalize() else {
            tracing::info!(cwd, "checkout not recorded: no such directory here");
            return None;
        };
        if let Some(allowed) = &self.source_roots
            && !allowed
                .iter()
                .filter_map(|a| a.canonicalize().ok())
                .any(|a| root.starts_with(&a))
        {
            tracing::info!(cwd, "checkout not recorded: outside EINK_SOURCE_ROOTS");
            return None;
        }
        Some(root)
    }

    /// Deliver one webhook in the background, counting and logging how it went. Every
    /// callback the server sends goes through here, so none can be fire-and-forget in
    /// the sense of "nobody will ever know".
    pub fn send_webhook(&self, url: String, body: impl serde::Serialize + Send + 'static) {
        let client = self.http.clone();
        let backoff = self.webhook_backoff;
        let body = match serde_json::to_value(body) {
            Ok(body) => body,
            Err(error) => {
                tracing::error!(%url, %error, "webhook body could not be serialized");
                crate::metrics::WEBHOOK_TOTAL
                    .with_label_values(&["error"])
                    .inc();
                return;
            }
        };
        tokio::spawn(async move {
            // A channel that is restarting is deaf for a second or two, not gone: the
            // pending feedback is durable, so a retry costs nothing and saves the note.
            // Three retries after the first attempt, with a backoff that stays inside a
            // human's patience.
            let backoff = backoff;
            for attempt in 0..=backoff.len() {
                let failure = match client.post(&url).json(&body).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        crate::metrics::WEBHOOK_TOTAL
                            .with_label_values(&["ok"])
                            .inc();
                        return;
                    }
                    Ok(resp) => format!("returned {}", resp.status()),
                    Err(error) => format!("failed: {error}"),
                };
                if attempt == backoff.len() {
                    tracing::warn!(%url, attempt, "webhook {failure}; giving up");
                    crate::metrics::WEBHOOK_TOTAL
                        .with_label_values(&["error"])
                        .inc();
                    return;
                }
                tracing::warn!(%url, attempt, "webhook {failure}; retrying");
                crate::metrics::WEBHOOK_TOTAL
                    .with_label_values(&["retried"])
                    .inc();
                tokio::time::sleep(backoff[attempt]).await;
            }
        });
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

    pub async fn ws_subscribe(&self, id: &str) -> broadcast::Receiver<ServerMessage> {
        self.ws_senders
            .write()
            .await
            .entry(id.to_string())
            .or_insert_with(|| broadcast::channel(16).0)
            .subscribe()
    }

    pub async fn ws_send(&self, id: &str, msg: ServerMessage) {
        if let Some(sender) = self.ws_senders.read().await.get(id) {
            let _ = sender.send(msg);
        }
    }

    pub async fn ws_cleanup(&self, id: &str) {
        self.ws_senders.write().await.remove(id);
    }

    /// Count the session directories that are actually on disk.
    ///
    /// `eink_sessions_stored` is memory, and the two disagree after a crash or a purge
    /// that could not finish; the runbook's "disk filling up" row needs the disk.
    pub async fn refresh_dir_gauge(&self) {
        let mut count = 0i64;
        if let Ok(mut entries) = tokio::fs::read_dir(self.state_dir.join("sessions")).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.file_type().await.is_ok_and(|kind| kind.is_dir()) {
                    count += 1;
                }
            }
        }
        crate::metrics::SESSION_DIRS_ON_DISK.set(count);
    }

    /// Remove directories of terminal, unstarred sessions older than the retention window.
    ///
    /// Called at startup and daily. The selection happens under the lock; the deletion
    /// does not, so a slow disk slows the sweep and not the API.
    pub async fn sweep_retention(&self) -> usize {
        let removed = {
            self.sessions
                .write()
                .await
                .retention_candidates(self.retention_days)
        };
        let count = removed.len();
        for session in removed {
            if let Err(error) = tokio::fs::remove_dir_all(&session.state_dir).await {
                tracing::warn!(
                    path = %session.state_dir.display(),
                    %error,
                    "retention could not remove a session directory"
                );
            }
        }
        if count > 0 {
            let stored = self.sessions.read().await.count() as i64;
            crate::metrics::SESSIONS_STORED.set(stored);
            tracing::info!(count, "retention removed old sessions");
        }
        self.refresh_dir_gauge().await;
        count
    }

    pub async fn expire_stale(&self, timeout: Duration) -> usize {
        let expired = self.sessions.write().await.expire_stale_ids(timeout);
        if expired.is_empty() {
            return 0;
        }
        // Expiry is persisted state: what is on disk is what a restart reloads.
        if let Err(error) = self.flush_writes().await {
            tracing::error!(error, "expired sessions were not persisted");
        }
        let webhooks: Vec<_> = {
            let mgr = self.sessions.read().await;
            expired
                .iter()
                .filter_map(|id| mgr.get(id).and_then(webhook_payload))
                .collect()
        };
        for id in &expired {
            self.notify_and_cleanup(id).await;
            self.ws_send(
                id,
                ServerMessage::Error {
                    message: "session expired".into(),
                },
            )
            .await;
            self.ws_cleanup(id).await;
        }
        // Expiry is a terminal transition like the other two, and the agent waiting on
        // the callback deserves to hear about it the same way.
        for (url, body) in webhooks {
            self.send_webhook(url, body);
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
        .route("/api/sessions/{id}/callback", put(set_callback))
        .route("/api/sessions/{id}/progress", post(agent_progress))
        .route("/api/sessions/{id}/star", put(set_starred))
        .route(
            "/api/sessions/{id}/annotation/{index}",
            get(annotation_image),
        )
        .route(
            "/api/sessions/{id}/image/{name}",
            post(upload_image).get(document_image),
        )
        .route("/api/sessions/{id}/source", get(source_snippet))
        .route("/api/sessions/{id}/screenshot", post(request_screenshot))
        .route(
            "/api/sessions/{id}/screenshot/{request_id}",
            get(get_screenshot).put(upload_screenshot),
        )
        .route(
            "/api/sessions/{id}/voice",
            get(crate::voice::capture::voice_ws),
        )
        .route(
            "/api/sessions/{id}/voice/{name}",
            get(crate::voice::capture::get_audio),
        )
        .route(
            "/api/sessions/{id}/speech",
            post(crate::voice::capture::speech),
        )
        .route("/api/sessions/{id}/submit", post(submit_review))
        .route("/api/sessions/{id}/request_update", post(request_update))
        .route("/ws/{id}", get(crate::ws::ws_handler))
        .route("/session/{id}", get(render_session))
        .route("/review/{id}", get(render_review))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_token,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            durable_writes,
        ))
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

/// A response to a mutation is a promise that the change is on disk.
///
/// Writes are queued (see `SessionManager::persist`), so the lock is never held during
/// the write; this is where the queue is drained before the client is told it worked.
/// Doing it here rather than in each handler means a new mutating route cannot forget.
async fn durable_writes(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let mutating = !matches!(
        *req.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    );
    let response = next.run(req).await;
    if mutating
        && response.status().is_success()
        && let Err(error) = state.flush_writes().await
    {
        // The handler has already changed what is in memory; what it cannot do is tell
        // the client the change is safe, so it does not.
        tracing::error!(error, "a mutation was not written to disk");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "the change could not be written to disk",
        )
            .into_response();
    }
    response
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

async fn health(State(state): State<AppState>) -> Response {
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
    let probed = state_dir.clone();
    let writable = tokio::task::spawn_blocking(move || state_dir_is_writable(&probed))
        .await
        .unwrap_or(false);

    // A disk that cannot be written will lose the next review, so that is not "ok". OCR
    // is optional -- the tablet reads its own handwriting when the server cannot -- so
    // its absence is reported but is not a failure.
    let status = if writable {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = Json(serde_json::json!({
        "status": if writable { "ok" } else { "degraded" },
        "exe": exe,
        "problems": if writable { vec![] } else { vec!["the state directory is not writable"] },
        "ready": {
            "version": env!("CARGO_PKG_VERSION"),
            "ocr": if state.ocr_engine.is_some() { "available" } else { "unavailable" },
            "ocr_optional": true,
            "voice": if state.voice.is_some() { "available" } else { "unavailable" },
            "speech": if state.voice.as_ref().is_some_and(|v| v.tts.is_some()) { "available" } else { "unavailable" },
            "state_dir": state_dir.to_string_lossy(),
            "state_dir_writable": writable,
            "sessions": stored,
            "active": active,
            "ws_heartbeat_seconds": state.ws_heartbeat.as_secs(),
        },
    }));
    (status, body).into_response()
}

async fn openapi_json() -> Json<serde_json::Value> {
    Json(openapi_spec())
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

#[cfg(test)]
mod route_spec_tests {
    /// Every `/api` route `build_app` serves, with its methods, read from this file.
    fn served_routes() -> Vec<(String, Vec<&'static str>)> {
        let source = include_str!("app.rs");
        let table = &source[source.find("Router::new()").unwrap()..];
        let table = &table[..table.find(".route_layer(").unwrap()];
        table
            .split(".route(")
            .skip(1)
            .filter_map(|route| {
                let path = route.split('"').nth(1)?.to_string();
                let methods = ["get", "post", "put", "delete"]
                    .into_iter()
                    .filter(|m| route.contains(&format!("{m}(")))
                    .collect();
                path.starts_with("/api/").then_some((path, methods))
            })
            .collect()
    }

    /// The spec is what an agent reads to learn the API; a route missing from it is one
    /// no agent will find.
    #[test]
    fn every_api_route_is_in_the_openapi_spec() {
        let spec = crate::api::openapi_spec();
        let routes = served_routes();
        assert!(
            routes.len() > 20,
            "the route table was not found: {routes:?}"
        );
        let missing: Vec<String> = routes
            .iter()
            .flat_map(|(path, methods)| methods.iter().map(move |m| (path, m)))
            .filter(|(path, m)| spec["paths"][path.as_str()][**m].is_null())
            .map(|(path, m)| format!("{} {path}", m.to_uppercase()))
            .collect();
        assert!(
            missing.is_empty(),
            "routes missing from openapi_spec: {missing:?}"
        );
    }
}
