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
GET    /api/sessions/{id}/source?ref=path:line  lines around a code reference (see below)
POST   /api/sessions/{id}/submit                finish the review (JSON or multipart)
POST   /api/sessions/{id}/request_update        send ink + OCR to the agent without finishing
GET    /ws/{id}                                 WebSocket: content updates, feedback, submitted/cancelled
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
