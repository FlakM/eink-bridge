use eink_bridge::app::{AppState, build_app};
use std::time::Duration;
use tokio::net::TcpListener;

async fn start_server() -> (String, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.keep(), 2);
    let app = build_app(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server a moment to start
    tokio::time::sleep(Duration::from_millis(50)).await;
    (url, handle)
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

    assert!(out.status.success());
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

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
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

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
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

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
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

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
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

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
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

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
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

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
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
        serde_json::json!({"type": "subscribe"}).to_string().into(),
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
        .to_string()
        .into(),
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
