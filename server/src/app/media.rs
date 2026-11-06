use super::*;

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

pub(super) async fn annotation_image(
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

/// Add an image to a session's document. The document is not rewritten: the caller
/// references it from the markdown as `image/<name>`, which the renderer resolves to this
/// route, so the same reference works before and after an update.
pub(super) async fn upload_image(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let Some(content_type) = document_image_type(&name) else {
        return bad_request(format!(
            "image name {name:?} must be a single file name ending in .png, .jpg, .jpeg, .webp or .gif"
        ));
    };
    // The session is checked before the bytes: a mistyped id should say "no such session",
    // not send the caller looking for a problem with a picture that was fine.
    let dir = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(session) if session.status != SessionStatus::Active => {
                return (
                    StatusCode::CONFLICT,
                    "the session is no longer active; its document cannot change",
                )
                    .into_response();
            }
            Some(session) => session.images_dir(),
        }
    };
    if body.is_empty() {
        return bad_request("the image body is empty".into());
    }
    if !bytes_are_image(content_type, &body) {
        return bad_request(format!(
            "the bytes are not a {} image",
            format_label(content_type)
        ));
    }
    if let Err(error) = tokio::fs::create_dir_all(&dir).await {
        tracing::error!(id = %id, error = %error, "failed to create the image directory");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Err(error) = tokio::fs::write(dir.join(&name), &body).await {
        tracing::error!(id = %id, name = %name, error = %error, "failed to write a document image");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    tracing::info!(id = %id, name = %name, bytes = body.len(), "document image stored");
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "name": name,
            "url": format!("/api/sessions/{id}/image/{name}"),
            "markdown": format!("![{}](image/{})", alt_from_name(&name), name),
        })),
    )
        .into_response()
}

pub(super) async fn document_image(
    State(state): State<AppState>,
    Path((id, name)): Path<(String, String)>,
) -> Response {
    let Some(content_type) = document_image_type(&name) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let dir = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            Some(session) => session.images_dir(),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    match tokio::fs::read(dir.join(&name)).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, content_type),
                // The page and these images share an origin. A browser must not decide
                // for itself that the bytes are something executable.
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                // A redrawn screenshot reuses its name, and a stale one would be worse
                // than a slow one.
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// How long an unanswered screenshot request stands before a repeat ask makes a new one.
/// Long enough that an agent retrying while it polls reuses the id, short enough that a
/// tablet which never answered is not waited on forever.
pub(super) const SCREENSHOT_COALESCE: Duration = Duration::from_secs(20);

/// Ask the tablet for a picture of what it is showing right now.
///
/// The agent has the HTML and the element map; what it does not have is the reader's
/// actual view — scrolled where, zoomed how much, ink over what. That is the part of a
/// layout bug that only the device can answer, and asking costs the reader nothing but a
/// brief "the agent wants to see this" line.
///
/// Answers 202 with the request id the PNG will be filed under. `delivered` says whether
/// a tablet has the session open; a caller that gets `false` can say so rather than wait.
pub(super) async fn request_screenshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(session) if session.status != SessionStatus::Active => {
                return (
                    StatusCode::CONFLICT,
                    "the session is no longer active; there is no page to photograph",
                )
                    .into_response();
            }
            Some(_) => {}
        }
    }

    let request_id = {
        let mut pending = state.screenshot_requests.write().await;
        match pending.get(&id) {
            Some(request) if request.is_pending() => request.id.clone(),
            _ => {
                let fresh = uuid::Uuid::new_v4().as_simple().to_string();
                pending.insert(
                    id.clone(),
                    ScreenshotRequest {
                        id: fresh.clone(),
                        asked_at: tokio::time::Instant::now(),
                        failure: None,
                    },
                );
                fresh
            }
        }
    };

    // Delivered means a tablet heard it. The agent's own observer socket also receives
    // the broadcast, and counting it once made the CLI wait twenty seconds for a device
    // that was not there.
    let delivered = state.tablets_listening(&id) > 0;
    state
        .ws_send(
            &id,
            ServerMessage::ScreenshotRequested {
                request_id: request_id.clone(),
            },
        )
        .await;
    tracing::info!(id = %id, request_id = %request_id, delivered, "screenshot requested");

    (
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "id": request_id,
            "url": format!("/api/sessions/{id}/screenshot/{request_id}"),
            "delivered": delivered,
        })),
    )
        .into_response()
}

/// The tablet's answer: the page as it is, composited with whatever is drawn on it.
///
/// Only the request the server is waiting on can be answered, so a caller cannot fill the
/// session with pictures nobody asked for. A JSON body (`{"error": "..."}`) is the tablet
/// saying it could not take the picture; the agent then hears why instead of waiting out
/// its timeout.
pub(super) async fn upload_screenshot(
    State(state): State<AppState>,
    Path((id, request_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // The id becomes a file name, so it has to be one this server could have made.
    if !is_request_id(&request_id) {
        return bad_request("the request id is not one this server issues".into());
    }
    let dir = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(session) => session.screenshots_dir(),
        }
    };
    {
        let pending = state.screenshot_requests.read().await;
        if pending.get(&id).map(|request| request.id.as_str()) != Some(request_id.as_str()) {
            return (
                StatusCode::NOT_FOUND,
                "no screenshot with that id is being waited for",
            )
                .into_response();
        }
    }
    if body.is_empty() {
        return bad_request("the screenshot body is empty".into());
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("image/png");
    if content_type.starts_with("application/json") {
        let reason = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| value["error"].as_str().map(str::to_string))
            .filter(|reason| !reason.trim().is_empty());
        let Some(reason) = reason else {
            return bad_request("a failure report needs an `error` string".into());
        };
        let mut pending = state.screenshot_requests.write().await;
        if let Some(request) = pending.get_mut(&id).filter(|r| r.id == request_id) {
            request.failure = Some(reason.clone());
        }
        tracing::warn!(id = %id, request_id = %request_id, reason = %reason, "screenshot failed on the tablet");
        return StatusCode::NO_CONTENT.into_response();
    }
    if !bytes_are_image(content_type, &body) {
        return bad_request(format!(
            "the bytes are not a {} image",
            format_label(content_type)
        ));
    }
    if let Err(error) = tokio::fs::create_dir_all(&dir).await {
        tracing::error!(id = %id, error = %error, "failed to create the screenshot directory");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if let Err(error) = tokio::fs::write(dir.join(format!("{request_id}.png")), &body).await {
        tracing::error!(id = %id, error = %error, "failed to write a screenshot");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    // Answered: the next ask is a new picture, not this one again.
    let mut pending = state.screenshot_requests.write().await;
    if pending
        .get(&id)
        .is_some_and(|request| request.id == request_id)
    {
        pending.remove(&id);
    }
    tracing::info!(id = %id, request_id = %request_id, bytes = body.len(), "screenshot stored");
    StatusCode::NO_CONTENT.into_response()
}

/// Fetch a screenshot the tablet has sent. 404 means "not yet", which is what a caller
/// polling for one will see until it arrives; 410 means the tablet tried and could not,
/// with its reason, so the caller stops polling.
pub(super) async fn get_screenshot(
    State(state): State<AppState>,
    Path((id, request_id)): Path<(String, String)>,
) -> Response {
    if !is_request_id(&request_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let dir = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            Some(session) => session.screenshots_dir(),
            None => return StatusCode::NOT_FOUND.into_response(),
        }
    };
    let failure = state
        .screenshot_requests
        .read()
        .await
        .get(&id)
        .filter(|request| request.id == request_id)
        .and_then(|request| request.failure.clone());
    if let Some(reason) = failure {
        return (
            StatusCode::GONE,
            format!("the tablet could not take the picture: {reason}"),
        )
            .into_response();
    }
    match tokio::fs::read(dir.join(format!("{request_id}.png"))).await {
        Ok(bytes) => (
            [
                (header::CONTENT_TYPE, "image/png"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                (header::CACHE_CONTROL, "no-store"),
            ],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// 32 lowercase hex characters: what `Uuid::as_simple` produces, and nothing else.
pub(super) fn is_request_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
