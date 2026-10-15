use super::chunks::{parse_chunks, render_chunk};

pub fn to_browser_html(markdown: &str, session_id: &str) -> String {
    let content = parse_chunks(markdown)
        .into_iter()
        .map(render_chunk)
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Review session</title>
<style>{css}</style>
</head>
<body data-session-id="{session_id}">
<main id="content">{content}</main>
<aside id="review-panel" aria-label="Review panel">
<header>
<h2>Review</h2>
<div id="status" aria-live="polite">0 comments</div>
</header>
<section id="review-status" aria-label="Review status">
<strong id="delivery-status" role="status" aria-live="polite">Draft — not sent</strong>
<div id="draft-status" role="status">No local changes yet.</div>
<div id="connection-status">Checking connection…</div>
</section>
<section id="selection-section">
<label for="comment-input" class="label">Comment</label>
<div id="selected-preview" class="muted">Click any paragraph, heading, or list item — or leave blank for a general note.</div>
<textarea id="comment-input" placeholder="Type a comment, then Ctrl+Enter to save"></textarea>
<div class="actions">
<button type="button" id="save-comment" class="primary">Save comment</button>
<button type="button" id="cancel-comment" class="secondary">Clear</button>
</div>
</section>
<section id="comments-section">
<div class="label">Saved comments</div>
<ol id="comments-list" aria-live="polite"></ol>
</section>
<section id="general-section">
<label for="typed-notes" class="label">Overall notes</label>
<textarea id="typed-notes" placeholder="Optional notes that aren't tied to a single element"></textarea>
</section>
<section id="verdict-section">
<div class="label">Verdict</div>
<div id="verdict-buttons" role="group" aria-label="Verdict">
<button type="button" data-verdict="" aria-pressed="true" class="active">No verdict</button>
<button type="button" data-verdict="lgtm" aria-pressed="false">LGTM</button>
<button type="button" data-verdict="changes" aria-pressed="false">CHANGES</button>
<button type="button" data-verdict="reject" aria-pressed="false">REJECT</button>
<button type="button" data-verdict="question" aria-pressed="false">QUESTION</button>
</div>
</section>
<section id="actions-section">
<button type="button" id="send-revision">Send feedback for revision</button>
<p class="action-hint">Keeps this review open for another round.</p>
<button type="button" id="submit-review" class="primary">Finish review</button>
<p class="action-hint">Sends your feedback and closes this review.</p>
<div id="finish-confirmation" hidden>
<p>Finish this review? You won’t be able to send more feedback afterward.</p>
<button type="button" id="confirm-finish" class="primary">Send and finish review</button>
<button type="button" id="keep-reviewing">Keep reviewing</button>
</div>
<div id="result-message" role="alert"></div>
<button type="button" id="retry-send" hidden>Retry sending</button>
</section>
<footer>
<div class="hint">
<strong>Keys:</strong>
<kbd>Tab</kbd> walk elements ·
<kbd>Enter</kbd> on element opens comment ·
<kbd>Ctrl+Enter</kbd> save ·
<kbd>Esc</kbd> clear ·
<kbd>Ctrl+Shift+S</kbd> finish confirmation
</div>
</footer>
</aside>
<script src="/assets/diagram/mermaid.min.js"></script>
<script src="/assets/diagram/elk.bundled.js"></script>
<script>{bootstrap_js}</script>
<script>{review_js}</script>
</body>
</html>"#,
        css = BROWSER_CSS,
        bootstrap_js = super::BOOTSTRAP_JS,
        review_js = REVIEW_JS,
        content = content,
        session_id = session_id,
    )
}

const BROWSER_CSS: &str = r#"
:root {
    --bg: #f4f4f5;
    --panel: #ffffff;
    --border: #d4d4d8;
    --accent: #2563eb;
    --accent-bg: #eff5ff;
    --accent-soft: #dbeafe;
    --danger: #b91c1c;
    --ok: #047857;
    --text: #111;
    --muted: #6b7280;
    --comment: #f97316;
    --comment-bg: #fff7ed;
}
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; height: 100%; background: var(--bg); color: var(--text); font-family: ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif; }
body { display: grid; grid-template-columns: 1fr 380px; min-height: 100vh; }

#content {
    min-width: 0;
    background: #fff;
    padding: 32px 56px 96px;
    max-width: 920px;
    margin: 0 auto;
    width: 100%;
    font-family: Georgia, "Times New Roman", serif;
    font-size: 18px;
    line-height: 1.7;
    border-right: 1px solid var(--border);
}
#content > *:first-child { margin-top: 0; }
#content h1 { font-size: 30px; margin: 28px 0 12px; border-bottom: 2px solid #111; padding-bottom: 6px; }
#content h2 { font-size: 24px; margin: 24px 0 10px; }
#content h3 { font-size: 20px; margin: 18px 0 8px; }
#content h4 { font-size: 17px; margin: 16px 0 6px; }
#content p { margin: 12px 0; }
#content ul, #content ol { margin: 12px 0; padding-left: 28px; }
#content li { margin: 4px 0; }
#content code { font-family: ui-monospace, "SFMono-Regular", Menlo, Consolas, monospace; font-size: 0.9em; background: #f3f4f6; padding: 2px 6px; border-radius: 4px; }
#content pre { background: #0f172a; color: #e5e7eb; padding: 14px 18px; border-radius: 6px; overflow-x: auto; font-size: 14px; line-height: 1.55; }
#content pre code { background: none; color: inherit; padding: 0; font-size: inherit; }
#content blockquote { border-left: 4px solid #93c5fd; padding-left: 14px; margin: 16px 0; color: #374151; }
#content table { border-collapse: collapse; margin: 16px 0; }
#content th, #content td { border: 1px solid #d1d5db; padding: 8px 12px; text-align: left; }
#content th { background: #f9fafb; }
#content a { color: var(--accent); }
#content img { max-width: 100%; height: auto; }

[data-eink-idx] { cursor: pointer; transition: background 80ms ease, box-shadow 80ms ease; outline-offset: 4px; border-radius: 3px; }
[data-eink-idx]:hover { background: var(--accent-bg); }
[data-eink-idx]:focus-visible { outline: 2px solid var(--accent); }
[data-eink-idx].selected { background: var(--accent-bg); box-shadow: inset 4px 0 0 var(--accent); }
[data-eink-idx].has-comment { background: var(--comment-bg); box-shadow: inset 4px 0 0 var(--comment); }
[data-eink-idx].has-comment.selected { background: var(--accent-bg); box-shadow: inset 4px 0 0 var(--accent), inset 8px 0 0 var(--comment); }

#review-panel {
    background: var(--panel);
    position: sticky;
    top: 0;
    align-self: start;
    height: 100vh;
    overflow-y: auto;
    padding: 18px 18px 24px;
    display: flex; flex-direction: column; gap: 16px;
    font-size: 14px;
}
#review-panel header { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
#review-panel header h2 { margin: 0; font-size: 16px; font-weight: 600; }
#review-panel #status { color: var(--muted); font-size: 12px; }
#review-panel section { display: flex; flex-direction: column; }
#review-panel .label { font-size: 11px; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); margin-bottom: 6px; font-weight: 600; }
#review-panel textarea { width: 100%; min-height: 72px; resize: vertical; padding: 8px 10px; border: 1px solid var(--border); border-radius: 4px; font: inherit; background: #fff; }
#review-panel textarea:focus { outline: 2px solid var(--accent); outline-offset: -1px; border-color: var(--accent); }
#review-panel .actions { display: flex; gap: 8px; margin-top: 8px; }
#review-panel button { min-height: 44px; padding: 6px 12px; border: 1px solid var(--border); background: #fff; border-radius: 4px; cursor: pointer; font: inherit; line-height: 1.2; }
#review-panel button:focus-visible { outline: 3px solid var(--accent); outline-offset: 2px; }
[hidden] { display: none !important; }
#review-status { border: 2px solid var(--border); border-radius: 6px; padding: 12px; gap: 6px; background: #f9fafb; }
#review-status[data-state="error"] { border-color: var(--danger); }
#review-status[data-state="sent"] { border-color: var(--ok); }
#draft-status, #connection-status, .action-hint { font-size: 12px; line-height: 1.5; }
.action-hint { margin: 0 0 8px; color: #4b5563; }
#finish-confirmation { padding: 12px; border: 2px solid #111; border-radius: 6px; }
#finish-confirmation p { margin-top: 0; }
#finish-confirmation button + button { margin-top: 8px; }
#review-panel button:hover:not(:disabled) { background: #f3f4f6; }
#review-panel button:disabled { opacity: 0.5; cursor: not-allowed; }
#review-panel button.primary { background: var(--accent); color: #fff; border-color: var(--accent); }
#review-panel button.primary:hover:not(:disabled) { background: #1d4ed8; }
#review-panel button.secondary { background: #f3f4f6; }

#verdict-buttons { display: flex; flex-wrap: wrap; gap: 6px; }
#verdict-buttons button { padding: 4px 10px; font-size: 12px; }
#verdict-buttons button.active { background: #111; color: #fff; border-color: #111; }

#comments-list { list-style: none; padding: 0; margin: 0; max-height: 320px; overflow-y: auto; }
#comments-list:empty::after { content: "No comments yet."; color: var(--muted); font-size: 12px; }
#comments-list li { padding: 10px 0; border-bottom: 1px solid #f1f5f9; display: flex; flex-direction: column; gap: 4px; }
#comments-list li:last-child { border-bottom: none; }
#comments-list .preview { color: var(--muted); font-size: 11px; text-transform: uppercase; letter-spacing: 0.04em; display: flex; gap: 6px; align-items: center; }
#comments-list .preview .tag { background: var(--accent-soft); color: #1d4ed8; padding: 1px 6px; border-radius: 3px; font-family: ui-monospace, monospace; }
#comments-list .preview .text { font-family: Georgia, serif; text-transform: none; letter-spacing: 0; color: #374151; flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
#comments-list .body { font-size: 13px; white-space: pre-wrap; word-wrap: break-word; }
#comments-list .row-actions { display: flex; gap: 6px; }
#comments-list .row-actions button { padding: 2px 8px; font-size: 11px; }

#selected-preview { padding: 8px 10px; background: #f9fafb; border: 1px dashed var(--border); border-radius: 4px; margin-bottom: 8px; font-size: 13px; min-height: 38px; line-height: 1.4; word-break: break-word; }
#selected-preview .tag { font-family: ui-monospace, monospace; color: #1d4ed8; background: var(--accent-soft); padding: 1px 6px; border-radius: 3px; margin-right: 6px; }
#selected-preview.muted { color: var(--muted); }
#selected-preview.active { background: var(--accent-bg); border-color: var(--accent); border-style: solid; color: var(--text); }

#result-message { margin-top: 8px; min-height: 18px; font-size: 13px; }
#result-message.ok { color: var(--ok); }
#result-message.error { color: var(--danger); }

#review-panel footer { margin-top: auto; color: var(--muted); font-size: 11px; line-height: 1.5; padding-top: 8px; border-top: 1px solid #f1f5f9; }
#review-panel kbd { background: #f3f4f6; border: 1px solid var(--border); border-bottom-width: 2px; border-radius: 3px; padding: 1px 5px; font-family: ui-monospace, monospace; font-size: 10px; }

#actions-section { gap: 6px; }
#actions-section button { width: 100%; padding: 8px 12px; }

@media (max-width: 1100px) {
    body { grid-template-columns: 1fr; }
    #content { border-right: none; border-bottom: 1px solid var(--border); }
    #review-panel { position: static; height: auto; }
}
@media (max-width: 600px) {
    #content { padding: 24px 20px 40px; }
    #review-panel { padding: 20px; }
}
"#;

const REVIEW_JS: &str = r#"
(function() {
    var SELECTOR = 'h1, h2, h3, h4, h5, h6, p, li, blockquote, pre, table, .diagram-block';
    var sessionId = document.body.dataset.sessionId;
    var draftKey = 'eink-review-draft:' + sessionId;
    var busy = false;
    var terminal = false;
    var retryAction = null;
    var retryKind = null;
    var sentSignature = null;
    var localSaveFailed = false;
    var documentText = document.getElementById('content').innerHTML;
    var state = {
        elements: [],
        selectedIdx: null,
        annotations: [],
        verdict: ''
    };

    function slugify(s) {
        return 's-' + s.trim().toLowerCase().replace(/[^\w\s-]/g, '').replace(/\s+/g, '-').replace(/-+/g, '-').replace(/^-|-$/g, '');
    }

    function buildIndex() {
        var content = document.getElementById('content');
        if (!content) return;
        content.querySelectorAll('h1, h2, h3, h4, h5, h6').forEach(function(h) {
            if (!h.id) h.id = slugify(h.textContent || '');
        });
        var currentSection = null;
        content.querySelectorAll(SELECTOR).forEach(function(el, i) {
            var tag = el.tagName;
            if (tag === 'H1' || tag === 'H2' || tag === 'H3') currentSection = el.id || null;
            el.dataset.einkIdx = String(i);
            el.tabIndex = 0;
            state.elements.push({
                el: el,
                tag: tag,
                sectionId: currentSection,
                text: (el.textContent || '').replace(/\s+/g, ' ').trim().slice(0, 240)
            });
        });
    }

    function clearSelectionMarkers() {
        state.elements.forEach(function(e) { e.el.classList.remove('selected'); });
    }

    function selectElement(idx, opts) {
        if (busy || terminal) return;
        opts = opts || {};
        clearSelectionMarkers();
        if (idx === null || idx < 0 || idx >= state.elements.length) {
            state.selectedIdx = null;
            updateSelectedPreview();
            return;
        }
        state.selectedIdx = idx;
        var entry = state.elements[idx];
        entry.el.classList.add('selected');
        updateSelectedPreview();
        if (opts.focusInput !== false) document.getElementById('comment-input').focus();
        if (opts.scroll !== false) {
            var rect = entry.el.getBoundingClientRect();
            if (rect.top < 0 || rect.bottom > window.innerHeight) {
                entry.el.scrollIntoView({behavior: 'smooth', block: 'center'});
            }
        }
    }

    function updateSelectedPreview() {
        var el = document.getElementById('selected-preview');
        if (state.selectedIdx === null) {
            el.textContent = 'No element selected — comment will be a general note.';
            el.classList.add('muted');
            el.classList.remove('active');
        } else {
            var entry = state.elements[state.selectedIdx];
            el.innerHTML = '';
            var tagSpan = document.createElement('span');
            tagSpan.className = 'tag';
            tagSpan.textContent = entry.tag.toLowerCase();
            var textSpan = document.createElement('span');
            textSpan.textContent = entry.text || '(empty)';
            el.appendChild(tagSpan);
            el.appendChild(textSpan);
            el.classList.remove('muted');
            el.classList.add('active');
        }
    }

    function saveComment() {
        if (busy || terminal) return;
        var input = document.getElementById('comment-input');
        var comment = input.value.trim();
        if (!comment) { input.focus(); return; }
        var entry = state.selectedIdx !== null ? state.elements[state.selectedIdx] : null;
        var ann = {
            id: 'a' + Date.now() + '-' + Math.random().toString(36).slice(2, 6),
            comment: comment,
            elementIdx: state.selectedIdx,
            elementInfo: entry ? {
                section_id: entry.sectionId,
                tag: entry.tag.toLowerCase(),
                text: entry.text
            } : null
        };
        state.annotations.push(ann);
        if (entry) entry.el.classList.add('has-comment');
        input.value = '';
        clearSelectionMarkers();
        state.selectedIdx = null;
        updateSelectedPreview();
        renderComments();
        updateStatus();
        draftChanged();
    }

    function deleteAnnotation(id) {
        if (busy || terminal) return;
        var i = state.annotations.findIndex(function(a) { return a.id === id; });
        if (i < 0) return;
        var ann = state.annotations[i];
        state.annotations.splice(i, 1);
        if (ann.elementIdx !== null) {
            var stillHas = state.annotations.some(function(a) { return a.elementIdx === ann.elementIdx; });
            if (!stillHas) state.elements[ann.elementIdx].el.classList.remove('has-comment');
        }
        renderComments();
        updateStatus();
        draftChanged();
    }

    function renderComments() {
        var list = document.getElementById('comments-list');
        list.innerHTML = '';
        state.annotations.forEach(function(a) {
            var li = document.createElement('li');

            var preview = document.createElement('div');
            preview.className = 'preview';
            var tag = document.createElement('span');
            tag.className = 'tag';
            tag.textContent = a.elementInfo ? a.elementInfo.tag : 'note';
            var prevText = document.createElement('span');
            prevText.className = 'text';
            prevText.textContent = a.elementInfo ? (a.elementInfo.text || '(empty)') : 'general note';
            preview.appendChild(tag);
            preview.appendChild(prevText);

            var body = document.createElement('div');
            body.className = 'body';
            body.textContent = a.comment;

            var actions = document.createElement('div');
            actions.className = 'row-actions';
            if (a.elementIdx !== null) {
                var jumpBtn = document.createElement('button');
                jumpBtn.type = 'button';
                jumpBtn.textContent = 'Show';
                jumpBtn.addEventListener('click', function() { selectElement(a.elementIdx, {focusInput: false}); });
                actions.appendChild(jumpBtn);
            }
            var delBtn = document.createElement('button');
            delBtn.type = 'button';
            delBtn.textContent = 'Delete';
            delBtn.addEventListener('click', function() { deleteAnnotation(a.id); });
            actions.appendChild(delBtn);

            li.appendChild(preview);
            li.appendChild(body);
            li.appendChild(actions);
            list.appendChild(li);
        });
    }

    function updateStatus() {
        var s = document.getElementById('status');
        var n = state.annotations.length;
        var bits = [n + ' comment' + (n === 1 ? '' : 's')];
        if (state.verdict) bits.push('verdict: ' + state.verdict);
        s.textContent = bits.join(' · ');
    }

    function payloadAnnotations() {
        return state.annotations.map(function(a) {
            var group = { strokes: [], recognized_text: a.comment };
            if (a.elementInfo) {
                group.anchor = { type: 'explicit', elements: [a.elementInfo] };
            } else {
                group.anchor = null;
            }
            return group;
        });
    }

    function showResult(msg, ok) {
        var el = document.getElementById('result-message');
        el.textContent = msg;
        el.classList.toggle('ok', ok === true);
        el.classList.toggle('error', ok === false);
    }

    function setBusy(busy) {
        document.getElementById('review-panel').setAttribute('aria-busy', String(busy));
        document.querySelectorAll('#review-panel button, #review-panel textarea').forEach(function(el) {
            el.disabled = busy || terminal;
        });
    }

    function setDelivery(message, status) {
        document.getElementById('delivery-status').textContent = message;
        document.getElementById('review-status').dataset.state = status || 'draft';
    }

    function persistDraft() {
        if (terminal) return;
        try {
            localStorage.setItem(draftKey, JSON.stringify({
                documentText: documentText,
                annotations: state.annotations,
                selectedIdx: state.selectedIdx,
                comment: document.getElementById('comment-input').value,
                notes: document.getElementById('typed-notes').value,
                verdict: state.verdict,
                sentSignature: sentSignature,
                delivery: document.getElementById('delivery-status').textContent,
                deliveryState: document.getElementById('review-status').dataset.state,
                error: document.getElementById('result-message').textContent,
                retryKind: retryKind
            }));
            localSaveFailed = false;
            document.getElementById('draft-status').textContent = 'Saved on this device — available after reload.';
        } catch (e) {
            localSaveFailed = true;
            document.getElementById('draft-status').textContent = 'Local save unavailable. Keep this tab open to avoid losing feedback.';
        }
    }

    function draftChanged() {
        if (busy || terminal) return;
        setDelivery('Draft — changes not sent');
        persistDraft();
        document.getElementById('finish-confirmation').hidden = true;
    }

    function restoreDraft() {
        try {
            var draft = JSON.parse(localStorage.getItem(draftKey) || 'null');
            if (!draft) return;
            if (!Array.isArray(draft.annotations) || !draft.annotations.every(function(a) {
                return a && typeof a.id === 'string' && typeof a.comment === 'string'
                    && (a.elementIdx === null || Number.isInteger(a.elementIdx));
            })) throw new Error('Invalid local draft');
            var changed = draft.documentText !== documentText;
            state.annotations = draft.annotations || [];
            // Never silently attach an old comment to a different document element.
            state.annotations.forEach(function(a) {
                if (changed || !state.elements[a.elementIdx]) {
                    if (changed && a.elementInfo) a.comment = 'Regarding “' + a.elementInfo.text + '”:\n' + a.comment;
                    a.elementIdx = null;
                    a.elementInfo = null;
                } else {
                    state.elements[a.elementIdx].el.classList.add('has-comment');
                }
            });
            document.getElementById('comment-input').value = draft.comment || '';
            document.getElementById('typed-notes').value = draft.notes || '';
            state.verdict = draft.verdict || '';
            selectElement(changed || !Number.isInteger(draft.selectedIdx) ? null : draft.selectedIdx, {focusInput: false, scroll: false});
            document.querySelectorAll('#verdict-buttons button').forEach(function(b) {
                b.classList.toggle('active', b.dataset.verdict === state.verdict);
                b.setAttribute('aria-pressed', String(b.dataset.verdict === state.verdict));
            });
            renderComments();
            document.getElementById('draft-status').textContent = 'Draft restored from this device.';
            if (!changed) {
                sentSignature = draft.sentSignature || null;
                if (draft.delivery) setDelivery(draft.delivery, draft.deliveryState);
                if (draft.retryKind) {
                    retryKind = draft.retryKind;
                    retryAction = retryKind === 'finish' ? submitReview : sendForRevision;
                    document.getElementById('retry-send').hidden = false;
                    showResult(draft.error || 'Delivery was not confirmed. Your draft is safe on this device.', false);
                }
            } else {
                showResult('Document changed. Your draft was restored; previous comments are now general notes. Check their targets before sending.', false);
            }
        } catch (e) {
            showResult('Could not restore the saved draft. The original stored copy has not been replaced.', false);
        }
    }

    function updateConnection() {
        document.getElementById('connection-status').textContent = navigator.onLine
            ? 'Browser online — server connection checked when sending.'
            : 'Offline — feedback stays on this device until you retry.';
    }

    function postJSON(url, body) {
        var controller = new AbortController();
        var timeout = setTimeout(function() { controller.abort(); }, 20000);
        return fetch(url, {
            method: 'POST',
            headers: {'Content-Type': 'application/json'},
            body: JSON.stringify(body),
            signal: controller.signal
        }).finally(function() { clearTimeout(timeout); });
    }

    function sendForRevision() {
        sendFeedback(false);
    }

    function submitReview() {
        if (busy || terminal) return;
        document.getElementById('finish-confirmation').hidden = false;
        document.getElementById('confirm-finish').focus();
    }

    async function sendFeedback(finish) {
        if (busy || terminal) return;
        if (document.getElementById('comment-input').value.trim()) saveComment();
        var payload = {
            schema_version: 1,
            document_version: Number((document.querySelector('meta[name=eink-version]') || {}).content) || null,
            typed_notes: document.getElementById('typed-notes').value,
            annotations: payloadAnnotations()
        };
        if (finish) payload.verdict = state.verdict || null;
        if (!finish && payload.typed_notes.trim()) {
            // request_update forwards annotations, not typed_notes, to revision consumers.
            payload.annotations.push({strokes: [], recognized_text: payload.typed_notes.trim(), anchor: null});
        }
        if (!finish && payload.annotations.length === 0) {
            showResult('Add a comment or overall notes before requesting a revision.', false);
            return;
        }
        var signature = JSON.stringify(payload);
        if (!finish && signature === sentSignature) {
            setDelivery('Feedback already sent — waiting for an updated document', 'sent');
            return;
        }
        persistDraft();
        busy = true;
        setBusy(true);
        retryAction = null;
        retryKind = null;
        document.getElementById('retry-send').hidden = true;
        document.getElementById('finish-confirmation').hidden = true;
        showResult('', null);
        setDelivery(finish ? 'Sending and finishing review…' : 'Sending feedback…', 'sending');
        try {
            var r = await postJSON('/api/sessions/' + sessionId + (finish ? '/submit' : '/request_update'), payload);
            if (!r.ok) {
                if (r.status === 404 || r.status === 409 || r.status === 410) {
                    terminal = true;
                    setDelivery('Review unavailable — feedback kept on this device', 'error');
                    showResult('This review no longer accepts feedback. Your feedback remains in this tab; check the local save status before closing it.', false);
                    return;
                }
                throw new Error('Server returned HTTP ' + r.status + '.');
            }
            if (finish) {
                terminal = true;
                setDelivery('Review finished — feedback sent', 'sent');
                document.getElementById('draft-status').textContent = 'Feedback accepted by the server. You can close this tab.';
                try { localStorage.removeItem(draftKey); } catch (e) { /* Sent successfully even if storage is unavailable. */ }
            } else {
                sentSignature = signature;
                setDelivery('Feedback sent — waiting for an updated document', 'sent');
                showResult('Review stays open. Your feedback is also kept on this device.', true);
                persistDraft();
            }
        } catch (e) {
            setDelivery(localSaveFailed ? 'Delivery not confirmed — keep this tab open' : 'Delivery not confirmed — draft kept', 'error');
            showResult((e.name === 'AbortError' ? 'The request timed out.' : e.message) + ' The server may have received it. Retry when connected.', false);
            retryAction = finish ? submitReview : sendForRevision;
            retryKind = finish ? 'finish' : 'revision';
            document.getElementById('retry-send').hidden = false;
            persistDraft();
        } finally {
            busy = false;
            setBusy(false);
            updateConnection();
        }
    }

    function setupEventHandlers() {
        var content = document.getElementById('content');
        content.addEventListener('click', function(ev) {
            if (ev.target.closest('a')) return;
            var t = ev.target;
            while (t && t !== content && !t.dataset.einkIdx) t = t.parentElement;
            if (t && t.dataset && t.dataset.einkIdx) {
                ev.preventDefault();
                selectElement(parseInt(t.dataset.einkIdx, 10));
            }
        });
        content.addEventListener('keydown', function(ev) {
            var t = document.activeElement;
            if (!t || !t.dataset || !t.dataset.einkIdx) return;
            if (ev.key === 'Enter' && !ev.shiftKey && !ev.ctrlKey && !ev.metaKey) {
                ev.preventDefault();
                selectElement(parseInt(t.dataset.einkIdx, 10));
            }
        });
        document.getElementById('save-comment').addEventListener('click', saveComment);
        document.getElementById('cancel-comment').addEventListener('click', function() {
            document.getElementById('comment-input').value = '';
            selectElement(null);
            draftChanged();
        });
        document.getElementById('comment-input').addEventListener('keydown', function(ev) {
            if (ev.key === 'Enter' && (ev.ctrlKey || ev.metaKey)) {
                ev.preventDefault();
                saveComment();
            } else if (ev.key === 'Escape') {
                ev.preventDefault();
                this.value = '';
                selectElement(null);
                draftChanged();
            }
        });
        document.getElementById('verdict-buttons').addEventListener('click', function(ev) {
            var btn = ev.target.closest('button');
            if (!btn) return;
            var v = btn.dataset.verdict || '';
            state.verdict = v;
            this.querySelectorAll('button').forEach(function(b) {
                var active = (b.dataset.verdict || '') === v;
                b.classList.toggle('active', active);
                b.setAttribute('aria-pressed', String(active));
            });
            updateStatus();
            draftChanged();
        });
        ['comment-input', 'typed-notes'].forEach(function(id) {
            document.getElementById(id).addEventListener('input', draftChanged);
        });
        document.getElementById('confirm-finish').addEventListener('click', function() { sendFeedback(true); });
        document.getElementById('keep-reviewing').addEventListener('click', function() {
            document.getElementById('finish-confirmation').hidden = true;
            document.getElementById('submit-review').focus();
        });
        document.getElementById('retry-send').addEventListener('click', function() { if (retryAction) retryAction(); });
        window.addEventListener('online', updateConnection);
        window.addEventListener('offline', updateConnection);
        window.addEventListener('beforeunload', function(ev) {
            if (busy || (localSaveFailed && !terminal)) { ev.preventDefault(); ev.returnValue = ''; }
        });
        document.getElementById('send-revision').addEventListener('click', sendForRevision);
        document.getElementById('submit-review').addEventListener('click', submitReview);
        document.addEventListener('keydown', function(ev) {
            if ((ev.ctrlKey || ev.metaKey) && ev.shiftKey && ev.key.toLowerCase() === 's') {
                ev.preventDefault();
                submitReview();
            }
        });
    }

    document.addEventListener('DOMContentLoaded', function() {
        buildIndex();
        setupEventHandlers();
        restoreDraft();
        updateStatus();
        updateSelectedPreview();
        updateConnection();
        fetch('/api/sessions/' + sessionId).then(function(r) {
            if (!r.ok) return null;
            return r.json();
        }).then(function(session) {
            if (!session || session.status === 'Active' || busy || terminal) return;
            terminal = true;
            setDelivery('Review closed — ' + session.status.toLowerCase(), 'sent');
            document.getElementById('draft-status').textContent = 'Any local draft is kept on this device. This review no longer accepts feedback.';
            setBusy(false);
        }).catch(function() {
            document.getElementById('connection-status').textContent = 'Server unreachable — you can keep drafting on this device.';
        });
    });
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_session_id_and_panel_scaffold() {
        let html = to_browser_html("# Hello\n\nfirst paragraph", "abc123");
        assert!(html.contains(r#"data-session-id="abc123""#));
        assert!(html.contains(r#"<aside id="review-panel""#));
        assert!(html.contains(r#"id="comment-input""#));
        assert!(html.contains(r#"id="submit-review""#));
        assert!(html.contains("<h1>Hello</h1>"));
        assert!(html.contains("first paragraph"));
    }

    #[test]
    fn includes_diagram_assets() {
        let html = to_browser_html("text", "id");
        assert!(html.contains("/assets/diagram/mermaid.min.js"));
        assert!(html.contains("/assets/diagram/elk.bundled.js"));
    }

    #[test]
    fn includes_verdict_buttons() {
        let html = to_browser_html("text", "id");
        for v in ["lgtm", "changes", "reject", "question"] {
            assert!(html.contains(&format!(r#"data-verdict="{v}""#)));
        }
    }

    #[test]
    fn includes_persistent_status_and_explicit_finish_confirmation() {
        let html = to_browser_html("# Review", "session");
        assert!(html.contains(r#"id="delivery-status" role="status" aria-live="polite""#));
        assert!(html.contains(r#"id="draft-status" role="status""#));
        assert!(html.contains(r#"id="result-message" role="alert""#));
        assert!(html.contains(r#"id="retry-send" hidden"#));
        assert!(html.contains(r#"id="finish-confirmation" hidden"#));
        assert!(html.contains("Send feedback for revision"));
        assert!(html.contains("Send and finish review"));
        assert!(html.contains("Keep reviewing"));
        assert!(html.contains(r#"aria-label="Verdict""#));
    }
}
