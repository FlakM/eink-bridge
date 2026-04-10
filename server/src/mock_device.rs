use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message as WsMessage;

#[derive(Parser)]
#[command(
    name = "eink-mock-device",
    about = "Simulate a Boox device for testing"
)]
struct Cli {
    #[arg(long, default_value = "http://localhost:3333")]
    server: String,

    /// Notes to submit
    #[arg(long, default_value = "Mock device feedback")]
    notes: String,

    /// Optional image file to attach
    #[arg(long)]
    image: Option<PathBuf>,

    /// Poll interval in seconds
    #[arg(long, default_value = "2")]
    poll_interval: u64,

    /// Only process this specific session ID (implies --once)
    #[arg(long)]
    session_id: Option<String>,

    /// Handle one session and exit
    #[arg(long)]
    once: bool,

    /// Delay before submitting (simulates human think time)
    #[arg(long, default_value = "0")]
    delay: u64,

    /// Simulate interactive mode: send request_update via WS before submitting.
    /// Waits for annotation_result then version_updated before submitting.
    #[arg(long)]
    interactive: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = Client::new();

    loop {
        let sessions = if let Some(ref id) = cli.session_id {
            // Fetch the specific session directly
            let resp = client
                .get(format!("{}/api/sessions/{id}", cli.server))
                .send()
                .await?;
            if resp.status().is_success() {
                let s: serde_json::Value = resp.json().await?;
                if s["status"].as_str() == Some("Active") {
                    vec![s]
                } else {
                    tokio::time::sleep(Duration::from_secs(cli.poll_interval)).await;
                    continue;
                }
            } else {
                tokio::time::sleep(Duration::from_secs(cli.poll_interval)).await;
                continue;
            }
        } else {
            poll_active(&client, &cli.server).await?
        };
        for session in sessions {
            let id = session["id"].as_str().unwrap_or_default();
            if id.is_empty() {
                continue;
            }
            eprintln!("mock-device: found session {id}, processing...");

            // Fetch HTML page (validates it loads)
            let html_resp = client
                .get(format!("{}/session/{id}", cli.server))
                .send()
                .await?;
            if !html_resp.status().is_success() {
                eprintln!("mock-device: failed to load HTML for {id}");
                continue;
            }
            let html = html_resp.text().await?;
            eprintln!("mock-device: loaded HTML ({} bytes)", html.len());

            if cli.delay > 0 {
                tokio::time::sleep(Duration::from_secs(cli.delay)).await;
            }

            if cli.interactive {
                simulate_request_update(&cli.server, id).await;
            }

            // Submit review
            let mut form = reqwest::multipart::Form::new().text("typed_notes", cli.notes.clone());
            if let Some(img_path) = &cli.image {
                let bytes = std::fs::read(img_path)?;
                let part = reqwest::multipart::Part::bytes(bytes)
                    .file_name("annotation.png")
                    .mime_str("image/png")?;
                form = form.part("annotation", part);
            }

            let resp = client
                .post(format!("{}/api/sessions/{id}/submit", cli.server))
                .multipart(form)
                .send()
                .await?;

            if resp.status().is_success() {
                eprintln!("mock-device: submitted review for session {id}");
            } else {
                eprintln!("mock-device: submit failed for {id}: {}", resp.status());
            }

            if cli.once || cli.session_id.is_some() {
                return Ok(());
            }
        }

        tokio::time::sleep(Duration::from_secs(cli.poll_interval)).await;
    }
}

async fn poll_active(client: &Client, server: &str) -> anyhow::Result<Vec<serde_json::Value>> {
    let resp = client
        .get(format!("{server}/api/sessions?status=active"))
        .send()
        .await?;
    if !resp.status().is_success() {
        return Ok(vec![]);
    }
    Ok(resp.json().await?)
}

/// Simulate a tablet tapping "Request Update":
///   1. Connect WS, send request_update with empty annotations
///   2. Wait for annotation_result (server processed the annotations)
///   3. Wait for version_updated (agent pushed new content via HTTP PUT)
async fn simulate_request_update(server: &str, session_id: &str) {
    let ws_url = server
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1)
        + "/ws/"
        + session_id;

    let Ok(Ok((mut ws, _))) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(&ws_url),
    )
    .await
    else {
        eprintln!("mock-device: WS connect failed for interactive update");
        return;
    };

    eprintln!("mock-device: connected WS for session {session_id}");

    let msg = serde_json::json!({
        "type": "request_update",
        "annotations": [],
        "typed_notes": "mock annotation"
    });
    if ws.send(WsMessage::Text(msg.to_string())).await.is_err() {
        eprintln!("mock-device: failed to send request_update");
        return;
    }
    eprintln!("mock-device: sent request_update — waiting for annotation_result...");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let msg = tokio::time::timeout(remaining, ws.next()).await;
        match msg {
            Ok(Some(Ok(WsMessage::Text(text)))) => {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                let t = v["type"].as_str().unwrap_or("?");
                eprintln!("mock-device: ws ← {t}");
                match t {
                    "annotation_result" => {
                        eprintln!(
                            "mock-device: annotation_result received — waiting for version_updated (agent should call `eink-review update`)..."
                        );
                    }
                    "version_updated" => {
                        let version = v["version"].as_u64().unwrap_or(0);
                        eprintln!("mock-device: version_updated v{version} — document reloaded");
                        break;
                    }
                    _ => {}
                }
            }
            _ => break,
        }
    }
}
