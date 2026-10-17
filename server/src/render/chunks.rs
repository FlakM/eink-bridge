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
        if let Some((kind, run)) = parse_fence_start(line) {
            let mut source_lines = Vec::new();
            let mut end_idx = idx + 1;
            while end_idx < lines.len() && !closes_fence(lines[end_idx], run) {
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

/// The length of the backtick run a line opens or closes a fence with, or 0 for a line
/// that is not a fence.
fn backtick_run(line: &str) -> usize {
    let trimmed = line.trim();
    trimmed.bytes().take_while(|b| *b == b'`').count()
}

/// A closing fence carries a run at least as long as the opening one and nothing else,
/// which is what lets a snippet quote a fence of its own -- an outer fence longer than
/// anything inside it, holding a shorter fence as ordinary text. Without this the inner
/// fence closes the block early and the snippet degrades to a plain code block whose
/// first line is the raw header: a silent loss, and a markdown review quotes fences
/// constantly.
fn closes_fence(line: &str, opening_run: usize) -> bool {
    let run = backtick_run(line);
    run >= opening_run && line.trim()[run..].trim().is_empty()
}

/// The kind a fence opens, with the length of its backtick run. Only an opening fence of
/// three or more backticks counts; a longer run lets the block contain shorter fences.
pub(crate) fn parse_fence_start(line: &str) -> Option<(&str, usize)> {
    let run = backtick_run(line);
    if run < 3 {
        return None;
    }

    let kind = match line.trim()[run..].trim() {
        "mermaid" => "mermaid",
        "mindmap" => "mindmap",
        "graph" => "graph",
        "diff" => "diff",
        // Presentation primitives: a captioned code snippet, and a sequence the reader
        // can step through by tapping (e-ink cannot animate, so a tap-through reveal is
        // the honest form of movement).
        "snippet" => "snippet",
        "steps" => "steps",
        _ => return None,
    };
    Some((kind, run))
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
