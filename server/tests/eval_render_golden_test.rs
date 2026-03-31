mod common;

#[tokio::test]
async fn render_fixtures_match_goldens() {
    let fixture_paths = common::eval::fixture_paths("tests/fixtures/eval/render", "md");
    assert!(!fixture_paths.is_empty(), "expected render fixtures");

    for fixture in fixture_paths {
        let markdown = common::eval::read(&fixture);
        let dir = tempfile::tempdir().unwrap();
        let app = common::test_app(dir.path().to_path_buf());
        let html = common::render_html(app, &markdown).await;
        let normalized = common::eval::normalize_html(&html);
        let golden = common::eval::repo_server_dir()
            .join("tests/golden/render")
            .join(fixture.file_stem().unwrap())
            .with_extension("html");
        common::eval::assert_or_update(&golden, &normalized);
    }
}
