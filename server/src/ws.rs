use axum::{
    extract::{
        Path, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::api::{AnnotationGroup, SessionResultResponse};
use crate::app::AppState;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Subscribe,
    RequestUpdate {
        annotations: Vec<AnnotationGroup>,
        typed_notes: String,
    },
    UpdateContent {
        content: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    ProcessingStatus {
        step: String,
        message: String,
    },
    AnnotationResult {
        annotations: Vec<AnnotationGroup>,
        version: u32,
    },
    VersionUpdated {
        version: u32,
    },
    SessionSubmitted {
        result: Box<SessionResultResponse>,
    },
    Error {
        message: String,
    },
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    {
        let mgr = state.sessions.read().await;
        if mgr.get(&id).is_none() {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    ws.on_upgrade(move |socket| handle_ws(socket, id, state))
}

async fn handle_ws(mut socket: WebSocket, id: String, state: AppState) {
    info!(session_id = %id, "ws connected");
    let mut rx = state.ws_subscribe(&id).await;

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(msg) => {
                        let json = serde_json::to_string(&msg).unwrap_or_default();
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            warn!(session_id = %id, "ws send failed, closing");
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(session_id = %id, skipped = n, "ws receiver lagged");
                        continue;
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        info!(session_id = %id, msg_type = extract_type(&text), "ws message received");
                        handle_client_message(&state, &id, &text).await;
                    }
                    Some(Ok(Message::Close(_))) => {
                        info!(session_id = %id, "ws closed by client");
                        break;
                    }
                    None => {
                        info!(session_id = %id, "ws stream ended");
                        break;
                    }
                    Some(Err(e)) => {
                        warn!(session_id = %id, error = %e, "ws error");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    info!(session_id = %id, "ws disconnected");
}

fn extract_type(text: &str) -> &str {
    // fast path: find "type":"<value>" without full parse
    let after = text
        .find("\"type\"")
        .and_then(|i| text[i + 6..].find('"').map(|j| i + 6 + j + 1));
    if let Some(start) = after {
        let end = text[start..]
            .find('"')
            .map(|k| start + k)
            .unwrap_or(text.len());
        return &text[start..end];
    }
    "unknown"
}

async fn handle_client_message(state: &AppState, id: &str, text: &str) {
    let msg = match serde_json::from_str::<ClientMessage>(text) {
        Ok(m) => m,
        Err(e) => {
            state
                .ws_send(
                    id,
                    ServerMessage::Error {
                        message: e.to_string(),
                    },
                )
                .await;
            return;
        }
    };

    match msg {
        ClientMessage::Subscribe => {}
        ClientMessage::RequestUpdate {
            annotations,
            typed_notes,
        } => {
            handle_request_update(state, id, annotations, typed_notes).await;
        }
        ClientMessage::UpdateContent { content } => {
            handle_update_content(state, id, content).await;
        }
    }
}

async fn handle_request_update(
    state: &AppState,
    id: &str,
    annotations: Vec<AnnotationGroup>,
    typed_notes: String,
) {
    let Some(annotations) = ({
        let mut mgr = state.sessions.write().await;
        mgr.request_update(id, annotations, typed_notes)
    }) else {
        state
            .ws_send(
                id,
                ServerMessage::Error {
                    message: "session not active".into(),
                },
            )
            .await;
        return;
    };

    let version = {
        let mgr = state.sessions.read().await;
        mgr.get(id).map(|s| s.version).unwrap_or(1)
    };

    if let Some(engine) = state.ocr_engine.clone() {
        state
            .ws_send(
                id,
                ServerMessage::ProcessingStatus {
                    step: "ocr".into(),
                    message: "Running OCR on annotations...".into(),
                },
            )
            .await;

        let ocr_state = state.clone();
        let ocr_id = id.to_string();
        let mut anns = annotations;
        tokio::spawn(async move {
            crate::ocr::ocr_annotation_groups(&engine, &mut anns).await;
            {
                let mut mgr = ocr_state.sessions.write().await;
                mgr.update_annotations(&ocr_id, anns.clone());
            }
            ocr_state
                .ws_send(
                    &ocr_id,
                    ServerMessage::AnnotationResult {
                        annotations: strip_strokes(anns),
                        version,
                    },
                )
                .await;
        });
    } else {
        state
            .ws_send(
                id,
                ServerMessage::AnnotationResult {
                    annotations: strip_strokes(annotations),
                    version,
                },
            )
            .await;
    }
}

async fn handle_update_content(state: &AppState, id: &str, content: String) {
    let result = {
        let mut mgr = state.sessions.write().await;
        mgr.apply_update(id, content)
    };

    match result {
        Some(version) => {
            state
                .ws_send(id, ServerMessage::VersionUpdated { version })
                .await;
        }
        None => {
            state
                .ws_send(
                    id,
                    ServerMessage::Error {
                        message: "session not in processing state".into(),
                    },
                )
                .await;
        }
    }
}

fn strip_strokes(annotations: Vec<AnnotationGroup>) -> Vec<AnnotationGroup> {
    annotations
        .into_iter()
        .map(|mut a| {
            a.strokes = vec![];
            a
        })
        .collect()
}
