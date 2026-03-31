use super::diagrams::render_diagram;
use super::markdown::render_markdown;

pub(crate) enum Chunk {
    Markdown(String),
    Diagram { kind: String, source: String },
}

pub(crate) fn parse_chunks(markdown: &str) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut markdown_buffer = String::new();
    let lines = markdown.lines().collect::<Vec<_>>();
    let mut idx = 0;

    while idx < lines.len() {
        let line = lines[idx];
        if let Some(kind) = parse_fence_start(line) {
            let mut source_lines = Vec::new();
            let mut end_idx = idx + 1;
            while end_idx < lines.len() && lines[end_idx].trim() != "```" {
                source_lines.push(lines[end_idx]);
                end_idx += 1;
            }
            if end_idx < lines.len() {
                flush_markdown(&mut chunks, &mut markdown_buffer);
                chunks.push(Chunk::Diagram {
                    kind: kind.to_string(),
                    source: source_lines.join("\n"),
                });
                idx = end_idx + 1;
                continue;
            }
        }

        markdown_buffer.push_str(line);
        markdown_buffer.push('\n');
        idx += 1;
    }

    flush_markdown(&mut chunks, &mut markdown_buffer);
    chunks
}

pub(crate) fn parse_fence_start(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if !trimmed.starts_with("```") {
        return None;
    }

    match trimmed.trim_start_matches("```").trim() {
        "mermaid" => Some("mermaid"),
        "mindmap" => Some("mindmap"),
        "graph" => Some("graph"),
        "diff" => Some("diff"),
        _ => None,
    }
}

fn flush_markdown(chunks: &mut Vec<Chunk>, markdown_buffer: &mut String) {
    if !markdown_buffer.is_empty() {
        chunks.push(Chunk::Markdown(std::mem::take(markdown_buffer)));
    }
}

pub(crate) fn render_chunk(chunk: Chunk) -> String {
    match chunk {
        Chunk::Markdown(markdown) => render_markdown(&markdown),
        Chunk::Diagram { kind, source } => render_diagram(&kind, &source),
    }
}
