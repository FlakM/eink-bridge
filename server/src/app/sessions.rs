use super::*;

#[derive(Deserialize, Default)]
pub(super) struct CreateParams {
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

pub(super) async fn create_session(
    State(state): State<AppState>,
    Query(params): Query<CreateParams>,
    caller: Caller,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let content_type = content_type(&headers);
    let (mut request, images) = if content_type.starts_with("application/json") {
        match serde_json::from_slice::<CreateSessionRequest>(&body) {
            Ok(request) => (request, Vec::new()),
            Err(error) => return bad_request(format!("invalid create-session JSON: {error}")),
        }
    } else if content_type.starts_with("multipart/form-data") {
        match parse_multipart_create(&headers, body).await {
            Ok(parsed) => parsed,
            Err(response) => return response,
        }
    } else {
        let content = match String::from_utf8(body.to_vec()) {
            Ok(content) => content,
            Err(error) => return bad_request(format!("request body must be valid UTF-8: {error}")),
        };
        (
            CreateSessionRequest {
                schema_version: Some(SCHEMA_VERSION),
                title: params.title.clone(),
                content,
                callback_url: params.callback_url.clone(),
                tags: HashMap::new(),
                starred: false,
                origin: None,
            },
            Vec::new(),
        )
    };
    if request.schema_version.is_none() {
        request.schema_version = Some(SCHEMA_VERSION);
    }
    if request.title.is_none() {
        request.title = params.title.clone();
    }
    if request.callback_url.is_none() {
        request.callback_url = params.callback_url.clone();
    }

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
        images,
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
    drop(mgr);
    state.refresh_dir_gauge().await;

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
pub(super) const SESSION_PAGE_MAX: usize = 200;

#[derive(Deserialize, Default)]
pub(super) struct ListParams {
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
pub(super) fn session_sort_key(
    session: &crate::session::Session,
) -> (bool, chrono::DateTime<chrono::Utc>, String) {
    (session.starred, session.updated_at, session.id.clone())
}

pub(super) fn encode_cursor(session: &crate::session::Session) -> String {
    format!(
        "{}-{}-{}",
        if session.starred { 1 } else { 0 },
        session.updated_at.timestamp_micros(),
        session.id
    )
}

/// Parse a cursor back into the key it was made from. A cursor the server cannot read is
/// a client bug, and answering with a 400 is better than silently restarting the list.
pub(super) fn decode_cursor(cursor: &str) -> Option<(bool, i64, String)> {
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

pub(super) async fn list_sessions(
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

pub(super) async fn get_session(
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
pub(super) struct DeleteParams {
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

pub(super) async fn delete_session(
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
        // Off the lock: the directory may hold megabytes of annotation images.
        if let Err(error) = tokio::fs::remove_dir_all(&removed.state_dir).await {
            tracing::warn!(path = %removed.state_dir.display(), %error, "failed to remove session dir");
        }
        state.refresh_dir_gauge().await;
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

pub(super) async fn purge_sessions(State(state): State<AppState>) -> Json<serde_json::Value> {
    let (removed, stored, starred) = {
        let mut mgr = state.sessions.write().await;
        let removed = mgr.purge_finished();
        (removed, mgr.count(), mgr.starred_count())
    };
    let count = removed.len();
    crate::metrics::SESSIONS_STORED.set(stored as i64);
    crate::metrics::SESSIONS_STARRED.set(starred as i64);
    // Collected under the lock, deleted after it: a purge of hundreds of sessions must
    // not stall every other mutation for the duration of the disk.
    for session in removed {
        if let Err(error) = tokio::fs::remove_dir_all(&session.state_dir).await {
            tracing::warn!(
                path = %session.state_dir.display(),
                %error,
                "failed to remove a purged session directory"
            );
        }
    }
    state.refresh_dir_gauge().await;
    tracing::info!(count, "sessions purged");
    Json(serde_json::json!({ "purged": count }))
}

pub(super) async fn set_starred(
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

/// Render a session's document with `render`, off the async workers: syntect over a
/// 10 MB review is CPU time, and every other request would wait behind it.
pub(super) async fn rendered_page(
    state: &AppState,
    id: &str,
    render: fn(&str, &str) -> String,
) -> Response {
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

pub(super) async fn render_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    rendered_page(&state, &id, render::to_eink_html).await
}

pub(super) async fn render_review(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    rendered_page(&state, &id, render::to_browser_html).await
}
