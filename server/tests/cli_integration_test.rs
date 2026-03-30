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
