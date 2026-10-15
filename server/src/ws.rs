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

use crate::api::SessionResultResponse;
use crate::app::AppState;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Subscribe,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    VersionUpdated {
        version: u32,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        consumed_annotations: Vec<i32>,
        #[serde(skip_serializing_if = "Vec::is_empty", default)]
        consumed_strokes: Vec<Vec<[f64; 2]>>,
    },
    SessionSubmitted {
        result: Box<SessionResultResponse>,
    },
    AnnotationResult {
        feedback_id: String,
        version: u32,
        annotations: Vec<crate::api::AnnotationGroup>,
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
    crate::metrics::WS_CONNECTIONS_ACTIVE.inc();
    let mut rx = state.ws_subscribe(&id).await;

    // Subscribe before reading the durable snapshot: an update cannot fall between
    // snapshot and subscription. Duplicates are possible; losing feedback is not.
    let (update, replay) = {
        let mgr = state.sessions.read().await;
        mgr.get(&id)
            .filter(|s| s.status == crate::session::SessionStatus::Active)
            .map(|s| (s.last_content_update.clone(), s.pending_feedback.clone()))
            .unwrap_or_default()
    };
    if let Some(update) = update
        && socket
            .send(Message::Text(update.to_string().into()))
            .await
            .is_err()
    {
        crate::metrics::WS_CONNECTIONS_ACTIVE.dec();
        return;
    }
    if let Some(feedback) = replay
        && socket
            .send(Message::Text(feedback.to_string().into()))
            .await
            .is_err()
    {
        crate::metrics::WS_CONNECTIONS_ACTIVE.dec();
        return;
    }

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
                        // Only Subscribe is expected; ignore unknown messages.
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
    crate::metrics::WS_CONNECTIONS_ACTIVE.dec();
}

fn extract_type(text: &str) -> &str {
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
