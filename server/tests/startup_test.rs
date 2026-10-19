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

#[tokio::test]
async fn a_stop_signal_lets_queued_writes_land_before_the_process_ends() {
    // systemd stops the service with SIGTERM. The default handler ends the process on
    // the spot, and with it whatever the write queue still held.
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

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-serve"))
        .env("XDG_CONFIG_HOME", config_home.path())
        .env("XDG_DATA_HOME", data_home.path())
        .env("EINK_OCR_OPENAI_ENABLED", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().expect("a running child has a pid");

    let base = format!("http://127.0.0.1:{port}");
    let mut up = false;
    for _ in 0..60 {
        if reqwest::get(format!("{base}/api/health")).await.is_ok() {
            up = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(up, "the server should have started");
    let created: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/api/sessions"))
        .body("# Written just before the stop")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap().to_string();

    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .unwrap()
            .success()
    );
    let output = tokio::time::timeout(std::time::Duration::from_secs(15), child.wait_with_output())
        .await
        .expect("the server should stop on SIGTERM")
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "a requested stop is not a failure: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "{stderr}");

    let on_disk = std::fs::read_to_string(
        data_home
            .path()
            .join("eink-bridge")
            .join("sessions")
            .join(&id)
            .join("session.json"),
    )
    .expect("the session created before the stop is on disk");
    assert!(on_disk.contains("Written just before the stop"));
}
