use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub fn repo_server_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn fixture_paths(relative_dir: &str, extension: &str) -> Vec<PathBuf> {
    let dir = repo_server_dir().join(relative_dir);
    let mut paths: Vec<_> = fs::read_dir(dir)
        .expect("fixture directory exists")
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some(extension))
        .collect();
    paths.sort();
    paths
}

pub fn read(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
}

pub fn normalize_html(html: &str) -> String {
    let html = replace_between(
        html,
        "<style>",
        "</style>",
        "<style>...styles omitted...</style>",
    );
    let html = replace_inline_scripts(&html);
    // session ID is random; normalize so goldens are stable
    regex_replace_session_id(&html)
}

fn regex_replace_session_id(html: &str) -> String {
    let mut out = String::new();
    let needle = "data-session-id=\"";
    let mut cursor = 0;
    while let Some(rel) = html[cursor..].find(needle) {
        let start = cursor + rel + needle.len();
        out.push_str(&html[cursor..start]);
        let end = html[start..]
            .find('"')
            .map(|i| start + i)
            .unwrap_or(html.len());
        out.push_str("<session-id>");
        cursor = end;
    }
    out.push_str(&html[cursor..]);
    out
}

fn replace_inline_scripts(html: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    while let Some(start_rel) = html[cursor..].find("<script") {
        let start = cursor + start_rel;
        let Some(tag_end_rel) = html[start..].find('>') else {
            out.push_str(&html[cursor..]);
            return out;
        };
        let tag_end = start + tag_end_rel + 1;
        let open_tag = &html[start..tag_end];
        let Some(close_rel) = html[tag_end..].find("</script>") else {
            out.push_str(&html[cursor..]);
            return out;
        };
        let close = tag_end + close_rel;
        out.push_str(&html[cursor..start]);
        if open_tag.contains("application/json") || open_tag.contains("src=") {
            out.push_str(&html[start..close + "</script>".len()]);
        } else {
            out.push_str(open_tag);
            out.push_str("...script omitted...");
            out.push_str("</script>");
        }
        cursor = close + "</script>".len();
    }
    out.push_str(&html[cursor..]);
    out
}

fn replace_between(text: &str, start_tag: &str, end_tag: &str, replacement: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0;
    while let Some(start_rel) = text[cursor..].find(start_tag) {
        let start = cursor + start_rel;
        let Some(end_rel) = text[start..].find(end_tag) else {
            out.push_str(&text[cursor..]);
            return out;
        };
        let end = start + end_rel + end_tag.len();
        out.push_str(&text[cursor..start]);
        out.push_str(replacement);
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

pub fn normalize_json(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(normalize_json).collect()),
        Value::Object(map) => {
            let mut normalized = serde_json::Map::new();
            for (key, value) in map {
                let normalized_value = match key.as_str() {
                    "id" => Value::String("<id>".into()),
                    "url" => normalize_url(value),
                    "created_at" | "updated_at" => Value::String("<timestamp>".into()),
                    "annotation_images" => normalize_annotation_paths(value),
                    _ => normalize_json(value),
                };
                normalized.insert(key, normalized_value);
            }
            Value::Object(normalized)
        }
        other => other,
    }
}

fn normalize_url(value: Value) -> Value {
    match value {
        Value::String(url) if url.starts_with("/session/") => Value::String("/session/<id>".into()),
        other => normalize_json(other),
    }
}

fn normalize_annotation_paths(value: Value) -> Value {
    match value {
        Value::Array(paths) => Value::Array(
            paths
                .into_iter()
                .enumerate()
                .map(|(idx, _)| Value::String(format!("<annotation-{}>", idx + 1)))
                .collect(),
        ),
        other => normalize_json(other),
    }
}

pub fn assert_or_update(path: &Path, actual: &str) {
    if std::env::var("UPDATE_GOLDENS").ok().as_deref() == Some("1") {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("golden parent directory created");
        }
        fs::write(path, actual)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        return;
    }

    let expected = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("missing golden {}: {error}", path.display()));
    assert_eq!(expected, actual, "golden mismatch for {}", path.display());
}
