use pulldown_cmark::{Options, Parser, html};

const EINK_CSS: &str = r#"
* { margin: 0; padding: 0; box-sizing: border-box; }
body {
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 16px;
    line-height: 1.6;
    color: #000;
    background: #fff;
    max-width: 680px;
    margin: 0 auto;
    padding: 24px 16px;
}
h1 { font-size: 26px; margin: 32px 0 16px; border-bottom: 2px solid #000; padding-bottom: 8px; }
h2 { font-size: 22px; margin: 28px 0 12px; }
h3 { font-size: 18px; font-weight: bold; margin: 24px 0 8px; }
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

.nav { display: flex; justify-content: space-between; align-items: center; padding: 12px 0; border-bottom: 1px solid #ccc; margin-bottom: 24px; position: sticky; top: 0; background: #fff; z-index: 10; }
.nav button { font-size: 18px; padding: 12px 24px; border: 2px solid #333; background: #fff; min-width: 64px; min-height: 48px; cursor: pointer; }
.nav .page-info { font-size: 14px; color: #333; }

.review-area { border-top: 2px solid #333; margin-top: 32px; padding-top: 16px; }
.review-area textarea {
    width: 100%;
    min-height: 120px;
    font-family: Georgia, serif;
    font-size: 16px;
    line-height: 1.5;
    padding: 12px;
    border: 1px solid #333;
    resize: vertical;
}
.submit-btn {
    display: block;
    width: 100%;
    font-size: 20px;
    padding: 16px;
    margin-top: 16px;
    border: 2px solid #000;
    background: #000;
    color: #fff;
    cursor: pointer;
    font-weight: bold;
    min-height: 64px;
}
.review-actions { display: flex; gap: 8px; margin-top: 12px; flex-wrap: wrap; }
.review-actions label, .review-actions button {
    font-size: 16px; padding: 12px 16px; border: 2px solid #333;
    background: #fff; cursor: pointer; min-height: 48px;
}
.review-actions input[type="file"] { display: none; }
"#;

const EINK_JS: &str = r#"
(function() {
    const content = document.getElementById('content');
    const pageInfo = document.getElementById('page-info');
    const prevBtn = document.getElementById('prev');
    const nextBtn = document.getElementById('next');

    // Simple pagination: split content into viewport-sized pages
    let currentPage = 0;
    const pageHeight = window.innerHeight - 200; // reserve space for nav + review area

    function getPages() {
        const children = Array.from(content.children);
        const pages = [[]];
        let height = 0;
        for (const child of children) {
            const h = child.offsetHeight + 16;
            if (height + h > pageHeight && pages[pages.length - 1].length > 0) {
                pages.push([]);
                height = 0;
            }
            pages[pages.length - 1].push(child);
            height += h;
        }
        return pages;
    }

    function render() {
        const pages = getPages();
        const total = pages.length;
        if (currentPage >= total) currentPage = total - 1;
        if (currentPage < 0) currentPage = 0;

        for (const child of content.children) {
            child.style.display = 'none';
        }
        if (pages[currentPage]) {
            for (const el of pages[currentPage]) {
                el.style.display = '';
            }
        }
        pageInfo.textContent = `Page ${currentPage + 1} / ${total}`;
        prevBtn.disabled = currentPage === 0;
        nextBtn.disabled = currentPage >= total - 1;
    }

    prevBtn.onclick = () => { currentPage--; render(); };
    nextBtn.onclick = () => { currentPage++; render(); };

    // Submit
    document.getElementById('submit-btn').onclick = async () => {
        const notes = document.getElementById('notes').value;
        const form = new FormData();
        form.append('typed_notes', notes);
        for (const input of ['annotation-file', 'camera-file']) {
            const files = document.getElementById(input).files;
            for (let i = 0; i < files.length; i++) {
                form.append('annotation', files[i]);
            }
        }

        const sessionId = document.body.dataset.sessionId;
        const resp = await fetch(`/api/sessions/${sessionId}/submit`, {
            method: 'POST',
            body: form,
        });
        if (resp.ok) {
            document.getElementById('submit-btn').textContent = 'Submitted!';
            document.getElementById('submit-btn').disabled = true;
        }
    };

    // JS bridge for Kotlin app
    window.getTypedNotes = () => document.getElementById('notes').value;
    window.setSubmitted = () => {
        document.getElementById('submit-btn').textContent = 'Submitted!';
        document.getElementById('submit-btn').disabled = true;
    };

    render();
})();
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
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>E-Ink Review</title>
<style>{css}</style>
</head>
<body data-session-id="{session_id}">
<div class="nav">
  <button id="prev">&lt;</button>
  <span id="page-info" class="page-info"></span>
  <button id="next">&gt;</button>
</div>
<div id="content">{content}</div>
<div class="review-area">
  <textarea id="notes" placeholder="Type your review notes here..."></textarea>
  <div class="review-actions">
    <label for="annotation-file">Attach image</label>
    <input type="file" id="annotation-file" accept="image/*" multiple>
    <label for="camera-file">Camera</label>
    <input type="file" id="camera-file" accept="image/*" capture="environment">
  </div>
  <button id="submit-btn" class="submit-btn">Submit Review</button>
</div>
<script>{js}</script>
</body>
</html>"#,
        css = EINK_CSS,
        session_id = session_id,
        content = html_content,
        js = EINK_JS,
    )
}
