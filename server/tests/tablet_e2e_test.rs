use reqwest::Client;
/// End-to-end tests against the real running server.
///
/// Uses `eink-mock-device` to simulate the Boox tablet — no human required.
/// Tests verify the exact same HTTP flows the Android app exercises.
///
/// Run:
///   cargo test --test tablet_e2e_test -- --ignored --nocapture
///
/// Environment:
///   EINK_SERVER   server base URL (default: http://localhost:3333)
///
/// Requirements:
///   eink-serve must be running: systemctl --user start eink-serve
use std::time::Duration;

fn server_url() -> String {
    std::env::var("EINK_SERVER").unwrap_or_else(|_| "http://localhost:3333".into())
}

async fn assert_server_reachable(client: &Client) {
    let url = format!("{}/api/health", server_url());
    let resp = client
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .unwrap_or_else(|e| {
            panic!("server not reachable at {url}: {e}\nRun: systemctl --user start eink-serve")
        });
    assert!(
        resp.status().is_success(),
        "health check failed: {}",
        resp.status()
    );
}

async fn create_session(client: &Client, content: &str, title: &str) -> String {
    let url = format!(
        "{}/api/sessions?title={}",
        server_url(),
        urlencoding::encode(title)
    );
    let resp = client
        .post(&url)
        .body(content.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    body["id"].as_str().unwrap().to_string()
}

/// Minimal valid 1×1 PNG bytes.
fn tiny_png() -> Vec<u8> {
    vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xde, 0x00, 0x00, 0x00, 0x0c, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xf8,
        0xcf, 0xc0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xe2, 0x21, 0xbc, 0x33, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ]
}

/// Spawn mock device targeting a specific session on the real server.
async fn run_mock_device_for(session_id: &str, notes: &str, image_path: Option<&str>) {
    let url = server_url();
    let mut args = vec![
        "--server",
        url.as_str(),
        "--session-id",
        session_id,
        "--notes",
        notes,
        "--poll-interval",
        "1",
    ];
    if let Some(p) = image_path {
        args.extend_from_slice(&["--image", p]);
    }

    let out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-mock-device"))
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stderr.contains("submitted review"),
        "mock device failed for session {session_id}: {stderr}"
    );
}

/// Poll /result until 200, retrying 204s for up to 10 seconds.
async fn poll_result(client: &Client, id: &str) -> serde_json::Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out polling result for {id}"
        );
        let resp = client
            .get(format!("{}/api/sessions/{id}/result", server_url()))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .unwrap();
        match resp.status().as_u16() {
            200 => return resp.json().await.unwrap(),
            204 => tokio::time::sleep(Duration::from_millis(200)).await,
            s => panic!("unexpected status {s} for session {id}"),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

/// Session HTML renders correctly — no device interaction needed.
#[tokio::test]
#[ignore]
async fn session_renders_valid_html() {
    let client = Client::new();
    assert_server_reachable(&client).await;

    let content = "# Render Check\n\n## Section A\n\nSome prose.\n\n```rust\nfn greet() {}\n```";
    let id = create_session(&client, content, "Render Check").await;

    let resp = client
        .get(format!("{}/session/{id}", server_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let html = resp.text().await.unwrap();
    assert!(html.contains("<!DOCTYPE html>"), "missing DOCTYPE");
    assert!(html.contains("Render Check"), "title missing");
    assert!(html.contains("Section A"), "heading missing");
    assert!(html.contains("greet"), "code block missing");
    eprintln!("session {id} renders OK — {}/session/{id}", server_url());
}

/// Rich document with diagrams and code renders without error.
#[tokio::test]
#[ignore]
async fn rich_document_renders() {
    let client = Client::new();
    assert_server_reachable(&client).await;

    let content = "# Rich Doc\n\n```mermaid\nflowchart LR\n  A-->B\n```\n\n```rust\nfn f() {}\n```\n\n- a\n- b\n";
    let id = create_session(&client, content, "Rich Doc").await;

    let resp = client
        .get(format!("{}/session/{id}", server_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let html = resp.text().await.unwrap();
    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("mermaid"));
    eprintln!("rich session {id} — {}/session/{id}", server_url());
}

/// Session list is sorted by updated_at descending.
#[tokio::test]
#[ignore]
async fn session_list_sorted_by_updated_at() {
    let client = Client::new();
    assert_server_reachable(&client).await;

    let id1 = create_session(&client, "# Older", "Ordering-Old").await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let id2 = create_session(&client, "# Newer", "Ordering-New").await;

    let sessions: Vec<serde_json::Value> = client
        .get(format!("{}/api/sessions", server_url()))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let find_updated_at = |target: &str| -> String {
        sessions
            .iter()
            .find(|s| s["id"].as_str() == Some(target))
            .and_then(|s| s["updated_at"].as_str().map(str::to_owned))
            .unwrap_or_else(|| panic!("session {target} not in list"))
    };
    let t1 = find_updated_at(&id1);
    let t2 = find_updated_at(&id2);
    assert!(t2 > t1, "newer ({t2}) should be > older ({t1})");

    let order: Vec<&str> = sessions
        .iter()
        .filter_map(|s| s["id"].as_str())
        .filter(|&id| id == id1 || id == id2)
        .collect();
    assert_eq!(
        order,
        vec![id2.as_str(), id1.as_str()],
        "id2 must appear before id1"
    );
    eprintln!("ordering OK: {id2} before {id1}");
}

/// Mock device loads the session HTML (like the Android WebView), then submits typed notes.
#[tokio::test]
#[ignore]
async fn mock_device_submits_notes() {
    let client = Client::new();
    assert_server_reachable(&client).await;

    let id = create_session(&client, "# Notes Test\n\nReview this.", "Notes Test").await;
    run_mock_device_for(&id, "tablet feedback", None).await;

    let result = poll_result(&client, &id).await;
    assert_eq!(result["typed_notes"], "tablet feedback");
    assert_eq!(result["status"], "submitted");
    eprintln!("notes submitted OK for session {id}");
}

/// Mock device loads HTML, draws (submits a PNG annotation), and submits.
/// The saved file must exist on disk with non-zero size.
#[tokio::test]
#[ignore]
async fn mock_device_submits_with_drawing() {
    let client = Client::new();
    assert_server_reachable(&client).await;

    let img = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(img.path(), tiny_png()).unwrap();

    let id = create_session(&client, "# Drawing Test\n\nDraw on this.", "Drawing Test").await;
    run_mock_device_for(&id, "drew something", Some(img.path().to_str().unwrap())).await;

    let result = poll_result(&client, &id).await;
    assert_eq!(result["typed_notes"], "drew something");

    let images = result["annotation_images"].as_array().unwrap();
    assert!(!images.is_empty(), "no annotation images in result");
    for img_val in images {
        let path = img_val.as_str().unwrap();
        assert!(
            std::path::Path::new(path).exists(),
            "annotation not on disk: {path}"
        );
        let bytes = std::fs::read(path).unwrap();
        assert!(!bytes.is_empty(), "annotation file is empty");
        eprintln!("  annotation: {path} ({} bytes)", bytes.len());
    }
    eprintln!("drawing submitted OK for session {id}");
}

/// Cancel via API gives 410 on subsequent result fetch — same flow as Android long-press cancel.
#[tokio::test]
#[ignore]
async fn cancel_gives_410_on_result() {
    let client = Client::new();
    assert_server_reachable(&client).await;

    let id = create_session(&client, "# Cancel Flow", "Cancel Flow").await;

    let resp = client
        .delete(format!("{}/api/sessions/{id}", server_url()))
        .send()
        .await
        .unwrap();
    assert!(
        resp.status().is_success(),
        "cancel failed: {}",
        resp.status()
    );

    let result_resp = client
        .get(format!("{}/api/sessions/{id}/result", server_url()))
        .send()
        .await
        .unwrap();
    assert_eq!(result_resp.status().as_u16(), 410);
    eprintln!("cancel→410 verified for session {id}");
}

/// Full CLI round-trip via real server: push → mock device reads HTML + submits drawing → CLI result.
#[tokio::test]
#[ignore]
async fn cli_push_mock_draws_cli_result() {
    let client = Client::new();
    assert_server_reachable(&client).await;

    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# CLI Round-trip\n\nFull flow via real server.").unwrap();

    let push_out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args([
            "--server",
            &server_url(),
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
    assert!(
        push_out.status.success(),
        "push failed: {}",
        String::from_utf8_lossy(&push_out.stderr)
    );
    let id = String::from_utf8(push_out.stdout)
        .unwrap()
        .trim()
        .to_string();
    assert!(!id.is_empty());

    let img = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(img.path(), tiny_png()).unwrap();

    run_mock_device_for(&id, "round-trip notes", Some(img.path().to_str().unwrap())).await;

    let result_out = tokio::process::Command::new(env!("CARGO_BIN_EXE_eink-review"))
        .args(["--server", &server_url(), "result", &id])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
        .wait_with_output()
        .await
        .unwrap();
    assert!(result_out.status.success());
    let stdout = String::from_utf8(result_out.stdout).unwrap();
    assert!(
        stdout.contains("round-trip notes"),
        "notes missing: {stdout}"
    );
    assert!(
        stdout.contains("Attached Images"),
        "image path missing: {stdout}"
    );
    eprintln!("full round-trip OK — session {id}");
    eprintln!("{stdout}");
}
