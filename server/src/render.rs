#[path = "render/browser.rs"]
mod browser;
#[path = "render/chunks.rs"]
mod chunks;
#[path = "render/diagrams.rs"]
mod diagrams;
#[path = "render/html_utils.rs"]
mod html_utils;
#[path = "render/markdown.rs"]
mod markdown;
#[path = "render/models.rs"]
mod models;

pub use browser::to_browser_html;
use chunks::{parse_chunks, render_chunk};

#[cfg(test)]
use chunks::{Chunk, parse_fence_start};
#[cfg(test)]
use diagrams::{diagram_label, render_diagram};
#[cfg(test)]
use html_utils::escape_html;

const EINK_CSS: &str = include_str!("../assets/eink.css");
const TOC_JS: &str = include_str!("../assets/toc.js");
const BOOTSTRAP_JS: &str = include_str!("../assets/bootstrap.js");
const CODEREF_JS: &str = include_str!("../assets/coderef.js");
const ELEMENT_MAP_JS: &str = include_str!("../assets/element-map.js");

/// Turn `path:line` in prose into a tap target.
///
/// The Marks list is `- A -> path:line`, and that is exactly where the reader wants to
/// look. On a tablet that should not mean opening an editor: tapping fetches a few lines
/// around the reference and shows them in place, in the same paper-white code block every
/// other snippet uses, so a short read costs a tap instead of a context switch.
///
/// Nothing inside a code block is touched -- those lines are already on the page.
fn link_code_references(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    let mut in_code = false;
    while let Some(start) = rest.find('<') {
        let text = &rest[..start];
        out.push_str(&link_refs_in_text(text, in_code));
        let end = rest[start..]
            .find('>')
            .map(|end| start + end + 1)
            .unwrap_or(rest.len());
        let tag = &rest[start..end];
        out.push_str(tag);
        let name = tag
            .trim_start_matches('<')
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '>')
            .next()
            .unwrap_or("");
        if matches!(name, "pre" | "code") {
            in_code = !tag.starts_with("</");
        }
        rest = &rest[end..];
    }
    out.push_str(&link_refs_in_text(rest, in_code));
    out
}

/// Point a document's own image references at the session that owns them.
///
/// A brief says `![caption](image/shot.png)`; the server stores that file under the
/// session's `images/` directory and serves it from `/api/sessions/{id}/image/{name}`.
/// Resolving here rather than in the markdown keeps the content portable: the same brief
/// can be pushed as a new session and still show its pictures, and an update round does
/// not have to know the session id. A reference that is not a bare `image/<name>` (an
/// absolute URL, a data URI) is left alone.
fn resolve_session_images(html: &str, session_id: &str) -> String {
    if !html.contains("src=\"image/") && !html.contains("src='image/") {
        return html.to_string();
    }
    let prefix = format!("/api/sessions/{session_id}/image/");
    html.replace("src=\"image/", &format!("src=\"{prefix}"))
        .replace("src='image/", &format!("src='{prefix}"))
}

fn link_refs_in_text(text: &str, in_code: bool) -> String {
    if in_code || text.is_empty() {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    for character in text.chars() {
        if character.is_ascii_alphanumeric() || "/._-:".contains(character) {
            token.push(character);
            continue;
        }
        out.push_str(&linked_or_plain(&token));
        token.clear();
        out.push(character);
    }
    out.push_str(&linked_or_plain(&token));
    out
}

fn linked_or_plain(token: &str) -> String {
    match split_reference(token) {
        Some(_) => format!(
            "<button class=\"coderef\" data-ref=\"{}\" type=\"button\">{}</button>",
            html_utils::escape_html(token),
            html_utils::escape_html(token)
        ),
        None => token.to_string(),
    }
}

/// Split `path/file.rs:170` into its parts, or nothing if it does not look like one.
fn split_reference(token: &str) -> Option<(&str, u32)> {
    let (path, line) = token.rsplit_once(':')?;
    if path.len() < 3 || path.contains(':') {
        return None;
    }
    let line: u32 = line.parse().ok().filter(|line| *line > 0)?;
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    const SOURCE: &[&str] = &[
        "rs", "kt", "java", "ts", "tsx", "js", "jsx", "py", "go", "rb", "c", "h", "cpp", "hpp",
        "cs", "swift", "php", "scala", "sh", "sql", "toml", "yaml", "yml", "json", "md", "nix",
        "css", "html", "xml", "proto", "gradle",
    ];
    SOURCE.contains(&extension.as_str()).then_some((path, line))
}

/// Render a window of a source file as a snippet block, the way the rest of the review
/// renders code.
///
/// Used by the tap-a-reference expansion: it has to look exactly like every other code
/// block on the page -- highlighted with the same palette, paper white, wrapping, with the
/// cited line marked -- or the reader is looking at two different kinds of code.
pub fn source_block(
    language: &str,
    reference: &str,
    lines: &[&str],
    cited: Option<usize>,
) -> String {
    let mut header = String::new();
    if !language.is_empty() {
        header.push_str(language);
        header.push(' ');
    }
    header.push_str(reference);
    if let Some(cited) = cited {
        header.push_str(&format!(" #L{cited}"));
    }
    let source = format!("{header}\n{}", lines.join("\n"));
    diagrams::render_snippet(&source)
}

pub fn to_eink_html(markdown: &str, session_id: &str) -> String {
    let content = parse_chunks(markdown)
        .into_iter()
        .map(render_chunk)
        .collect::<Vec<_>>()
        .join("");
    let content = link_code_references(&content);
    let content = resolve_session_images(&content, session_id);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0, minimum-scale=0.3, maximum-scale=3.0, user-scalable=yes">
<title>E-Ink Review</title>
<style>{css}</style>
</head>
<body data-session-id="{session_id}">
<div id="content"><button id="toc-toggle">&#9776; Contents</button><nav id="toc" class="collapsed"></nav>{content}</div>
<script src="/assets/diagram/mermaid.min.js"></script>
<script src="/assets/diagram/elk.bundled.js"></script>
<script>{bootstrap_js}</script>
<script>{toc_js}</script>
<script>{element_map_js}</script>
<script>{coderef_js}</script>
</body>
</html>"#,
        css = EINK_CSS,
        session_id = session_id,
        content = content,
        bootstrap_js = BOOTSTRAP_JS,
        toc_js = TOC_JS,
        element_map_js = ELEMENT_MAP_JS,
        coderef_js = CODEREF_JS,
    )
}

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;
