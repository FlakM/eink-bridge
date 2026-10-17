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
    // A fence longer than any run inside the body, so code that quotes a fence stays
    // inside the block instead of opening a nested one (which broke the markup and
    // mangled the line marks). Mirrors how `parse_chunks` reads a fence run.
    let fence = "`".repeat(longest_backtick_run(&body).max(3) + usize::from(body.contains('`')));
    let fenced = format!("{fence}{language}\n{body}\n{fence}");
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
    let (rest, hits) = split_line_marks(header.trim());
    let (head, caption) = match rest.split_once('|') {
        Some((head, caption)) => (head.trim(), caption.trim()),
        None => (rest.trim(), ""),
    };
    // The first token is a language only if it could be one. A file path, a number, or
    // anything with a `/`, `:`, `.` or `#` is naming a location, and reading it as a
    // language silently drops the reference from the caption -- the reader loses which
    // file the snippet came from.
    let (language, reference) = match head.split_once(' ') {
        Some((first, rest)) if looks_like_language(first) => (first.trim(), rest.trim()),
        _ => ("", head),
    };
    (
        language.to_string(),
        reference.to_string(),
        caption.to_string(),
        hits,
    )
}

fn looks_like_language(token: &str) -> bool {
    !token.is_empty()
        && !token.contains(['/', '\\', ':', '.', '#'])
        && token
            .chars()
            .all(|c| c.is_alphanumeric() || "+-_".contains(c))
}

/// Line marks are the trailing `#L12,15` token, as documented. Taking the first `#L` in
/// the header instead truncates a caption that mentions a line ("fixes #L42 regression")
/// and turns that number into a highlight on the wrong line.
fn split_line_marks(header: &str) -> (&str, Vec<usize>) {
    let Some(last) = header.split_whitespace().last() else {
        return (header, Vec::new());
    };
    let Some(spec) = last.strip_prefix("#L") else {
        return (header, Vec::new());
    };
    let hits: Vec<usize> = spec
        .split(',')
        .filter_map(|part| part.trim().split('-').next()?.parse::<usize>().ok())
        .collect();
    if hits.is_empty() {
        // `#Labc` is not a mark; leave the header intact rather than eat the text.
        return (header, Vec::new());
    }
    let cut = header.trim_end().len() - last.len();
    (header[..cut].trim_end(), hits)
}

fn longest_backtick_run(text: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

/// Wrap the requested source lines, counting inside the rendered `<pre>` so the marker
/// lands on the line the reader is being pointed at.
///
/// The rendered HTML splits source lines across open `<span>`s (a trailing `;\n` lives
/// inside the span it ends), so a line cannot simply be wrapped: the mark would close
/// before the span does and the markup would come out unbalanced. Each line is rebuilt
/// self-contained -- the spans open at its start are reopened and closed at its end -- so
/// a marked line is always a well-formed fragment carrying the colours it had.
fn mark_lines(html: &str, hits: &[usize]) -> String {
    if hits.is_empty() || html.matches("<pre").count() != 1 {
        return html.to_string();
    }
    let Some(open_end) = html.find('>') else {
        return html.to_string();
    };
    let Some(close_start) = html.rfind("</pre>") else {
        return html.to_string();
    };
    let prefix = &html[..open_end + 1];
    // The renderer always emits a newline straight after the `<pre>`; it separates the
    // tag from the code and is not a line, but counting it shifted every mark by one.
    let inner = html[open_end + 1..close_start]
        .strip_prefix('\n')
        .unwrap_or("");
    let suffix = &html[close_start..];

    let mut lines: Vec<String> = Vec::new();
    let mut open: Vec<String> = Vec::new();
    let mut line = String::new();
    // Whether anything but tags has been seen since the last line break; the scaffolding
    // left after the final newline is not a line either.
    let mut pending = false;
    let mut rest = inner;
    while !rest.is_empty() {
        let Some(start) = rest.find('<') else {
            line.push_str(rest);
            pending = true;
            rest = "";
            flush_lines(&mut line, &mut lines, &open, &mut pending);
            continue;
        };
        if start > 0 {
            // Append the text and resolve its newlines *before* reading the next tag, or
            // a tag that opens at the end of a line is treated as already open when the
            // newline is flushed and a stray closing tag is emitted.
            line.push_str(&rest[..start]);
            pending = true;
            rest = &rest[start..];
            flush_lines(&mut line, &mut lines, &open, &mut pending);
            continue;
        }
        let end = rest.find('>').map(|end| end + 1).unwrap_or(rest.len());
        let tag = &rest[..end];
        rest = &rest[end..];
        if tag.starts_with("</") {
            open.pop();
        } else if !tag.ends_with("/>") {
            open.push(tag.to_string());
        }
        line.push_str(tag);
        flush_lines(&mut line, &mut lines, &open, &mut pending);
    }
    if pending {
        lines.push(line);
    }

    let marked: Vec<String> = lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if hits.contains(&(index + 1)) {
                format!("<mark class=\"snippet-hit\">{line}</mark>")
            } else {
                line
            }
        })
        .collect();
    format!("{prefix}{}{suffix}", marked.join("\n"))
}

/// Close the current line, reopening the spans that continue onto the next one so every
/// line is a well-formed fragment that keeps the colours it had.
fn flush_lines(line: &mut String, lines: &mut Vec<String>, open: &[String], pending: &mut bool) {
    while let Some(newline) = line.find('\n') {
        let (head, tail) = line.split_at(newline);
        let tail = tail[1..].to_string();
        let mut complete = head.to_string();
        complete.push_str(&closing_tags(open));
        lines.push(complete);
        *pending = tail.chars().any(|c| c != '<' && c != '>');
        *line = format!("{}{}", open.join(""), tail);
    }
}

fn closing_tags(open: &[String]) -> String {
    open.iter()
        .map(|tag| {
            let name = tag
                .trim_start_matches('<')
                .split(|c: char| c.is_whitespace() || c == '>')
                .next()
                .unwrap_or("span");
            format!("</{name}>")
        })
        .collect()
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
    let noun = if steps.len() == 1 { "step" } else { "steps" };
    format!(
        r#"<section class="steps-block" data-kind="steps">
<div class="diagram-header"><span>{label}</span><span>{count} {noun}</span></div>
<ol class="steps-list">{items}</ol>
<div class="steps-controls"><button type="button" data-role="step-next">Next step</button><button type="button" data-role="step-reset">Start over</button></div>
<script type="application/json" class="diagram-payload">{payload}</script>
</section>"#,
        label = diagram_label("steps"),
        count = steps.len(),
        noun = noun,
        items = items.join(""),
        payload = payload
    )
}
