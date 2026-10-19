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

/// How a review ended, as the message the live path would have sent at the time.
///
/// A client that reconnects after the submit, cancellation or expiry must be told, or it
/// sits on a silent socket until its own deadline: `eink-review push` used to wait its
/// full thirty minutes for a result that had already been written.
pub fn farewell(session: &crate::session::Session) -> Option<ServerMessage> {
    use crate::session::SessionStatus;
    match session.status {
        SessionStatus::Active => None,
        SessionStatus::Submitted => Some(ServerMessage::SessionSubmitted {
            result: Box::new(SessionResultResponse::from_session(session)),
        }),
        SessionStatus::Cancelled => Some(ServerMessage::Error {
            message: "session cancelled".into(),
        }),
        SessionStatus::Expired => Some(ServerMessage::Error {
            message: "session expired".into(),
        }),
    }
}

async fn farewell_for(state: &AppState, id: &str) -> Option<ServerMessage> {
    state.sessions.read().await.get(id).and_then(farewell)
}

/// What a client needs in order to be in sync: the last content update and any feedback
/// it has not acknowledged, for a session that is still being read.
async fn durable_snapshot(state: &AppState, id: &str) -> Vec<serde_json::Value> {
    let mgr = state.sessions.read().await;
    mgr.get(id)
        .filter(|s| s.status == crate::session::SessionStatus::Active)
        .map(|s| [s.last_content_update.clone(), s.pending_feedback.clone()])
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .collect()
}

async fn send_snapshot(socket: &mut WebSocket, state: &AppState, id: &str) -> Result<(), ()> {
    for message in durable_snapshot(state, id).await {
        if socket
            .send(Message::Text(message.to_string().into()))
            .await
            .is_err()
        {
            return Err(());
        }
    }
    Ok(())
}

async fn handle_ws(mut socket: WebSocket, id: String, state: AppState) {
    info!(session_id = %id, "ws connected");
    crate::metrics::WS_CONNECTIONS_ACTIVE.inc();
    // Subscribe before reading the durable snapshot: an update cannot fall between
    // snapshot and subscription. Duplicates are possible; losing feedback is not.
    let mut rx = state.ws_subscribe(&id).await;
    if send_snapshot(&mut socket, &state, &id).await.is_err() {
        crate::metrics::WS_CONNECTIONS_ACTIVE.dec();
        return;
    }
    // Checked after subscribing, so a review that ends between the handshake and here is
    // heard either on the broadcast or from the snapshot, never missed by both.
    if let Some(message) = farewell_for(&state, &id).await {
        info!(session_id = %id, "ws connected to a finished session; saying how it ended");
        let json = serde_json::to_string(&message).unwrap_or_default();
        let _ = socket.send(Message::Text(json.into())).await;
        let _ = socket.send(Message::Close(None)).await;
        state.ws_cleanup(&id).await;
        info!(session_id = %id, "ws disconnected");
        crate::metrics::WS_CONNECTIONS_ACTIVE.dec();
        return;
    }

    let heartbeat = state.ws_heartbeat_seconds;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(heartbeat.max(1)));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await; // the first tick fires immediately
    let mut last_seen = tokio::time::Instant::now();

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
                        // The client is connected but fell behind, so it has missed
                        // messages -- and the ones it lost may include the only copy of a
                        // note. Re-send the durable state instead of pretending it is in
                        // sync; the snapshot is what a reconnecting client gets.
                        warn!(session_id = %id, skipped = n, "ws receiver lagged, resyncing");
                        if send_snapshot(&mut socket, &state, &id).await.is_err() {
                            break;
                        }
                        last_seen = tokio::time::Instant::now();
                    }
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        last_seen = tokio::time::Instant::now();
                        info!(session_id = %id, msg_type = extract_type(&text), "ws message received");
                        // Only Subscribe is expected; ignore unknown messages.
                    }
                    Some(Ok(Message::Pong(_))) | Some(Ok(Message::Ping(_))) => {
                        last_seen = tokio::time::Instant::now();
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
            // A half-open socket looks connected to both ends: a NAT rebind or a tablet
            // that went to sleep leaves the server writing into a void and the gauge
            // reporting a client that is not there. Ping, and give up on silence.
            _ = ticker.tick(), if heartbeat > 0 => {
                if last_seen.elapsed() > std::time::Duration::from_secs(heartbeat * 3) {
                    warn!(session_id = %id, "no word from the client, closing");
                    break;
                }
                if socket.send(Message::Ping(Vec::new().into())).await.is_err() {
                    warn!(session_id = %id, "ws ping failed, closing");
                    break;
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

    #[tokio::test]
    async fn the_durable_snapshot_is_what_a_client_needs_to_be_in_sync() {
        use crate::app::AppState;
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_config(dir.path().to_path_buf(), 5);
        let id = {
            let mut mgr = state.sessions.write().await;
            mgr.create(
                "# Review".into(),
                None,
                None,
                Default::default(),
                false,
                None,
            )
            .id
        };

        // Nothing has happened yet.
        assert!(durable_snapshot(&state, &id).await.is_empty());
        // An unknown session has nothing to say either.
        assert!(durable_snapshot(&state, "ghost").await.is_empty());

        {
            let mut mgr = state.sessions.write().await;
            // A fresh session is at version 1, and feedback is only accepted for the
            // version it was written against.
            let update = serde_json::json!({"type": "version_updated", "version": 1});
            mgr.record_content_update(&id, update.clone(), false);
            let feedback = serde_json::json!({
                "type": "annotation_result", "feedback_id": "abc", "version": 1,
                "annotations": [{"recognized_text": "WRITE A TLDR"}],
            });
            assert!(mgr.record_feedback(&id, 1, feedback.clone()));
        }
        let snapshot = durable_snapshot(&state, &id).await;
        assert_eq!(snapshot.len(), 2, "the update and the unacknowledged note");
        assert!(snapshot.iter().any(|m| m["type"] == "version_updated"));
        assert!(snapshot.iter().any(|m| m["type"] == "annotation_result"));
    }

    #[tokio::test]
    async fn a_finished_session_says_how_it_ended_instead_of_going_quiet() {
        // The live path tells a connected client about a submit, a cancel or an expiry.
        // A client connecting afterwards hears the same message, in the same shape.
        use crate::app::AppState;
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_config(dir.path().to_path_buf(), 5);
        let mut mgr = state.sessions.write().await;
        let make = |mgr: &mut crate::session::SessionManager| {
            mgr.create(
                "# Review".into(),
                None,
                None,
                Default::default(),
                false,
                None,
            )
            .id
        };
        let live = make(&mut mgr);
        assert!(farewell(mgr.get(&live).unwrap()).is_none());

        let submitted = make(&mut mgr);
        mgr.submit(&submitted, "LGTM".into(), vec![], None, None, vec![]);
        let message =
            serde_json::to_value(farewell(mgr.get(&submitted).unwrap()).unwrap()).unwrap();
        assert_eq!(message["type"], "session_submitted");
        assert_eq!(message["result"]["status"], "Submitted");
        assert_eq!(message["result"]["verdict"], "lgtm");

        let cancelled = make(&mut mgr);
        mgr.cancel(&cancelled);
        let message =
            serde_json::to_value(farewell(mgr.get(&cancelled).unwrap()).unwrap()).unwrap();
        assert_eq!(message["type"], "error");
        assert_eq!(message["message"], "session cancelled");

        let expired = make(&mut mgr);
        mgr.expire_stale(std::time::Duration::ZERO);
        let message = serde_json::to_value(farewell(mgr.get(&expired).unwrap()).unwrap()).unwrap();
        assert_eq!(message["type"], "error");
        assert_eq!(message["message"], "session expired");
    }

    #[tokio::test]
    async fn a_finished_session_has_no_snapshot_to_replay() {
        // Replaying to a session that is over would look like the review was reopened.
        use crate::app::AppState;
        let dir = tempfile::tempdir().unwrap();
        let state = AppState::with_config(dir.path().to_path_buf(), 5);
        let id = {
            let mut mgr = state.sessions.write().await;
            mgr.create(
                "# Review".into(),
                None,
                None,
                Default::default(),
                false,
                None,
            )
            .id
        };
        {
            let mut mgr = state.sessions.write().await;
            mgr.record_content_update(&id, serde_json::json!({"version": 2}), false);
            mgr.submit(&id, "LGTM".into(), vec![], None, None, vec![]);
        }
        assert!(durable_snapshot(&state, &id).await.is_empty());
    }

    #[test]
    fn the_client_message_shape_is_the_one_the_cli_sends() {
        let subscribe: ClientMessage = serde_json::from_str(r#"{"type":"subscribe"}"#).unwrap();
        assert!(matches!(subscribe, ClientMessage::Subscribe));
        // Anything else is ignored rather than fatal, so an unknown tag must not parse.
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"nonsense"}"#).is_err());
    }
}
