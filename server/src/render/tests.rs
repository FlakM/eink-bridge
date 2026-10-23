use super::*;

fn render(markdown: &str) -> String {
    to_eink_html(markdown, "test-id")
}

// --- CSS / typography ---
// CSS tests operate on EINK_CSS directly so the right rule is checked,
// not just any rule that happens to contain the same value.

fn css_rule(selector: &str) -> String {
    // Search for the selector at the start of a line to avoid false matches
    // (e.g. "h1 {" inside ".toc-h1 {")
    let needle = format!("\n{selector}");
    let start = EINK_CSS
        .find(needle.as_str())
        .unwrap_or_else(|| panic!("selector '{selector}' not found at line start in CSS"))
        + 1; // skip the leading \n
    let after = &EINK_CSS[start..];
    let open = after.find('{').unwrap();
    let close = after.find('}').unwrap();
    after[open + 1..close].to_string()
}

#[test]
fn body_font_size_is_18px() {
    assert!(
        css_rule("body {").contains("font-size: 18px"),
        "body rule must have font-size: 18px"
    );
}

#[test]
fn heading_sizes_are_correct() {
    assert!(
        css_rule("h1 {").contains("font-size: 28px"),
        "h1 must be 28px"
    );
    assert!(
        css_rule("h2 {").contains("font-size: 24px"),
        "h2 must be 24px"
    );
    assert!(
        css_rule("h3 {").contains("font-size: 20px"),
        "h3 must be 20px"
    );
}

#[test]
fn inline_code_has_strong_border() {
    assert!(
        css_rule("code {").contains("border: 1px solid #999"),
        "inline code needs a visible border"
    );
}

#[test]
fn inline_code_font_size_matches_body() {
    assert!(
        css_rule("code {").contains("font-size: 16px"),
        "inline code should be 16px"
    );
}

#[test]
fn inline_code_renders_code_tag_in_output() {
    let html = render("use `foo::bar` in your code");
    assert!(
        html.contains("<code>foo::bar</code>"),
        "inline code must render as <code> tag"
    );
}

// --- TOC ---

#[test]
fn toc_shows_for_single_heading() {
    let html = render("# Only Heading\n\nsome text");
    assert!(
        html.contains("id=\"toc-toggle\""),
        "TOC toggle should always appear"
    );
    assert!(
        html.contains("headings.length < 1"),
        "threshold should be < 1"
    );
}

#[test]
fn toc_includes_h1_links() {
    // h1 must get the toc-h1 CSS class in the TOC JS
    let html = render("# Top Level\n\nsome text");
    assert!(
        html.contains("toc-h1"),
        "TOC JS must emit toc-h1 class for h1 elements"
    );
    // The JS must not skip h1
    assert!(
        !html.contains("if (level === 'h1') return;"),
        "h1 must not be excluded from TOC"
    );
}

// --- Diagram blocks ---

#[test]
fn mermaid_block_renders_diagram_container() {
    let html = render("```mermaid\nflowchart LR\n  A --> B\n```");
    assert!(html.contains("class=\"diagram-block\" data-kind=\"mermaid\""));
    assert!(html.contains("diagram-payload"));
    assert!(html.contains("Rendering Diagram"));
}

#[test]
fn mermaid_sequence_diagram_source_survives_in_payload() {
    let source = "sequenceDiagram\n  participant A\n  A->>B: hello";
    let html = render(&format!("```mermaid\n{source}\n```"));
    assert!(html.contains("data-kind=\"mermaid\""));
    assert!(
        html.contains("sequenceDiagram"),
        "sequence diagram source must pass through to payload"
    );
}

#[test]
fn mindmap_block_renders_diagram_container() {
    let html = render("```mindmap\nroot: Plan\nnodes:\n  - label: Step\n```");
    assert!(html.contains("class=\"diagram-block\" data-kind=\"mindmap\""));
    assert!(html.contains("Rendering Mind Map"));
}

#[test]
fn graph_block_renders_diagram_container() {
    let html = render("```graph\nnodes:\n  - id: a\n    label: A\nedges: []\n```");
    assert!(html.contains("class=\"diagram-block\" data-kind=\"graph\""));
    assert!(html.contains("Rendering Graph"));
}

#[test]
fn invalid_mindmap_shows_parse_error() {
    // MindMapNode requires `label: String`; omitting it causes a serde_yaml error
    let html = render("```mindmap\nnodes:\n  - id: broken\n```");
    assert!(
        html.contains("Parse Error"),
        "missing label field must produce a parse error"
    );
    assert!(
        html.contains("diagram-error"),
        "error class must be present"
    );
    assert!(
        html.contains("diagram-source"),
        "original source must be shown"
    );
}

#[test]
fn invalid_graph_yaml_shows_parse_error() {
    let html = render("```graph\n: bad: [yaml\n```");
    assert!(html.contains("Parse Error"));
    assert!(html.contains("diagram-error"));
    assert!(html.contains("diagram-source"));
}

#[test]
fn unsupported_fenced_block_is_plain_code() {
    let html = render("```python\nprint('hello')\n```");
    assert!(html.contains("<pre"), "should render as pre block");
    assert!(
        !html.contains("class=\"diagram-block\""),
        "should not be a diagram block"
    );
}

// --- Syntax highlighting ---

/// The rendered page without its stylesheet, so an assertion is about markup rather
/// than about a selector that happens to share a name with it.
fn body_of(html: &str) -> &str {
    html.split("</style>").nth(1).unwrap_or("")
}

/// Strip tags so an assertion compares what the reader sees: syntax highlighting
/// splits a line like `let y = 2;` across several spans.
fn visible_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        match rest[start..].find('>') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.replace("&quot;", "\"").replace("&#39;", "'")
}

fn marked_lines(html: &str) -> Vec<String> {
    html.match_indices("<mark class=\"snippet-hit\">")
        .map(|(start, tag)| {
            visible_text(
                html[start + tag.len()..]
                    .split("</mark>")
                    .next()
                    .unwrap_or(""),
            )
        })
        .collect()
}

#[test]
fn a_line_mark_lands_on_the_line_it_names() {
    // The renderer emits a newline between <pre> and the code. Counting it as a line
    // put every mark one line high, so the reader was sent above the cited line.
    let html =
        render("```snippet\nrust a.rs | note #L2\nlet x = 1;\nlet y = 2;\nlet z = 3;\n```\n");
    let marked = marked_lines(&html);
    assert_eq!(marked.len(), 1, "one line cited, one marked");
    assert!(
        marked[0].contains("let y = 2;"),
        "marked the wrong line: {:?}",
        marked[0]
    );
    assert!(!marked[0].contains("let x = 1;"), "{:?}", marked[0]);
}

#[test]
fn several_line_marks_land_on_their_own_lines() {
    let html =
        render("```snippet\nrust a.rs | note #L1,3\nlet x = 1;\nlet y = 2;\nlet z = 3;\n```\n");
    let marked = marked_lines(&html);
    assert_eq!(marked.len(), 2, "two lines cited, two marked: {marked:?}");
    assert!(marked[0].contains("let x = 1;"), "{:?}", marked[0]);
    assert!(marked[1].contains("let z = 3;"), "{:?}", marked[1]);
}

#[test]
fn a_marked_block_keeps_one_rendered_line_per_line_of_code() {
    // Unwinding a line rebuilds it, and the scaffolding after the last newline is not
    // a line: counting it added a trailing blank line to every marked block.
    let html =
        render("```snippet\nrust a.rs | note #L2\nlet x = 1;\nlet y = 2;\nlet z = 3;\n```\n");
    let body = html.split("</style>").nth(1).expect("the page has a body");
    let inner = body
        .split("<pre")
        .nth(1)
        .expect("a code block")
        .split_once('>')
        .expect("the tag ends")
        .1
        .split("</pre>")
        .next()
        .unwrap();
    assert_eq!(
        inner.split('\n').count(),
        3,
        "three lines of code, three lines rendered: {inner:?}"
    );
}

#[test]
fn marked_code_is_well_formed_html() {
    // A marked line used to close its <mark> before the span that carried the
    // newline, leaving unbalanced markup and a highlight that spilled off the line.
    let html = render(
        "```snippet\nrust a.rs | note #L1,2,3\nspawn(async move {\n    let x = 1;\n}\n```\n",
    );
    let block = html
        .split("<figure class=\"snippet-block\"")
        .nth(1)
        .expect("a snippet block")
        .split("</figure>")
        .next()
        .unwrap();
    assert_eq!(block.matches("<pre").count(), 1, "one code block: {block}");
    assert_eq!(
        block.matches("<span").count(),
        block.matches("</span>").count(),
        "unbalanced spans: {block}"
    );
    assert_eq!(
        block.matches("<mark").count(),
        block.matches("</mark>").count(),
        "unbalanced marks: {block}"
    );
}

#[test]
fn a_snippet_may_quote_a_fence_of_its_own() {
    // Reviewing a markdown change means quoting a fence. With a shorter fence the
    // inner ``` closed the block early: the snippet became an unhighlighted code
    // block whose first line was the raw header.
    let html = render(
        "````snippet\nmarkdown docs/readme.md | a fence sample\nUse a fence:\n```rust\nlet x = 1;\n```\n````\n\n## After\n",
    );
    assert!(html.contains("class=\"snippet-block\""), "still a snippet");
    assert!(
        html.contains("docs/readme.md — a fence sample"),
        "caption kept"
    );
    assert!(html.contains("let x = 1;"), "the quoted code renders");
    assert!(html.contains("<h2>After</h2>"), "the document continues");
    let block = html
        .split("<figure class=\"snippet-block\"")
        .nth(1)
        .unwrap()
        .split("</figure>")
        .next()
        .unwrap();
    assert_eq!(
        block.matches("<pre").count(),
        1,
        "no nested code block: {block}"
    );
}

#[test]
fn a_caption_that_mentions_a_line_keeps_its_tail() {
    // Only the trailing token is a line mark. Taking the first `#L` anywhere in the
    // header truncated the caption at the mention and highlighted the wrong line.
    let html = render("```snippet\nrust a.rs | fixes #L42 regression\nlet x = 1;\n```\n");
    assert!(
        html.contains("a.rs — fixes #L42 regression"),
        "the caption keeps its tail: {html}"
    );
    assert!(!html.contains("snippet-hit\">"), "a mention is not a mark");
}

#[test]
fn a_reference_with_spaces_stays_in_the_caption() {
    let html = render("```snippet\nrust server/src/app.rs:170 (expiry) | note\nlet x = 1;\n```\n");
    assert!(visible_text(&html).contains("server/src/app.rs:170 (expiry) — note"));
    assert!(
        html.contains("data-lang=\"rust\""),
        "the language is still read"
    );
}

#[test]
fn a_header_without_a_language_names_the_file_instead() {
    // The reference is what is left of the head, so the gate has to look at the first
    // token: a path with an annotation after it is a location, not a language, and
    // reading it as one drops the path from the caption.
    let html =
        render("```snippet\nserver/src/app.rs:170 (expiry) | the expiry loop\nlet x = 1;\n```\n");
    assert!(
        visible_text(&html).contains("server/src/app.rs:170 (expiry) — the expiry loop"),
        "the file stays named: {html}"
    );
    assert!(
        !html.contains("data-lang"),
        "a path is a location, not a language"
    );
}

#[test]
fn line_marks_outside_the_code_are_ignored() {
    for header in ["rust a.rs | note #L99", "rust a.rs | note #Labc"] {
        let html = render(&format!("```snippet\n{header}\nlet x = 1;\n```\n"));
        assert!(
            !html.contains("snippet-hit\">"),
            "{header} should not mark a line"
        );
        assert!(
            visible_text(&html).contains("let x = 1;"),
            "{header} still renders its code"
        );
    }
}

#[test]
fn review_text_in_a_snippet_or_step_block_is_escaped() {
    // The caption, the language badge and the steps come from a pasted brief.
    let html = render("```snippet\nrust a.rs<script> | <b>caption</b>\nlet x = 1;\n```\n");
    assert!(
        html.contains("data-lang=\"rust\""),
        "the real language is kept"
    );
    assert!(!html.contains("<b>caption</b>"), "caption markup escaped");
    assert!(html.contains("&lt;b&gt;caption&lt;/b&gt;"));
    assert!(!html.contains("a.rs<script>"));

    let html = render("```steps\nUse <script>alert(1)</script> | the \"detail\"\n```\n");
    assert!(!html.contains("<script>alert(1)</script>"));
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(html.contains("the &quot;detail&quot;"));
}

#[test]
fn a_snippet_header_may_name_just_the_file_or_just_the_point() {
    // Both are legitimate briefs: a file reference with no comment, or a caption
    // standing on its own when the file is obvious from the section.
    let reference_only = render("```snippet\nrust server/src/app.rs:170\nlet x = 1;\n```\n");
    assert!(
        visible_text(&reference_only).contains("server/src/app.rs:170"),
        "a bare reference is the caption: {reference_only}"
    );
    assert!(reference_only.contains("data-lang=\"rust\""));

    let caption_only = render("```snippet\n| the expiry loop\nlet x = 1;\n```\n");
    assert!(
        visible_text(&caption_only).contains("the expiry loop"),
        "a caption with no reference: {caption_only}"
    );
    assert!(!caption_only.contains("data-lang"), "no language was given");
}

#[test]
fn a_step_may_be_a_title_with_no_detail() {
    // Not every step needs a supporting clause, and a missing one must not leave an
    // empty span behind.
    let html = render("```steps\nAsk for the diff\nRead it\n```\n");
    let body = body_of(&html);
    assert!(body.contains("class=\"steps-block\""));
    assert!(body.contains("<span class=\"step-title\">Ask for the diff</span>"));
    assert!(!body.contains("step-detail"), "no empty details: {body}");
    assert!(body.contains("2 steps"));
}

#[test]
fn code_in_a_language_the_highlighter_does_not_know_still_renders() {
    // A brief may quote any language; an unknown one must fall back to plain text
    // rather than dropping the code or panicking.
    let html = render("```brainfuck\n+++[->+++<]\n```\n");
    let body = body_of(&html);
    assert!(body.contains("<pre"), "the code block renders: {body}");
    assert!(
        body.contains("+++[-&gt;+++&lt;]"),
        "escaped, not dropped: {body}"
    );
    // The text renders in the default ink, and no palette colour is invented for a
    // language the highlighter has never seen.
    assert!(body.contains("color:#1a1a1a"), "default ink: {body}");
    for palette_colour in ["#0f3d8a", "#0a6b38", "#8a4b00", "#4b2a8f", "#0b5c70"] {
        assert!(
            !body.contains(palette_colour),
            "{palette_colour} is syntax highlighting the highlighter did not recognise: {body}"
        );
    }
}

#[test]
fn a_single_step_is_not_plural() {
    assert!(render("```steps\nOnly step | detail\n```\n").contains("1 step</span>"));
    assert!(render("```steps\nOne | a\nTwo | b\n```\n").contains("2 steps</span>"));
}

#[test]
fn an_empty_steps_block_still_renders() {
    let html = render("```steps\n```\n");
    assert!(html.contains("class=\"steps-block\""));
    assert!(html.contains("0 steps"));
}

#[test]
fn parse_chunks_closes_on_a_fence_at_least_as_long_as_the_opening() {
    let chunks = parse_chunks("```diff\n+let x = 1;\n````\n## After\n");
    match &chunks[0] {
        Chunk::Diagram { source, .. } => assert_eq!(source, "+let x = 1;"),
        _ => panic!("expected a diff diagram chunk"),
    }
    assert!(
        matches!(chunks[1], Chunk::Markdown(_)),
        "the heading is not swallowed"
    );
}

#[test]
fn parse_chunks_reads_a_shorter_fence_inside_a_longer_one() {
    let chunks = parse_chunks(
        "````snippet\nmarkdown docs/x.md | note\nUse a fence:\n```rust\nlet x = 1;\n```\n````\n",
    );
    assert_eq!(chunks.len(), 1, "the inner fence does not close the block");
    match &chunks[0] {
        Chunk::Diagram { kind, source } => {
            assert_eq!(kind, "snippet");
            assert!(
                source.contains("```rust"),
                "the inner fence stays in the source"
            );
            assert!(source.contains("let x = 1;"));
        }
        _ => panic!("expected a snippet chunk"),
    }
}

#[test]
fn snippet_block_carries_the_reference_caption_and_highlighted_line() {
    let html = render(
        "# t\n\n```snippet\nrust server/src/app.rs:170 | the expiry loop #L2\nfn main() {\n    notifiers.remove(&id);\n}\n```\n",
    );
    assert!(
        html.contains("class=\"snippet-block\""),
        "snippet block missing"
    );
    assert!(html.contains("data-lang=\"rust\""), "language not recorded");
    assert!(
        visible_text(&html).contains("server/src/app.rs:170 — the expiry loop"),
        "caption should name the reference and the point"
    );
    assert!(
        html.contains("mark class=\"snippet-hit\""),
        "cited line not marked"
    );
}

#[test]
fn snippet_with_a_blank_header_is_just_code() {
    // Every part of the header is optional: an empty first line means no caption.
    let html = render("# t\n\n```snippet\n\nlet x = 1;\n```\n");
    assert!(html.contains("class=\"snippet-block\""));
    assert!(html.contains("let x = 1"));
    assert!(!html.contains("<figcaption>"), "no empty caption element");
}

#[test]
fn steps_block_is_readable_without_scripting() {
    // The reveal is progressive enhancement: the words must be in the HTML, not
    // injected by the script, or the brief is empty for a reader with JS off.
    let html = render(
        "# t\n\n```steps\nClient sends | the page token rides in the query\nQuery runs | the token becomes an OFFSET\n```\n",
    );
    assert!(html.contains("class=\"steps-block\""));
    assert!(html.contains("the token becomes an OFFSET"));
    assert!(html.contains("the page token rides in the query"));
    assert!(html.contains("2 steps"));
    assert!(
        html.contains("step-next"),
        "needs a way forward on a touch screen"
    );
}

#[test]
fn code_blocks_wrap_instead_of_scrolling() {
    // A horizontal scrollbar cannot be dragged with a pen on e-ink.
    let css = css_rule("pre {");
    assert!(
        css.contains("white-space: pre-wrap"),
        "code must wrap: {css}"
    );
    assert!(
        !css.contains("overflow-x: auto"),
        "no horizontal scrolling: {css}"
    );
    assert!(
        css.contains("background: #ffffff"),
        "grey fill dithers on e-ink"
    );
}

#[test]
fn block_code_is_a_point_larger_than_body_text() {
    assert!(css_rule("pre code {").contains("font-size: 19px"));
    assert!(css_rule("body {").contains("font-size: 18px"));
}

#[test]
fn comments_are_italic_and_keywords_bold() {
    // Shape, not shade: the two only need to differ in weight and slant for the
    // structure to read when the hues collapse in poor light.
    let theme = include_str!("../../assets/eink-theme.tmTheme");
    let comment = theme
        .split("<key>name</key>")
        .find(|chunk| chunk.contains("Comment"))
        .unwrap();
    assert!(
        comment.contains("<string>italic</string>"),
        "comments should slant"
    );
    let keyword = theme
        .split("<key>name</key>")
        .find(|chunk| chunk.contains(">Keyword<"))
        .unwrap();
    assert!(
        keyword.contains("<string>bold</string>"),
        "keywords should be bold"
    );
}

/// Every rule in the stylesheet, as (selector, declarations).
///
/// At-rule preludes are dropped and their contents parsed as ordinary rules, so a
/// `@media` block cannot hide a rule from the invariants below.
fn css_rules() -> Vec<(String, String)> {
    let mut rules = Vec::new();
    let cleaned = strip_css_comments(EINK_CSS);
    let mut rest = cleaned.as_str();
    while let Some(open) = rest.find('{') {
        let selector = rest[..open].trim().to_string();
        let Some(close) = rest[open..].find('}') else {
            break;
        };
        let body = rest[open + 1..open + close].to_string();
        if selector.starts_with('@') {
            // Descend into the at-rule and keep parsing its contents.
            rest = &rest[open + 1..];
            continue;
        }
        rest = &rest[open + close + 1..];
        if !selector.is_empty() {
            rules.push((selector, body));
        }
    }
    rules
}

fn strip_css_comments(css: &str) -> String {
    // Comments can hold braces, which would confuse the rule walk below.
    let mut out = String::with_capacity(css.len());
    let mut rest = css;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

/// Everything the e-ink page ships. The stylesheet and the page script are separate
/// constants, and the invariants below span both.
fn page_source() -> String {
    format!("{EINK_CSS}\n{TOC_JS}\n{BOOTSTRAP_JS}\n{ELEMENT_MAP_JS}")
}

/// The selectors a tap can bind to, read from the page's own element map so this
/// stays true when a new block kind is added.
fn annotatable_selectors() -> Vec<String> {
    let page = page_source();
    let query = page
        .split("var QUERY = '")
        .nth(1)
        .expect("the element map declares a QUERY")
        .split('\'')
        .next()
        .unwrap();
    query.split(',').map(|s| s.trim().to_string()).collect()
}

/// The subject of a selector: the rightmost compound, whose box an outline wraps.
fn subject(selector: &str) -> String {
    selector
        .split(',')
        .next_back()
        .unwrap()
        .split_whitespace()
        .next_back()
        .unwrap_or("")
        .to_string()
}

#[test]
fn the_highlight_is_an_outline_that_a_clipping_ancestor_would_hide() {
    // This is the premise of the two tests below. If the highlight ever becomes an
    // inset shadow or an overlay, revisit them: their reasoning changes with it.
    let page = page_source();
    assert!(
        page.contains("outline = '4px solid '"),
        "the bind/flash highlight is expected to be an outline"
    );
    assert!(
        page.contains("outlineOffset = '3px'"),
        "the outline sits outside the element box, so ancestors can clip it"
    );
}

#[test]
fn only_viewports_clip_their_contents() {
    // Clipping hides annotation outlines drawn outside an element, so every clipping
    // rule is a deliberate exception and has to be named here. Adding
    // `overflow: hidden` to a content wrapper is what broke annotating code twice.
    const ALLOWED: &[&str] = &[
        // Pan/zoom canvases hold a single SVG and no annotatable content; they must
        // scroll or the diagram is unreachable on a small screen.
        ".mindmap-viewport",
        ".graph-viewport",
    ];
    for (selector, body) in css_rules() {
        let clips = body
            .split(';')
            .filter_map(|decl| decl.split_once(':'))
            .any(|(prop, value)| {
                prop.trim().starts_with("overflow")
                    && matches!(value.trim(), "hidden" | "auto" | "scroll")
            });
        if clips {
            assert!(
                ALLOWED.contains(&subject(&selector).as_str()),
                "'{selector}' clips its contents, which hides an annotation outline \
                     inside it. If that is intended, add it to ALLOWED with a reason."
            );
        }
    }
}

#[test]
fn content_that_can_be_annotated_is_never_clipped() {
    // The same rule stated from the reader's side: for every element a tap can bind
    // to, and every wrapper we render around one, the box must be free to draw.
    let mut checked = annotatable_selectors();
    checked.push(".snippet-block".into());
    checked.push(".diff-block".into());
    checked.push(".steps-block".into());
    for selector in checked {
        let tag = selector.trim_start_matches('.');
        for (rule_selector, body) in css_rules() {
            if subject(&rule_selector) != selector && subject(&rule_selector) != tag {
                continue;
            }
            assert!(
                !body.contains("overflow: hidden"),
                "'{rule_selector}' wraps annotatable {selector} and clips its outline"
            );
        }
    }
}

#[test]
fn every_syntax_colour_is_readable_on_paper() {
    // "The colours are not visible" was a canvas that had gone dark, not a missing
    // palette. Measure the theme the renderer actually loads: the sheet must stay
    // light, and every ink must clear WCAG AA against it.
    let theme = syntect::highlighting::ThemeSet::load_from_reader(&mut std::io::Cursor::new(
        include_bytes!("../../assets/eink-theme.tmTheme").to_vec(),
    ))
    .expect("the shipped theme parses");
    let bg = theme.settings.background.expect("the theme sets a canvas");
    assert!(
        luminance(bg) > 0.75,
        "the code canvas must stay paper light, not {:?}",
        (bg.r, bg.g, bg.b)
    );
    let mut inks: Vec<(f64, String)> = Vec::new();
    let mut collect = |name: &str, c: syntect::highlighting::Color| {
        inks.push((contrast(c, bg), name.to_string()))
    };
    if let Some(fg) = theme.settings.foreground {
        collect("default ink", fg);
    }
    for scope in &theme.scopes {
        if let Some(c) = scope.style.foreground {
            let name = scope
                .scope
                .selectors
                .iter()
                .map(|s| s.path.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            collect(&name, c);
        }
    }
    assert!(inks.len() > 20, "the theme should carry a real palette");
    for (ratio, name) in inks {
        assert!(
            ratio >= 4.5,
            "'{name}' is too faint on the canvas (contrast {ratio:.2})"
        );
    }
}

fn luminance(c: syntect::highlighting::Color) -> f64 {
    fn channel(value: u8) -> f64 {
        let v = value as f64 / 255.0;
        if v <= 0.03928 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    }
    0.2126 * channel(c.r) + 0.7152 * channel(c.g) + 0.0722 * channel(c.b)
}

fn contrast(a: syntect::highlighting::Color, b: syntect::highlighting::Color) -> f64 {
    let (hi, lo) = {
        let (la, lb) = (luminance(a), luminance(b));
        if la > lb { (la, lb) } else { (lb, la) }
    };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn every_block_kind_can_be_bound_by_a_tap() {
    // A block the element map cannot see is a block nobody can annotate. New kinds
    // are the likely way this breaks, so each one is rendered and checked here.
    let samples: &[(&str, &str)] = &[
        ("paragraph", "A paragraph.\n"),
        ("heading", "## Heading\n"),
        ("quote", "> quoted\n"),
        ("list", "- one\n- two\n"),
        ("table", "| a | b |\n|---|---|\n| 1 | 2 |\n"),
        ("image", "![alt](diagram.png)\n"),
        ("code", "```rust\nfn main() {}\n```\n"),
        (
            "snippet",
            "```snippet\nrust a.rs:1 | note\nlet x = 1;\n```\n",
        ),
        ("steps", "```steps\nFirst | one\n```\n"),
        ("diff", "```diff\n+let x = 1;\n```\n"),
        ("mermaid", "```mermaid\nflowchart LR\n  A --> B\n```\n"),
        (
            "mindmap",
            "```mindmap\nroot: Plan\nnodes:\n  - label: Step\n```\n",
        ),
        (
            "graph",
            "```graph\nnodes:\n  - id: a\n    label: A\nedges: []\n```\n",
        ),
    ];
    let selectors = annotatable_selectors();
    for (kind, markdown) in samples {
        let html = render(markdown);
        let mut body = html.split("</style>").nth(1).unwrap_or("").to_string();
        while let (Some(open), Some(close)) = (body.find("<script"), body.find("</script>")) {
            body.replace_range(open..close + "</script>".len(), "");
        }
        let bound = selectors
            .iter()
            .any(|selector| match selector.strip_prefix('.') {
                Some(class) => body.contains(&format!("class=\"{class}\"")),
                None => body.contains(&format!("<{selector}")),
            });
        assert!(
            bound,
            "a {kind} block renders with nothing a tap can bind to; add its wrapper \
                 to QUERY in assets/element-map.js"
        );
    }
}

#[test]
fn the_pen_owns_taps_so_a_steps_block_does_not_steal_them() {
    assert!(
        css_rule(".steps-list {").contains("pointer-events: none"),
        "the list must not swallow the taps used to annotate"
    );
    assert!(
        css_rule(".steps-controls button {").contains("pointer-events: auto"),
        "the sequence still needs its own controls"
    );
    let page = page_source();
    assert!(
        !page.contains("block.addEventListener('click'"),
        "a click handler on the whole block swallows the taps used to select elements"
    );
    assert!(
        page.contains("next.addEventListener('click'"),
        "advancing belongs to the controls and nowhere else"
    );
}

#[test]
fn a_steps_block_still_reads_without_script() {
    // The sequence is progressive enhancement: every step must be legible before the
    // page marks one current, otherwise a reader without script gets a faint list.
    let html = render("```steps\nFirst | one\nSecond | two\n```");
    let list = html
        .split("<ol class=\"steps-list\">")
        .nth(1)
        .expect("a steps list renders")
        .split("</ol>")
        .next()
        .unwrap();
    assert!(
        !list.contains("data-state"),
        "the markup must not pre-dim any step; the script adds the state"
    );
    assert!(
        css_rule(".steps-list li {").contains("opacity: 1"),
        "steps read at full strength by default"
    );
    assert!(
        css_rule(".steps-list li[data-state=\"todo\"] {").contains("opacity: 0.35"),
        "only unreached steps recede"
    );
}

#[test]
fn code_blocks_get_syntax_highlighting() {
    let html = render("```rust\nfn main() {}\n```");
    // syntect wraps in <pre style="..."> with inline colors
    assert!(
        html.contains("<pre style="),
        "should have inline styles from syntect"
    );
}

#[test]
fn solarized_theme_produces_colored_spans() {
    let html = render("```rust\nfn main() { let x = 1; }\n```");
    // Solarized theme produces colored <span> elements
    assert!(html.contains("<span style="), "should have colored spans");
}

// --- General structure ---

#[test]
fn output_is_valid_html_document() {
    let html = render("# Title\n\nHello");
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("<html lang=\"en\">"));
    assert!(html.contains("</html>"));
    assert!(html.contains("/assets/diagram/mermaid.min.js"));
    assert!(html.contains("/assets/diagram/elk.bundled.js"));
}

#[test]
fn markdown_headings_appear_in_body() {
    let html = render("# Title\n## Section\n### Sub");
    assert!(html.contains("<h1>") || html.contains("<h1 "));
    assert!(html.contains("<h2>") || html.contains("<h2 "));
    assert!(html.contains("<h3>") || html.contains("<h3 "));
}

#[test]
fn session_id_embedded_in_html() {
    let html = to_eink_html("hello", "abc123");
    assert!(html.contains("data-session-id=\"abc123\""));
}

#[test]
fn a_document_image_is_served_from_the_session_that_owns_it() {
    // The brief says where the image is inside the document; the page has to say where
    // the server serves it from, because a relative URL would resolve against /session/{id}
    // and 404.
    let html = to_eink_html("![error page](image/shot.png)", "abc123");
    assert!(
        html.contains("src=\"/api/sessions/abc123/image/shot.png\""),
        "the image was not pointed at the session route: {html}"
    );
    assert!(
        !html.contains("src=\"image/"),
        "a bare image/ path survived"
    );
}

#[test]
fn an_image_that_is_not_a_session_image_is_left_alone() {
    // An absolute or external URL belongs to whoever wrote it.
    for markdown in [
        "![x](https://example.com/a.png)",
        "![x](image-files/a.png)",
        "![x](/elsewhere/a.png)",
    ] {
        let html = to_eink_html(markdown, "abc123");
        assert!(
            !html.contains("/api/sessions/abc123/image/"),
            "{markdown} was rewritten as a session image"
        );
    }
}

#[test]
fn assets_script_tags_present() {
    let html = render("hello");
    assert!(html.contains("src=\"/assets/diagram/mermaid.min.js\""));
    assert!(html.contains("src=\"/assets/diagram/elk.bundled.js\""));
}

// --- parse_fence_start ---

#[test]
fn parse_fence_start_recognizes_known_kinds() {
    assert_eq!(
        parse_fence_start("```mermaid").map(|(k, _)| k),
        Some("mermaid")
    );
    assert_eq!(
        parse_fence_start("```mindmap").map(|(k, _)| k),
        Some("mindmap")
    );
    assert_eq!(parse_fence_start("```graph").map(|(k, _)| k), Some("graph"));
}

#[test]
fn parse_fence_start_ignores_unknown_kinds() {
    assert_eq!(parse_fence_start("```rust"), None);
    assert_eq!(parse_fence_start("```python"), None);
    assert_eq!(parse_fence_start("```"), None);
    assert_eq!(parse_fence_start("```  "), None);
}

#[test]
fn parse_fence_start_ignores_non_fence_lines() {
    assert_eq!(parse_fence_start("# heading"), None);
    assert_eq!(parse_fence_start("regular text"), None);
    assert_eq!(parse_fence_start(""), None);
    assert_eq!(parse_fence_start("  mermaid"), None);
}

#[test]
fn parse_fence_start_tolerates_surrounding_whitespace() {
    assert_eq!(
        parse_fence_start("  ```mermaid  ").map(|(k, _)| k),
        Some("mermaid")
    );
    assert_eq!(
        parse_fence_start("\t```graph").map(|(k, _)| k),
        Some("graph")
    );
}

// --- parse_chunks ---

#[test]
fn parse_chunks_returns_single_markdown_chunk_for_plain_text() {
    let chunks = parse_chunks("# Hello\n\nworld");
    assert_eq!(chunks.len(), 1);
    assert!(matches!(&chunks[0], Chunk::Markdown(_)));
}

#[test]
fn parse_chunks_splits_diagram_from_surrounding_markdown() {
    let md = "before\n```mermaid\nA --> B\n```\nafter";
    let chunks = parse_chunks(md);
    assert_eq!(chunks.len(), 3);
    assert!(matches!(&chunks[0], Chunk::Markdown(s) if s.contains("before")));
    assert!(matches!(&chunks[1], Chunk::Diagram { kind, .. } if kind == "mermaid"));
    assert!(matches!(&chunks[2], Chunk::Markdown(s) if s.contains("after")));
}

#[test]
fn parse_chunks_captures_diagram_source_correctly() {
    let md = "```graph\nnodes:\n  - id: a\n```";
    let chunks = parse_chunks(md);
    assert_eq!(chunks.len(), 1);
    if let Chunk::Diagram { kind, source } = &chunks[0] {
        assert_eq!(kind, "graph");
        assert!(source.contains("nodes:"));
    } else {
        panic!("expected Diagram chunk");
    }
}

#[test]
fn parse_chunks_treats_unclosed_fence_as_markdown() {
    let md = "```mermaid\nA --> B\n(no closing fence)";
    let chunks = parse_chunks(md);
    // unclosed fence: all content lands in markdown
    assert!(chunks.iter().all(|c| matches!(c, Chunk::Markdown(_))));
}

#[test]
fn parse_chunks_handles_adjacent_diagram_blocks() {
    let md = "```mermaid\nA\n```\n```graph\nnodes: []\n```";
    let chunks = parse_chunks(md);
    let diagrams: Vec<_> = chunks
        .iter()
        .filter(|c| matches!(c, Chunk::Diagram { .. }))
        .collect();
    assert_eq!(diagrams.len(), 2);
}

#[test]
fn parse_chunks_empty_input_gives_no_chunks() {
    assert!(parse_chunks("").is_empty());
}

// --- escape_html ---

#[test]
fn escape_html_escapes_all_special_chars() {
    assert_eq!(escape_html("&"), "&amp;");
    assert_eq!(escape_html("<"), "&lt;");
    assert_eq!(escape_html(">"), "&gt;");
    assert_eq!(escape_html("\""), "&quot;");
    assert_eq!(escape_html("'"), "&#39;");
}

#[test]
fn escape_html_handles_combined_input() {
    assert_eq!(
        escape_html("<script>alert('xss')</script>"),
        "&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"
    );
}

#[test]
fn escape_html_leaves_plain_text_unchanged() {
    assert_eq!(escape_html("hello world 123"), "hello world 123");
    assert_eq!(escape_html(""), "");
}

// --- diagram_label ---

#[test]
fn diagram_label_maps_known_kinds() {
    assert_eq!(diagram_label("mermaid"), "Diagram");
    assert_eq!(diagram_label("mindmap"), "Mind Map");
    assert_eq!(diagram_label("graph"), "Graph");
}

#[test]
fn diagram_label_falls_back_for_unknown() {
    assert_eq!(diagram_label("unknown"), "Diagram");
    assert_eq!(diagram_label(""), "Diagram");
}

// --- edge cases ---

#[test]
fn empty_input_produces_valid_html() {
    let html = render("");
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(html.contains("</html>"));
}

#[test]
fn a_diagram_source_cannot_close_the_payload_script() {
    let source = "A --> B </SCRIPT><script>alert(1)</script> & </script >";
    let html = render(&format!("```mermaid\n{source}\n```"));
    let payload = html
        .split("<script type=\"application/json\" class=\"diagram-payload\">")
        .nth(1)
        .and_then(|rest| rest.split("</script>").next())
        .expect("a payload block");
    assert!(
        !payload.contains(['<', '>', '&']),
        "payload must not contain a character that can spell a tag: {payload}"
    );
    let value: serde_json::Value = serde_json::from_str(payload).expect("valid JSON");
    assert_eq!(value["source"], source);
    let own = render("```mermaid\nA\n```")
        .to_ascii_lowercase()
        .matches("<script")
        .count();
    assert_eq!(
        html.to_ascii_lowercase().matches("<script").count(),
        own,
        "only the renderer's own script tags may appear"
    );
}

// --- raw HTML from the brief ---

#[test]
fn a_script_in_a_paragraph_is_dropped() {
    let html = render("Hello <script>alert(1)</script> world\n");
    assert!(!html.contains("alert(1)"));
    assert!(html.contains("<p>Hello  world</p>"), "{html}");
}

#[test]
fn an_event_handler_attribute_is_dropped() {
    let html =
        render("<img src=\"x.png\" onerror=\"alert(1)\">\n\n<p onclick=\"alert(1)\">tap</p>\n");
    assert!(
        !html.contains("onerror") && !html.contains("onclick"),
        "{html}"
    );
    assert!(html.contains("<img src=\"x.png\">"), "{html}");
    assert!(html.contains("<p>tap</p>"), "{html}");
}

#[test]
fn a_colour_span_survives_byte_for_byte() {
    let html = render("A <span style=\"color: red\">risk</span> here.\n");
    assert!(
        html.contains("<p>A <span style=\"color: red\">risk</span> here.</p>"),
        "{html}"
    );
}

#[test]
fn the_inline_touches_a_review_uses_survive() {
    let html = render(
        "Press <kbd>Ctrl</kbd> on <mark>this</mark> x<sup>2</sup> H<sub>2</sub>O<br>\n\n<details>\n<summary>More</summary>\n\nBody *text*\n\n</details>\n",
    );
    for tag in [
        "<kbd>Ctrl</kbd>",
        "<mark>this</mark>",
        "<sup>2</sup>",
        "<sub>2</sub>",
        "<br>",
        "<details>",
        "<summary>More</summary>",
        "<p>Body <em>text</em></p>",
    ] {
        assert!(html.contains(tag), "{tag} missing from {html}");
    }
}

#[test]
fn a_style_that_is_not_a_colour_is_dropped() {
    let html = render(
        "<span style=\"background: url(http://evil/x)\">a</span> <span style=\"color: red; position: fixed\">b</span>\n",
    );
    assert!(!html.contains("evil"), "{html}");
    assert!(
        html.contains("<p><span>a</span> <span>b</span></p>"),
        "{html}"
    );
}

#[test]
fn a_javascript_link_loses_its_destination() {
    let html =
        render("[tap](javascript:alert(1)) and [ok](https://example.com/x) and [rel](/api/x)\n");
    assert!(!html.contains("javascript:"), "{html}");
    assert!(
        html.contains("<a href=\"https://example.com/x\">ok</a>"),
        "{html}"
    );
    assert!(html.contains("<a href=\"/api/x\">rel</a>"), "{html}");
}

#[test]
fn markdown_around_raw_html_keeps_its_own_markup() {
    let html =
        render("| a | b |\n|:--|--:|\n| <kbd>x</kbd> | 2 |\n\n- [x] done <mark>now</mark>\n");
    assert!(html.contains("style=\"text-align: left\""), "{html}");
    assert!(html.contains("type=\"checkbox\""), "{html}");
}

#[test]
fn render_diagram_error_contains_source() {
    let bad_yaml = ": invalid: [yaml";
    let out = render_diagram("graph", bad_yaml);
    assert!(out.contains("Parse Error"));
    assert!(out.contains("diagram-error"));
    // source should be shown and HTML-escaped
    assert!(out.contains("diagram-source"));
}

// --- diff block ---

#[test]
fn diff_block_parses_from_fence() {
    assert_eq!(parse_fence_start("```diff").map(|(k, _)| k), Some("diff"));
}

#[test]
fn diff_added_lines_get_class() {
    let html = render("```diff\n+new line\n```");
    assert!(
        html.contains("diff-add"),
        "added line should have diff-add class"
    );
    assert!(html.contains("+new line"));
}

#[test]
fn diff_deleted_lines_get_class() {
    let html = render("```diff\n-old line\n```");
    assert!(
        html.contains("diff-del"),
        "deleted line should have diff-del class"
    );
    assert!(html.contains("-old line"));
}

#[test]
fn diff_hunk_headers_get_class() {
    let html = render("```diff\n@@ -1,3 +1,4 @@\n```");
    assert!(html.contains("diff-hunk"));
}

#[test]
fn diff_context_lines_get_class() {
    let html = render("```diff\n unchanged line\n```");
    assert!(html.contains("diff-ctx"));
}

#[test]
fn diff_html_escapes_content() {
    let html = render("```diff\n+<script>evil()</script>\n```");
    // The raw unescaped tag must not appear inside a diff-add div
    assert!(
        !html.contains("class=\"diff-add\"><script>"),
        "raw script tag in diff must be escaped"
    );
    assert!(html.contains("&lt;script&gt;"));
}

#[test]
fn diff_block_wraps_in_diff_block_class() {
    let html = render("```diff\n+line\n```");
    assert!(html.contains("class=\"diff-block\""));
}

#[test]
fn plain_code_block_does_not_get_diff_styling() {
    let html = render("```bash\n+not a diff\n```");
    // The CSS defines .diff-add but no element should carry class="diff-add"
    assert!(
        !html.contains("class=\"diff-add\""),
        "plain bash block must not get diff styling"
    );
}

mod coderef_tests {
    use super::*;

    const INLINE: &str = "<p>See server/src/app.rs:170 for the expiry loop.</p>";
    const WITH_CODE: &str = "<pre><span>server/src/app.rs:170</span></pre><p>and app.rs:12</p>";

    #[test]
    fn a_reference_in_prose_becomes_a_tap_target() {
        let linked = link_code_references(INLINE);
        assert!(
            linked.contains("<button class=\"coderef\" data-ref=\"server/src/app.rs:170\" type=\"button\">server/src/app.rs:170</button>"),
            "{linked}"
        );
    }

    #[test]
    fn nothing_inside_a_code_block_is_touched() {
        // Those lines are already on the page; a tap target there would do nothing useful.
        let linked = link_code_references(WITH_CODE);
        let code = linked.split("</pre>").next().unwrap();
        assert!(
            !code.contains("coderef"),
            "the code block was rewritten: {code}"
        );
        assert!(linked.contains("data-ref=\"app.rs:12\""), "{linked}");
    }

    #[test]
    fn only_source_references_are_linked() {
        for text in [
            "<p>a 12:30 appointment</p>",
            "<p>ratio 16:9 in prose</p>",
            "<p>the label B → path:line literal</p>",
            "<p>https://example.com/a.rs:12</p>",
            "<p>Cargo.toml</p>",
        ] {
            let linked = link_code_references(text);
            assert!(
                !linked.contains("coderef"),
                "{text} should not be a reference: {linked}"
            );
        }
    }

    #[test]
    fn the_expansion_is_rendered_like_any_other_code_block() {
        // Plain text would make the expansion the only uncoloured code on the tablet.
        let block = source_block(
            "rust",
            "server/src/app.rs:168",
            &["fn main() {", "    let x = 1;", "}"],
            Some(2),
        );
        assert!(block.contains("snippet-block"), "it is a snippet: {block}");
        assert!(block.contains("data-lang=\"rust\""), "{block}");
        assert!(
            block.contains("color:#"),
            "highlighted with the palette: {block}"
        );
        // The highlighted line is split across spans, so the mark is what to look for.
        assert!(
            block.contains("mark class=\"snippet-hit\""),
            "the cited line is marked: {block}"
        );
        assert!(
            block.contains("server/src/app.rs:168 — ") || block.contains("server/src/app.rs:168"),
            "{block}"
        );
    }

    #[test]
    fn a_language_the_highlighter_does_not_know_renders_plain() {
        let block = source_block("", "notes.txt:1", &["just text"], None);
        assert!(block.contains("just text"));
        assert!(!block.contains("data-lang"), "{block}");
    }

    #[test]
    fn a_marks_list_line_is_linked() {
        // The brief's Marks section is exactly this shape.
        let marks = "<ul><li>A &rarr; server/src/app.rs:170</li></ul>";
        assert!(link_code_references(marks).contains("data-ref=\"server/src/app.rs:170\""));
    }
}
