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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_type_is_read_for_logging() {
        assert_eq!(extract_type(r#"{"type":"subscribe"}"#), "subscribe");
        assert_eq!(extract_type(r#"{"type": "subscribe"}"#), "subscribe");
        assert_eq!(
            extract_type(r#"{"type":"annotation_result","feedback_id":"abc"}"#),
            "annotation_result"
        );
    }

    #[test]
    fn an_unreadable_message_reports_unknown_rather_than_panicking() {
        // This runs inside the socket task: a panic here would kill the connection the
        // reader's feedback arrives on.
        for text in [
            "",
            "not json",
            "{}",
            r#"{"type":}"#,
            r#"{"type""#,
            r#"{"type"}"#,
            r#""type""#,
            r#"{"other":"type"}"#,
            "{\"type\":\"波兰语\"}",
            r#"{"type":"a\"b"}"#,
            "🖊",
            &"x".repeat(10_000),
        ] {
            let _ = extract_type(text);
        }
        assert_eq!(extract_type("{}"), "unknown");
        assert_eq!(extract_type(r#"{"other":"type"}"#), "unknown");
    }

    #[test]
    fn a_server_message_serialises_the_way_the_client_reads_it() {
        // The app switches on these tags and field names.
        let version = serde_json::to_value(ServerMessage::VersionUpdated {
            version: 3,
            consumed_annotations: vec![0, 2],
            consumed_strokes: vec![vec![[1.0, 2.0]]],
        })
        .unwrap();
        assert_eq!(version["type"], "version_updated");
        assert_eq!(version["version"], 3);
        assert_eq!(version["consumed_annotations"][1], 2);
        assert_eq!(version["consumed_strokes"][0][0][0], 1.0);

        let error = serde_json::to_value(ServerMessage::Error {
            message: "cancelled".into(),
        })
        .unwrap();
        assert_eq!(error["type"], "error");
        assert_eq!(error["message"], "cancelled");
    }

    #[test]
    fn an_empty_acknowledgement_does_not_send_empty_arrays() {
        // The CLI treats absent and empty differently: absent means "the client already
        // consumed the annotation", empty would mean "nothing was consumed".
        let message = serde_json::to_value(ServerMessage::VersionUpdated {
            version: 2,
            consumed_annotations: Vec::new(),
            consumed_strokes: Vec::new(),
        })
        .unwrap();
        assert!(message.get("consumed_annotations").is_none());
        assert!(message.get("consumed_strokes").is_none());
    }

    #[test]
    fn the_client_message_shape_is_the_one_the_cli_sends() {
        let subscribe: ClientMessage = serde_json::from_str(r#"{"type":"subscribe"}"#).unwrap();
        assert!(matches!(subscribe, ClientMessage::Subscribe));
        // Anything else is ignored rather than fatal, so an unknown tag must not parse.
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"nonsense"}"#).is_err());
    }
}
