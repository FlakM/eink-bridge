use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use eink_bridge::api::SCHEMA_VERSION;
use eink_bridge::images::{
    alt_from_name, bytes_are_image, document_image_type, format_label, image_name,
};
use eink_bridge::session::SessionOrigin;

#[derive(Parser)]
#[command(name = "eink-review", about = "E-ink review session CLI")]
struct Cli {
    /// Server base URL (default: $EINK_SERVER_URL, then localhost:3333)
    #[arg(long, env = "EINK_SERVER_URL", default_value = "http://localhost:3333")]
    server: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Push content for review
    Push {
        /// File to push (use - for stdin). Omit when the push is only --image
        file: Option<String>,
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
        /// Attach a tag to the session (repeatable), k=v
        #[arg(long = "tag", value_name = "K=V")]
        tags: Vec<String>,
        /// Attach an image (repeatable). A markdown reference to the file is retargeted
        /// where it stands; an unreferenced image is added at the end of the document
        #[arg(long = "image", value_name = "PATH")]
        images: Vec<String>,
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
        /// Refuse the update unless the document is at this version
        #[arg(long)]
        expected_version: Option<u32>,
        /// Acknowledge this feedback id in the same request
        #[arg(long)]
        feedback_id: Option<String>,
        /// Zero-based annotation indices incorporated, comma-separated
        #[arg(long, value_delimiter = ',')]
        consumed: Vec<u32>,
        /// Attach an image (repeatable). A markdown reference to the file is retargeted
        /// where it stands; an unreferenced image is added at the end of the document
        #[arg(long = "image", value_name = "PATH")]
        images: Vec<String>,
    },
    /// Add an image to a session and print the markdown that shows it
    Image {
        /// Session ID
        id: String,
        /// Image file (png, jpg, jpeg, webp or gif)
        file: String,
        /// Alt text; defaults to the file name without its extension
        #[arg(long)]
        alt: Option<String>,
    },
    /// Print a session's current markdown
    Content {
        /// Session ID
        id: String,
        /// Print the whole response as JSON (content, version, status)
        #[arg(long)]
        json: bool,
    },
    /// Print the lines around a path:line reference in the session's checkout
    Source {
        /// Session ID
        id: String,
        /// A reference like src/lib.rs:120
        reference: String,
        /// Lines of context on each side
        #[arg(long, default_value = "6")]
        context: usize,
    },
    /// Report what the agent is doing while it rewrites (shown on the tablet)
    Progress {
        /// Session ID
        id: String,
        /// One short line, in the reader's words
        message: String,
        /// Which step this is (1-based); requires --of
        #[arg(long)]
        step: Option<u32>,
        /// How many steps there are
        #[arg(long)]
        of: Option<u32>,
        /// The feedback this work answers; defaults to the session's pending feedback
        #[arg(long)]
        feedback_id: Option<String>,
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
    /// Permanently remove a session from disk
    Remove {
        /// Session ID
        id: String,
        /// Remove even a review that is still being read
        #[arg(long)]
        force: bool,
    },
    /// List sessions
    List {
        /// Filter by status
        #[arg(long)]
        status: Option<String>,
        /// Print the raw JSON array
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let client = Client::new();

    let command_name = match &cli.command {
        Command::Push { .. } => "push",
        Command::Content { .. } => "content",
        Command::Progress { .. } => "progress",
        Command::Source { .. } => "source",
        Command::Update { .. } => "update",
        Command::Watch { .. } => "watch",
        Command::Result { .. } => "result",
        Command::Cancel { .. } => "cancel",
        Command::Remove { .. } => "remove",
        Command::List { .. } => "list",
        Command::Star { .. } => "star",
        Command::Unstar { .. } => "unstar",
        Command::Image { .. } => "image",
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
            tags,
            images,
        } => {
            cmd_push(
                &client,
                &cli.server,
                file.as_deref(),
                title,
                timeout,
                non_blocking,
                json,
                interactive,
                callback_url,
                star,
                tags,
                images,
            )
            .await
        }
        Command::Update {
            id,
            file,
            expected_version,
            feedback_id,
            consumed,
            images,
        } => {
            cmd_update(
                &cli.server,
                &id,
                &file,
                expected_version,
                feedback_id,
                consumed,
                images,
            )
            .await
        }
        Command::Image { id, file, alt } => cmd_image(&client, &cli.server, &id, &file, alt).await,
        Command::Content { id, json } => cmd_content(&client, &cli.server, &id, json).await,
        Command::Progress {
            id,
            message,
            step,
            of,
            feedback_id,
        } => cmd_progress(&client, &cli.server, &id, &message, step, of, feedback_id).await,
        Command::Source {
            id,
            reference,
            context,
        } => cmd_source(&client, &cli.server, &id, &reference, context).await,
        Command::Watch { id, timeout } => cmd_push_interactive(&cli.server, &id, timeout).await,
        Command::Result { id, json } => cmd_result(&client, &cli.server, &id, json).await,
        Command::Cancel { id } => cmd_cancel(&client, &cli.server, &id).await,
        Command::Remove { id, force } => cmd_remove(&client, &cli.server, &id, force).await,
        Command::List { status, json } => cmd_list(&client, &cli.server, status, json).await,
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

/// The Prometheus text the CLI pushes for one command. Label values are escaped: the
/// pushgateway rejects a whole batch on one malformed line, and the push is
/// fire-and-forget, so a broken body would be lost without a word.
fn cli_metrics_body(command: &str, duration_secs: f64, result: &str) -> String {
    let command = escape_label(command);
    let result = escape_label(result);
    format!(
        "# HELP eink_cli_commands_total CLI command invocations\n\
         # TYPE eink_cli_commands_total counter\n\
         eink_cli_commands_total{{command=\"{command}\",result=\"{result}\"}} 1\n\
         # HELP eink_cli_duration_seconds CLI command wall-clock duration\n\
         # TYPE eink_cli_duration_seconds gauge\n\
         eink_cli_duration_seconds{{command=\"{command}\",result=\"{result}\"}} {duration_secs:.3}\n"
    )
}

fn escape_label(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}

async fn push_cli_metrics(command: &str, duration_secs: f64, result: &str) {
    let Ok(gw) = std::env::var("EINK_PUSHGATEWAY_URL") else {
        return;
    };
    let metrics = cli_metrics_body(command, duration_secs, result);
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
    file: Option<&str>,
    title: Option<String>,
    timeout_minutes: u64,
    non_blocking: bool,
    json_output: bool,
    interactive: bool,
    callback_url: Option<String>,
    star: bool,
    tags: Vec<String>,
    images: Vec<String>,
) -> anyhow::Result<()> {
    let mut tag_map = std::collections::HashMap::new();
    for tag in tags {
        let Some((key, value)) = tag.split_once('=') else {
            anyhow::bail!("--tag expects k=v, got {tag:?}");
        };
        if key.is_empty() {
            anyhow::bail!("--tag expects a key before '='");
        }
        tag_map.insert(key.to_string(), value.to_string());
    }
    let mut content = match file {
        None => String::new(),
        Some("-") => {
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            buf
        }
        Some(path) => std::fs::read_to_string(path)?,
    };
    if file.is_none() && images.is_empty() {
        anyhow::bail!("nothing to push: give a file to read or --image to attach");
    }
    let base_dir = match file {
        Some(path) if path != "-" => Path::new(path).parent().map(Path::to_path_buf),
        _ => None,
    }
    .unwrap_or_else(|| PathBuf::from("."));
    let images = collect_images(&mut content, &base_dir, &images)?;
    check_upload_size(&content, &images)?;

    // A push that carries only pictures needs a name in the session list; the first
    // picture's file name is the honest one.
    let title = match title.or_else(|| extract_title(&content)) {
        Some(title) => Some(title),
        None if file.is_none() => images
            .first()
            .map(|image| alt_from_name(&image.name).to_string()),
        None => None,
    };
    let origin = capture_origin();

    let request = client.post(format!("{server}/api/sessions"));
    let request = if images.is_empty() {
        request.json(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "title": title,
            "content": content,
            "callback_url": callback_url,
            "starred": star,
            "tags": tag_map,
            "origin": origin,
        }))
    } else {
        // The images travel with the create request so the first render already shows
        // them: uploading afterwards would bump the version and redraw the page.
        let mut form = reqwest::multipart::Form::new()
            .text("content", content)
            .text("starred", if star { "true" } else { "false" })
            .text("tags", serde_json::to_string(&tag_map)?)
            .text("origin", serde_json::to_string(&origin)?);
        if let Some(title) = &title {
            form = form.text("title", title.clone());
        }
        if let Some(callback_url) = &callback_url {
            form = form.text("callback_url", callback_url.clone());
        }
        for image in &images {
            form = form.part(
                "image",
                reqwest::multipart::Part::bytes(image.read()?).file_name(image.name.clone()),
            );
        }
        request.multipart(form)
    };

    let resp = request.send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        anyhow::bail!("failed to create session: {status}: {detail}");
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

/// How much a create or an image upload may weigh. The server enforces the same bound
/// (see `MAX_BODY_SIZE` in `app.rs`); checking here names the file that is too big instead
/// of failing the request with a bare 413.
const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

/// An image the document displays, ready to travel with the request that carries it.
#[derive(Debug)]
struct DocumentImage {
    name: String,
    path: PathBuf,
}

impl DocumentImage {
    fn read(&self) -> anyhow::Result<Vec<u8>> {
        Ok(std::fs::read(&self.path)?)
    }
}

/// Turn a document's local image references into `image/<name>` and collect the files that
/// must be uploaded with it.
///
/// A reference is local when it is a relative path that exists on disk next to the
/// markdown. Anything else — `https://…`, an absolute path, a `data:` URI, or an
/// `image/…/` reference whose file is not here — is left exactly as written: it is either
/// already a session image or a picture the reader's browser fetches itself.
fn collect_images(
    content: &mut String,
    base_dir: &Path,
    extra: &[String],
) -> anyhow::Result<Vec<DocumentImage>> {
    let references: Vec<String> = pulldown_cmark::Parser::new(content.as_str())
        .filter_map(|event| match event {
            pulldown_cmark::Event::Start(pulldown_cmark::Tag::Image { dest_url, .. }) => {
                Some(dest_url.to_string())
            }
            _ => None,
        })
        .collect();

    let mut images: Vec<DocumentImage> = Vec::new();
    for reference in references {
        if is_external_image(&reference) {
            continue;
        }
        let path = base_dir.join(&reference);
        if !path.is_file() {
            continue;
        }
        let name = image_name(&reference).to_string();
        check_image_file(&name, &path)?;
        retarget_image(content, &reference, &name)?;
        add_image(&mut images, DocumentImage { name, path })?;
    }

    for given in extra {
        let path = PathBuf::from(given);
        if !path.is_file() {
            anyhow::bail!("no such image: {given}");
        }
        let name = image_name(given).to_string();
        check_image_file(&name, &path)?;
        let reference = format!("image/{name}");
        if !content.contains(&reference) {
            // A push that is only a picture starts with it, not with two blank lines.
            if !content.trim().is_empty() {
                if !content.ends_with('\n') {
                    content.push('\n');
                }
                content.push('\n');
            }
            content.push_str(&format!("![{}]({reference})\n", alt_from_name(&name)));
        }
        add_image(&mut images, DocumentImage { name, path })?;
    }
    Ok(images)
}

/// A reference that does not point at a file next to the document.
fn is_external_image(reference: &str) -> bool {
    let reference = reference.trim();
    if reference.is_empty() || reference.starts_with('/') || reference.starts_with("data:") {
        return true;
    }
    // Any URL scheme, including `image:` and `https:`.
    reference
        .split_once(':')
        .is_some_and(|(scheme, _)| scheme.len() > 1 && !scheme.contains('/'))
}

fn check_image_file(name: &str, path: &Path) -> anyhow::Result<()> {
    let Some(content_type) = document_image_type(name) else {
        anyhow::bail!(
            "cannot attach {}: an image name must be a plain file name ending in .png, .jpg, .jpeg, .webp or .gif",
            path.display()
        );
    };
    let bytes = std::fs::read(path)?;
    if !bytes_are_image(content_type, &bytes) {
        anyhow::bail!(
            "{} is named like a {} image but its bytes are something else",
            path.display(),
            format_label(content_type)
        );
    }
    Ok(())
}

/// Keep the picture where the author put it. The markdown is text, so this rewrites the
/// destination of the reference that names the file, not the whole document.
fn retarget_image(content: &mut String, reference: &str, name: &str) -> anyhow::Result<()> {
    let target = format!("image/{name}");
    if reference == target {
        return Ok(());
    }
    let before = content.clone();
    *content = content
        .replace(&format!("]({reference})"), &format!("]({target})"))
        .replace(&format!("](<{reference}>)"), &format!("]({target})"))
        .replace(&format!("]({reference} "), &format!("]({target} "));
    if *content == before {
        // The file is there and a reference names it, but not in a shape this rewrite
        // recognises. Silently leaving the local path would ship a broken image.
        anyhow::bail!(
            "found {reference} on disk but could not rewrite the markdown reference to it; write it as ![alt]({reference})"
        );
    }
    Ok(())
}

fn add_image(images: &mut Vec<DocumentImage>, image: DocumentImage) -> anyhow::Result<()> {
    if let Some(existing) = images.iter().find(|existing| existing.name == image.name) {
        if existing.path != image.path {
            anyhow::bail!(
                "two different images are both named {}; rename one of them ({} and {})",
                image.name,
                existing.path.display(),
                image.path.display()
            );
        }
        return Ok(());
    }
    images.push(image);
    Ok(())
}

fn check_upload_size(content: &str, images: &[DocumentImage]) -> anyhow::Result<()> {
    let mut total = content.len() + 2048; // multipart framing
    for image in images {
        total += std::fs::metadata(&image.path).map_or(0, |meta| meta.len()) as usize;
    }
    if total > MAX_UPLOAD_BYTES {
        anyhow::bail!(
            "the document and its {} image(s) total {} MiB; the server accepts {} MiB per request — shrink the images or push them in a second round",
            images.len(),
            total / (1024 * 1024),
            MAX_UPLOAD_BYTES / (1024 * 1024)
        );
    }
    Ok(())
}

async fn upload_image(
    client: &Client,
    server: &str,
    id: &str,
    image: &DocumentImage,
) -> anyhow::Result<()> {
    let content_type = document_image_type(&image.name).unwrap_or("application/octet-stream");
    let resp = client
        .post(format!("{server}/api/sessions/{id}/image/{}", image.name))
        .header("content-type", content_type)
        .body(image.read()?)
        .send()
        .await?;
    match resp.status().as_u16() {
        201 => Ok(()),
        400 => anyhow::bail!(
            "image {} refused: {}",
            image.name,
            resp.text().await.unwrap_or_default()
        ),
        404 => anyhow::bail!("session {id} not found"),
        409 => anyhow::bail!("session {id} is no longer active; its document cannot change"),
        413 => anyhow::bail!("image {} is too large for the server", image.name),
        status => anyhow::bail!("unexpected status {status} while uploading {}", image.name),
    }
}

async fn upload_images(
    client: &Client,
    server: &str,
    id: &str,
    images: &[DocumentImage],
) -> anyhow::Result<()> {
    for image in images {
        upload_image(client, server, id, image).await?;
    }
    Ok(())
}

async fn cmd_image(
    client: &Client,
    server: &str,
    id: &str,
    file: &str,
    alt: Option<String>,
) -> anyhow::Result<()> {
    let image = DocumentImage {
        name: image_name(file).to_string(),
        path: PathBuf::from(file),
    };
    if !image.path.is_file() {
        anyhow::bail!("no such image: {file}");
    }
    check_image_file(&image.name, &image.path)?;
    upload_image(client, server, id, &image).await?;
    let alt = alt.unwrap_or_else(|| alt_from_name(&image.name).to_string());
    println!("![{alt}](image/{})", image.name);
    Ok(())
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

async fn cmd_update(
    server: &str,
    id: &str,
    file: &str,
    expected_version: Option<u32>,
    feedback_id: Option<String>,
    consumed: Vec<u32>,
    images: Vec<String>,
) -> anyhow::Result<()> {
    let mut content = std::fs::read_to_string(file)?;
    let base_dir = Path::new(file)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let images = collect_images(&mut content, &base_dir, &images)?;
    check_upload_size(&content, &images)?;
    let client = Client::new();
    // New pictures have to exist before the document that points at them is served.
    upload_images(&client, server, id, &images).await?;
    // A plain-text update stays the default: it is what the watcher and older agents do,
    // and it cannot acknowledge anything by accident.
    let guarded = expected_version.is_some() || feedback_id.is_some() || !consumed.is_empty();
    let mut request = client.put(format!("{server}/api/sessions/{id}/content"));
    request = if guarded {
        request.header("content-type", "application/json").body(
            serde_json::json!({
                "content": content,
                "expected_version": expected_version,
                "feedback_id": feedback_id,
                "consumed_annotations": consumed,
            })
            .to_string(),
        )
    } else {
        request.header("content-type", "text/plain").body(content)
    };
    let resp = request.send().await?;
    match resp.status().as_u16() {
        200 => {
            let body: serde_json::Value = resp.json().await?;
            let version = body["version"].as_u64().unwrap_or(0);
            eprintln!("version updated to {version}");
            Ok(())
        }
        400 => {
            let reason = resp.text().await.unwrap_or_default();
            anyhow::bail!("update refused: {reason}")
        }
        404 => anyhow::bail!("session not found"),
        409 => anyhow::bail!(
            "stale version, or the feedback id is not the one pending; fetch /feedback and retry"
        ),
        s => anyhow::bail!("unexpected status: {s}"),
    }
}

async fn cmd_content(client: &Client, server: &str, id: &str, json: bool) -> anyhow::Result<()> {
    let resp = client
        .get(format!("{server}/api/sessions/{id}/content"))
        .send()
        .await?;
    match resp.status().as_u16() {
        200 => {
            let body: serde_json::Value = resp.json().await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&body)?);
            } else {
                eprintln!(
                    "session {id} version {} ({})",
                    body["version"].as_u64().unwrap_or(0),
                    body["status"].as_str().unwrap_or("unknown")
                );
                println!("{}", body["content"].as_str().unwrap_or_default());
            }
            Ok(())
        }
        404 => anyhow::bail!("session {id} not found"),
        s => anyhow::bail!("unexpected status: {s}"),
    }
}

async fn cmd_progress(
    client: &Client,
    server: &str,
    id: &str,
    message: &str,
    step: Option<u32>,
    of: Option<u32>,
    feedback_id: Option<String>,
) -> anyhow::Result<()> {
    // The server refuses progress that does not name the pending feedback, so the common
    // case (the note that just arrived) should not require the agent to repeat its id.
    let feedback_id = match feedback_id {
        Some(feedback_id) => feedback_id,
        None => {
            let snapshot = client
                .get(format!("{server}/api/sessions/{id}/feedback"))
                .send()
                .await?;
            if !snapshot.status().is_success() {
                anyhow::bail!("cannot read the pending feedback: {}", snapshot.status());
            }
            let body: serde_json::Value = snapshot.json().await?;
            body["pending"]["feedback_id"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| anyhow::anyhow!("this session has no pending feedback"))?
        }
    };
    let resp = client
        .post(format!("{server}/api/sessions/{id}/progress"))
        .json(&serde_json::json!({
            "feedback_id": feedback_id,
            "message": message,
            "step": step,
            "of": of,
        }))
        .send()
        .await?;
    match resp.status().as_u16() {
        204 => Ok(()),
        400 => {
            let reason = resp.text().await.unwrap_or_default();
            anyhow::bail!("progress refused: {reason}")
        }
        404 => anyhow::bail!("session {id} not found"),
        409 => anyhow::bail!(
            "progress refused: that feedback is not pending (it may already be answered)"
        ),
        s => anyhow::bail!("unexpected status: {s}"),
    }
}

async fn cmd_source(
    client: &Client,
    server: &str,
    id: &str,
    reference: &str,
    context: usize,
) -> anyhow::Result<()> {
    let url = reqwest::Url::parse_with_params(
        &format!("{server}/api/sessions/{id}/source"),
        &[
            ("ref", reference),
            ("context", &context.min(40).to_string()),
        ],
    )?;
    let resp = client.get(url).send().await?;
    match resp.status().as_u16() {
        200 => {
            let body: serde_json::Value = resp.json().await?;
            let start = body["start"].as_u64().unwrap_or(1);
            let end = body["end"].as_u64().unwrap_or(start);
            let total = body["total"].as_u64().unwrap_or(end);
            println!(
                "{}:{start}-{end} of {total}",
                body["path"].as_str().unwrap_or(reference)
            );
            for (offset, line) in body["lines"].as_array().into_iter().flatten().enumerate() {
                println!(
                    "{:>5} {}",
                    start + offset as u64,
                    line.as_str().unwrap_or_default()
                );
            }
            Ok(())
        }
        400 => {
            let reason = resp.text().await.unwrap_or_default();
            anyhow::bail!("bad reference: {reason}")
        }
        404 => {
            let reason = resp.text().await.unwrap_or_default();
            anyhow::bail!("cannot read {reference}: {reason}")
        }
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
            // 3 is already "reject" in `verdict_exit_code`; a pipeline must not confuse a
            // review that has not happened with one that failed. 5 is free.
            std::process::exit(PENDING_EXIT_CODE);
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

async fn cmd_remove(client: &Client, server: &str, id: &str, force: bool) -> anyhow::Result<()> {
    // `?remove=true` forgets the session and deletes its files; a bare DELETE cancels it.
    // A review still being read is refused without `force`, because removing it throws
    // away work someone is in the middle of.
    let mut url = format!("{server}/api/sessions/{id}?remove=true");
    if force {
        url.push_str("&force=true");
    }
    let resp = client.delete(url).send().await?;

    match resp.status().as_u16() {
        200 => {
            eprintln!("session {id} removed");
            Ok(())
        }
        404 => anyhow::bail!("session {id} not found"),
        409 => anyhow::bail!("session {id} is still being read; cancel it first, or pass --force"),
        s => anyhow::bail!("failed to remove: {s}"),
    }
}

async fn cmd_list(
    client: &Client,
    server: &str,
    status: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    let mut url = format!("{server}/api/sessions");
    if let Some(s) = &status {
        url = format!("{url}?status={s}");
    }

    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        let code = resp.status();
        let detail = resp.text().await.unwrap_or_default();
        anyhow::bail!("listing failed: {code}: {detail}");
    }
    let sessions: Vec<serde_json::Value> = resp.json().await?;

    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }

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

/// The exit code for "no verdict yet". Deliberately not 3: that is `reject` below, and
/// "nobody has reviewed this" must not read as "reviewed and refused".
pub const PENDING_EXIT_CODE: i32 = 5;

/// The exit code a submitted review carries, which is what a pipeline gates on.
/// 0 = lgtm (or no verdict at all), 2 = changes, 3 = reject, 4 = question, 5 = pending.
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

    fn png() -> Vec<u8> {
        b"\x89PNG\r\n\x1a\n0000".to_vec()
    }

    #[test]
    fn a_picture_next_to_the_brief_is_retargeted_at_the_session_route() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("shot.png"), png()).unwrap();
        let mut content =
            "![the error page](shot.png)\n\n![web](https://example.com/x.png)\n".to_string();

        let images = collect_images(&mut content, dir.path(), &[]).unwrap();

        assert_eq!(images.len(), 1);
        assert_eq!(images[0].name, "shot.png");
        assert!(
            content.contains("![the error page](image/shot.png)"),
            "{content}"
        );
        // A remote picture is not the server's to store, and an absolute path is not a
        // file beside the brief.
        assert!(content.contains("https://example.com/x.png"), "{content}");
    }

    #[test]
    fn a_document_that_is_only_a_picture_starts_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("shot.png");
        std::fs::write(&shot, png()).unwrap();
        let mut content = String::new();

        collect_images(
            &mut content,
            dir.path(),
            &[shot.to_string_lossy().to_string()],
        )
        .unwrap();

        assert_eq!(content, "![shot](image/shot.png)\n");
    }

    #[test]
    fn an_image_the_brief_does_not_mention_is_added_at_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let sketch = dir.path().join("whiteboard.png");
        std::fs::write(&sketch, png()).unwrap();
        let mut content = "# Plan\n\nOnly words.\n".to_string();

        collect_images(
            &mut content,
            dir.path(),
            &[sketch.to_string_lossy().to_string()],
        )
        .unwrap();

        assert!(
            content.ends_with("![whiteboard](image/whiteboard.png)\n"),
            "{content}"
        );
    }

    #[test]
    fn a_picture_already_referenced_by_name_is_not_added_twice() {
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("shot.png");
        std::fs::write(&shot, png()).unwrap();
        let mut content = "![x](image/shot.png)\n".to_string();

        let images = collect_images(
            &mut content,
            dir.path(),
            &[shot.to_string_lossy().to_string()],
        )
        .unwrap();

        assert_eq!(images.len(), 1);
        assert_eq!(content.matches("image/shot.png").count(), 1, "{content}");
    }

    #[test]
    fn a_file_that_is_not_an_image_is_refused_before_the_upload() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("shot.png"), b"just text").unwrap();
        let mut content = "![x](shot.png)\n".to_string();
        let message = collect_images(&mut content, dir.path(), &[])
            .unwrap_err()
            .to_string();
        assert!(message.contains("bytes are something else"), "{message}");
    }

    #[test]
    fn two_images_cannot_share_a_name() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("one");
        let second = dir.path().join("two");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("shot.png"), png()).unwrap();
        std::fs::write(second.join("shot.png"), png()).unwrap();
        let mut content = String::new();
        let message = collect_images(
            &mut content,
            dir.path(),
            &[
                first.join("shot.png").to_string_lossy().to_string(),
                second.join("shot.png").to_string_lossy().to_string(),
            ],
        )
        .unwrap_err()
        .to_string();
        assert!(message.contains("both named shot.png"), "{message}");
    }

    #[test]
    fn only_a_path_to_a_file_beside_the_brief_counts_as_local() {
        assert!(is_external_image("https://example.com/a.png"));
        assert!(is_external_image("image:shot.png"));
        assert!(is_external_image("/tmp/shot.png"));
        assert!(is_external_image("data:image/png;base64,AAAA"));
        assert!(is_external_image(""));
        assert!(!is_external_image("shot.png"));
        assert!(!is_external_image("pictures/shot.png"));
    }

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
    fn the_pushed_metrics_are_a_well_formed_exposition() {
        let body = cli_metrics_body("push", 12.3456, "ok");
        assert!(body.contains("# TYPE eink_cli_commands_total counter"));
        assert!(body.contains("# TYPE eink_cli_duration_seconds gauge"));
        assert!(body.contains("eink_cli_commands_total{command=\"push\",result=\"ok\"} 1"));
        assert!(
            body.contains("eink_cli_duration_seconds{command=\"push\",result=\"ok\"} 12.346"),
            "the duration is a metric, not a guess: {body}"
        );
        // Every non-comment line is `name{labels} value`, which is what a scraper needs.
        for line in body.lines().filter(|line| !line.starts_with('#')) {
            let (sample, value) = line.rsplit_once(' ').expect("a sample has a value");
            assert!(sample.contains('{') && sample.ends_with('}'), "{line}");
            assert!(value.parse::<f64>().is_ok(), "{line}");
        }
    }

    #[test]
    fn a_label_value_cannot_break_the_pushed_body() {
        // A quote or a newline in a label would end the line early; the pushgateway would
        // reject the batch and the CLI would never know.
        assert_eq!(escape_label("plain"), "plain");
        assert_eq!(escape_label("a\"b"), "a\\\"b");
        assert_eq!(escape_label("a\\b"), "a\\\\b");
        assert_eq!(escape_label("a\nb"), "a\\nb");

        let body = cli_metrics_body("push\"\nbad", 1.0, "err\\or");
        assert_eq!(
            body.lines().filter(|line| !line.starts_with('#')).count(),
            2,
            "a newline in a label must not add a line: {body}"
        );
        assert!(body.contains("result=\"err\\\\or\""));
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
