use eink_bridge::app::{AppState, build_app};
use std::time::Duration;
use tokio::net::TcpListener;

async fn start_server() -> (String, tokio::task::JoinHandle<()>) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::with_config(dir.keep(), 5);
    let app = build_app(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let url = format!("http://{addr}");

    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    tokio::time::sleep(Duration::from_millis(50)).await;
    (url, handle)
}

/// Full round-trip: CLI push -> mock device detects + submits -> CLI receives notes
#[tokio::test]
async fn full_e2e_loop() {
    let (server_url, _handle) = start_server().await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# E2E Test\n\nEnd-to-end content").unwrap();

    // Start mock device in background (--once mode, fast polling)
    let mock_handle = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-mock-device"))
        .args([
            "--server",
            &server_url,
            "--notes",
            "E2E mock feedback",
            "--poll-interval",
            "1",
            "--once",
        ])
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    // Push via CLI (blocking) — will wait until mock device submits
    let cli_output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--timeout",
            "1",
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

    assert!(
        cli_output.status.success(),
        "CLI push failed: stdout={} stderr={}",
        String::from_utf8_lossy(&cli_output.stdout),
        String::from_utf8_lossy(&cli_output.stderr)
    );

    let stdout = String::from_utf8(cli_output.stdout).unwrap();
    assert!(
        stdout.contains("E2E mock feedback"),
        "Expected mock feedback in output, got: {stdout}"
    );
    assert!(stdout.contains("review notes"));

    // Mock device should have exited (--once)
    let mock_output = mock_handle.wait_with_output().await.unwrap();
    assert!(mock_output.status.success());
    let mock_stderr = String::from_utf8(mock_output.stderr).unwrap();
    assert!(mock_stderr.contains("submitted review"));
}

/// E2E with annotation image
#[tokio::test]
async fn e2e_with_image() {
    let (server_url, _handle) = start_server().await;

    // Create a tiny PNG-like test file
    let img_tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(img_tmp.path(), b"FAKE_PNG_DATA").unwrap();

    let doc_tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(doc_tmp.path(), "# Image E2E\n\nWith attachment").unwrap();

    // Start mock device with image
    let mock_handle = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-mock-device"))
        .args([
            "--server",
            &server_url,
            "--notes",
            "With image",
            "--image",
            img_tmp.path().to_str().unwrap(),
            "--poll-interval",
            "1",
            "--once",
        ])
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let cli_output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url,
            "push",
            "--timeout",
            "1",
            doc_tmp.path().to_str().unwrap(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert!(cli_output.status.success());
    let stdout = String::from_utf8(cli_output.stdout).unwrap();
    assert!(stdout.contains("With image"));
    assert!(stdout.contains("Attached Images"));

    let mock_output = mock_handle.wait_with_output().await.unwrap();
    assert!(mock_output.status.success());
}
