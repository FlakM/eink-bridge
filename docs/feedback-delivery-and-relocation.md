# Feedback delivery and ink relocation

## Delivery contract

- `POST /api/sessions/{id}/request_update` stores the recognized annotation batch
  before broadcasting it. A successful HTTP response means **saved on the server**,
  not that an agent has read or answered it.
- Each batch has a `feedback_id` and source document `version`. A client can send
  `document_version` to reject feedback authored against an outdated page.
- `GET /api/sessions/{id}/feedback` exposes the current version, pending batch and
  latest content-update instructions. WebSocket subscribers receive snapshots on
  connection; duplicates are possible. Consumers deduplicate by `feedback_id`.
- A JSON content update can supply `expected_version`, `feedback_id`, and
  `consumed_annotations`. The latter are **zero-based annotation array indices**,
  never native bind-group IDs. Matching feedback is acknowledged; ordinary edits
  leave it pending. Stale rewrites return 409.
- The server resolves consumed annotations into exact `consumed_strokes` in
  `version_updated`, including ordinary handwriting without explicit links.
- OpenCode uses a one-event background watcher; rearm after each handled event.
  Claude Code can use the same watcher or the MCP channel. Channel subscription
  fetches pending feedback to recover callbacks received before subscription.

The retained state is the latest request, not an unlimited event journal. A new
request replaces the pending snapshot with the current visible annotations.

## Relocation contract

- The native client persists the rendered element map alongside local ink. On page
  load it submits previous/current `{i, tag, text}` maps to `POST /reanchor`.
  Client-rendered geometry is necessary: font size, viewport, wrapping, and diagrams
  determine actual coordinates, which the server cannot infer from markdown alone.
- The server returns a target element index (or null) for each previous element.
  It matches normalized exact text first, then conservative edit-distance similarity
  for small edits. Positional section IDs are not identity. Ambiguous/deleted targets
  remain unresolved. An unchanged full text sequence allows pure reflow even with
  repeated labels.
- The client translates whole free-stroke clusters or explicit groups by their
  anchor's top-left displacement. Links, marker positions, and stroke indices are
  updated together; stale OCR coordinates are cleared and recognition rescheduled.
- Ink is hidden and input paused until reconciliation finishes. Failed relocation
  retains the previous snapshot for retry. Unresolved ink is removed from the active
  overlay only after an original-copy recovery archive is written to the app's
  `eink_bridge` preferences (`archive_<session>_<timestamp>`).
- Legacy ink without a saved source map cannot safely be relocated and follows the
  same archival path. Archives currently have no in-app restore UI.
- WebSocket reconnects replay the latest version update. Multiple missed content
  updates are not an event history; exact retirement instructions cover the latest
  update, while retained ink is matched directly from the last saved layout.

## Regression checks

```sh
just test-one --test=ws_test
just test-android
just test-delivery
just eval
```

Delivery tests exercise the portable watcher subprocess and the actual Claude MCP
stdio protocol. They do not claim a live end-to-end LLM run in both agent products.
