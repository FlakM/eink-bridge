use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use std::sync::LazyLock;
use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::SyntaxSet;

use super::html_utils::escape_html;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<syntect::highlighting::Theme> = LazyLock::new(|| {
    let ts = ThemeSet::load_defaults();
    ts.themes["Solarized (light)"].clone()
});

pub(crate) fn render_markdown(markdown: &str) -> String {
    let options = Options::all();
    let parser = Parser::new_ext(markdown, options);
    let mut html_output = String::new();
    let mut code_buf: Option<(String, String)> = None;
    let mut passthrough: Vec<Event<'_>> = Vec::new();

    for event in parser {
        match (&event, &mut code_buf) {
            (Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(lang))), None) => {
                flush_events(&mut passthrough, &mut html_output);
                code_buf = Some((lang.to_string(), String::new()));
            }
            (Event::Text(text), Some((_, buf))) => {
                buf.push_str(text);
            }
            (Event::End(TagEnd::CodeBlock), Some(_)) => {
                let (lang, code) = code_buf.take().expect("code block state present");
                html_output.push_str(&highlight_code(&lang, &code));
            }
            _ => passthrough.push(event),
        }
    }
    flush_events(&mut passthrough, &mut html_output);
    html_output
}

fn flush_events<'a>(events: &mut Vec<Event<'a>>, output: &mut String) {
    if !events.is_empty() {
        html::push_html(output, events.drain(..));
    }
}

fn highlight_code(lang: &str, code: &str) -> String {
    let ss = &*SYNTAX_SET;
    let syntax = ss
        .find_syntax_by_token(lang)
        .unwrap_or_else(|| ss.find_syntax_plain_text());
    match highlighted_html_for_string(code, ss, syntax, &THEME) {
        Ok(html) => html,
        Err(_) => format!("<pre><code>{}</code></pre>", escape_html(code)),
    }
}
