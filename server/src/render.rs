use pulldown_cmark::{Options, Parser, html};

const EINK_CSS: &str = r#"
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 28px;
    line-height: 1.7;
    color: #000;
    background: #fff;
    width: 4000px;
    min-height: 6000px;
    padding: 0;
    margin: 0;
}
#content {
    max-width: 1600px;
    margin: 40px auto 2000px auto;
    padding: 0 40px;
}
h1 { font-size: 42px; margin: 48px 0 24px; border-bottom: 2px solid #000; padding-bottom: 12px; }
h2 { font-size: 34px; margin: 40px 0 16px; }
h3 { font-size: 28px; font-weight: bold; margin: 32px 0 12px; }
p { margin: 12px 0; }
code {
    font-family: 'Courier New', monospace;
    font-size: 13px;
    background: #f0f0f0;
    padding: 2px 4px;
    border: 1px solid #ccc;
}
pre {
    background: #f0f0f0;
    border: 1px solid #ccc;
    padding: 12px;
    margin: 16px 0;
    overflow-x: auto;
}
pre code { border: none; padding: 0; background: none; }
blockquote {
    border-left: 3px solid #333;
    padding-left: 16px;
    margin: 16px 0;
    color: #333;
}
ul, ol { margin: 12px 0; padding-left: 24px; }
li { margin: 4px 0; }
table { border-collapse: collapse; width: 100%; margin: 16px 0; }
th, td { border: 1px solid #333; padding: 8px 12px; text-align: left; }
th { font-weight: bold; background: #f0f0f0; }
img { max-width: 100%; height: auto; }
a { color: #000; text-decoration: underline; }
hr { border: none; border-top: 1px solid #333; margin: 24px 0; }
"#;

pub fn to_eink_html(markdown: &str, session_id: &str) -> String {
    let options = Options::all();
    let parser = Parser::new_ext(markdown, options);
    let mut html_content = String::new();
    html::push_html(&mut html_content, parser);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=1700, initial-scale=1.0, minimum-scale=0.2, maximum-scale=4.0, user-scalable=yes">
<title>E-Ink Review</title>
<style>{css}</style>
</head>
<body data-session-id="{session_id}">
<div id="content">{content}</div>
<script>document.getElementById('content').scrollIntoView({{behavior:'instant',block:'start'}});</script>
</body>
</html>"#,
        css = EINK_CSS,
        session_id = session_id,
        content = html_content,
    )
}
