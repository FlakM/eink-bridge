# API

Machine-readable contract sources:

- `GET /api/openapi.json`
- `server/src/api.rs`

## Endpoints

```text
GET    /api/health
GET    /api/openapi.json
POST   /api/sessions
GET    /api/sessions
GET    /api/sessions/{id}
DELETE /api/sessions/{id}
GET    /api/sessions/{id}/result
POST   /api/sessions/{id}/submit
GET    /session/{id}
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
- `annotation`: repeated PNG file part

The Android app uses multipart. AI clients can use JSON.

## Webhooks

If `callback_url` is set at session creation time, terminal state changes POST the same structured payload shape used by `GET /api/sessions/{id}/result`.

## Tags

`tags` are arbitrary string key-value pairs stored with the session and returned by list, get, result, and webhook payloads.
