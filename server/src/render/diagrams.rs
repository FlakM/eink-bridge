use serde_json::{Value, json};

use super::html_utils::escape_html;
use super::markdown::render_markdown;
use super::models::{DiagramPayload, GraphDoc, MindMapDoc, MindMapNode};

fn populate_notes_html(node: &mut MindMapNode) {
    if let Some(notes) = node.notes.as_deref() {
        node.notes_html = Some(render_markdown(notes));
    }
    for child in &mut node.children {
        populate_notes_html(child);
    }
}

pub(crate) fn render_diff(source: &str) -> String {
    let lines: String = source
        .lines()
        .map(|line| {
            let (class, content) = if line.starts_with("@@") {
                ("diff-hunk", line)
            } else if line.starts_with('+') {
                ("diff-add", line)
            } else if line.starts_with('-') {
                ("diff-del", line)
            } else {
                ("diff-ctx", line)
            };
            format!("<div class=\"{class}\">{}</div>", escape_html(content))
        })
        .collect();
    format!("<div class=\"diff-block\">{lines}</div>")
}

pub(crate) fn render_diagram(kind: &str, source: &str) -> String {
    if kind == "snippet" {
        return render_snippet(source);
    }
    if kind == "steps" {
        return render_steps(source);
    }
    if kind == "diff" {
        return render_diff(source);
    }
    let data = match kind {
        "mindmap" => match serde_yaml::from_str::<MindMapDoc>(source) {
            Ok(mut doc) => {
                for node in &mut doc.nodes {
                    populate_notes_html(node);
                }
                Some(serde_json::to_value(doc).unwrap_or(Value::Null))
            }
            Err(error) => {
                return format!(
                    r#"<section class="diagram-block" data-kind="{kind}">
<div class="diagram-header"><span>{label}</span><span>Parse Error</span></div>
<div class="diagram-body"><pre class="diagram-error">{error}</pre><pre class="diagram-source">{source}</pre></div>
</section>"#,
                    kind = kind,
                    label = diagram_label(kind),
                    error = escape_html(&error.to_string()),
                    source = escape_html(source),
                );
            }
        },
        "graph" => match serde_yaml::from_str::<GraphDoc>(source) {
            Ok(doc) => Some(serde_json::to_value(doc).unwrap_or(Value::Null)),
            Err(error) => {
                return format!(
                    r#"<section class="diagram-block" data-kind="{kind}">
<div class="diagram-header"><span>{label}</span><span>Parse Error</span></div>
<div class="diagram-body"><pre class="diagram-error">{error}</pre><pre class="diagram-source">{source}</pre></div>
</section>"#,
                    kind = kind,
                    label = diagram_label(kind),
                    error = escape_html(&error.to_string()),
                    source = escape_html(source),
                );
            }
        },
        _ => None,
    };

    let payload = DiagramPayload { kind, source, data };
    let payload_json = serde_json::to_string(&payload)
        .unwrap_or_else(|_| json!({ "kind": kind, "source": source }).to_string())
        .replace("</script>", "<\\/script>");

    format!(
        r#"<section class="diagram-block" data-kind="{kind}">
<div class="diagram-header"><span>{label}</span><span>{kind}</span></div>
<div class="diagram-body"><div class="diagram-placeholder">Rendering {label}...</div></div>
<script type="application/json" class="diagram-payload">{payload_json}</script>
</section>"#,
        kind = kind,
        label = diagram_label(kind),
        payload_json = payload_json,
    )
}

pub(crate) fn diagram_label(kind: &str) -> &str {
    match kind {
        "mermaid" => "Diagram",
        "mindmap" => "Mind Map",
        "graph" => "Graph",
        "snippet" => "Code",
        "steps" => "Sequence",
        _ => "Diagram",
    }
}

/// Why text and not a screenshot: this is a code review on e-ink, where the reader
/// resizes, zooms and reflows. Highlighted text stays crisp at any zoom, reflows to the
/// column, is searchable, costs bytes instead of megabytes, and needs no image decode.
/// A screenshot is the right tool only for something text cannot express — a rendered
/// UI, a layout, a chart — never for a call site.
///
/// Source format, first line: `LANG REF | CAPTION #L12,15`
/// All three parts after LANG are optional; the rest of the block is the code.
pub(crate) fn render_snippet(source: &str) -> String {
    let mut lines = source.lines();
    let header = lines.next().unwrap_or("");
    let code: Vec<&str> = lines.collect();
    let (language, reference, caption, hits) = parse_snippet_header(header);

    let body = code.join("\n");
    let fenced = format!("```{language}\n{body}\n```");
    let highlighted = super::markdown::render_markdown(&fenced);
    let highlighted = mark_lines(&highlighted, &hits);

    let title = match (reference.is_empty(), caption.is_empty()) {
        (true, true) => String::new(),
        (false, true) => reference.clone(),
        (true, false) => caption.clone(),
        (false, false) => format!("{reference} — {caption}"),
    };
    let figcaption = if title.is_empty() {
        String::new()
    } else {
        format!(
            "<figcaption>{}</figcaption>",
            super::html_utils::escape_html(&title)
        )
    };
    let language_attr = if language.is_empty() {
        String::new()
    } else {
        format!(
            " data-lang=\"{}\"",
            super::html_utils::escape_html(&language)
        )
    };

    format!("<figure class=\"snippet-block\"{language_attr}>{figcaption}{highlighted}</figure>")
}

fn parse_snippet_header(header: &str) -> (String, String, String, Vec<usize>) {
    let mut rest = header.trim().to_string();
    let mut hits = Vec::new();
    if let Some(index) = rest.find("#L") {
        let tail = rest[index + 2..].to_string();
        rest = rest[..index].trim_end().to_string();
        hits = tail
            .split(',')
            .filter_map(|part| part.trim().split('-').next()?.parse::<usize>().ok())
            .collect();
    }
    let (head, caption) = match rest.split_once('|') {
        Some((head, caption)) => (head.trim(), caption.trim()),
        None => (rest.trim(), ""),
    };
    let (language, reference) = match head.split_once(' ') {
        Some((language, reference)) => (language.trim(), reference.trim()),
        None => ("", head),
    };
    (
        language.to_string(),
        reference.to_string(),
        caption.to_string(),
        hits,
    )
}

/// Wrap the requested source lines, counting inside the rendered `<pre>` so the marker
/// lands on the line the reader is being pointed at.
fn mark_lines(html: &str, hits: &[usize]) -> String {
    if hits.is_empty() || !html.contains("<pre") {
        return html.to_string();
    }
    let Some(open_end) = html.find('>') else {
        return html.to_string();
    };
    let Some(close_start) = html.rfind("</pre>") else {
        return html.to_string();
    };
    let prefix = &html[..open_end + 1];
    let inner = &html[open_end + 1..close_start];
    let suffix = &html[close_start..];
    let marked: Vec<String> = inner
        .split('\n')
        .enumerate()
        .map(|(index, line)| {
            if hits.contains(&(index + 1)) {
                format!("<mark class=\"snippet-hit\">{line}</mark>")
            } else {
                line.to_string()
            }
        })
        .collect();
    format!("{prefix}{}{suffix}", marked.join("\n"))
}

/// A sequence the reader steps through. Rendered as a real ordered list so it is still
/// readable when scripting is off; the script only adds the reveal.
pub(crate) fn render_steps(source: &str) -> String {
    let steps: Vec<&str> = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let items: Vec<String> = steps
        .iter()
        .map(|step| {
            // `Title | detail` gives a scannable lead and the supporting clause.
            match step.split_once('|') {
                Some((title, detail)) => format!(
                    "<li><span class=\"step-title\">{}</span><span class=\"step-detail\">{}</span></li>",
                    super::html_utils::escape_html(title.trim()),
                    super::html_utils::escape_html(detail.trim())
                ),
                None => format!(
                    "<li><span class=\"step-title\">{}</span></li>",
                    super::html_utils::escape_html(step)
                ),
            }
        })
        .collect();
    let payload = serde_json::json!({ "kind": "steps", "count": items.len() }).to_string();
    format!(
        r#"<section class="steps-block" data-kind="steps">
<div class="diagram-header"><span>{label}</span><span>{count} steps</span></div>
<ol class="steps-list">{items}</ol>
<div class="steps-controls"><button type="button" data-role="step-next">Next step</button><button type="button" data-role="step-reset">Start over</button></div>
<script type="application/json" class="diagram-payload">{payload}</script>
</section>"#,
        label = diagram_label("steps"),
        count = steps.len(),
        items = items.join(""),
        payload = payload
    )
}
