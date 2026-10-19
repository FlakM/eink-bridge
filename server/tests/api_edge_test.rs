//! The edges of the HTTP surface: the things a client asks for when something is
//! unfinished, misspelled, or already over.
//!
//! Every case here is a request the CLI, the tablet or a curious operator can actually
//! make, and each one has a wrong answer that looks harmless: a removal that only
//! cancels, a purge that takes a starred review with it, a malformed field that comes
//! back as a 500.
mod common;

use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use common::test_app;
use eink_bridge::app::{AppState, build_app};

async fn create(app: &axum::Router, body: &str) -> String {
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    body["id"].as_str().unwrap().to_string()
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(Request::get(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, value)
}

async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Bytes) {
    let resp = app.clone().oneshot(request).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (status, bytes)
}

fn session_dir(state_dir: &std::path::Path, id: &str) -> std::path::PathBuf {
    state_dir.join("sessions").join(id)
}

#[tokio::test]
async fn remove_forgets_the_session_and_deletes_its_files() {
    // `eink-review remove` used to send a plain DELETE, which only cancels: the review,
    // its markdown and its annotation images stayed on disk for good.
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review\n\nBody").await;
    assert!(session_dir(dir.path(), &id).exists());

    // A review that is still being read is not removed on a bare request: that would
    // throw away work in progress on a mistyped id.
    let (status, body) = send(
        &app,
        Request::delete(format!("/api/sessions/{id}?remove=true"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        String::from_utf8_lossy(&body).contains("cancel it first"),
        "the refusal should say what to do instead: {}",
        String::from_utf8_lossy(&body)
    );
    assert!(
        session_dir(dir.path(), &id).exists(),
        "a refused removal must not have touched the files"
    );

    // Cancelling first is the graceful path, and then removal is allowed.
    let (status, _) = send(
        &app,
        Request::delete(format!("/api/sessions/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(
        &app,
        Request::delete(format!("/api/sessions/{id}?remove=true"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !session_dir(dir.path(), &id).exists(),
        "the session's files should be gone"
    );

    let (status, _) = get_json(&app, &format!("/api/sessions/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (_, list) = get_json(&app, "/api/sessions").await;
    assert!(
        !list.to_string().contains(&id),
        "a removed session must not be listed: {list}"
    );

    let (status, _) = send(
        &app,
        Request::delete(format!("/api/sessions/{id}?remove=true"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "removing twice is not an error"
    );
}

#[tokio::test]
async fn a_forced_removal_takes_a_live_review_away() {
    // The reader may simply give up on a session; `--force` is the deliberate word.
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Abandoned").await;

    let (status, _) = send(
        &app,
        Request::delete(format!("/api/sessions/{id}?remove=true&force=true"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!session_dir(dir.path(), &id).exists());
    let (status, _) = get_json(&app, &format!("/api/sessions/{id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_plain_delete_cancels_and_keeps_the_files_for_the_archive() {
    // Cancelling is the graceful path: the reader may still want the page that was sent.
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review\n\nBody").await;

    let (status, _) = send(
        &app,
        Request::delete(format!("/api/sessions/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(session_dir(dir.path(), &id).exists(), "the files stay");
    let (status, detail) = get_json(&app, &format!("/api/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(detail["status"], "Cancelled");
}

#[tokio::test]
async fn purge_takes_finished_reviews_and_leaves_the_rest_alone() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());

    let active = create(&app, "# Still being read").await;
    let submitted = create(&app, "# Finished").await;
    let starred = create(&app, "# Keep me pinned").await;
    let cancelled = create(&app, "# Dropped").await;

    for (id, notes) in [(&submitted, "LGTM"), (&starred, "LGTM")] {
        let mp = format!(
            "--b\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\n{notes}\r\n--b--\r\n"
        );
        send(
            &app,
            Request::post(format!("/api/sessions/{id}/submit"))
                .header("content-type", "multipart/form-data; boundary=b")
                .body(Body::from(mp))
                .unwrap(),
        )
        .await;
    }
    send(
        &app,
        Request::put(format!("/api/sessions/{starred}/star"))
            .header("content-type", "application/json")
            .body(Body::from(r#"{"starred": true}"#))
            .unwrap(),
    )
    .await;
    send(
        &app,
        Request::delete(format!("/api/sessions/{cancelled}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    let (status, body) = send(
        &app,
        Request::delete("/api/sessions")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let purged: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        purged["purged"], 2,
        "the submitted and the cancelled ones: {purged}"
    );

    let (_, list) = get_json(&app, "/api/sessions").await;
    let ids: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["id"].as_str())
        .collect();
    assert!(
        ids.contains(&active.as_str()),
        "a review in progress survives"
    );
    assert!(ids.contains(&starred.as_str()), "a pinned review survives");
    assert!(!ids.contains(&submitted.as_str()));
    assert!(!ids.contains(&cancelled.as_str()));
}

#[tokio::test]
async fn metrics_are_served_in_the_format_prometheus_scrapes() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let resp = app
        .clone()
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert!(
        content_type.starts_with("text/plain; version=0.0.4"),
        "a scraper reads the OpenMetrics text format, got {content_type:?}"
    );
    let text =
        String::from_utf8_lossy(&resp.into_body().collect().await.unwrap().to_bytes()).to_string();
    assert!(text.contains("# TYPE eink_sessions_active gauge"), "{text}");
    assert!(text.contains("eink_sessions_active "), "{text}");
    assert!(
        text.contains("# TYPE eink_ocr_requests_total counter"),
        "{text}"
    );
    assert!(
        text.contains("eink_ocr_requests_total{result=\"ok\"} 0"),
        "the vocabulary is exported before the first request: {text}"
    );
}

#[tokio::test]
async fn an_unparseable_reanchor_map_is_refused_rather_than_crashing() {
    // The element map comes from the reader's page and can be arbitrarily large; the
    // limit is what keeps the quadratic comparison bounded.
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review").await;

    let huge = serde_json::json!({
        "previous": (0..1001).map(|i| serde_json::json!({"i": i, "tag": "P", "text": "x"})).collect::<Vec<_>>(),
        "current": [],
    });
    let (status, body) = send(
        &app,
        Request::post(format!("/api/sessions/{id}/reanchor"))
            .header("content-type", "application/json")
            .body(Body::from(huge.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body).contains("limits"));

    let long = serde_json::json!({
        "previous": [{"i": 0, "tag": "P", "text": "x".repeat(151)}],
        "current": [],
    });
    let (status, _) = send(
        &app,
        Request::post(format!("/api/sessions/{id}/reanchor"))
            .header("content-type", "application/json")
            .body(Body::from(long.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "one long element is enough"
    );

    let (status, _) = send(
        &app,
        Request::post("/api/sessions/ghost/reanchor")
            .header("content-type", "application/json")
            .body(Body::from(long.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_acknowledgement_must_name_the_feedback_it_consumes() {
    // Consuming annotations without the matching id would throw away strokes the reader
    // never saw acknowledged.
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review").await;
    send(
        &app,
        Request::post(format!("/api/sessions/{id}/request_update"))
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"annotations":[{"anchor":null,"strokes":[[[1.0,2.0]]],"recognized_text":"hi"}]}"#,
            ))
            .unwrap(),
    )
    .await;

    let (_, feedback) = get_json(&app, &format!("/api/sessions/{id}/feedback")).await;
    let feedback_id = feedback["pending"]["feedback_id"]
        .as_str()
        .unwrap()
        .to_string();

    let body = |id: Option<&str>, consumed: &str| {
        let mut value = serde_json::json!({
            "content": "# Review\n\nv2",
            "consumed_annotations": serde_json::from_str::<serde_json::Value>(consumed).unwrap(),
        });
        if let Some(feedback_id) = id {
            value["feedback_id"] = serde_json::json!(feedback_id);
        }
        value.to_string()
    };

    // No id at all.
    let (status, body_bytes) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from(body(None, "[0]")))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body_bytes).contains("feedback_id"));

    // Wrong id.
    let (status, _) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from(body(Some("not-the-one"), "[0]")))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Right id, impossible index.
    let (status, body_bytes) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from(body(Some(&feedback_id), "[7]")))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body_bytes).contains("range"));

    // Right id, real index: accepted. The acknowledgement itself is carried on the
    // socket (see ws_test); the HTTP reply only reports the new version.
    let (status, body_bytes) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from(body(Some(&feedback_id), "[0]")))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let reply: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(reply["version"], 2);

    // The feedback is spent: a second consumption cannot name the same id.
    let (status, _) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from(body(Some(&feedback_id), "[0]")))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "the feedback was already consumed"
    );
}

#[tokio::test]
async fn a_stale_content_update_is_refused_so_edits_are_not_lost() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review\n\nv1").await;

    let (status, _) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"content": "# Review\n\nv2", "expected_version": 1}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, _) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::json!({"content": "clobber", "expected_version": 1}).to_string(),
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "an outdated writer is told to re-read"
    );
    let (_, content) = get_json(&app, &format!("/api/sessions/{id}/content")).await;
    assert!(content["content"].as_str().unwrap().contains("v2"));
}

#[tokio::test]
async fn a_content_update_must_be_json_or_utf8_text() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review").await;

    let (status, body) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "application/json")
            .body(Body::from("{ not json"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body).contains("invalid JSON"));

    let (status, body) = send(
        &app,
        Request::put(format!("/api/sessions/{id}/content"))
            .header("content-type", "text/plain")
            .body(Body::from(vec![0xff, 0xfe, 0xfd]))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body).contains("UTF-8"));
}

#[tokio::test]
async fn creating_a_session_only_accepts_markdown_json_or_plain_text() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());

    // A .md upload is read as markdown by the CLI.
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "text/markdown")
                .body(Body::from("# From a file"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let (status, body) = send(
        &app,
        Request::post("/api/sessions")
            .header("content-type", "application/json")
            .body(Body::from("{ not json"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body).contains("invalid create-session JSON"));

    // Anything that is not JSON is taken as the document text: the CLI sends a markdown
    // file and curl sends whatever the shell gave it. Binary is caught by the UTF-8
    // guard, not by an allow-list of media types.
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/pdf")
                .body(Body::from("%PDF-1.4"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::CREATED,
        "a non-JSON body is a document, however odd its media type"
    );

    let (status, body) = send(
        &app,
        Request::post("/api/sessions")
            .header("content-type", "text/plain")
            .body(Body::from(vec![0x00, 0xff, 0xfe]))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(String::from_utf8_lossy(&body).contains("valid UTF-8"));
}

#[tokio::test]
async fn a_review_cannot_be_submitted_twice_or_to_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review").await;

    async fn submit(app: &axum::Router, target: &str) -> StatusCode {
        let mp =
            "--b\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nLGTM\r\n--b--\r\n";
        app.clone()
            .oneshot(
                Request::post(format!("/api/sessions/{target}/submit"))
                    .header("content-type", "multipart/form-data; boundary=b")
                    .body(Body::from(mp))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    assert_eq!(submit(&app, &id).await, StatusCode::OK);
    assert_eq!(
        submit(&app, &id).await,
        StatusCode::CONFLICT,
        "a second submit would overwrite the verdict the first one carried"
    );
    assert_eq!(submit(&app, "ghost").await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_submission_with_a_broken_field_is_told_which_field() {
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review").await;

    // Annotations must be JSON, and the error must name them.
    let mp = "--b\r\nContent-Disposition: form-data; name=\"annotations\"\r\n\r\n{ not json\r\n--b--\r\n";
    let (status, body) = send(
        &app,
        Request::post(format!("/api/sessions/{id}/submit"))
            .header("content-type", "multipart/form-data; boundary=b")
            .body(Body::from(mp))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        String::from_utf8_lossy(&body).contains("invalid annotations JSON"),
        "{}",
        String::from_utf8_lossy(&body)
    );

    // A verdict field on a multipart post is read the same way the JSON path reads it:
    // an unusable tag is reported rather than dropped on the floor.
    let mp = "--b\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nlooks fine\r\n--b\r\nContent-Disposition: form-data; name=\"verdict\"\r\n\r\nmaybe\r\n--b--\r\n";
    let (status, body) = send(
        &app,
        Request::post(format!("/api/sessions/{id}/submit"))
            .header("content-type", "multipart/form-data; boundary=b")
            .body(Body::from(mp))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        String::from_utf8_lossy(&body).contains("invalid verdict"),
        "{}",
        String::from_utf8_lossy(&body)
    );

    // A body with no boundary at all cannot be read, and must not be a 500.
    let (status, _) = send(
        &app,
        Request::post(format!("/api/sessions/{id}/submit"))
            .header("content-type", "multipart/form-data")
            .body(Body::from("garbage"))
            .unwrap(),
    )
    .await;
    assert!(status.is_client_error(), "got {status}");
}

#[tokio::test]
async fn an_uploaded_annotation_is_stored_and_served_back_byte_for_byte() {
    // The tablet's bitmap is the reviewer's actual handwriting: if it lands truncated or
    // is not served, the reader loses the note even though the submit said OK.
    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let id = create(&app, "# Review").await;

    let png: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDRfake-bytes";
    let mut mp = Vec::new();
    mp.extend_from_slice(b"--b\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nCHANGES: see the note\r\n");
    mp.extend_from_slice(
        b"--b\r\nContent-Disposition: form-data; name=\"annotation\"; filename=\"ink.png\"\r\nContent-Type: image/png\r\n\r\n",
    );
    mp.extend_from_slice(png);
    mp.extend_from_slice(b"\r\n--b--\r\n");

    let (status, _) = send(
        &app,
        Request::post(format!("/api/sessions/{id}/submit"))
            .header("content-type", "multipart/form-data; boundary=b")
            .body(Body::from(mp))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (_, result) = get_json(&app, &format!("/api/sessions/{id}/result")).await;
    assert_eq!(result["has_annotation"], true);
    let images = result["annotation_images"].as_array().unwrap();
    assert_eq!(images.len(), 1, "{result}");
    let path = images[0].as_str().unwrap();

    // The path is a real file on the server, which is where a client on this machine
    // reads it.
    let stored = std::fs::read(path).expect("the image was written");
    assert_eq!(stored.as_slice(), png, "the bytes landed changed");

    // A remote consumer -- a webhook receiver, a browser, an agent on another host --
    // cannot open that path, so the same images are also addressable.
    let urls = result["annotation_urls"].as_array().unwrap();
    assert_eq!(urls.len(), 1, "one image, one url: {result}");
    let url = urls[0].as_str().unwrap();
    assert_eq!(url, format!("/api/sessions/{id}/annotation/0"));

    let resp = app
        .clone()
        .oneshot(Request::get(url).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
    let served = resp.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        served.as_ref(),
        png,
        "the served bytes differ from the upload"
    );

    // Nothing outside the list is reachable, and the id is part of the address.
    for missing in [
        format!("/api/sessions/{id}/annotation/1"),
        format!("/api/sessions/{id}/annotation/999"),
        "/api/sessions/ghost/annotation/0".to_string(),
    ] {
        let (status, _) = send(&app, Request::get(&missing).body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    }
}

#[tokio::test]
async fn server_side_ocr_is_optional_and_says_so() {
    // The tablet can ask the server to read its handwriting. Where no engine is
    // configured the answer must be a clear "not available" rather than an empty
    // transcription, which would look like the reader wrote nothing.
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::with_config(dir.path().to_path_buf(), 5);
    state.ocr_engine = None;
    let app = build_app(state);

    let (status, body) = send(
        &app,
        Request::post("/api/ocr")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"strokes": [[[1.0, 2.0], [3.0, 4.0]]]}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(String::from_utf8_lossy(&body).contains("not available"));
}

#[tokio::test]
async fn server_side_ocr_of_an_empty_canvas_is_an_empty_answer() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::with_config(dir.path().to_path_buf(), 5);
    state.ocr_engine = Some(std::sync::Arc::new(
        eink_bridge::ocr::OcrEngine::new().expect("an HTTP client can be built"),
    ));
    let app = build_app(state);

    let (status, body) = send(
        &app,
        Request::post("/api/ocr")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"strokes": [], "pressures": []}"#))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let reply: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(reply["text"], "");
}

#[tokio::test]
async fn an_annotation_index_cannot_escape_the_session_directory() {
    // The recorded paths are written by the server, so a value that escapes means the
    // session file was edited by hand -- and the index is client-supplied.
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions").join("abc");
    let annotations = session_dir.join("annotations");
    std::fs::create_dir_all(&annotations).unwrap();
    std::fs::write(annotations.join("img_1.png"), b"\x89PNG-in").unwrap();
    let outside = dir.path().join("secret.png");
    std::fs::write(&outside, b"secret").unwrap();
    // A symlink that points out of the directory after resolving.
    std::os::unix::fs::symlink(&outside, annotations.join("link.png")).unwrap();

    assert!(
        eink_bridge::app::resolve_annotation_path(
            &session_dir,
            annotations.join("img_1.png").to_str().unwrap()
        )
        .is_some()
    );

    for escape in [
        outside.to_str().unwrap(),
        outside.to_str().unwrap(),
        annotations.join("link.png").to_str().unwrap(),
        "/etc/passwd",
        "../secret.png",
        "annotations/../../secret.png",
    ] {
        assert!(
            eink_bridge::app::resolve_annotation_path(&session_dir, escape).is_none(),
            "{escape} should be refused"
        );
    }
}

#[tokio::test]
async fn the_webhook_metric_counts_what_actually_happened() {
    // The runbook sends the reader to eink_webhook_total when a callback does not arrive.
    // A push is fire-and-forget, so the counter is the only evidence either way.
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    eink_bridge::metrics::init();

    let hits = Arc::new(AtomicUsize::new(0));
    let counted = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let app = axum::Router::new().route(
            "/hook",
            axum::routing::post(move || {
                let counted = counted.clone();
                async move {
                    counted.fetch_add(1, Ordering::SeqCst);
                    StatusCode::OK
                }
            }),
        );
        let _ = axum::serve(listener, app).await;
    });

    let dir = tempfile::tempdir().unwrap();
    let app = test_app(dir.path().to_path_buf());
    let listening = format!(
        "/api/sessions?callback_url={}",
        urlencoding::encode(&format!("http://127.0.0.1:{port}/hook"))
    );
    let resp = app
        .clone()
        .oneshot(Request::post(listening).body(Body::from("# Hook")).unwrap())
        .await
        .unwrap();
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    let mp = "--b\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nLGTM\r\n--b--\r\n";
    send(
        &app,
        Request::post(format!("/api/sessions/{id}/submit"))
            .header("content-type", "multipart/form-data; boundary=b")
            .body(Body::from(mp))
            .unwrap(),
    )
    .await;

    // The push is spawned, so wait for the counter rather than for a sleep.
    let mut counted_metric = false;
    for _ in 0..40 {
        let (_, metrics) = send(&app, Request::get("/metrics").body(Body::empty()).unwrap()).await;
        let text = String::from_utf8_lossy(&metrics);
        if text.contains("eink_webhook_total{result=\"ok\"} 1") {
            counted_metric = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        counted_metric,
        "a delivered webhook must be counted; the receiver saw {}",
        hits.load(Ordering::SeqCst)
    );
    assert_eq!(hits.load(Ordering::SeqCst), 1, "exactly one delivery");

    // And a callback that nobody answers is counted as a failure, not silently dropped.
    let dead = {
        let probe = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        port
    };
    let dead_url = format!(
        "/api/sessions?callback_url={}",
        urlencoding::encode(&format!("http://127.0.0.1:{dead}/hook"))
    );
    let resp = app
        .clone()
        .oneshot(Request::post(dead_url).body(Body::from("# Hook")).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Cancelling also fires the callback.
    send(
        &app,
        Request::delete(format!("/api/sessions/{id}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let mut counted_failure = false;
    for _ in 0..40 {
        let (_, metrics) = send(&app, Request::get("/metrics").body(Body::empty()).unwrap()).await;
        if String::from_utf8_lossy(&metrics).contains("eink_webhook_total{result=\"error\"} 1") {
            counted_failure = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        counted_failure,
        "an undeliverable webhook must be counted as an error"
    );
}

#[tokio::test]
async fn health_says_what_is_actually_wrong() {
    // "ok" only means the process is up. The three things that actually fail are the OCR
    // engine, the disk, and the shape of the session store.
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::with_config(dir.path().to_path_buf(), 5);
    state.ocr_engine = None;
    state.ws_heartbeat_seconds = 7;
    let app = build_app(state);
    let _first = create(&app, "# One").await;

    let (status, health) = get_json(&app, "/api/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(health["status"], "ok");
    assert!(health["exe"].as_str().is_some_and(|e| !e.is_empty()));
    let ready = &health["ready"];
    assert_eq!(ready["ocr"], "unavailable", "and it should say so");
    assert_eq!(ready["state_dir_writable"], true);
    assert_eq!(ready["sessions"], 1);
    assert_eq!(ready["active"], 1);
    assert_eq!(ready["ws_heartbeat_seconds"], 7);
    assert!(
        ready["version"].as_str().is_some_and(|v| !v.is_empty()),
        "the running version is the first thing wanted in a bug report"
    );

    // An engine that is configured reports as available.
    let mut state = AppState::with_config(dir.path().to_path_buf(), 5);
    state.ocr_engine = Some(std::sync::Arc::new(
        eink_bridge::ocr::OcrEngine::new().expect("a client can be built"),
    ));
    let app = build_app(state);
    let (_, health) = get_json(&app, "/api/health").await;
    assert_eq!(health["ready"]["ocr"], "available");

    // A state directory that cannot be written is visible before it costs a review.
    // A path that is a file, not a directory: its mode looks writable and nothing can be
    // created inside it.
    let blocked = dir.path().join("blocked");
    std::fs::write(&blocked, b"in the way").unwrap();
    let app = build_app(AppState::with_config(blocked, 5));
    let (_, health) = get_json(&app, "/api/health").await;
    assert_eq!(
        health["ready"]["state_dir_writable"], false,
        "an unwritable state directory must be visible: {health}"
    );
}
