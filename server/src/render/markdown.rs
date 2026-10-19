use ammonia::Builder;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::LazyLock;
use syntect::highlighting::ThemeSet;
use syntect::html::highlighted_html_for_string;
use syntect::parsing::SyntaxSet;

use super::html_utils::escape_html;

static SYNTAX_SET: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);
static THEME: LazyLock<syntect::highlighting::Theme> = LazyLock::new(|| {
    const EINK_THEME: &str = include_str!("../../assets/eink-theme.tmTheme");
    ThemeSet::load_from_reader(&mut Cursor::new(EINK_THEME.as_bytes()))
        .expect("eink-theme.tmTheme is bundled and must parse")
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

/// Raw HTML in a brief reaches the tablet on the same origin as the API, so it is
/// sanitised before it is written. pulldown-cmark emits an opening tag, its text and the
/// closing tag as separate events, which is why the whole batch between two code blocks
/// is cleaned as one fragment rather than tag by tag. A batch without raw HTML is written
/// untouched, so a brief that uses none renders byte for byte as before.
fn flush_events<'a>(events: &mut Vec<Event<'a>>, output: &mut String) {
    if events.is_empty() {
        return;
    }
    if events.iter().any(is_untrusted) {
        let mut rendered = String::new();
        html::push_html(&mut rendered, events.drain(..));
        output.push_str(&SANITIZER.clean(&rendered).to_string());
    } else {
        html::push_html(output, events.drain(..));
    }
}

fn is_untrusted(event: &Event<'_>) -> bool {
    match event {
        Event::Html(_) | Event::InlineHtml(_) => true,
        Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
            dest_url.split_once(':').is_some_and(|(scheme, _)| {
                !scheme.contains('/') && !matches!(scheme, "http" | "https" | "mailto")
            })
        }
        _ => false,
    }
}

/// What a brief may say in raw HTML: the tags markdown itself produces, plus the inline
/// touches a review uses (`<span style="color: red">`, `<mark>`, `<kbd>`, `<sup>`,
/// `<sub>`, `<details>`/`<summary>`, `<br>`). Everything else, scripts and event
/// handlers first, is dropped.
static SANITIZER: LazyLock<Builder<'static>> = LazyLock::new(|| {
    let mut builder = Builder::empty();
    builder
        .tags(HashSet::from([
            "a",
            "blockquote",
            "br",
            "code",
            "dd",
            "del",
            "details",
            "div",
            "dl",
            "dt",
            "em",
            "h1",
            "h2",
            "h3",
            "h4",
            "h5",
            "h6",
            "hr",
            "img",
            "input",
            "kbd",
            "li",
            "mark",
            "ol",
            "p",
            "pre",
            "span",
            "strong",
            "sub",
            "summary",
            "sup",
            "table",
            "tbody",
            "td",
            "th",
            "thead",
            "tr",
            "ul",
        ]))
        .generic_attributes(HashSet::from(["class", "id", "style", "title"]))
        .tag_attributes(HashMap::from([
            ("a", HashSet::from(["href"])),
            ("img", HashSet::from(["src", "alt"])),
            ("ol", HashSet::from(["start"])),
            ("input", HashSet::from(["type", "checked", "disabled"])),
            ("details", HashSet::from(["open"])),
        ]))
        .tag_attribute_values(HashMap::from([(
            "input",
            HashMap::from([("type", HashSet::from(["checkbox"]))]),
        )]))
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .link_rel(None)
        .attribute_filter(|_, attribute, value| {
            (attribute != "style" || is_safe_style(value)).then_some(value.into())
        });
    builder
});

/// Only colour and the table alignment markdown emits; a value that could carry a URL or
/// an expression never gets through.
fn is_safe_style(style: &str) -> bool {
    style
        .split(';')
        .map(str::trim)
        .filter(|declaration| !declaration.is_empty())
        .all(|declaration| {
            declaration
                .split_once(':')
                .is_some_and(|(property, value)| {
                    matches!(property.trim(), "color" | "background-color" | "text-align")
                        && !value.trim().is_empty()
                        && value
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || " #,.%()-".contains(c))
                })
        })
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
