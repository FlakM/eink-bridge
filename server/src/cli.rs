use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use std::io::Read as _;
use std::process::Command as StdCommand;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use eink_bridge::api::SCHEMA_VERSION;
use eink_bridge::session::SessionOrigin;

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
        /// Output result as JSON
        #[arg(long)]
        json: bool,
        /// Interactive mode: stay connected via WebSocket, emit structured events
        #[arg(long)]
        interactive: bool,
        /// Webhook URL to POST annotation_result and submitted events to
        #[arg(long)]
        callback_url: Option<String>,
        /// Mark the session as starred (pinned — survives purge/expire, reloads across reboots)
        #[arg(long)]
        star: bool,
    },
    /// Pin a session so it survives purge, expire, and server restarts
    Star {
        /// Session ID
        id: String,
    },
    /// Unpin a previously starred session
    Unstar {
        /// Session ID
        id: String,
    },
    /// Push updated content to an existing session (interactive mode)
    Update {
        /// Session ID
        id: String,
        /// File with updated content
        file: String,
    },
    /// Watch an existing session for events via WebSocket (emits same events as push --interactive)
    Watch {
        /// Session ID
        id: String,
        /// Timeout in minutes
        #[arg(long, default_value = "30")]
        timeout: u64,
    },
    /// Get result of a session
    Result {
        /// Session ID
        id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Cancel an active session (keeps it in history as Cancelled)
    Cancel {
        /// Session ID
        id: String,
    },
    /// Permanently remove a session (any status) from disk
    Remove {
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

    let command_name = match &cli.command {
        Command::Push { .. } => "push",
        Command::Update { .. } => "update",
        Command::Watch { .. } => "watch",
        Command::Result { .. } => "result",
        Command::Cancel { .. } => "cancel",
        Command::Remove { .. } => "remove",
        Command::List { .. } => "list",
        Command::Star { .. } => "star",
        Command::Unstar { .. } => "unstar",
    };

    let start = std::time::Instant::now();
    let result = match cli.command {
        Command::Push {
            file,
            title,
            timeout,
            non_blocking,
            json,
            interactive,
            callback_url,
            star,
        } => {
            cmd_push(
                &client,
                &cli.server,
                &file,
                title,
                timeout,
                non_blocking,
                json,
                interactive,
                callback_url,
                star,
            )
            .await
        }
        Command::Update { id, file } => cmd_update(&cli.server, &id, &file).await,
        Command::Watch { id, timeout } => cmd_push_interactive(&cli.server, &id, timeout).await,
        Command::Result { id, json } => cmd_result(&client, &cli.server, &id, json).await,
        Command::Cancel { id } => cmd_cancel(&client, &cli.server, &id).await,
        Command::Remove { id } => cmd_remove(&client, &cli.server, &id).await,
        Command::List { status } => cmd_list(&client, &cli.server, status).await,
        Command::Star { id } => cmd_set_starred(&client, &cli.server, &id, true).await,
        Command::Unstar { id } => cmd_set_starred(&client, &cli.server, &id, false).await,
    };

    let duration = start.elapsed().as_secs_f64();
    let result_label = if result.is_ok() { "ok" } else { "error" };
    push_cli_metrics(command_name, duration, result_label).await;

    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn push_cli_metrics(command: &str, duration_secs: f64, result: &str) {
    let Ok(gw) = std::env::var("EINK_PUSHGATEWAY_URL") else {
        return;
    };
    let metrics = format!(
        "# HELP eink_cli_commands_total CLI command invocations\n\
         # TYPE eink_cli_commands_total counter\n\
         eink_cli_commands_total{{command=\"{command}\",result=\"{result}\"}} 1\n\
         # HELP eink_cli_duration_seconds CLI command wall-clock duration\n\
         # TYPE eink_cli_duration_seconds gauge\n\
         eink_cli_duration_seconds{{command=\"{command}\",result=\"{result}\"}} {duration_secs:.3}\n"
    );
    let url = format!("{gw}/metrics/job/eink-review/instance/cli");
    let Ok(client) = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
    else {
        return;
    };
    let _ = client
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(metrics)
        .send()
        .await;
}

#[allow(clippy::too_many_arguments)]
async fn cmd_push(
    client: &Client,
    server: &str,
    file: &str,
    title: Option<String>,
    timeout_minutes: u64,
    non_blocking: bool,
    json_output: bool,
    interactive: bool,
    callback_url: Option<String>,
    star: bool,
) -> anyhow::Result<()> {
    let content = if file == "-" {
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        std::fs::read_to_string(file)?
    };

    let title = title.or_else(|| extract_title(&content));
    let origin = capture_origin();

    let resp = client
        .post(format!("{server}/api/sessions"))
        .json(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "title": title,
            "content": content,
            "callback_url": callback_url,
            "starred": star,
            "origin": origin,
        }))
        .send()
        .await?;
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

    if interactive {
        return cmd_push_interactive(server, id, timeout_minutes).await;
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
                if json_output {
                    println!("{}", serde_json::to_string_pretty(&body)?);
                } else {
                    print_result(id, &body);
                }
                exit_for_verdict(&body);
                return Ok(());
            }
            204 => continue, // long-poll timeout, retry
            410 => anyhow::bail!("session was cancelled"),
            404 => anyhow::bail!("session not found"),
            s => anyhow::bail!("unexpected status: {s}"),
        }
    }
}

async fn cmd_push_interactive(server: &str, id: &str, timeout_minutes: u64) -> anyhow::Result<()> {
    use std::io::Write as _;
    println!("SESSION_ID:{id}");
    std::io::stdout().flush().ok();

    let deadline = Instant::now() + Duration::from_secs(timeout_minutes * 60);
    let mut retry_delay = Duration::from_millis(150);

    'reconnect: loop {
        if Instant::now() > deadline {
            anyhow::bail!("timeout waiting for review");
        }

        let url = ws_url(server, &format!("/ws/{id}"));
        let ws_result = tokio::time::timeout(
            Duration::from_secs(5).min(deadline.saturating_duration_since(Instant::now())),
            tokio_tungstenite::connect_async(&url),
        )
        .await;

        let mut ws = match ws_result {
            Err(_) => {
                eprintln!("EVENT:RECONNECTING reason=connect_timeout");
                observer_retry(&mut retry_delay, deadline).await;
                continue 'reconnect;
            }
            Ok(Err(e)) => {
                if let tokio_tungstenite::tungstenite::Error::Http(response) = &e
                    && matches!(response.status().as_u16(), 404 | 410)
                {
                    anyhow::bail!("review session unavailable: {}", response.status());
                }
                eprintln!("EVENT:RECONNECTING reason={e}");
                observer_retry(&mut retry_delay, deadline).await;
                continue 'reconnect;
            }
            Ok(Ok((ws, _))) => ws,
        };

        if ws
            .send(WsMessage::Text(
                serde_json::json!({"type": "subscribe"}).to_string(),
            ))
            .await
            .is_err()
        {
            observer_retry(&mut retry_delay, deadline).await;
            continue 'reconnect;
        }

        retry_delay = Duration::from_millis(150);
        eprintln!("EVENT:WATCH_READY session_id={id}");
        let mut last_ping = Instant::now();
        let mut last_received = Instant::now();
        loop {
            if Instant::now() > deadline {
                anyhow::bail!("timeout waiting for review");
            }

            if last_received.elapsed() >= Duration::from_secs(20) {
                eprintln!("EVENT:RECONNECTING reason=heartbeat_timeout");
                observer_retry(&mut retry_delay, deadline).await;
                continue 'reconnect;
            }
            if last_ping.elapsed() >= Duration::from_secs(10) {
                if ws.send(WsMessage::Ping(Default::default())).await.is_err() {
                    eprintln!("EVENT:RECONNECTING reason=ping_failed");
                    observer_retry(&mut retry_delay, deadline).await;
                    continue 'reconnect;
                }
                last_ping = Instant::now();
            }

            let msg = tokio::time::timeout(
                Duration::from_secs(5).min(deadline.saturating_duration_since(Instant::now())),
                ws.next(),
            )
            .await;
            if matches!(&msg, Ok(Some(Ok(_)))) {
                last_received = Instant::now();
            }

            match msg {
                Err(_) => continue, // idle window, go ping
                Ok(Some(Ok(WsMessage::Text(text)))) => {
                    let v: serde_json::Value = serde_json::from_str(&text)?;
                    match v["type"].as_str() {
                        Some("annotation_result") => {
                            let mut out = v.clone();
                            if let Some(anns) = out["annotations"].as_array_mut() {
                                for ann in anns.iter_mut() {
                                    if let Some(obj) = ann.as_object_mut() {
                                        obj.remove("strokes");
                                        obj.remove("pressures");
                                    }
                                }
                            }
                            println!("EVENT:ANNOTATION_RESULT {out}");
                            std::io::stdout().flush().ok();
                        }
                        Some("session_submitted") => {
                            println!("EVENT:SUBMITTED {text}");
                            std::io::stdout().flush().ok();
                            let body = &v["result"];
                            print_result(id, body);
                            exit_for_verdict(body);
                            return Ok(());
                        }
                        Some("error") => {
                            anyhow::bail!(
                                "server error: {}",
                                v["message"].as_str().unwrap_or("unknown")
                            );
                        }
                        _ => {}
                    }
                }
                Ok(Some(Ok(WsMessage::Ping(data)))) => {
                    if ws.send(WsMessage::Pong(data)).await.is_err() {
                        eprintln!("EVENT:RECONNECTING reason=pong_failed");
                        continue 'reconnect;
                    }
                }
                Ok(Some(Ok(WsMessage::Pong(_)))) => {}
                Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => {
                    eprintln!("EVENT:RECONNECTING reason=ws_closed");
                    observer_retry(&mut retry_delay, deadline).await;
                    continue 'reconnect;
                }
                Ok(Some(Err(e))) => {
                    eprintln!("EVENT:RECONNECTING reason={e}");
                    observer_retry(&mut retry_delay, deadline).await;
                    continue 'reconnect;
                }
                Ok(Some(Ok(_))) => {}
            }
        }
    }
}

async fn observer_retry(delay: &mut Duration, deadline: Instant) {
    tokio::time::sleep((*delay).min(deadline.saturating_duration_since(Instant::now()))).await;
    *delay = (*delay * 2).min(Duration::from_secs(2));
}

async fn cmd_update(server: &str, id: &str, file: &str) -> anyhow::Result<()> {
    let content = std::fs::read_to_string(file)?;
    let client = Client::new();
    let resp = client
        .put(format!("{server}/api/sessions/{id}/content"))
        .header("content-type", "text/plain")
        .body(content)
        .send()
        .await?;
    match resp.status().as_u16() {
        200 => {
            let body: serde_json::Value = resp.json().await?;
            let version = body["version"].as_u64().unwrap_or(0);
            eprintln!("version updated to {version}");
            Ok(())
        }
        404 => anyhow::bail!("session not found"),
        409 => anyhow::bail!("session not in processing state"),
        s => anyhow::bail!("unexpected status: {s}"),
    }
}

fn ws_url(server: &str, path: &str) -> String {
    let base = server
        .trim_end_matches('/')
        .replacen("http://", "ws://", 1)
        .replacen("https://", "wss://", 1);
    format!("{base}{path}")
}

async fn cmd_result(
    client: &Client,
    server: &str,
    id: &str,
    json_output: bool,
) -> anyhow::Result<()> {
    let resp = client
        .get(format!("{server}/api/sessions/{id}/result"))
        .send()
        .await?;

    match resp.status().as_u16() {
        200 => {
            let body: serde_json::Value = resp.json().await?;
            if json_output {
                println!("{}", serde_json::to_string_pretty(&body)?);
            } else {
                print_result(id, &body);
            }
            exit_for_verdict(&body);
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

async fn cmd_remove(client: &Client, server: &str, id: &str) -> anyhow::Result<()> {
    // `?remove=true` forgets the session and deletes its files; a bare DELETE cancels it.
    let resp = client
        .delete(format!("{server}/api/sessions/{id}?remove=true"))
        .send()
        .await?;

    match resp.status().as_u16() {
        200 => {
            eprintln!("session {id} removed");
            Ok(())
        }
        404 => anyhow::bail!("session {id} not found"),
        s => anyhow::bail!("failed to remove: {s}"),
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
        println!("{}", format_list_row(s));
    }
    Ok(())
}

fn exit_for_verdict(body: &serde_json::Value) {
    let code = verdict_exit_code(body);
    if code != 0 {
        std::process::exit(code);
    }
}

/// The exit code a submitted review carries, which is what a pipeline gates on.
/// 0 = lgtm (or no verdict at all), 2 = changes, 3 = reject, 4 = question.
fn verdict_exit_code(body: &serde_json::Value) -> i32 {
    match body["verdict"].as_str() {
        Some("changes") => 2,
        Some("reject") => 3,
        Some("question") => 4,
        _ => 0,
    }
}

/// How many characters of an anchored element to show before eliding.
const ELEMENT_PREVIEW_CHARS: usize = 60;

fn format_result(id: &str, body: &serde_json::Value) -> String {
    let mut out = String::new();
    out.push_str(&format!("--- review notes (session {id}) ---\n\n"));
    if let Some(v) = body["verdict"].as_str() {
        out.push_str(&format!("Verdict: {}\n\n", v.to_uppercase()));
    }
    if let Some(notes) = body["typed_notes"].as_str()
        && !notes.is_empty()
    {
        out.push_str(&format!("## Typed Notes\n{notes}\n\n"));
    }
    if let Some(annotations) = body["annotations"].as_array()
        && !annotations.is_empty()
    {
        out.push_str("## Annotations\n");
        for ann in annotations {
            match ann.get("anchor") {
                Some(anchor) => {
                    let anchor_type = anchor["type"].as_str().unwrap_or("unknown");
                    if let Some(elems) = anchor["elements"].as_array() {
                        let label: Vec<String> = elems
                            .iter()
                            .map(|e| {
                                let tag = e["tag"].as_str().unwrap_or("?");
                                let text = e["text"].as_str().unwrap_or("");
                                format!("{tag}: {}", elide(text, ELEMENT_PREVIEW_CHARS))
                            })
                            .collect();
                        out.push_str(&format!("### On {} ({})\n", label.join(", "), anchor_type));
                    }
                }
                None => out.push_str("### Unanchored\n"),
            }
            if let Some(text) = ann["recognized_text"].as_str()
                && !text.is_empty()
            {
                out.push_str(&format!("> {text}\n"));
            }
            out.push('\n');
        }
    }
    if let Some(images) = body["annotation_images"].as_array()
        && !images.is_empty()
    {
        out.push_str("## Attached Images\n");
        for img in images {
            if let Some(path) = img.as_str() {
                out.push_str(&format!("{path}\n"));
            }
        }
    }
    out
}

/// Shorten to at most `limit` characters, counting characters rather than bytes: slicing
/// a string at a byte offset panics when it lands inside a multi-byte character, and the
/// notes here are prose in whatever language the reviewer writes.
fn elide(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}…")
}

fn print_result(id: &str, body: &serde_json::Value) {
    print!("{}", format_result(id, body));
}

fn format_list_row(session: &serde_json::Value) -> String {
    let id = session["id"].as_str().unwrap_or("?");
    let title = session["title"].as_str().unwrap_or("(untitled)");
    let status = session["status"].as_str().unwrap_or("?");
    let star = if session["starred"].as_bool().unwrap_or(false) {
        "*"
    } else {
        " "
    };
    let cwd = session["origin"]["cwd"].as_str().unwrap_or("");
    if cwd.is_empty() {
        format!("{star} {id}  {status:<12}  {title}")
    } else {
        format!("{star} {id}  {status:<12}  {title}  [{cwd}]")
    }
}

/// Capture metadata about the invocation environment: cwd, hostname, git state.
/// Every field is optional — nothing is fatal if it can't be read.
fn capture_origin() -> SessionOrigin {
    let cwd = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    let host = read_hostname();
    let (git_branch, git_remote) = read_git_state();
    SessionOrigin {
        cwd,
        host,
        tool: Some("eink-review".into()),
        git_branch,
        git_remote,
    }
}

fn read_hostname() -> Option<String> {
    // Prefer the live kernel hostname; fall back to /etc/hostname (may be stale after hostnamectl).
    StdCommand::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|h| h.trim().to_string())
                .filter(|s| !s.is_empty())
        })
}

fn read_git_state() -> (Option<String>, Option<String>) {
    let branch = StdCommand::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && s != "HEAD");
    let remote = StdCommand::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    (branch, remote)
}

async fn cmd_set_starred(
    client: &Client,
    server: &str,
    id: &str,
    starred: bool,
) -> anyhow::Result<()> {
    let resp = client
        .put(format!("{server}/api/sessions/{id}/star"))
        .json(&serde_json::json!({ "starred": starred }))
        .send()
        .await?;
    match resp.status().as_u16() {
        200 => {
            eprintln!(
                "session {id} {}",
                if starred { "starred" } else { "unstarred" }
            );
            Ok(())
        }
        404 => anyhow::bail!("session {id} not found"),
        s => anyhow::bail!("failed to set starred: {s}"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_review_exit_code_is_the_contract_a_pipeline_gates_on() {
        // 0 passes, so a wrong code here silently lets "changes" through CI.
        assert_eq!(verdict_exit_code(&json!({"verdict": "lgtm"})), 0);
        assert_eq!(verdict_exit_code(&json!({"verdict": "changes"})), 2);
        assert_eq!(verdict_exit_code(&json!({"verdict": "reject"})), 3);
        assert_eq!(verdict_exit_code(&json!({"verdict": "question"})), 4);
    }

    #[test]
    fn a_review_without_a_verdict_passes() {
        // Only typed notes, no tag: nothing to gate on, so the pipeline must not fail.
        assert_eq!(
            verdict_exit_code(&json!({"typed_notes": "just some notes"})),
            0
        );
        assert_eq!(verdict_exit_code(&json!({"verdict": null})), 0);
        assert_eq!(verdict_exit_code(&json!({})), 0);
        assert_eq!(verdict_exit_code(&json!({"verdict": "LGTM"})), 0);
    }

    #[test]
    fn long_element_text_is_elided_without_panicking_on_a_character_boundary() {
        // Slicing at byte 60 panicked when the cut landed inside a multi-byte character,
        // and these notes are prose in whatever language the reviewer writes.
        let text = format!("{}ł{}", "a".repeat(59), "b".repeat(20));
        assert!(text.len() > 60, "the fixture must be long enough in bytes");
        let body = json!({
            "annotations": [{
                "anchor": {"type": "explicit", "elements": [{"tag": "P", "text": text}]},
            }],
        });
        let rendered = format_result("abc", &body);
        assert!(rendered.contains("P: "), "{rendered}");
        assert!(
            rendered.contains('…'),
            "the reader should see that it was cut"
        );
        assert!(
            !rendered.contains(&"b".repeat(20)),
            "the tail should be elided: {rendered}"
        );
    }

    #[test]
    fn eliding_counts_characters_not_bytes() {
        assert_eq!(elide("short", 60), "short");
        assert_eq!(elide(&"x".repeat(60), 60), "x".repeat(60));
        let long = "x".repeat(61);
        assert_eq!(elide(&long, 60).chars().count(), 61); // 60 kept + the ellipsis
        assert_eq!(elide("ł".repeat(70).as_str(), 60).chars().count(), 61);
    }

    #[test]
    fn a_submitted_review_renders_notes_anchors_and_images() {
        let body = json!({
            "verdict": "changes",
            "typed_notes": "CHANGES: the retry budget is wrong",
            "annotations": [
                {"anchor": {"type": "proximity", "elements": [{"tag": "P", "text": "the retry loop"}]},
                 "recognized_text": "WRITE A TLDR"},
                {"recognized_text": "freehand note"},
            ],
            "annotation_images": ["/state/annotations/img_1.png"],
        });
        let out = format_result("abc123", &body);
        assert!(out.starts_with("--- review notes (session abc123) ---"));
        assert!(out.contains("Verdict: CHANGES"));
        assert!(out.contains("## Typed Notes"));
        assert!(out.contains("CHANGES: the retry budget is wrong"));
        assert!(out.contains("## Annotations"));
        assert!(out.contains("### On P: the retry loop (proximity)"));
        assert!(out.contains("> WRITE A TLDR"));
        assert!(
            out.contains("### Unanchored"),
            "an unbound note still shows"
        );
        assert!(out.contains("/state/annotations/img_1.png"));
    }

    #[test]
    fn an_empty_review_renders_only_its_heading() {
        // Nothing to report: no verdict, no notes, no ink. It must not print empty
        // section headings, which read like a review that lost its content.
        let out = format_result("abc", &json!({}));
        assert_eq!(out, "--- review notes (session abc) ---\n\n");
        assert!(!out.contains("## Typed Notes"));
        assert!(!out.contains("## Annotations"));
        assert!(!out.contains("## Attached Images"));
    }

    #[test]
    fn an_empty_note_is_not_printed_as_a_section() {
        let body = json!({"typed_notes": "", "annotation_images": [], "annotations": []});
        let out = format_result("abc", &body);
        assert!(!out.contains("## Typed Notes"));
        assert!(!out.contains("## Attached Images"));
    }

    #[test]
    fn a_session_row_shows_the_star_the_status_and_where_it_came_from() {
        let row = format_list_row(&json!({
            "id": "abc123", "title": "Review the retry loop", "status": "Submitted",
            "starred": true, "origin": {"cwd": "/home/flakm/programming/x"},
        }));
        assert_eq!(
            row,
            "* abc123  Submitted     Review the retry loop  [/home/flakm/programming/x]"
        );

        let unstared = format_list_row(&json!({"id": "abc123", "status": "Active"}));
        assert!(unstared.starts_with("  abc123  Active"));
        assert!(
            unstared.contains("(untitled)"),
            "a missing title is named as one"
        );
        assert!(!unstared.contains('['), "no cwd recorded, no bracket");
    }

    #[test]
    fn a_row_survives_a_session_missing_its_optional_fields() {
        let row = format_list_row(&json!({"id": "abc123"}));
        assert!(row.contains("abc123"));
        assert!(row.contains('?'), "an unknown status is visible, not blank");
    }

    #[test]
    fn the_observer_url_follows_the_scheme_of_the_server() {
        assert_eq!(
            ws_url("http://localhost:3333", "/ws/abc"),
            "ws://localhost:3333/ws/abc"
        );
        assert_eq!(
            ws_url("https://review.example.com", "/ws/abc"),
            "wss://review.example.com/ws/abc"
        );
        // A trailing slash must not produce a double slash, which some proxies reject.
        assert_eq!(
            ws_url("http://localhost:3333/", "/ws/abc"),
            "ws://localhost:3333/ws/abc"
        );
    }

    #[test]
    fn finds_h1_on_first_line() {
        assert_eq!(
            extract_title("# My Doc\n\nsome text"),
            Some("My Doc".into())
        );
    }

    #[test]
    fn finds_h1_not_on_first_line() {
        assert_eq!(
            extract_title("intro\n\n# Buried Title\n\ntext"),
            Some("Buried Title".into())
        );
    }

    #[test]
    fn trims_trailing_whitespace_from_title() {
        assert_eq!(extract_title("#  Padded  \n\n"), Some("Padded".into()));
    }

    #[test]
    fn returns_none_when_no_h1() {
        assert_eq!(extract_title("## Only H2\n\nsome text"), None);
        assert_eq!(extract_title("plain text only"), None);
        assert_eq!(extract_title(""), None);
    }

    #[test]
    fn ignores_h2_and_deeper() {
        assert_eq!(extract_title("## Section\n### Sub"), None);
    }

    #[test]
    fn requires_space_after_hash() {
        // "#NoSpace" is not a valid h1
        assert_eq!(extract_title("#NoSpace\n\ntext"), None);
    }

    #[test]
    fn returns_first_h1_when_multiple_exist() {
        assert_eq!(extract_title("# First\n\n# Second"), Some("First".into()));
    }

    #[test]
    fn works_with_indented_heading() {
        // trimmed before checking, so indented h1 is recognized
        assert_eq!(
            extract_title("  # Indented\n\ntext"),
            Some("Indented".into())
        );
    }
}
