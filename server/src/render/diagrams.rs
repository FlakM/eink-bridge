use serde_json::{Value, json};

use super::html_utils::escape_html;
use super::models::{DiagramPayload, GraphDoc, MindMapDoc};

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
    if kind == "diff" {
        return render_diff(source);
    }
    let data = match kind {
        "mindmap" => match serde_yaml::from_str::<MindMapDoc>(source) {
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
        _ => "Diagram",
    }
}
