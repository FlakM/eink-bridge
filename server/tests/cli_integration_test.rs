use eink_bridge::app::{AppState, build_app};
use std::time::Duration;
use tokio::net::TcpListener;

#[tokio::test]
async fn observer_reconnects_without_a_one_second_pause() {
    use futures_util::{SinkExt, StreamExt};
    use tokio::io::{AsyncBufReadExt, BufReader};
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let mock = tokio::spawn(async move {
        for feedback_id in ["first", "second"] {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            ws.next().await.unwrap().unwrap(); // subscribe
            ws.send(tokio_tungstenite::tungstenite::Message::Text(serde_json::json!({
                "type":"annotation_result","feedback_id":feedback_id,"version":1,"annotations":[]
            }).to_string())).await.unwrap();
            if feedback_id == "first" {
                ws.close(None).await.unwrap();
            } else {
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    });
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server, "watch", "test", "--timeout", "1"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    loop {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if line.starts_with("EVENT:ANNOTATION_RESULT") {
            assert!(line.contains("first"));
            break;
        }
    }
    let next = tokio::time::timeout(Duration::from_millis(800), async {
        loop {
            let line = lines.next_line().await.unwrap().unwrap();
            if line.starts_with("EVENT:ANNOTATION_RESULT") {
                break line;
            }
        }
    })
    .await
    .expect("observer should resume in under 800ms after a dropped connection");
    assert!(next.contains("second"));
    child.kill().await.unwrap();
    mock.abort();
}

async fn start_server() -> (String, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.keep(), 2);
    let app = build_app(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");

    let handle = tokio::spawn(async move {
        // Without ConnectInfo every caller looks unknown, and an unknown caller is
        // treated as remote -- which quietly disabled `/source` in these tests.
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    wait_until_ready(&url).await;
    (url, handle)
}

/// Wait until the server is accepting connections instead of guessing how long it needs:
/// a loaded machine can take much longer than the usual 50 ms, and a test that fails
/// because of that teaches people to ignore failures.
async fn wait_until_ready(url: &str) {
    for _ in 0..500 {
        if reqwest::get(format!("{url}/api/health")).await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("the test server never answered on {url}");
}

/// Submit a session with typed notes via HTTP.
async fn http_submit(client: &reqwest::Client, server_url: &str, id: &str, notes: &str) {
    let form = reqwest::multipart::Form::new().text("typed_notes", notes.to_string());
    client
        .post(format!("{server_url}/api/sessions/{id}/submit"))
        .multipart(form)
        .send()
        .await
        .unwrap();
}

/// Cancel a session via HTTP.
async fn http_cancel(client: &reqwest::Client, server_url: &str, id: &str) {
    client
        .delete(format!("{server_url}/api/sessions/{id}"))
        .send()
        .await
        .unwrap();
}

/// List active sessions and return the first ID found, waiting briefly for it to appear.
async fn first_active_session_id(client: &reqwest::Client, server_url: &str) -> String {
    for _ in 0..20 {
        let sessions: Vec<serde_json::Value> = client
            .get(format!("{server_url}/api/sessions?status=active"))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if let Some(id) = sessions.first().and_then(|s| s["id"].as_str()) {
            return id.to_string();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("no active session appeared");
}

#[tokio::test]
async fn cli_push_async_and_result() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# CLI Test\n\nHello from CLI").unwrap();

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--async",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(output.status.success(), "push failed: {:?}", output);
    let session_id = String::from_utf8(output.stdout).unwrap().trim().to_string();
    assert!(!session_id.is_empty());

    // Submit via HTTP
    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().text("typed_notes", "CLI feedback");
    client
        .post(format!("{server_url}/api/sessions/{session_id}/submit"))
        .multipart(form)
        .send()
        .await
        .unwrap();

    // Get result via CLI
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "result", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("CLI feedback"));
    assert!(stdout.contains(&session_id));
}

#[tokio::test]
async fn cli_list_sessions() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# List Test").unwrap();

    // Create a session
    tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--async",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    // List
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "list"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Active"));
    assert!(stdout.contains("List Test"));
}

#[tokio::test]
async fn cli_cancel_session() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Cancel Test").unwrap();

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--async",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    let session_id = String::from_utf8(output.stdout).unwrap().trim().to_string();

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "cancel", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("cancelled"));
}

#[tokio::test]
async fn cli_list_with_status_filter() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Filter Test").unwrap();

    // Create and immediately cancel a session
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--async",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    let session_id = String::from_utf8(output.stdout).unwrap().trim().to_string();

    tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "cancel", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    // `list --status active` must not include the cancelled session
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "list", "--status", "active"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("Filter Test"),
        "cancelled session must not appear in --status active list"
    );

    // `list --status cancelled` must include it
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "list", "--status", "cancelled"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Filter Test"),
        "cancelled session must appear in --status cancelled list"
    );
}

#[tokio::test]
async fn cli_push_json_output() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# JSON Test").unwrap();

    let session_id = {
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
            .args([
                "--server",
                &server_url,
                "push",
                "--async",
                tmp.path().to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().text("typed_notes", "json notes");
    client
        .post(format!("{server_url}/api/sessions/{session_id}/submit"))
        .multipart(form)
        .send()
        .await
        .unwrap();

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "result", "--json", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["typed_notes"], "json notes");
    assert!(json["annotation_images"].is_array());
}

#[tokio::test]
async fn cli_push_extracts_title_from_h1() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Auto Extracted Title\n\nbody text").unwrap();

    tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--async",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "list"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Auto Extracted Title"));
}

#[tokio::test]
async fn cli_result_on_active_session_shows_pending() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Active Session").unwrap();

    let session_id = {
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
            .args([
                "--server",
                &server_url,
                "push",
                "--async",
                tmp.path().to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "result", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(5),
        "a review that has not happened must not exit 0: a pipeline would read it as a pass"
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("still active"),
        "expected 'still active' in stderr, got: {stderr}"
    );
}

#[tokio::test]
async fn cli_cancel_nonexistent_exits_with_error() {
    let (server_url, _handle) = start_server().await;

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "cancel", "no-such-session"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(
        !out.status.success(),
        "expected non-zero exit for nonexistent session"
    );
}

#[tokio::test]
async fn cli_result_on_nonexistent_session_exits_with_error() {
    let (server_url, _handle) = start_server().await;

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "result", "no-such-id"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("not found"), "got: {stderr}");
}

#[tokio::test]
async fn cli_result_on_cancelled_session_exits_with_error() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Cancelled Result Test").unwrap();

    let session_id = {
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
            .args([
                "--server",
                &server_url,
                "push",
                "--async",
                tmp.path().to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "cancel", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "result", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("cancelled"), "got: {stderr}");
}

#[tokio::test]
async fn cli_push_nonexistent_server_exits_with_error() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Push Error Test").unwrap();

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            "http://127.0.0.1:1",
            "push",
            "--async",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(!out.status.success());
}

#[tokio::test]
async fn cli_push_reads_from_stdin() {
    let (server_url, _handle) = start_server().await;

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "push", "--async", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let out = {
        use tokio::io::AsyncWriteExt as _;
        let mut child = out;
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"# Stdin Title\n\nbody")
            .await
            .unwrap();
        child.wait_with_output().await.unwrap()
    };

    assert!(out.status.success(), "push from stdin failed: {:?}", out);
    let session_id = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(!session_id.is_empty());

    let list_out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "list"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    let stdout = String::from_utf8(list_out.stdout).unwrap();
    assert!(stdout.contains("Stdin Title"), "got: {stdout}");
}

#[tokio::test]
async fn cli_print_result_omits_empty_sections() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Empty Sections Test").unwrap();

    let session_id = {
        let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
            .args([
                "--server",
                &server_url,
                "push",
                "--async",
                tmp.path().to_str().unwrap(),
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap()
            .wait_with_output()
            .await
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    // Submit with empty notes and no image
    let client = reqwest::Client::new();
    let form = reqwest::multipart::Form::new().text("typed_notes", "");
    client
        .post(format!("{server_url}/api/sessions/{session_id}/submit"))
        .multipart(form)
        .send()
        .await
        .unwrap();

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "result", &session_id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        !stdout.contains("## Typed Notes"),
        "empty notes should not produce section header"
    );
    assert!(
        !stdout.contains("## Attached Images"),
        "no images should not produce section header"
    );
}

// ── blocking push ─────────────────────────────────────────────────────────────

/// `push <file>` blocks until the reviewer submits, then prints the notes.
#[tokio::test]
async fn cli_blocking_push_waits_and_prints_result() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Blocking Push\n\nbody").unwrap();

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let id = first_active_session_id(&client, &server_url).await;
    http_submit(&client, &server_url, &id, "blocking review notes").await;

    let out = child.wait_with_output().await.unwrap();
    assert!(
        out.status.success(),
        "expected exit 0, got: {:?}",
        out.status
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("blocking review notes"));
    assert!(stdout.contains(&id));
}

/// `push --json <file>` prints the submission as pretty-printed JSON.
#[tokio::test]
async fn cli_blocking_push_json_output() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# JSON Blocking Push").unwrap();

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--json",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let id = first_active_session_id(&client, &server_url).await;
    http_submit(&client, &server_url, &id, "json blocking notes").await;

    let out = child.wait_with_output().await.unwrap();
    assert!(out.status.success());
    let json: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout should be valid JSON");
    assert_eq!(json["typed_notes"], "json blocking notes");
    assert!(json["annotation_images"].is_array());
}

/// `push <file>` exits non-zero when the session is cancelled while waiting.
#[tokio::test]
async fn cli_blocking_push_cancelled_during_wait_exits_error() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Cancel While Blocking").unwrap();

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let id = first_active_session_id(&client, &server_url).await;
    http_cancel(&client, &server_url, &id).await;

    let out = child.wait_with_output().await.unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("cancelled"), "got: {stderr}");
}

// ── verdict exit codes ────────────────────────────────────────────────────────

/// LGTM verdict → exit 0 (reviewer approved).
#[tokio::test]
async fn cli_verdict_lgtm_exits_zero() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# LGTM Test").unwrap();

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let id = first_active_session_id(&client, &server_url).await;
    http_submit(&client, &server_url, &id, "LGTM").await;

    let out = child.wait_with_output().await.unwrap();
    assert_eq!(out.status.code(), Some(0));
}

/// CHANGES verdict → exit 2 (changes requested).
#[tokio::test]
async fn cli_verdict_changes_exits_two() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Changes Test").unwrap();

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let id = first_active_session_id(&client, &server_url).await;
    http_submit(&client, &server_url, &id, "CHANGES: fix the nits").await;

    let out = child.wait_with_output().await.unwrap();
    assert_eq!(out.status.code(), Some(2));
}

/// REJECT verdict → exit 3 (reviewer rejected the change).
#[tokio::test]
async fn cli_verdict_reject_exits_three() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Reject Test").unwrap();

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let id = first_active_session_id(&client, &server_url).await;
    http_submit(&client, &server_url, &id, "REJECT: wrong approach").await;

    let out = child.wait_with_output().await.unwrap();
    assert_eq!(out.status.code(), Some(3));
}

/// QUESTION verdict → exit 4 (reviewer has a question).
#[tokio::test]
async fn cli_verdict_question_exits_four() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Question Test").unwrap();

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let id = first_active_session_id(&client, &server_url).await;
    http_submit(&client, &server_url, &id, "QUESTION: what does this do?").await;

    let out = child.wait_with_output().await.unwrap();
    assert_eq!(out.status.code(), Some(4));
}

// ── push options ──────────────────────────────────────────────────────────────

/// `push --title <title>` overrides the H1 extracted from the document.
#[tokio::test]
async fn cli_push_explicit_title_overrides_h1() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Document H1\n\nbody").unwrap();

    tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--async",
            "--title",
            "Explicit Title",
            tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "list"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Explicit Title"), "got: {stdout}");
    assert!(
        !stdout.contains("Document H1"),
        "H1 must not appear: {stdout}"
    );
}

// ── interactive update (HTTP PUT) ─────────────────────────────────────────────

/// Full interactive round-trip:
///   push --interactive → device sends request_update via WS → session enters
///   Processing → `eink-review update` sends HTTP PUT → server broadcasts
///   version_updated → WS subscriber sees it → session returns to Active.
#[tokio::test]
async fn cli_update_http_put_increments_version() {
    use futures_util::{SinkExt as _, StreamExt as _};
    use tokio_tungstenite::tungstenite::Message as WsMessage;

    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    // Create session via HTTP
    let resp = client
        .post(format!("{server_url}/api/sessions"))
        .json(&serde_json::json!({
            "schema_version": 1,
            "title": "Interactive Update Test",
            "content": "# Interactive Update Test\n\nOriginal content",
        }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.unwrap();
    let session_id = body["id"].as_str().unwrap().to_string();

    // Connect via WS (simulates the tablet) and subscribe
    let ws_url = server_url.replacen("http://", "ws://", 1) + "/ws/" + &session_id;
    let (mut ws, _) = tokio_tungstenite::connect_async(&ws_url).await.unwrap();
    ws.send(WsMessage::Text(
        serde_json::json!({"type": "subscribe"}).to_string(),
    ))
    .await
    .unwrap();

    // Send request_update from the "device" — transitions Active → Processing
    ws.send(WsMessage::Text(
        serde_json::json!({
            "type": "request_update",
            "annotations": [],
            "typed_notes": ""
        })
        .to_string(),
    ))
    .await
    .unwrap();

    // Drain WS messages until annotation_result (or timeout after 3s)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let msg = tokio::time::timeout(remaining, ws.next()).await;
        match msg {
            Ok(Some(Ok(WsMessage::Text(text)))) => {
                let v: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                if v["type"] == "annotation_result" {
                    break;
                }
            }
            _ => break, // timeout or close — proceed anyway
        }
    }

    // Now call `eink-review update` which must use HTTP PUT
    let update_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        update_file.path(),
        "# Interactive Update Test\n\nUpdated content v2",
    )
    .unwrap();

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "update",
            &session_id,
            update_file.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(
        out.status.success(),
        "eink-review update failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("version updated to 2"),
        "expected version 2 in stderr, got: {stderr}"
    );

    // Session must be back in Active state
    let detail: serde_json::Value = client
        .get(format!("{server_url}/api/sessions/{session_id}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["status"], "Active");

    // Verify the updated content is served
    let html = client
        .get(format!("{server_url}/session/{session_id}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        html.contains("Updated content v2"),
        "expected updated content in HTML, got: {html}"
    );
}

/// Run the CLI against the test server and return its output.
async fn run_cli(server_url: &str, args: &[&str]) -> std::process::Output {
    tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", server_url])
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap()
}

/// Push a document and return the session id the CLI printed.
async fn push_document(server_url: &str, content: &str) -> String {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), content).unwrap();
    let output = run_cli(
        server_url,
        &["push", "--async", tmp.path().to_str().unwrap()],
    )
    .await;
    assert!(output.status.success());
    // `push --async` prints the session id on its own line.
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[tokio::test]
async fn cli_star_and_unstar_pin_a_review() {
    // Pinning is what keeps a finished review through purge, expiry and restart, so the
    // command has to reach the server.
    let (server_url, _handle) = start_server().await;
    let id = push_document(&server_url, "# Pinned review").await;

    let starred = run_cli(&server_url, &["star", &id]).await;
    assert!(starred.status.success());
    assert!(String::from_utf8_lossy(&starred.stderr).contains("starred"));

    let listed = run_cli(&server_url, &["list"]).await;
    let stdout = String::from_utf8_lossy(&listed.stdout).to_string();
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with('*') && line.contains(&id)),
        "the pinned review should be marked: {stdout}"
    );

    let unstarred = run_cli(&server_url, &["unstar", &id]).await;
    assert!(unstarred.status.success());
    assert!(String::from_utf8_lossy(&unstarred.stderr).contains("unstarred"));
    let listed = run_cli(&server_url, &["list"]).await;
    let stdout = String::from_utf8_lossy(&listed.stdout).to_string();
    assert!(
        stdout
            .lines()
            .any(|line| line.starts_with(' ') && line.contains(&id)),
        "the star should be gone: {stdout}"
    );
}

#[tokio::test]
async fn cli_remove_takes_the_review_away_and_cancel_keeps_it() {
    // These two commands used to do the same thing, so a "removed" review kept its files
    // for good. The difference is the point of the command.
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();

    let cancelled = push_document(&server_url, "# Cancelled review").await;
    let removed = push_document(&server_url, "# Removed review").await;

    assert!(
        run_cli(&server_url, &["cancel", &cancelled])
            .await
            .status
            .success()
    );
    // A live review is refused, so `remove` has to be deliberate about it.
    let refused = run_cli(&server_url, &["remove", &removed]).await;
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("still being read"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(
        run_cli(&server_url, &["remove", "--force", &removed])
            .await
            .status
            .success()
    );

    let listed = run_cli(&server_url, &["list"]).await;
    let stdout = String::from_utf8_lossy(&listed.stdout).to_string();
    assert!(
        stdout.contains(&cancelled),
        "a cancelled review stays: {stdout}"
    );
    assert!(stdout.contains("Cancelled"));
    assert!(
        !stdout.contains(&removed),
        "a removed review is gone: {stdout}"
    );

    // The cancelled one can still be read back; the removed one cannot.
    let cancelled_result = client
        .get(format!("{server_url}/api/sessions/{cancelled}/result"))
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled_result.status(), 410);
    let removed_result = client
        .get(format!("{server_url}/api/sessions/{removed}/result"))
        .send()
        .await
        .unwrap();
    assert_eq!(removed_result.status(), 404);

    // And removing something that is not there is reported as such.
    let again = run_cli(&server_url, &["remove", &removed]).await;
    assert!(!again.status.success());
    assert!(String::from_utf8_lossy(&again.stderr).contains("not found"));
}

#[tokio::test]
async fn cli_update_replaces_the_document_and_bumps_the_version() {
    let (server_url, _handle) = start_server().await;
    let client = reqwest::Client::new();
    let id = push_document(&server_url, "# Original").await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Rewritten\n\nThe second draft").unwrap();
    let output = run_cli(&server_url, &["update", &id, tmp.path().to_str().unwrap()]).await;
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("version updated to 2"));

    let body: serde_json::Value = client
        .get(format!("{server_url}/api/sessions/{id}/content"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["version"], 2);
    assert!(body["content"].as_str().unwrap().contains("second draft"));

    // Updating something that does not exist is an error the operator can act on.
    let missing = run_cli(
        &server_url,
        &["update", "does-not-exist", tmp.path().to_str().unwrap()],
    )
    .await;
    assert!(!missing.status.success());
    assert!(String::from_utf8_lossy(&missing.stderr).contains("not found"));
}

#[tokio::test]
async fn the_cli_pushes_its_own_command_metrics() {
    // The CLI reports its commands to a pushgateway so that "did the review machine
    // actually run" is answerable. The body has to be a well-formed exposition or the
    // pushgateway rejects it, and the command is fire-and-forget.
    use std::sync::{Arc, Mutex};

    let received: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = received.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let pushgateway = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri, body: String| {
            let sink = sink.clone();
            async move {
                if let Ok(mut sink) = sink.lock() {
                    sink.push((uri.path().to_string(), body));
                }
                axum::http::StatusCode::OK
            }
        });
        let _ = axum::serve(listener, app).await;
    });

    let (server_url, _handle) = start_server().await;
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url, "list"])
        .env("EINK_PUSHGATEWAY_URL", &pushgateway)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert!(output.status.success());

    // The push happens after the command, so give it a moment.
    let mut posted = received.lock().unwrap().clone();
    for _ in 0..40 {
        if !posted.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        posted = received.lock().unwrap().clone();
    }
    let (path, body) = posted.first().expect("the CLI should push its metrics");
    assert_eq!(path, "/metrics/job/eink-review/instance/cli");
    assert!(
        body.contains("eink_cli_commands_total{command=\"list\",result=\"ok\"} 1"),
        "the push should name the command that ran: {body}"
    );
    assert!(body.contains("eink_cli_duration_seconds{command=\"list\",result=\"ok\"} "));
    for line in body.lines().filter(|line| !line.starts_with('#')) {
        assert!(
            line.rsplit_once(' ')
                .is_some_and(|(_, value)| value.parse::<f64>().is_ok()),
            "malformed line would be rejected wholesale: {line}"
        );
    }
}

/// Run the CLI and return (exit code, stdout, stderr).
async fn run_cli_raw(args: &[&str], env: &[(&str, &str)]) -> (i32, String, String) {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"));
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().await.unwrap();
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[tokio::test]
async fn the_cli_reads_content_and_source_and_guards_an_update() {
    let (server, handle) = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.md");
    let second = dir.path().join("second.md");
    std::fs::write(&first, "# First\n\nOne paragraph.\n").unwrap();
    std::fs::write(&second, "# Second\n\nTwo paragraphs.\n").unwrap();
    let first = first.to_str().unwrap();
    let second = second.to_str().unwrap();

    let (code, stdout, stderr) = run_cli_raw(
        &[
            "--server", &server, "push", "--async", "--tag", "env=test", first,
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "{stderr}");
    let id = stdout.trim().to_string();

    // `list --json` is the machine-readable list, tags included.
    let (code, stdout, stderr) = run_cli_raw(&["--server", &server, "list", "--json"], &[]).await;
    assert_eq!(code, 0, "{stderr}");
    let listed: Vec<serde_json::Value> = serde_json::from_str(&stdout).unwrap();
    assert_eq!(listed[0]["id"], id);
    assert_eq!(listed[0]["tags"]["env"], "test");

    // `content` is the raw markdown on stdout, so it can be redirected.
    let (code, stdout, stderr) = run_cli_raw(&["--server", &server, "content", &id], &[]).await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("# First"), "{stdout}");

    // `source` reads from the checkout the review was pushed from.
    let (code, stdout, stderr) = run_cli_raw(
        &[
            "--server",
            &server,
            "source",
            &id,
            "src/cli.rs:10",
            "--context",
            "1",
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("src/cli.rs:9-11"), "{stdout}");
    assert!(
        stdout.contains("use eink_bridge::session::SessionOrigin"),
        "the cited lines must be printed: {stdout}"
    );

    // A stale version is refused and the document is untouched.
    let (code, _, stderr) = run_cli_raw(
        &[
            "--server",
            &server,
            "update",
            &id,
            second,
            "--expected-version",
            "99",
        ],
        &[],
    )
    .await;
    assert_ne!(code, 0, "a stale update must fail");
    assert!(stderr.contains("stale version"), "{stderr}");
    let (_, stdout, _) = run_cli_raw(&["--server", &server, "content", &id], &[]).await;
    assert!(
        stdout.contains("# First"),
        "the refused update must not land: {stdout}"
    );

    // The matching version lands.
    let (code, _, stderr) = run_cli_raw(
        &[
            "--server",
            &server,
            "update",
            &id,
            second,
            "--expected-version",
            "1",
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "{stderr}");
    let (_, stdout, _) = run_cli_raw(&["--server", &server, "content", &id], &[]).await;
    assert!(stdout.contains("# Second"), "{stdout}");

    // A feedback id that is not pending is refused, and the refusal names the reason
    // rather than reporting a stale version.
    let (code, _, stderr) = run_cli_raw(
        &[
            "--server",
            &server,
            "update",
            &id,
            second,
            "--feedback-id",
            "not-pending",
            "--consumed",
            "0",
        ],
        &[],
    )
    .await;
    assert_ne!(code, 0);
    assert!(stderr.contains("update refused"), "{stderr}");

    handle.abort();
}

#[tokio::test]
async fn a_review_that_has_not_happened_is_not_success() {
    let (server, handle) = start_server().await;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("review.md");
    std::fs::write(&file, "# Waiting\n").unwrap();
    let file = file.to_str().unwrap();
    let (_, stdout, _) = run_cli_raw(&["--server", &server, "push", "--async", file], &[]).await;
    let id = stdout.trim().to_string();

    let (code, _, stderr) = run_cli_raw(&["--server", &server, "result", &id], &[]).await;
    assert_eq!(
        code, 5,
        "pending must be distinguishable from lgtm: {stderr}"
    );
    assert!(stderr.contains("still active"), "{stderr}");
    handle.abort();
}

#[tokio::test]
async fn the_server_url_defaults_to_the_environment() {
    let (server, handle) = start_server().await;
    let (code, stdout, stderr) =
        run_cli_raw(&["list", "--json"], &[("EINK_SERVER_URL", &server)]).await;
    assert_eq!(code, 0, "EINK_SERVER_URL was ignored: {stderr}");
    let _: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    handle.abort();
}

#[tokio::test]
async fn the_cli_narrates_progress_for_the_pending_feedback() {
    let (server, handle) = start_server().await;
    let id = push_document(&server, "# Review\n\nBody").await;
    let client = reqwest::Client::new();
    client
        .post(format!("{server}/api/sessions/{id}/request_update"))
        .json(&serde_json::json!({
            "annotations": [{"anchor": null, "strokes": [], "recognized_text": "tighter"}]
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    // The pending feedback is the common case; the agent should not have to repeat its id.
    let (code, _, stderr) = run_cli_raw(
        &[
            "--server", &server, "progress", &id, "Read 1 note on *Checks*", "--step", "1",
            "--of", "3",
        ],
        &[],
    )
    .await;
    assert_eq!(code, 0, "{stderr}");

    // A line about a note that is not pending is refused rather than overwriting the
    // one the reader is watching.
    let (code, _, stderr) = run_cli_raw(
        &[
            "--server", &server, "progress", &id, "Rewriting", "--feedback-id", "already-answered",
        ],
        &[],
    )
    .await;
    assert_ne!(code, 0);
    assert!(stderr.contains("not pending"), "{stderr}");
    handle.abort();
}
