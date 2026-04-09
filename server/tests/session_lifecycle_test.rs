mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn cancel_nonexistent_session_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(
            Request::delete("/api/sessions/ghost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn submit_nonexistent_session_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());
    let boundary = "----b";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nnotes\r\n--{boundary}--\r\n"
    );
    let resp = app
        .oneshot(
            Request::post("/api/sessions/ghost/submit")
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_sessions_returns_all_without_filter() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    for content in ["# One", "# Two", "# Three"] {
        app.clone()
            .oneshot(
                Request::post("/api/sessions")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();
    }

    let resp = app
        .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.len(), 3);
}

#[tokio::test]
async fn list_sessions_sorted_by_updated_at_descending() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let mut ids = vec![];
    for content in ["# First", "# Second"] {
        let resp = app
            .clone()
            .oneshot(
                Request::post("/api/sessions")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        ids.push(body["id"].as_str().unwrap().to_string());
    }

    // Cancel the first session so its updated_at advances past the second
    app.clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{}", ids[0]))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    // Most recently updated (the cancelled one) should be first
    assert_eq!(body[0]["id"].as_str().unwrap(), ids[0]);
}

#[tokio::test]
async fn get_result_for_cancelled_session_returns_410() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# x"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    app.clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

#[tokio::test]
async fn create_with_title() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .oneshot(
            Request::post("/api/sessions?title=My+Doc")
                .body(Body::from("# Test"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap();

    let app2 = common::test_app(dir.path().to_path_buf());
    let resp = app2
        .oneshot(
            Request::get(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["title"], "My Doc");
}

#[tokio::test]
async fn create_with_json_request_persists_tags() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "schema_version": 1,
                        "title": "Tagged Doc",
                        "content": "# Tagged",
                        "tags": {"type": "plan", "agent": "claude"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let resp = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let detail: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(detail["schema_version"], 1);
    assert_eq!(detail["tags"]["type"], "plan");
    assert_eq!(detail["tags"]["agent"], "claude");
}

#[tokio::test]
async fn session_persists_across_restart() {
    let dir = tempfile::tempdir().unwrap();

    // Create session with first app instance
    let app = common::test_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Persist test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    // Create new app instance (simulates restart) from same state_dir
    let app2 = common::test_app(dir.path().to_path_buf());
    let resp = app2
        .oneshot(
            Request::get(format!("/api/sessions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["status"], "Active");
}

#[tokio::test]
async fn session_expiry() {
    use eink_bridge::session::SessionManager;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let mut mgr = SessionManager::new(dir.path().to_path_buf());
    mgr.create(
        "# Will expire".into(),
        None,
        None,
        std::collections::HashMap::new(),
        false,
        None,
    );

    // Expire with zero timeout (everything is stale)
    mgr.expire_stale(Duration::ZERO);

    let sessions = mgr.list();
    assert_eq!(sessions.len(), 1);
    assert_eq!(
        sessions[0].status,
        eink_bridge::session::SessionStatus::Expired
    );
}

#[tokio::test]
async fn list_filters_by_status() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create two sessions
    let app_clone = app.clone();
    app_clone
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# One"))
                .unwrap(),
        )
        .await
        .unwrap();

    let app_clone = app.clone();
    let resp = app_clone
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Two"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id2 = body["id"].as_str().unwrap().to_string();

    // Cancel the second one
    let app_clone = app.clone();
    app_clone
        .oneshot(
            Request::delete(format!("/api/sessions/{id2}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // List only active
    let resp = app
        .oneshot(
            Request::get("/api/sessions?status=active")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body.len(), 1);
}

#[tokio::test]
async fn list_filters_by_starred_and_sorts_starred_first() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create a plain session
    app.clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Plain"))
                .unwrap(),
        )
        .await
        .unwrap();

    // Create a starred session via JSON
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(
                    r##"{"content":"# Pinned","starred":true,"origin":{"cwd":"/proj"}}"##,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let pinned_id = body["id"].as_str().unwrap().to_string();

    // Full list: starred should come first
    let resp = app
        .clone()
        .oneshot(Request::get("/api/sessions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let list: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["id"].as_str().unwrap(), pinned_id);
    assert_eq!(list[0]["starred"].as_bool(), Some(true));
    assert_eq!(list[0]["origin"]["cwd"].as_str(), Some("/proj"));

    // Filter starred=true returns only the pinned one
    let resp = app
        .clone()
        .oneshot(
            Request::get("/api/sessions?starred=true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let list: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["id"].as_str().unwrap(), pinned_id);

    // Filter starred=false returns only the plain one
    let resp = app
        .oneshot(
            Request::get("/api/sessions?starred=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let list: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["starred"].as_bool(), Some(false));
}

#[tokio::test]
async fn put_star_toggles_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Doc"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    // Star it
    let resp = app
        .clone()
        .oneshot(
            Request::put(format!("/api/sessions/{id}/star"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"starred":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["starred"].as_bool(), Some(true));

    // Unstar it
    let resp = app
        .clone()
        .oneshot(
            Request::put(format!("/api/sessions/{id}/star"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"starred":false}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Star an unknown session → 404
    let resp = app
        .oneshot(
            Request::put("/api/sessions/doesnotexist/star")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"starred":true}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn submit_with_image() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create session
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Image test"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    // Submit with multipart containing typed_notes and a fake image
    let boundary = "----boundary123";
    let multipart_body = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"typed_notes\"\r\n\r\n\
         Great work!\r\n\
         --{boundary}\r\n\
         Content-Disposition: form-data; name=\"annotation\"; filename=\"test.png\"\r\n\
         Content-Type: image/png\r\n\r\n\
         FAKEPNG\r\n\
         --{boundary}--\r\n"
    );

    let resp = app
        .clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(multipart_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify result
    let resp = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["typed_notes"], "Great work!");
    let images = body["annotation_images"].as_array().unwrap();
    assert_eq!(images.len(), 1);
    // The path must actually exist on disk
    let path = images[0].as_str().unwrap();
    assert!(
        std::path::Path::new(path).exists(),
        "annotation file not on disk: {path}"
    );
}

#[tokio::test]
async fn get_result_for_nonexistent_session_returns_404() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());
    let resp = app
        .oneshot(
            Request::get("/api/sessions/ghost/result")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn get_result_for_expired_session_returns_410() {
    use eink_bridge::app::AppState;
    use eink_bridge::app::build_app;
    use std::time::Duration;

    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(dir.path().to_path_buf());

    let id = {
        let mut mgr = state.sessions.write().await;
        let s = mgr.create(
            "# Expired".into(),
            None,
            None,
            std::collections::HashMap::new(),
            false,
            None,
        );
        mgr.expire_stale(Duration::ZERO);
        s.id
    };

    let app = build_app(state);
    let resp = app
        .oneshot(
            Request::get(format!("/api/sessions/{id}/result"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::GONE);
}

#[tokio::test]
async fn render_session_works_for_submitted_and_cancelled_states() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    // Create and submit
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Done"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let submitted_id = body["id"].as_str().unwrap().to_string();

    let boundary = "b";
    let mp = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nnotes\r\n--{boundary}--\r\n"
    );
    app.clone()
        .oneshot(
            Request::post(format!("/api/sessions/{submitted_id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(mp))
                .unwrap(),
        )
        .await
        .unwrap();

    // Create and cancel
    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Dropped"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let cancelled_id = body["id"].as_str().unwrap().to_string();
    app.clone()
        .oneshot(
            Request::delete(format!("/api/sessions/{cancelled_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Both sessions must still render HTML
    for id in [&submitted_id, &cancelled_id] {
        let resp = app
            .clone()
            .oneshot(
                Request::get(format!("/session/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "session {id} should still render"
        );
        let html = String::from_utf8(
            resp.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
        .unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
    }
}

#[tokio::test]
async fn submit_already_submitted_session_returns_409() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Double submit"))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let boundary = "b";
    let mp = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"typed_notes\"\r\n\r\nnotes\r\n--{boundary}--\r\n"
    );

    // First submit
    app.clone()
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(mp.clone()))
                .unwrap(),
        )
        .await
        .unwrap();

    // Second submit — should fail
    let resp = app
        .oneshot(
            Request::post(format!("/api/sessions/{id}/submit"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(Body::from(mp))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn list_sessions_unknown_status_returns_empty() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    app.clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# X"))
                .unwrap(),
        )
        .await
        .unwrap();

    let resp = app
        .oneshot(
            Request::get("/api/sessions?status=nonsense")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Vec<serde_json::Value> =
        serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert!(
        body.is_empty(),
        "unknown status filter should return no sessions"
    );
}
