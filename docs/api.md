# API

Machine-readable contract sources:

- `GET /api/openapi.json`
- `server/src/api.rs`

## Endpoints

```text
GET    /api/health                              readiness block; 503 when the state dir cannot be written
GET    /metrics                                 Prometheus text format
GET    /api/openapi.json
POST   /api/ocr                                 strokes -> text; 503 without an OCR engine
POST   /api/sessions                            create (JSON or text/plain)
GET    /api/sessions                            list; ?status= ?starred= ?limit= ?cursor=, x-next-cursor header
DELETE /api/sessions                            purge finished, unstarred sessions
GET    /api/sessions/{id}
DELETE /api/sessions/{id}                       cancel; ?remove=true deletes files, &force=true also for Active
GET    /api/sessions/{id}/result                long-poll (see below)
GET    /api/sessions/{id}/content               current markdown and version
PUT    /api/sessions/{id}/content               replace content; expected_version -> 409 on a stale writer
POST   /api/sessions/{id}/reanchor              map paragraphs of an old element map onto the new one
GET    /api/sessions/{id}/feedback              unacknowledged annotation feedback
PUT    /api/sessions/{id}/star                  pin a session so purge and expiry leave it alone
GET    /api/sessions/{id}/annotation/{index}    uploaded annotation PNG
GET    /api/sessions/{id}/image/{name}          a picture the document displays
POST   /api/sessions/{id}/image/{name}          attach a picture to the document
GET    /api/sessions/{id}/source?ref=path:line  lines around a code reference (see below)
POST   /api/sessions/{id}/screenshot            ask the tablet for a picture of what it shows (see below)
GET    /api/sessions/{id}/screenshot/{request}  the picture; 404 until it arrives, 410 if it never will
PUT    /api/sessions/{id}/screenshot/{request}  the tablet's answer: a PNG, or {"error"} as JSON
GET    /api/sessions/{id}/voice                 WebSocket: the tablet speaks a note (see below)
GET    /api/sessions/{id}/voice/{capture}.wav   what the reader said, kept with the session
POST   /api/sessions/{id}/speech                read text aloud: {"text"} in, audio/wav out
POST   /api/sessions/{id}/submit                finish the review (JSON or multipart)
POST   /api/sessions/{id}/request_update        send ink + OCR to the agent without finishing
GET    /ws/{id}                                 WebSocket: content updates, feedback, submitted/cancelled;
                                                ?client=tablet marks the reader's device (see Screenshots)
GET    /session/{id}                            rendered page for the tablet
GET    /review/{id}                             rendered page for a browser
```

## Create Session

Preferred AI-native request:

```json
{
  "schema_version": 1,
  "title": "Review plan",
  "content": "# Review plan\n\nCheck the flow.",
  "callback_url": "http://localhost:8787/hook",
  "tags": {
    "type": "code-review",
    "repo": "eink-bridge"
  }
}
```

Legacy compatibility is still supported:

- `text/plain` body with markdown
- optional query params: `?title=...&callback_url=...`

Response:

```json
{
  "schema_version": 1,
  "id": "abcd1234ef56",
  "url": "/session/abcd1234ef56"
}
```

## Result Schema

Submitted sessions return:

```json
{
  "schema_version": 1,
  "id": "abcd1234ef56",
  "title": "Review plan",
  "status": "Submitted",
  "created_at": "2026-03-30T10:00:00Z",
  "updated_at": "2026-03-30T10:02:22Z",
  "verdict": "changes",
  "typed_notes": "CHANGES: simplify the loop",
  "annotation_images": ["/tmp/.../img.png"],
  "has_annotation": true,
  "review_duration_seconds": 142,
  "tags": {
    "type": "code-review"
  }
}
```

Long-poll behavior:

- `200`: submitted result available
- `204`: still active, no change yet
- `404`: unknown session
- `410`: cancelled or expired session

A session expires `session_timeout` after its last change, not after its creation. Starred
sessions never expire.

## Error codes shared by mutating endpoints

- `409`: a stale writer (`expected_version` or `document_version` does not match), or a
  second submit
- `413`: more than 5000 strokes or 500 000 points in one OCR or update request
- `500`: the change could not be written to disk; nothing was acknowledged
- `503`: relocating notes (`reanchor`) exceeded its 10 s budget; keep the notes where they
  are

## Submit Review

The server accepts two formats on `POST /api/sessions/{id}/submit`.

JSON submission:

```json
{
  "schema_version": 1,
  "typed_notes": "LGTM: ship it",
  "verdict": "lgtm"
}
```

Multipart submission:

- `typed_notes`: text field
- `verdict`: `lgtm` or `changes`
- `document_version`: the version the ink was drawn on; a mismatch is a 409
- `annotation`: repeated PNG file part (must be a PNG; anything else is a 400)
- `annotations`: JSON array of annotation groups (strokes bound to elements, OCR text)
- `stroke_data`: raw stroke JSON

The Android app uses multipart. AI clients can use JSON. A second submit is a 409, and
nothing is written to disk until the session has accepted the submission.

The result carries both `annotation_images` (filesystem paths on the server, for a local
agent) and `annotation_urls` (relative URLs, for anything else).

## Document images

A brief may carry pictures — a screenshot of a rendered page, a sketch, a photo of a
whiteboard — not only diagrams. The markdown refers to one as a bare session path:

```markdown
![the error page as it renders today](image/error.png)

*Figure 1 — what the reader sees, not what the code says.*
```

The renderer resolves `image/<name>` to `/api/sessions/{id}/image/<name>`, so the same
content can be pushed as a new session or rewritten in place without knowing the id.

`POST /api/sessions/{id}/image/{name}` takes the raw bytes. The name is one path segment
of ASCII letters, digits, dot, dash or underscore with an extension from `.png`, `.jpg`,
`.jpeg`, `.webp`, `.gif`, and the bytes must actually be that format — a `.png` that is
not a PNG is a 400, and so is `script.svg`, which is a document that can carry script and
these images are served from the page's own origin. Only an active session accepts a new
image (409 otherwise). The answer is `{name, url, markdown}`.

`GET /api/sessions/{id}/image/{name}` serves it with its content type,
`X-Content-Type-Options: nosniff` and `Cache-Control: no-store` (a redrawn screenshot
reuses its name). Removing the session removes its pictures.

To avoid a page that renders without the picture and then reloads with it, the images can
travel in the create request itself as `multipart/form-data`: the same fields as the JSON
request plus a repeated `image` file part, whose file name is the `image/<name>` name. The
CLI does this for you:

```bash
eink-review push brief.md                      # ![x](shot.png) next to the brief is uploaded
eink-review push brief.md --image shot.png     # appended if the brief does not reference it
eink-review push --image shot.png --title "Rendered page"
eink-review image <session-id> sketch.png --alt "whiteboard sketch"   # prints the markdown
eink-review update <session-id> brief.md --image late.png
```

## Screenshots

The element map says what is on the page; only the tablet knows where the reader has
scrolled, how far in they are zoomed and what they have drawn over the text. An agent
that needs that asks for a picture:

```text
POST /api/sessions/{id}/screenshot
202 {"id": "<32 hex>", "url": "/api/sessions/{id}/screenshot/<id>", "delivered": true}
```

The server broadcasts `{"type": "screenshot_requested", "request_id": ...}` on the
session's WebSocket. The tablet draws the page with the ink over it and `PUT`s the PNG to
the `url`; the caller polls `GET` on it, which answers 404 until the picture is there.
`eink-review screenshot <id>` does the whole exchange and prints the file it wrote.

- `delivered` is true only when a tablet has the session open, meaning a WebSocket client
  that connected with `?client=tablet`. The agent's own observer shares the socket and does
  not count; a caller that gets `false` should say "no tablet is on this session" rather
  than wait.
- A repeat ask while the first is unanswered reuses its id (for up to 20 s), so a retry
  does not flash the reader twice. An answered or failed request is never reused: asking
  again after an answer takes a new picture.
- Only the request being waited for can be answered, and once. A `PUT` under any other id
  is 404, so nobody can fill a session with pictures nobody asked for.
- If the tablet cannot take the picture (the page has not drawn yet), it `PUT`s
  `{"error": "..."}` as `application/json`; the `GET` then answers 410 with the reason
  and the CLI fails at once instead of waiting out its timeout.
- The picture is stored under the session (`screenshots/<id>.png`), served with `nosniff`
  and `no-store`, and removed with the session.

## Voice

Touch a block, speak, and the words become a note on that block. The tablet records; the
server listens, cleans and keeps the audio; the page draws the card. The tablet opens
`GET /api/sessions/{id}/voice` as a WebSocket (with `?token=` on the LAN) and streams
16 kHz mono signed 16-bit PCM as binary frames. Text frames come back, each with a
rising `seq`:

```text
{"type":"ready","seq":0,"capture":"<32 hex>","stt":{"provider":"whisper","model":"small.en","language":"en"}}
{"type":"partial","seq":3,"text":"so um drop the cache"}
{"type":"final","seq":9,"raw":"so um drop the cache column","text":"Drop the cache column.",
 "seconds":6.2,"confidence":0.81,"repairs":[{"heard":"re-anchor rs","wrote":"reanchor.rs"}],
 "guarded":false,"audio":"/api/sessions/{id}/voice/<capture>.wav","severity":"note"}
{"type":"edit","seq":10,"instruction":"shorter","applied":true,"text":"...","severity":"note","edits":1}
{"type":"done","seq":11,"text":"...","severity":"note","note":{...}}
{"type":"error","seq":n,"message":"nothing was heard"}
```

- The capture closes itself after 1.5 s of silence following speech, on `{"type":"stop"}`,
  and at 300 s or 4 MB. Silence alone is never sent to the recogniser: it is an `error`,
  not an empty note.
- `text` is the transcript after the cleanup contract (fillers out, a self-correction keeps
  the correction, identifiers take the document's spelling, hedges stay, nothing is
  summarised). `guarded: true` means the cleaned text said a word the reader did not, so
  `text` is `raw` verbatim. `confidence` is the worst segment of the clip.
- After `final`, `{"type":"edit","instruction":...}` applies a closed verb at once:
  `shorter`, `longer`, `drop two`, `drop the last one`, `turn that into a list`, `make it
  a question`, `mark it a blocker`, `undo`. Anything else goes to the cleaner model and
  through the content-word guard, and comes back `applied: false` with a `message` when it
  is refused. `{"type":"undo"}`, `{"type":"done"}` and `{"type":"discard"}` do what they
  say; `done` answers with the `note` to put under `voice` in an annotation group:

```json
{"anchor":{"type":"explicit","elements":[...]},"strokes":[],"recognized_text":"Drop the cache column.",
 "source":"voice","voice":{"raw":"so um drop the cache column","edits":[],"repairs":[],
 "confidence":0.81,"stt":{"provider":"whisper","model":"small.en","language":"en"},
 "seconds":6.2,"audio":"/api/sessions/{id}/voice/<capture>.wav","severity":"note"}}
```

- `source` is `pen`, `tap` or `voice` (absent means pen). `recognized_text` stays the field
  an agent reads; `voice.raw` is there for when it reads wrong, and a low `confidence` is
  the cue to ask for a screenshot or a question before acting.
- One capture per session: a second upgrade is 409, like a second submit. A finished
  session is 409, no recogniser is 503.
- The audio is `voice/<capture>.wav` under the session, served with `nosniff` and
  `no-store`, and removed with the session; `discard` removes it at once.
- `POST /api/sessions/{id}/speech` with `{"text": "..."}` (at most 2000 characters)
  answers with the text read by the server's one voice as `audio/wav`; 503 when none is
  configured. Nothing is stored.

Server side, `EINK_ASR_URL` names the whisper server (default `http://127.0.0.1:8090`,
empty disables voice), `EINK_ASR_OPENAI_ENABLED=1` adds the hosted recogniser as a
fallback, `EINK_VOICE_LLM_URL`/`EINK_VOICE_LLM_MODEL` name the cleaner (default: the OCR
Ollama), and `EINK_PIPER_VOICE` (an `.onnx` voice, with `EINK_PIPER_BIN` when `piper` is
not on the service's PATH) enables speech.

## Source references

`GET /api/sessions/{id}/source?ref=src/app.rs:120&context=6` returns the lines around a
reference, rendered through the same highlighter as a snippet block. The root is the
checkout the session was created from, and it is only recorded when the creating request
came from the server's own host; a session created from the LAN has no root and the
endpoint answers 404. `EINK_SOURCE_ROOTS` (colon-separated) further limits where a
checkout may live.

## Webhooks

If `callback_url` is set at session creation time, terminal state changes (submitted,
cancelled, expired) POST the same payload shape used by `GET /api/sessions/{id}/result`.
A `request_update` POSTs an `annotation_result` payload: the feedback id, the document
version, and the annotation groups. The URL must be http or https; link-local addresses
are refused. Deliveries have a 10 s timeout and are counted in `eink_webhook_total`.

## Tags

`tags` are arbitrary string key-value pairs stored with the session and returned by list, get, result, and webhook payloads.
