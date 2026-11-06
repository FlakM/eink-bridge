use super::*;

/// The lines around a `path:line` reference in a review, so the reader can look at the code
/// on the tablet instead of opening an editor.
///
/// The file is read from the checkout the server recorded for the session -- the worktree
/// a CLI on this machine pushed from -- and only from there: the path is joined to it,
/// canonicalised, and required to stay inside, so a session document cannot turn into a
/// way to read the machine. `origin.cwd` is never the root: any client on the LAN can
/// write that field.
#[derive(Deserialize)]
pub(super) struct SourceParams {
    #[serde(rename = "ref")]
    reference: String,
    #[serde(default)]
    context: Option<usize>,
}

pub(super) const SOURCE_CONTEXT_MAX: usize = 20;

pub(super) async fn source_snippet(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<SourceParams>,
) -> Response {
    let root = {
        let mgr = state.sessions.read().await;
        match mgr.get(&id) {
            None => return StatusCode::NOT_FOUND.into_response(),
            Some(session) => match &session.source_root {
                Some(root) => root.clone(),
                // Without a recorded checkout there is nothing to read from, and guessing
                // would be worse than saying so.
                None => {
                    return (
                        StatusCode::NOT_FOUND,
                        "this review did not record a checkout the server may read",
                    )
                        .into_response();
                }
            },
        }
    };

    let Some((relative, wanted)) = split_source_reference(&params.reference) else {
        return (
            StatusCode::BAD_REQUEST,
            "expected a reference like path/file.rs:120",
        )
            .into_response();
    };
    let context = params.context.unwrap_or(6).min(SOURCE_CONTEXT_MAX);
    let Ok(root) = root.canonicalize() else {
        return (StatusCode::NOT_FOUND, "the checkout is gone").into_response();
    };
    let candidate = root.join(&relative);
    let Ok(path) = candidate.canonicalize() else {
        return (StatusCode::NOT_FOUND, "no such file in the checkout").into_response();
    };
    if !path.starts_with(&root) || !path.is_file() {
        return (StatusCode::NOT_FOUND, "outside the checkout").into_response();
    }
    let Ok(text) = tokio::fs::read_to_string(&path).await else {
        return (StatusCode::NOT_FOUND, "unreadable").into_response();
    };
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    if wanted > total {
        return (StatusCode::NOT_FOUND, format!("the file has {total} lines")).into_response();
    }
    let start = wanted.saturating_sub(context + 1);
    let end = (wanted + context).min(total);
    let window = &lines[start..end];
    // Rendered here rather than in the browser: the page has one highlighter, and it is
    // this one. The cited line is marked the same way a snippet marks it.
    let html = crate::render::source_block(
        language_for(&relative),
        &format!("{relative}:{}", start + 1),
        window,
        Some(wanted - start),
    );
    Json(serde_json::json!({
        "path": relative,
        "start": start + 1,
        "end": end,
        "total": total,
        "lines": window,
        "html": html,
    }))
    .into_response()
}

/// The highlighter's name for a file, or nothing when it does not know the language (the
/// page falls back to plain text, exactly as it does for a snippet it cannot highlight).
pub(super) fn language_for(path: &str) -> &'static str {
    match path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "rust",
        "kt" | "kts" => "kotlin",
        "ts" => "typescript",
        "tsx" => "tsx",
        "js" | "jsx" | "mjs" => "javascript",
        "py" => "python",
        "rb" => "ruby",
        "go" => "go",
        "java" => "java",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "c++",
        "cs" => "c#",
        "swift" => "swift",
        "php" => "php",
        "scala" => "scala",
        "sh" | "bash" => "bash",
        "sql" => "sql",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "toml" => "toml",
        "md" => "markdown",
        "nix" => "nix",
        "css" => "css",
        "html" | "htm" => "html",
        "xml" => "xml",
        "proto" => "protobuf",
        "gradle" => "groovy",
        _ => "",
    }
}

/// Split `path/file.rs:120` into the path and the line, refusing anything that could
/// escape the checkout before the filesystem is touched.
pub(super) fn split_source_reference(reference: &str) -> Option<(String, usize)> {
    let (relative, line) = reference.rsplit_once(':')?;
    let line: usize = line.trim().parse().ok()?;
    let relative = relative.trim();
    if relative.is_empty() || line == 0 {
        return None;
    }
    let path = std::path::Path::new(relative);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return None;
    }
    Some((relative.to_string(), line))
}
