use clap::Parser;
use reqwest::Client;
use std::path::PathBuf;
use std::time::Duration;

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
