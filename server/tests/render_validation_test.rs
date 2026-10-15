mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn browser_review_route_renders_panel_for_active_session() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let create_resp = app
        .clone()
        .oneshot(
            Request::post("/api/sessions")
                .body(Body::from("# Title\n\nFirst paragraph.\n"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let body: serde_json::Value =
        serde_json::from_slice(&create_resp.into_body().collect().await.unwrap().to_bytes())
            .unwrap();
    let id = body["id"].as_str().unwrap().to_string();

    let resp = app
        .clone()
        .oneshot(
            Request::get(format!("/review/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(html.contains(&format!(r#"data-session-id="{id}""#)));
    assert!(html.contains(r#"<aside id="review-panel""#));
    assert!(html.contains(r#"id="comment-input""#));
    assert!(html.contains(r#"id="submit-review""#));
    assert!(html.contains("First paragraph."));

    let missing = app
        .oneshot(
            Request::get("/review/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn supported_syntax_blocks_render_expected_containers() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let cases = [
        (
            "mermaid",
            r#"```mermaid
flowchart LR
  A[Start] --> B[Done]
```"#,
            "class=\"diagram-block\" data-kind=\"mermaid\"",
        ),
        (
            "mindmap",
            r#"```mindmap
root: Review
nodes:
  - id: parser
    label: Parser
```"#,
            "class=\"diagram-block\" data-kind=\"mindmap\"",
        ),
        (
            "graph",
            r#"```graph
nodes:
  - id: cli
    label: CLI
  - id: server
    label: Server
edges:
  - from: cli
    to: server
```"#,
            "class=\"diagram-block\" data-kind=\"graph\"",
        ),
    ];

    for (_, markdown, needle) in cases {
        let html = common::render_html(app.clone(), markdown).await;
        assert!(html.contains(needle));
        assert!(html.contains("/assets/diagram/mermaid.min.js"));
        assert!(html.contains("/assets/diagram/elk.bundled.js"));
    }
}

#[tokio::test]
async fn invalid_structured_syntax_shows_parse_errors() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let cases = [
        (
            "mindmap",
            r#"```mindmap
nodes:
  - id: broken
```"#,
        ),
        (
            "graph",
            r#"```graph
nodes:
  - label: missing-id
```"#,
        ),
    ];

    for (_, markdown) in cases {
        let html = common::render_html(app.clone(), markdown).await;
        assert!(html.contains("Parse Error"));
        assert!(html.contains("diagram-error"));
        assert!(html.contains("diagram-source"));
    }
}

#[tokio::test]
async fn unsupported_fenced_block_stays_plain_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());

    let html = common::render_html(
        app,
        r#"```python
print('still a code block')
```"#,
    )
    .await;

    assert!(html.contains("print") && html.contains("<pre"));
    assert!(!html.contains("class=\"diagram-block\""));
}

#[tokio::test]
async fn render_features_showcase_document_contains_all_feature_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let app = common::test_app(dir.path().to_path_buf());
    let markdown = r#"# Showcase

```mermaid
flowchart LR
  A[Start] --> B[Done]
```

```mindmap
root: Plan
nodes:
  - label: Step 1
```

```graph
nodes:
  - id: a
    label: A
edges: []
```

```text
This block should remain a normal fenced code block.
```
"#;

    let html = common::render_html(app, markdown).await;

    assert!(html.contains("class=\"diagram-block\" data-kind=\"mermaid\""));
    assert!(html.contains("class=\"diagram-block\" data-kind=\"mindmap\""));
    assert!(html.contains("class=\"diagram-block\" data-kind=\"graph\""));
    assert!(html.contains("This block should remain a normal fenced code block."));
    assert!(html.contains("/assets/diagram/mermaid.min.js"));
    assert!(html.contains("/assets/diagram/elk.bundled.js"));
}
