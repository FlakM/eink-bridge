use clap::{Parser, Subcommand};
use reqwest::Client;
use std::io::Read as _;
use std::time::{Duration, Instant};

#[derive(Parser)]
#[command(name = "eink-review", about = "E-ink review session CLI")]
struct Cli {
    #[arg(long, default_value = "http://localhost:3333")]
    server: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Push content for review
    Push {
        /// File to push (use - for stdin)
        file: String,
        /// Session title
        #[arg(long)]
        title: Option<String>,
        /// Timeout in minutes
        #[arg(long, default_value = "30")]
        timeout: u64,
        /// Non-blocking: print session ID and exit
        #[arg(long = "async")]
        non_blocking: bool,
    },
    /// Get result of a session
    Result {
        /// Session ID
        id: String,
    },
    /// Cancel a session
    Cancel {
        /// Session ID
        id: String,
    },
    /// List sessions
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let client = Client::new();

    let result = match cli.command {
        Command::Push {
            file,
            title,
            timeout,
            non_blocking,
        } => cmd_push(&client, &cli.server, &file, title, timeout, non_blocking).await,
        Command::Result { id } => cmd_result(&client, &cli.server, &id).await,
        Command::Cancel { id } => cmd_cancel(&client, &cli.server, &id).await,
        Command::List { status } => cmd_list(&client, &cli.server, status).await,
    };

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn cmd_push(
    client: &Client,
    server: &str,
    file: &str,
    title: Option<String>,
    timeout_minutes: u64,
    non_blocking: bool,
) -> anyhow::Result<()> {
    let content = if file == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(file)?
    };

    let title = title.or_else(|| extract_title(&content));

    let mut url = format!("{server}/api/sessions");
    if let Some(t) = &title {
        url = format!("{url}?title={}", urlencoding::encode(t));
    }

    let resp = client.post(&url).body(content).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("failed to create session: {}", resp.status());
    }

    let body: serde_json::Value = resp.json().await?;
    let id = body["id"].as_str().unwrap_or_default();
    let session_url = format!("{server}/session/{id}");

    if non_blocking {
        println!("{id}");
        eprintln!("Session: {session_url}");
        return Ok(());
    }

    eprintln!("Session: {session_url}");
    eprintln!("Waiting for review... (timeout: {timeout_minutes}m)");

    let deadline = Instant::now() + Duration::from_secs(timeout_minutes * 60);
    loop {
        if Instant::now() > deadline {
            anyhow::bail!("timeout waiting for review");
        }

        let resp = client
            .get(format!("{server}/api/sessions/{id}/result"))
            .send()
            .await?;

        match resp.status().as_u16() {
            200 => {
                let body: serde_json::Value = resp.json().await?;
                print_result(id, &body);
                return Ok(());
            }
            204 => continue, // long-poll timeout, retry
            410 => anyhow::bail!("session was cancelled"),
            404 => anyhow::bail!("session not found"),
            s => anyhow::bail!("unexpected status: {s}"),
        }
    }
}

async fn cmd_result(client: &Client, server: &str, id: &str) -> anyhow::Result<()> {
    let resp = client
        .get(format!("{server}/api/sessions/{id}/result"))
        .send()
        .await?;

    match resp.status().as_u16() {
        200 => {
            let body: serde_json::Value = resp.json().await?;
            print_result(id, &body);
            Ok(())
        }
        204 => {
            eprintln!("session still active (not yet submitted)");
            Ok(())
        }
        410 => anyhow::bail!("session was cancelled"),
        404 => anyhow::bail!("session not found"),
        s => anyhow::bail!("unexpected status: {s}"),
    }
}

async fn cmd_cancel(client: &Client, server: &str, id: &str) -> anyhow::Result<()> {
    let resp = client
        .delete(format!("{server}/api/sessions/{id}"))
        .send()
        .await?;

    if resp.status().is_success() {
        eprintln!("session {id} cancelled");
        Ok(())
    } else {
        anyhow::bail!("failed to cancel: {}", resp.status())
    }
}

async fn cmd_list(client: &Client, server: &str, status: Option<String>) -> anyhow::Result<()> {
    let mut url = format!("{server}/api/sessions");
    if let Some(s) = &status {
        url = format!("{url}?status={s}");
    }

    let resp = client.get(&url).send().await?;
    let sessions: Vec<serde_json::Value> = resp.json().await?;

    if sessions.is_empty() {
        eprintln!("no sessions");
        return Ok(());
    }

    for s in &sessions {
        let id = s["id"].as_str().unwrap_or("?");
        let title = s["title"].as_str().unwrap_or("(untitled)");
        let status = s["status"].as_str().unwrap_or("?");
        println!("{id}  {status:<12}  {title}");
    }
    Ok(())
}

fn print_result(id: &str, body: &serde_json::Value) {
    println!("--- review notes (session {id}) ---");
    println!();
    if let Some(notes) = body["typed_notes"].as_str()
        && !notes.is_empty()
    {
        println!("## Typed Notes");
        println!("{notes}");
        println!();
    }
    if let Some(images) = body["annotation_images"].as_array()
        && !images.is_empty()
    {
        println!("## Attached Images");
        for img in images {
            if let Some(path) = img.as_str() {
                println!("{path}");
            }
        }
    }
}

fn extract_title(markdown: &str) -> Option<String> {
    for line in markdown.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix("# ") {
            return Some(heading.trim().to_string());
        }
    }
    None
}
