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
use crate::ocr::OcrEngine;
use crate::render;
use crate::session::{NewSession, Session, SessionManager, SessionStatus, SubmitResult};
use crate::verdict::{Verdict, parse_verdict};
use crate::ws::ServerMessage;

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
    /// Seconds between WebSocket pings; 0 disables the heartbeat. A client that says
    /// nothing for three intervals is dropped, which is what makes a half-open socket
    /// visible instead of silently dead.
    pub ws_heartbeat_seconds: u64,
    /// The one client every webhook goes out through, with a bound: a receiver that
    /// never answers used to hold a task forever.
    pub http: reqwest::Client,
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
        let http = reqwest::Client::builder()
            .timeout(WEBHOOK_TIMEOUT)
            .build()
            .expect("a default HTTP client can be built");
        let mgr = SessionManager::with_writer(state_dir, write_tx.clone());
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
            ws_heartbeat_seconds,
            http,
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
        }
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

/// A black-box comparison, so a wrong token cannot be found one byte at a time.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Where a WebView or a browser socket may carry the token in the URL, because neither
/// can set an `Authorization` header on the request that matters.
fn query_token_allowed(path: &str) -> bool {
    path.starts_with("/session/")
        || path.starts_with("/ws/")
        || (path.starts_with("/api/sessions/") && path.contains("/annotation/"))
}

fn query_value<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// `%XX` and `+`, because the WebSocket URL is built with `URLEncoder` and the token may
/// legitimately contain a character that has to be escaped.
fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn cookie_value<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    header.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (name, value) = pair.split_once('=')?;
        (name == key).then_some(value)
    })
}

/// The trust boundary: this API can read, edit and delete reviews, and it binds the LAN.
///
/// Loopback is exempt because the CLI and the channel run on the server's own machine.
/// Everything else presents `Authorization: Bearer`, a `?token=` where the WebView needs
/// it, or the `HttpOnly` cookie the page set. No token configured = today's open server.
async fn require_token(
    State(state): State<AppState>,
    request: Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(token) = state.token.clone() else {
        return next.run(request).await;
    };
    let path = request.uri().path().to_string();
    // Liveness and the metrics scrape are read-only counters, and a monitor that needs
    // the token to say "alive" is a monitor nobody finishes configuring.
    if path == "/api/health" || path == "/metrics" {
        return next.run(request).await;
    }

    let caller = request
        .extensions()
        .get::<Caller>()
        .and_then(|caller| caller.0)
        .or_else(|| {
            request
                .extensions()
                .get::<ConnectInfo<SocketAddr>>()
                .map(|connected| connected.0.ip())
        });
    if caller.is_some_and(|ip| ip.is_loopback()) {
        return next.run(request).await;
    }

    let headers = request.headers();
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    let from_query = query_token_allowed(&path)
        .then(|| {
            request
                .uri()
                .query()
                .and_then(|query| query_value(query, "token"))
        })
        .flatten()
        .and_then(percent_decode);
    let from_cookie = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| cookie_value(value, "eink_token"));

    let authorized = [bearer, from_query.as_deref(), from_cookie]
        .into_iter()
        .flatten()
        .any(|candidate| constant_time_eq(candidate.as_bytes(), token.as_bytes()));
    if !authorized {
        crate::metrics::HTTP_REQUESTS
            .with_label_values(&["GET", "unauthorized", "401"])
            .inc_by(0);
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let mut response = next.run(request).await;
    // The page's images and its JS fetches carry no Authorization header; the cookie is
    // what makes them work without putting the token in the markup.
    if path.starts_with("/session/") {
        if let Ok(cookie) = header::HeaderValue::from_str(&format!(
            "eink_token={token}; Path=/; HttpOnly; SameSite=Strict"
        )) {
            response.headers_mut().append(header::SET_COOKIE, cookie);
        }
    }
    response
}

#[cfg(test)]
mod auth_tests {
    use super::{percent_decode, query_token_allowed};

    #[test]
    fn a_query_token_is_decoded_and_only_allowed_where_a_header_cannot_go() {
        assert_eq!(percent_decode("s3%2Bcret").as_deref(), Some("s3+cret"));
        assert_eq!(percent_decode("a+b").as_deref(), Some("a b"));
        assert!(query_token_allowed("/session/abc"));
        assert!(query_token_allowed("/ws/abc"));
        assert!(query_token_allowed("/api/sessions/abc/annotation/0"));
        assert!(!query_token_allowed("/api/sessions"));
    }
}

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
        .route("/api/sessions/{id}/source", get(source_snippet))
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
            "state_dir": state_dir.to_string_lossy(),
            "state_dir_writable": writable,
            "sessions": stored,
            "active": active,
            "ws_heartbeat_seconds": state.ws_heartbeat_seconds,
        },
    }));
    (status, body).into_response()
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

/// Refuse ink the rasteriser should not be asked to draw. The body limit bounds bytes,
/// not work: half a million points is a 10 MB body and minutes of drawing.
fn strokes_within_limits<'a>(
    strokes: impl Iterator<Item = &'a Vec<[f64; 2]>>,
) -> Result<(), (StatusCode, String)> {
    let (count, points) = strokes.fold((0usize, 0usize), |(c, p), s| (c + 1, p + s.len()));
    if count > MAX_OCR_STROKES || points > MAX_OCR_POINTS {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "too much ink: at most {MAX_OCR_STROKES} strokes and {MAX_OCR_POINTS} points per request"
            ),
        ));
    }
    Ok(())
}

async fn ocr_strokes(State(state): State<AppState>, Json(body): Json<OcrRequest>) -> Response {
    if let Err(refused) = strokes_within_limits(body.strokes.iter()) {
        return refused.into_response();
    }
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

/// A callback must be somewhere the server should be posting to: http(s), with a host,
/// and not the link-local range where cloud metadata services listen. Loopback stays
/// allowed on purpose -- the local relay listens on 127.0.0.1.
pub fn validate_callback_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("callback_url is not a URL: {e}"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("callback_url must be http or https".into());
    }
    let Some(host) = parsed.host_str() else {
        return Err("callback_url has no host".into());
    };
    let link_local = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_link_local(),
        Ok(IpAddr::V6(ip)) => ip.is_unicast_link_local(),
        Err(_) => false,
    };
    if link_local {
        return Err("callback_url must not point at a link-local address".into());
    }
    Ok(())
}

async fn create_session(
    State(state): State<AppState>,
    Query(params): Query<CreateParams>,
    caller: Caller,
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

    if let Some(url) = &request.callback_url
        && let Err(reason) = validate_callback_url(url)
    {
        return bad_request(reason);
    }
    let source_root = state.source_root_for(
        caller,
        request
            .origin
            .as_ref()
            .and_then(|origin| origin.cwd.as_deref()),
    );

    let content_len = request.content.len();
    let mut mgr = state.sessions.write().await;
    let session = mgr.create_session(NewSession {
        content: request.content,
        title: request.title,
        callback_url: request.callback_url,
        tags: request.tags,
        starred: request.starred,
        origin: request.origin,
        source_root,
    });
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

/// The largest page the server will serve, whatever a client asks for.
const SESSION_PAGE_MAX: usize = 200;

#[derive(Deserialize, Default)]
struct ListParams {
    status: Option<String>,
    starred: Option<bool>,
    /// How many sessions to return. Absent means all of them, which is what every client
    /// did before paging existed.
    limit: Option<usize>,
    /// Opaque marker from the previous page's `x-next-cursor` header.
    cursor: Option<String>,
}

/// The order the list is served in, and the reason it is total: without the id a tie on
/// `updated_at` (two sessions created in the same millisecond) could swap places between
/// pages, and a client following the cursor would miss one or see one twice.
fn session_sort_key(
    session: &crate::session::Session,
) -> (bool, chrono::DateTime<chrono::Utc>, String) {
    (session.starred, session.updated_at, session.id.clone())
}

fn encode_cursor(session: &crate::session::Session) -> String {
    format!(
        "{}-{}-{}",
        if session.starred { 1 } else { 0 },
        session.updated_at.timestamp_micros(),
        session.id
    )
}

/// Parse a cursor back into the key it was made from. A cursor the server cannot read is
/// a client bug, and answering with a 400 is better than silently restarting the list.
fn decode_cursor(cursor: &str) -> Option<(bool, i64, String)> {
    let mut parts = cursor.splitn(3, '-');
    let starred = match parts.next()? {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    let micros = parts.next()?.parse().ok()?;
    let id = parts.next()?;
    if id.is_empty() || id.len() > 64 {
        return None;
    }
    Some((starred, micros, id.to_string()))
}

async fn list_sessions(
    State(state): State<AppState>,
    Query(params): Query<ListParams>,
) -> Response {
    if params.limit == Some(0) {
        return (StatusCode::BAD_REQUEST, "limit must be at least 1").into_response();
    }
    let after = match params.cursor.as_deref() {
        Some(cursor) => match decode_cursor(cursor) {
            Some(key) => Some(key),
            None => return (StatusCode::BAD_REQUEST, "unreadable cursor").into_response(),
        },
        None => None,
    };

    let limit = params.limit.map(|limit| limit.min(SESSION_PAGE_MAX));
    let (page, next) = {
        let mgr = state.sessions.read().await;
        let mut filtered: Vec<_> = mgr
            .list()
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
        // Newest first, pinned first, with the id as the tiebreaker (see `session_sort_key`).
        filtered.sort_by_key(|session| std::cmp::Reverse(session_sort_key(session)));

        if let Some((starred, micros, id)) = &after {
            filtered.retain(|s| {
                let key = session_sort_key(s);
                // Strictly after the cursor in the sort order.
                (&key.0, key.1.timestamp_micros(), &key.2) < (starred, *micros, id)
            });
        }

        let next = match limit {
            Some(limit) if filtered.len() > limit => Some(encode_cursor(filtered[limit - 1])),
            _ => None,
        };
        let page: Vec<SessionSummaryResponse> = filtered
            .iter()
            .take(limit.unwrap_or(usize::MAX))
            .map(|s| SessionSummaryResponse::from_session(s))
            .collect();
        (page, next)
    };

    let mut response = Json(page).into_response();
    if let Some(cursor) = next
        && let Ok(value) = axum::http::HeaderValue::from_str(&cursor)
    {
        response.headers_mut().insert("x-next-cursor", value);
    }
    response
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
        let (removed, stored, starred) = {
            let mut mgr = state.sessions.write().await;
            // Let any queued write for this session land first: the drain creates the
            // directory it writes into, so a straggler would resurrect what we just
            // deleted. Under the lock, because every write is queued under it, so none
            // can slip in between the barrier and the removal.
            let _ = state.flush_writes().await;
            let removed = mgr.remove(&id);
            (removed, mgr.count(), mgr.starred_count())
        };
        let Some(mut removed) = removed else {
            return StatusCode::NOT_FOUND.into_response();
        };
        crate::metrics::SESSIONS_STORED.set(stored as i64);
        crate::metrics::SESSIONS_STARRED.set(starred as i64);
        // A tablet or observer still holding the id must stop waiting on it.
        state.notify_and_cleanup(&id).await;
        if removed.status == SessionStatus::Active {
            // Taking a live review away is a cancellation to everyone watching it: the
            // gauge, the socket and the callback hear what a plain cancel would have told
            // them.
            crate::metrics::SESSIONS_CANCELLED.inc();
            crate::metrics::SESSIONS_ACTIVE.dec();
            removed.status = SessionStatus::Cancelled;
            state
                .ws_send(
                    &id,
                    ServerMessage::Error {
                        message: "session cancelled".into(),
                    },
                )
                .await;
            if let Some((url, body)) = webhook_payload(&removed) {
                state.send_webhook(url, body);
            }
        }
        state.ws_cleanup(&id).await;
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
                ServerMessage::Error {
                    message: "session cancelled".into(),
                },
            )
            .await;
        if let Some((url, body)) = webhook {
            state.send_webhook(url, body);
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

/// The lines around a `path:line` reference in a review, so the reader can look at the code
/// on the tablet instead of opening an editor.
///
/// The file is read from the checkout the server recorded for the session -- the worktree
/// a CLI on this machine pushed from -- and only from there: the path is joined to it,
/// canonicalised, and required to stay inside, so a session document cannot turn into a
/// way to read the machine. `origin.cwd` is never the root: any client on the LAN can
/// write that field.
#[derive(Deserialize)]
struct SourceParams {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    context: Option<usize>,
}

const SOURCE_CONTEXT_MAX: usize = 20;

async fn source_snippet(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<SourceParams>,
) -> Response {
    let root = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(session) => match &session.source_root {
                Some(root) => root.clone(),
                // Without a recorded checkout there is nothing to read from, and guessing
                // would be worse than saying so.
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        "this review did not record a checkout the server may read",
                    )
                        .into_response();
                }
            },
        }
    };

    let Some((relative, wanted)) = split_source_reference(&params.reference) else {
        return (
            StatusCode::BAD_REQUEST,
            "expected a reference like path/file.rs:120",
        )
            .into_response();
    };
    let context = params.context.unwrap_or(6).min(SOURCE_CONTEXT_MAX);
    let Ok(root) = root.canonicalize() else {
        return (StatusCode::NOT_FOUND, "the checkout is gone").into_response();
    };
    let candidate = root.join(&relative);
    let Ok(path) = candidate.canonicalize() else {
        return (StatusCode::NOT_FOUND, "no such file in the checkout").into_response();
    };
    if !path.starts_with(&root) || !path.is_file() {
        return (StatusCode::NOT_FOUND, "outside the checkout").into_response();
    }
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return (StatusCode::NOT_FOUND, "unreadable").into_response();
    };
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if wanted > total {
        return (StatusCode::NOT_FOUND, format!("the file has {total} lines")).into_response();
    }
    let start = wanted.saturating_sub(context + 1);
    let end = (wanted + context).min(total);
    let window = &lines[start..end];
    // Rendered here rather than in the browser: the page has one highlighter, and it is
    // this one. The cited line is marked the same way a snippet marks it.
    let html = crate::render::source_block(
        language_for(&relative),
        &format!("{relative}:{}", start + 1),
        window,
        Some(wanted - start),
    );
    Json(serde_json::json!({
        "path": relative,
        "start": start + 1,
        "end": end,
        "total": total,
        "lines": window,
        "html": html,
    }))
    .into_response()
}

/// The highlighter's name for a file, or nothing when it does not know the language (the
/// page falls back to plain text, exactly as it does for a snippet it cannot highlight).
fn language_for(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "kt" | "kts" => "kotlin",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "jsx" | "mjs" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "c++",
        "cs" => "c#",
        "swift" => "swift",
        "php" => "php",
        "scala" => "scala",
        "sh" | "bash" => "bash",
        "sql" => "sql",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "toml" => "toml",
        "md" => "markdown",
        "nix" => "nix",
        "css" => "css",
        "html" | "htm" => "html",
        "xml" => "xml",
        "proto" => "protobuf",
        "gradle" => "groovy",
        _ => "",
    }
}

/// Split `path/file.rs:120` into the path and the line, refusing anything that could
/// escape the checkout before the filesystem is touched.
fn split_source_reference(reference: &str) -> Option<(String, usize)> {
    let (relative, line) = reference.rsplit_once(':')?;
    let line: usize = line.trim().parse().ok()?;
    let relative = relative.trim();
    if relative.is_empty() || line == 0 {
        return None;
    }
    let path = std::path::Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    Some((relative.to_string(), line))
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
    let targets = tokio::time::timeout(
        state.reanchor_budget,
        tokio::task::spawn_blocking(move || {
            crate::reanchor::targets(&body.previous, &body.current)
        }),
    )
    .await;
    match targets {
        Ok(Ok(targets)) => Json(serde_json::json!({"targets":targets})).into_response(),
        Ok(Err(_)) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        // Better to leave the notes where they are than to hold the page while the
        // comparison runs on: the client keeps its current placement.
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "relocating the notes took too long; keep them where they are",
        )
            .into_response(),
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
            if mgr.clear_progress(&id, feedback_id.as_deref()) {
                crate::metrics::AGENT_PROGRESS_TOTAL
                    .with_label_values(&["cleared"])
                    .inc();
            }
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

/// Where the server may deliver a session's events.
///
/// The same scheme and address rules as creation ([`validate_callback_url`]) plus one
/// more: the channel that claims a session runs on this machine, so a non-loopback hook
/// is a mistake at best and a network probe at worst. `EINK_CALLBACK_ALLOW_REMOTE=1`
/// lifts the second rule for a deliberate remote receiver.
fn callback_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 2048 {
        return None;
    }
    validate_callback_url(trimmed).ok()?;
    let parsed = reqwest::Url::parse(trimmed).ok()?;
    let loopback = parsed.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !loopback && std::env::var("EINK_CALLBACK_ALLOW_REMOTE").as_deref() != Ok("1") {
        return None;
    }
    Some(trimmed.to_string())
}

async fn set_callback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: Caller,
    Json(body): Json<crate::api::SetCallbackRequest>,
) -> Response {
    if !caller.is_local() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(url) = callback_url(&body.url) else {
        return bad_request(
            "callback url must be http:// on this machine (set EINK_CALLBACK_ALLOW_REMOTE=1 for a remote receiver)"
                .into(),
        );
    };
    let mut mgr = state.sessions.write().await;
    if !mgr.set_callback(&id, Some(url.clone())) {
        return StatusCode::NOT_FOUND.into_response();
    }
    tracing::info!(id = %id, %url, "callback registered");
    StatusCode::NO_CONTENT.into_response()
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

#[derive(serde::Deserialize)]
struct ProgressBody {
    feedback_id: String,
    message: String,
    #[serde(default)]
    step: Option<u32>,
    #[serde(default)]
    of: Option<u32>,
}

/// The longest progress line a reviewer can read at a glance on e-ink. In characters,
/// not bytes: a line of Polish must not be rejected for being two bytes per letter.
const MAX_PROGRESS_CHARS: usize = 120;

/// A short line about what the agent is doing, while it does it.
///
/// This is status, not prose: the reader is looking at "Waiting for agent" and needs to
/// know whether their notes were read and which section is being rewritten. It is stored
/// on the session (so a reconnecting tablet gets the latest line) and cleared only by the
/// content update that names the same feedback id.
async fn agent_progress(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ProgressBody>,
) -> Response {
    let message = body.message.trim();
    if message.is_empty() || message.chars().count() > MAX_PROGRESS_CHARS {
        return bad_request(format!(
            "a progress line must be 1 to {MAX_PROGRESS_CHARS} characters"
        ));
    }
    let fence = match (body.step, body.of) {
        (None, None) => None,
        (Some(step), Some(of)) if step >= 1 && step <= of => Some((step, of)),
        _ => return bad_request("step and of must be given together, with 1 <= step <= of".into()),
    };

    let mut mgr = state.sessions.write().await;
    let Some(session) = mgr.get(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let pending = session
        .pending_feedback
        .as_ref()
        .and_then(|p| p["feedback_id"].as_str());
    if pending != Some(body.feedback_id.as_str()) {
        crate::metrics::AGENT_PROGRESS_TOTAL
            .with_label_values(&["rejected"])
            .inc();
        return StatusCode::CONFLICT.into_response();
    }

    let mut progress = serde_json::json!({
        "feedback_id": body.feedback_id,
        "message": message,
    });
    if let Some((step, of)) = fence {
        progress["step"] = step.into();
        progress["of"] = of.into();
    }
    mgr.record_progress(&id, progress.clone());
    drop(mgr);

    let event = crate::ws::ServerMessage::AgentProgress {
        feedback_id: progress["feedback_id"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        message: message.to_string(),
        step: fence.map(|(step, _)| step),
        of: fence.map(|(_, of)| of),
    };
    state.ws_send(&id, event).await;
    crate::metrics::AGENT_PROGRESS_TOTAL
        .with_label_values(&["accepted"])
        .inc();
    StatusCode::NO_CONTENT.into_response()
}

fn webhook_payload(session: &Session) -> Option<(String, SessionResultResponse)> {
    session
        .callback_url
        .clone()
        .map(|url| (url, SessionResultResponse::from_session(session)))
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
    // Say no before anything is parsed or written when the review is already over.
    let annotations_dir = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => {
                tracing::debug!(id = %id, "submit: session not found");
                return StatusCode::NOT_FOUND.into_response();
            }
            Some(s) if s.status != SessionStatus::Active => {
                tracing::debug!(id = %id, "submit: session not active");
                return StatusCode::CONFLICT.into_response();
            }
            Some(s) => s.annotations_dir(),
        }
    };

    let content_type = content_type(&headers);
    let parsed = if content_type.starts_with("application/json") {
        parse_json_submit(&body)
    } else if content_type.starts_with("multipart/form-data") {
        parse_multipart_submit(&headers, body).await
    } else {
        Err((
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            format!("unsupported content-type: {content_type}"),
        ))
    };
    let submission = match parsed {
        Ok(submission) => submission,
        Err((status, message)) => return (status, message).into_response(),
    };

    // Stage the images beside the session before taking the lock: a reader's bitmap can
    // be megabytes, and the lock must not wait on the disk. They move in only once the
    // session accepts, so a refusal leaves no orphan behind.
    let staging = annotations_dir.join(format!(".staging-{}", uuid::Uuid::new_v4().as_simple()));
    let staged = if submission.images.is_empty() {
        Vec::new()
    } else {
        let into = staging.clone();
        let images = submission.images;
        let written = tokio::task::spawn_blocking(move || {
            images
                .iter()
                .map(|image| crate::session::write_annotation(&into, image))
                .collect::<std::io::Result<Vec<String>>>()
        })
        .await;
        match written {
            Ok(Ok(paths)) => paths,
            Ok(Err(error)) => {
                tracing::error!(id = %id, %error, "annotation images could not be stored");
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the annotation images could not be stored",
                )
                    .into_response();
            }
            Err(_) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        }
    };

    let mut mgr = state.sessions.write().await;
    let current = mgr.get(&id).map(|s| s.version);
    let outcome = match current {
        Some(version) if submission.document_version.is_some_and(|v| v != version) => {
            Err(SubmitResult::NotActive)
        }
        _ => {
            let images = move_in(&staging, &annotations_dir, staged);
            match mgr.submit(
                &id,
                submission.typed_notes,
                images,
                submission.verdict,
                submission.stroke_data,
                submission.annotations,
            ) {
                SubmitResult::Ok => Ok(()),
                refused => Err(refused),
            }
        }
    };
    match outcome {
        Ok(()) => {
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
                        ServerMessage::SessionSubmitted {
                            result: Box::new(result),
                        },
                    )
                    .await;
                state.ws_cleanup(&id).await;
            }
            if let Some((url, body)) = webhook {
                state.send_webhook(url, body);
            }
            StatusCode::OK.into_response()
        }
        Err(refused) => {
            drop(mgr);
            let _ = tokio::fs::remove_dir_all(&staging).await;
            match refused {
                SubmitResult::NotFound => {
                    tracing::debug!(id = %id, "submit: session not found");
                    StatusCode::NOT_FOUND.into_response()
                }
                _ => {
                    tracing::debug!(id = %id, "submit: session not active or document changed");
                    StatusCode::CONFLICT.into_response()
                }
            }
        }
    }
}

/// Move staged images into the session's annotations directory, recording only the ones
/// that actually arrived. A rename is a metadata change, cheap enough to do while the
/// session is held.
fn move_in(staging: &std::path::Path, dir: &std::path::Path, staged: Vec<String>) -> Vec<String> {
    let mut images = Vec::with_capacity(staged.len());
    for path in staged {
        let from = std::path::Path::new(&path);
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = dir.join(name);
        match std::fs::rename(from, &to) {
            Ok(()) => images.push(to.to_string_lossy().into_owned()),
            Err(error) => tracing::error!(path, %error, "an annotation image was not kept"),
        }
    }
    let _ = std::fs::remove_dir(staging);
    images
}

async fn request_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SubmitReviewRequest>,
) -> Response {
    if let Err(refused) = strokes_within_limits(body.annotations.iter().flat_map(|g| &g.strokes)) {
        return refused.into_response();
    }
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
    let feedback_id = uuid::Uuid::new_v4().to_string();
    let msg = ServerMessage::AnnotationResult {
        feedback_id: feedback_id.clone(),
        version,
        annotations: annotations.clone(),
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
    state.ws_send(&id, msg).await;
    if let Some(url) = callback_url {
        state.send_webhook(
            url,
            AnnotationWebhook {
                r#type: "annotation_result",
                id,
                version,
                feedback_id,
                annotations,
            },
        );
    }
    StatusCode::OK.into_response()
}

/// The body of the callback that carries handwritten feedback mid-review.
#[derive(serde::Serialize)]
struct AnnotationWebhook {
    #[serde(rename = "type")]
    r#type: &'static str,
    id: String,
    version: u32,
    feedback_id: String,
    annotations: Vec<AnnotationGroup>,
}

/// What a submit carries once it is parsed, before anything has touched the disk.
struct Submission {
    typed_notes: String,
    images: Vec<Bytes>,
    verdict: Option<Verdict>,
    stroke_data: Option<serde_json::Value>,
    annotations: Vec<AnnotationGroup>,
    document_version: Option<u32>,
}

/// The eight bytes every PNG starts with. The images are served back as `image/png`, so
/// what is accepted has to be one.
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

fn parse_json_submit(body: &[u8]) -> Result<Submission, SubmitError> {
    let request: SubmitReviewRequest = serde_json::from_slice(body).map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid submit-review JSON: {error}"),
        )
    })?;
    let verdict = parse_verdict_override(request.verdict)?;
    Ok(Submission {
        typed_notes: request.typed_notes,
        images: Vec::new(),
        verdict,
        stroke_data: None,
        annotations: request.annotations,
        document_version: request.document_version,
    })
}

async fn parse_multipart_submit(
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Submission, SubmitError> {
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
    let mut document_version: Option<u32> = None;

    let field_error = |field: &str, error: multer::Error| {
        (
            StatusCode::BAD_REQUEST,
            format!("failed to read {field}: {error}"),
        )
    };
    while let Some(field) = multipart.next_field().await.map_err(|error| {
        (
            StatusCode::BAD_REQUEST,
            format!("failed to parse multipart body: {error}"),
        )
    })? {
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "typed_notes" => {
                typed_notes = field
                    .text()
                    .await
                    .map_err(|e| field_error("typed_notes", e))?;
            }
            "annotation" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| field_error("annotation", e))?;
                if !data.starts_with(PNG_MAGIC) {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        "annotation is not a PNG image".into(),
                    ));
                }
                images.push(data);
            }
            "stroke_data" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| field_error("stroke_data", e))?;
                if !text.is_empty() {
                    stroke_data = serde_json::from_str(&text).ok();
                }
            }
            "annotations" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| field_error("annotations", e))?;
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
                verdict_text = Some(field.text().await.map_err(|e| field_error("verdict", e))?);
            }
            "document_version" => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| field_error("document_version", e))?;
                document_version = Some(text.trim().parse().map_err(|_| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("invalid document_version: {text}"),
                    )
                })?);
            }
            _ => {}
        }
    }

    let verdict = parse_verdict_override(verdict_text)?;
    Ok(Submission {
        typed_notes,
        images,
        verdict,
        stroke_data,
        annotations,
        document_version,
    })
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

/// Render a session's document with `render`, off the async workers: syntect over a
/// 10 MB review is CPU time, and every other request would wait behind it.
async fn rendered_page(state: &AppState, id: &str, render: fn(&str, &str) -> String) -> Response {
    // Take what rendering needs and drop the lock: rendering a large document used to hold
    // the read lock, so every mutation waited behind it.
    let snapshot = {
        let mgr = state.sessions.read().await;
        mgr.get(id)
            .map(|s| (s.content.clone(), s.id.clone(), s.version))
    };
    let Some((content, session_id, version)) = snapshot else {
        tracing::debug!(id = %id, "render: session not found");
        return StatusCode::NOT_FOUND.into_response();
    };
    tracing::debug!(id = %id, "rendering HTML");
    let rendered = tokio::task::spawn_blocking(move || {
        // The first `</head>` is the page's; a document that mentions the tag in a code
        // block must not grow a second version marker.
        render(&content, &session_id).replacen(
            "</head>",
            &format!("<meta name=\"eink-version\" content=\"{version}\"></head>"),
            1,
        )
    })
    .await;
    match rendered {
        Ok(html) => Html(html).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

async fn render_session(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    rendered_page(&state, &id, render::to_eink_html).await
}

async fn render_review(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    rendered_page(&state, &id, render::to_browser_html).await
}
