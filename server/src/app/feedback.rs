use super::*;

#[derive(Deserialize)]
pub(super) struct OcrRequest {
    strokes: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    pressures: Vec<Vec<f64>>,
}

/// Refuse ink the rasteriser should not be asked to draw. The body limit bounds bytes,
/// not work: half a million points is a 10 MB body and minutes of drawing.
pub(super) fn strokes_within_limits<'a>(
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

pub(super) async fn ocr_strokes(
    State(state): State<AppState>,
    Json(body): Json<OcrRequest>,
) -> Response {
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

#[derive(Deserialize)]
pub(super) struct UpdateContentBody {
    content: String,
    #[serde(default)]
    consumed_annotations: Vec<i32>,
    #[serde(default)]
    expected_version: Option<u32>,
    #[serde(default)]
    feedback_id: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct ReanchorBody {
    previous: Vec<crate::reanchor::Element>,
    current: Vec<crate::reanchor::Element>,
}

pub(super) async fn pending_feedback(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(s) => Json(serde_json::json!({"status":s.status,"version":s.version,"pending":s.pending_feedback,"last_content_update":s.last_content_update})).into_response(),
    }
}

pub(super) async fn get_content(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let mgr = state.sessions.read().await;
    match mgr.get(&id) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(s) => {
            Json(serde_json::json!({"status":s.status,"version":s.version,"content":s.content}))
                .into_response()
        }
    }
}

pub(super) async fn reanchor(
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
    let deadline = std::time::Instant::now() + state.reanchor_budget;
    let targets = tokio::task::spawn_blocking(move || {
        crate::reanchor::targets_within(&body.previous, &body.current, Some(deadline))
    })
    .await;
    match targets {
        Ok(Some(targets)) => Json(serde_json::json!({"targets":targets})).into_response(),
        // Better to leave the notes where they are than to hold the page while the
        // comparison runs on: the client keeps its current placement. The comparison
        // stops at the same deadline rather than pinning a core for the rest of it.
        Ok(None) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "relocating the notes took too long; keep them where they are",
        )
            .into_response(),
        // The worker panicked or the runtime could not schedule it.
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub(super) async fn update_content(
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
        let mut consumed_notes = Vec::new();
        for &index in &consumed_annotations {
            let Some(annotation) = usize::try_from(index)
                .ok()
                .and_then(|i| annotations.and_then(|a| a.get(i)))
            else {
                return bad_request("consumed annotation index is out of range".into());
            };
            consumed_notes.extend(crate::ws::ConsumedNote::from_annotation(annotation));
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
                consumed_notes,
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
pub(super) fn callback_url(raw: &str) -> Option<String> {
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

pub(super) async fn set_callback(
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

#[derive(serde::Deserialize)]
pub(super) struct ProgressBody {
    feedback_id: String,
    message: String,
    #[serde(default)]
    step: Option<u32>,
    #[serde(default)]
    of: Option<u32>,
}

/// The longest progress line a reviewer can read at a glance on e-ink. In characters,
/// not bytes: a line of Polish must not be rejected for being two bytes per letter.
pub(super) const MAX_PROGRESS_CHARS: usize = 120;

/// A short line about what the agent is doing, while it does it.
///
/// This is status, not prose: the reader is looking at "Waiting for agent" and needs to
/// know whether their notes were read and which section is being rewritten. It is stored
/// on the session (so a reconnecting tablet gets the latest line) and cleared only by the
/// content update that names the same feedback id.
pub(super) async fn agent_progress(
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

pub(super) fn webhook_payload(session: &Session) -> Option<(String, SessionResultResponse)> {
    session
        .callback_url
        .clone()
        .map(|url| (url, SessionResultResponse::from_session(session)))
}

pub(super) async fn get_result(
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

pub(super) async fn submit_review(
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
pub(super) fn move_in(
    staging: &std::path::Path,
    dir: &std::path::Path,
    staged: Vec<String>,
) -> Vec<String> {
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

pub(super) async fn request_update(
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
pub(super) struct AnnotationWebhook {
    #[serde(rename = "type")]
    r#type: &'static str,
    id: String,
    version: u32,
    feedback_id: String,
    annotations: Vec<AnnotationGroup>,
}

/// What a submit carries once it is parsed, before anything has touched the disk.
pub(super) struct Submission {
    pub(super) typed_notes: String,
    pub(super) images: Vec<Bytes>,
    pub(super) verdict: Option<Verdict>,
    pub(super) stroke_data: Option<serde_json::Value>,
    pub(super) annotations: Vec<AnnotationGroup>,
    pub(super) document_version: Option<u32>,
}

/// The eight bytes every PNG starts with. The images are served back as `image/png`, so
/// what is accepted has to be one.
pub(super) const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";

pub(super) fn parse_json_submit(body: &[u8]) -> Result<Submission, SubmitError> {
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

pub(super) fn parse_verdict_override(
    verdict: Option<String>,
) -> Result<Option<Verdict>, SubmitError> {
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
