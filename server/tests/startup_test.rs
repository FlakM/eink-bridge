//! What happens when the server cannot start.
//!
//! The first thing that goes wrong after a reboot is usually the port still being held,
//! and a panic trace in the journal is no way to learn that. This runs the real binary
//! against an occupied port.
use std::process::Stdio;
use tokio::net::TcpListener;

#[tokio::test]
async fn an_occupied_port_is_reported_in_words() {
    // Hold a port, then point the server at it through its own config file.
    let holder = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = holder.local_addr().unwrap().port();

    let config_home = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    let config_dir = config_home.path().join("eink-bridge");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        format!("[server]\nhost = \"127.0.0.1\"\nport = {port}\n"),
    )
    .unwrap();

    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-serve"))
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("XDG_DATA_HOME", data_home.path())
        .env("EINK_OCR_OPENAI_ENABLED", "0")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "a failed start is not success"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot listen"),
        "the operator should be told in words: {stderr}"
    );
    assert!(
        stderr.contains(&port.to_string()),
        "and told which port: {stderr}"
    );
    assert!(
        !stderr.contains("panicked"),
        "a panic trace is not an error message: {stderr}"
    );
}

#[tokio::test]
async fn a_free_port_starts_and_answers_health() {
    // The other half of the contract: the same configuration works when nothing is in
    // the way, so the check above is about the conflict rather than about the config.
    let config_home = tempfile::tempdir().unwrap();
    let data_home = tempfile::tempdir().unwrap();
    let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let config_dir = config_home.path().join("eink-bridge");
    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("config.toml"),
        format!("[server]\nhost = \"127.0.0.1\"\nport = {port}\n"),
    )
    .unwrap();

    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-serve"))
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("XDG_DATA_HOME", data_home.path())
        .env("EINK_OCR_OPENAI_ENABLED", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let mut listening = false;
    for _ in 0..60 {
        if let Ok(response) = reqwest::get(format!("http://127.0.0.1:{port}/api/health")).await
            && response.status().is_success()
        {
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(body["status"], "ok");
            listening = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(listening, "the server should have started on a free port");
    let _ = child.kill().await;
}
