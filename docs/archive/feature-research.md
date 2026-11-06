# Feature Research: eink-bridge

Codebase as of 2026-03-30. Focused on AI-driven development workflows where tight feedback loops and testability matter most.

---

## 1. Feature Ideas

### Server / API

#### 1.1 Webhook / callback mode for session completion

**What it does.** Accept an optional `callback_url` query parameter on `POST /api/sessions`. When the session is submitted or cancelled, POST the result JSON to that URL instead of — or in addition to — holding the long-poll connection open.

**Why it matters.** The current long-poll model requires the CLI to hold a TCP connection open for up to 30 minutes. Claude Code agents that submit many sessions in parallel burn threads and can time out at the HTTP layer. A webhook lets agents fire-and-forget the push, register a callback endpoint, and receive the result asynchronously — matching how modern CI/CD pipelines and LLM tool calls are structured.

**How to test.**
- Unit: store `callback_url` in `Session` struct; assert it serialises/deserialises through `session.json`.
- Integration (`tests/webhook_test.rs`): spin up a second axum `oneshot` handler that records POSTs, create a session with `?callback_url=http://localhost:{port}/hook`, call `submit_review`, assert the hook received a POST with `status=submitted` within 100ms. No real device, no sleep needed.
- Fast path: the entire test runs in < 50ms because it is in-process.

**Implementation sketch.** Add `callback_url: Option<String>` to `Session`. In `submit_review` handler (app.rs), after calling `state.notify_session`, spawn a `tokio::spawn` that issues `reqwest::Client::post(url).json(&result).send()`. Failures are logged but not fatal. No new files needed — changes are `session.rs`, `app.rs`, and `CreateParams`.

---

#### 1.2 Session tags / metadata

**What it does.** Allow callers to attach arbitrary key-value tags (e.g. `type=code-review`, `repo=eink-bridge`, `agent=claude-code`) to a session at creation time. Tags are returned in list and result responses.

**Why it matters.** When multiple agents or developers share a server, there is no way to route, filter, or post-process sessions by context. A code-review session should be distinguishable from a plan review or a prose draft. Tags also unlock dashboard views, analytics, and selective webhook routing without server-side schema changes.

**How to test.**
- Unit (`session.rs`): `create` takes `tags: HashMap<String, String>`; assert they round-trip through `persist`/`load_from_disk`.
- Integration: POST with `X-Tags: type=code-review,repo=foo` header (or JSON body extension), GET list with `?tag=type:code-review`, assert only tagged sessions are returned.
- All tests < 5ms — in-process, no I/O beyond tempdir.

**Implementation sketch.** Add `tags: HashMap<String, String>` to `Session`. Extend `CreateParams` to accept a JSON body (`Content-Type: application/json`) so callers can send structured metadata alongside markdown. Alternatively, accept `X-Tag-*` headers. List endpoint gains a `?tag=key:value` filter.

---

#### 1.3 `PATCH /api/sessions/{id}` — live content update

**What it does.** Allow a caller to push a revised version of the markdown to an already-active session. The Android WebView gets notified via the existing long-poll / notify infrastructure and reloads the page.

**Why it matters.** AI agents iterate fast. If the agent realises mid-session that it sent stale context (e.g. the diff changed), today it must cancel and create a new session — losing any annotations the reviewer has already made. A PATCH keeps the session alive and updates the render in place.

**How to test.**
- Unit: add `update_content(&mut self, content: String)` to `SessionManager`; assert `updated_at` advances and content is stored.
- Integration (`tests/update_content_test.rs`): create a session, PATCH new markdown, GET `/session/{id}` HTML, assert the new content is rendered and the old content is not.
- Fast: two `oneshot` calls, in-process, < 10ms.

**Implementation sketch.** Add `PATCH /api/sessions/{id}` route in `app.rs`. Accept plain text body (same as POST). Call `mgr.update_content` then `state.notify_session`. The Android app should add a `window.location.reload()` triggered by detecting a `304`-or-content-change response — or more simply, the existing 5-second poll loop in `fetchSessions` will cause the WebView to re-render when the user navigates back.

---

### Render quality

#### 1.4 Unified diff view (`diff` fenced block)

**What it does.** Introduce a `diff` fenced block type in `render.rs`. Lines starting with `+` render with a light green left-border; `-` lines with a red left-border; `@@` hunk headers in muted italic. No color fills — just borders and font weight, safe on grayscale e-ink.

**Why it matters.** The most common AI output for code review is a diff. Right now, pushing a `git diff` to the Boox renders as unstyled monospace. A purpose-built diff renderer makes the `+`/`-` structure legible on e-ink without relying on color (which washes out on grayscale modes).

**How to test.**
- Unit (`render::tests`): write a test that calls `render(markdown_with_diff_block)` and asserts `class="diff-add"` and `class="diff-del"` appear in the HTML. This test runs in < 1ms.
- Add a negative test: a plain `bash` fenced block with a `+` prefix does not get diff styling.
- No new infrastructure needed.

**Implementation sketch.** In `render.rs`, in the `CodeBlockKind::Fenced` match arm, add a `"diff"` branch. Split lines, classify by first character, emit `<div class="diff-add">` / `<div class="diff-del">` / `<div class="diff-hunk">` / `<div class="diff-ctx">`. Add CSS constants (4 new rules). Zero new files.

---

#### 1.5 Review checklist block

**What it does.** Introduce a `checklist` fenced block. Renders as a large-font, touch-friendly checkbox list on the Boox. Checkboxes are big enough to tap with a finger or stylus. State is stored in `localStorage` so it survives page reload.

**Why it matters.** AI agents often produce review checklists ("Have you checked X? Does Y compile?"). Standard GFM task lists (`- [ ]`) render at body font size (28px), but the tap targets are tiny on a 10-inch e-ink screen. A dedicated block can render at 48px with 80px tap targets — actually usable on the device.

**How to test.**
- Unit: `render(checklist_markdown)` contains `<input type="checkbox"` and `class="checklist-item"`.
- Unit: empty checklist block produces a parse error, not a crash.
- Integration: session with checklist content loads without 500 error.
- All unit tests < 1ms.

**Implementation sketch.** New `"checklist"` branch in the fenced block match arm. Parse each line as `- item text`. Emit `<label class="checklist-item"><input type="checkbox" ...> text</label>`. Add JS to persist state via `localStorage` keyed by session ID + item index. CSS adds `min-height: 80px; font-size: 48px`.

---

### CLI / AI integration

#### 1.6 Structured result schema with `review_verdict`

**What it does.** The reviewer can optionally prefix their typed notes with a machine-readable verdict tag: `LGTM`, `CHANGES`, `REJECT`, or `QUESTION`. The server parses this out of `typed_notes` and exposes it as `verdict` in the result JSON. The CLI surfaces it as a distinct exit code.

**Why it matters.** AI agents need to branch on reviewer intent. Today the agent must parse free-text to decide "did the human approve this?" A structured verdict field lets agents write `if result.verdict == "LGTM" { merge() }` with no NLP. Exit codes let shell scripts do the same: `eink-review push plan.md && deploy.sh`.

**How to test.**
- Unit (`cli.rs` / `session.rs`): a function `parse_verdict(typed_notes: &str) -> Option<Verdict>` is pure and trivially testable. Test cases: `"LGTM: looks good"`, `"CHANGES needed"`, `"plain text"` (returns `None`).
- Integration: submit a session with `typed_notes="LGTM: ship it"`, GET result, assert `verdict: "lgtm"` in JSON.
- CLI: assert exit code is 0 for LGTM, 2 for CHANGES, 3 for REJECT (exit code 1 is reserved for errors).

**Implementation sketch.** Add `fn parse_verdict(s: &str) -> Option<Verdict>` to a new `verdict.rs`. `Verdict` is a small enum. `submit` in `session.rs` calls this and stores the result. `get_result` handler includes `verdict` in its JSON. CLI `push` inspects `verdict` and maps to exit code.

---

#### 1.7 `--auto-retry` on cancel

**What it does.** Add `--auto-retry N` to `eink-review push`. If the session is cancelled by the reviewer (HTTP 410), the CLI automatically re-pushes the same content up to N times, printing a warning.

**Why it matters.** Reviewers sometimes accidentally cancel a session by tapping the wrong button. An AI agent that gets a 410 today simply fails and returns an error to the LLM — which then has to issue another tool call to re-push. Auto-retry saves one full round trip with the LLM, which can be 5–15 seconds.

**How to test.**
- Unit: the retry logic is a loop in `cmd_push`; test it with a mock HTTP server that returns 410 twice then 200.
- The mock can be an in-process `axum` oneshot that counts calls.
- Fast: no real device, no real network.

**Implementation sketch.** Extend `Command::Push` with `#[arg(long, default_value = "0")] auto_retry: u32`. In `cmd_push`, on 410 response, if `retries_left > 0`, decrement and loop from the top. Log the retry attempt to stderr.

---

#### 1.8 `eink-review push --watch file.md`

**What it does.** Watch the source file for changes using `notify` crate. When the file changes, cancel the current session (if active) and push the updated content. Session ID is printed to stderr each time.

**Why it matters.** Developers and AI agents often iterate on a document while it is under review. Today they must manually cancel and re-push. Watch mode closes that loop: the reviewer always sees the latest version within 1–2 seconds of a save. This is the e-ink equivalent of hot reload.

**How to test.**
- Unit: the file-change handler is a function that takes `(old_session_id, new_content) -> ()` — testable in isolation.
- Integration: write a tempfile, start watch in background task, overwrite the tempfile, assert a second session appears within 500ms. This is a process-level test but can run in `cargo test` using `tokio::time::timeout`.
- Harder to test the cancel-then-push race; mock the server.

**Implementation sketch.** Add `notify` crate to `Cargo.toml`. In `cmd_push`, if `--watch` is set, spawn a `notify::RecommendedWatcher` that sends to a `tokio::sync::mpsc::channel`. Main loop: on file event, call cancel API for current session ID, re-push, update current session ID.

---

### Android UX

#### 1.9 Offline annotation queue

**What it does.** When the Boox is offline (no Wi-Fi) or the server is unreachable, completed reviews are stored locally in `SharedPreferences` or a SQLite file. On next connection, they are automatically submitted.

**Why it matters.** E-ink tablets are often used away from a desk — on a couch, in a café. If Wi-Fi drops during review, the annotation is currently lost (the Submit button shows an error toast and the user must redo it). An offline queue means the human's work is never lost.

**How to test.**
- Android unit test (Robolectric): `OfflineQueue.enqueue(sessionId, notes, pngBytes)` persists to disk; `OfflineQueue.drain(client)` sends pending items and clears them. No real network needed — mock OkHttp interceptor.
- Testability: 4/5 — Robolectric covers most of this without a device.

**Implementation sketch.** New `OfflineQueue.kt` class. On `submitAndGoBack`, if the network call throws or returns non-2xx, call `queue.enqueue(...)`. In `startPolling`, after `fetchSessions` succeeds (proving connectivity), call `queue.drain(client, serverUrl)`. Store queue as JSON in `SharedPreferences`.

---

#### 1.10 New session notification sound + auto-open

**What it does.** When a new session appears in the poll loop, play a short notification sound (system `MediaPlayer` with a built-in tone) and optionally auto-open it if there is only one active session.

**Why it matters.** The current code only vibrates. The Boox screen is off most of the time when the tablet is idle on a desk. A sound ensures the reviewer notices the new item without checking the screen. Auto-open for single sessions removes the tap on the session list, shaving 2–3 seconds off the response latency.

**How to test.**
- Testability: 2/5 for sound (requires audio HAL). Extract the decision logic — `shouldAutoOpen(sessions: List<SessionInfo>, prevCount: Int): Boolean` — into a pure function testable in a JUnit test without Android. Sound emission is an IO side effect; wrap it behind an interface and mock it.
- The pure logic test runs in < 1ms on the JVM.

**Implementation sketch.** In `fetchSessions`, after `adapter.submitList`, compute `newCount = sessions.count { it.status == "Active" }`. If `newCount > hadActive`, play tone via `ToneGenerator`. If `newCount == 1 && hadActive == 0`, call `openSession(sessions.first().id)`. Extract `shouldAutoOpen` as a top-level function in `MainActivity.kt`.

---

### Observability

#### 1.11 Session timing metrics endpoint

**What it does.** Add `GET /api/metrics` (Prometheus text format) exposing: `eink_sessions_created_total`, `eink_sessions_submitted_total`, `eink_review_duration_seconds` (histogram, from `created_at` to `updated_at` on submit), `eink_sessions_active`.

**Why it matters.** For AI-driven workflows, review latency is a core metric — it determines how fast the agent can iterate. Knowing the P50/P95 review time lets teams tune timeouts, identify stale sessions, and measure whether process changes (shorter prompts, better formatting) actually speed up the human reviewer.

**How to test.**
- Integration: create a session, submit it immediately, GET `/api/metrics`, assert `eink_sessions_created_total 1` and `eink_sessions_submitted_total 1` appear in the response body. In-process, < 20ms.
- Unit: the metric computation from `SessionManager::list()` is a pure function — test it with a hand-crafted list of sessions.

**Implementation sketch.** Add `metrics` feature to `Cargo.toml` using the `prometheus` crate (or simpler: compute metrics inline from `SessionManager::list()` on each request to avoid global state). New `metrics.rs` with `fn render_metrics(sessions: &[&Session]) -> String`. New route `GET /api/metrics` in `app.rs`.

---

#### 1.12 Annotation presence signal

**What it does.** The submit endpoint records whether the annotation PNG is non-empty (i.e. the reviewer actually drew something) and exposes `has_annotation: bool` alongside `annotation_images` in the result JSON. Also report `annotation_stroke_count` from the PNG header (if stored as a companion JSON sidecar).

**Why it matters.** An AI agent receiving a result with empty typed notes and no annotation has very little to work with. A `has_annotation` flag lets it prompt the user differently: "Your reviewer submitted without notes — did you mean to annotate?" It also lets agents skip OCR/image-read calls when there is nothing to see.

**How to test.**
- Unit: `fn annotation_is_empty(png: &[u8]) -> bool` — check for a 1x1 transparent PNG vs a real stroke. Pure function, < 1ms.
- Integration: submit a session with and without annotation, assert `has_annotation` differs in the result JSON.

**Implementation sketch.** In `submit_review` (app.rs), after saving the annotation, check if the PNG data is above a threshold size (e.g. > 1KB to exclude empty transparent canvases). Store `has_annotation: bool` in `Session`. The Android app already exports `null` when `buf.isEmpty`, so when `pngData == null`, no annotation part is sent — which maps directly to `has_annotation: false` server-side.

---

## 2. AI-Agent Integration Patterns

### 2.1 What the current API gives an agent

The current shape is:
```
POST /api/sessions  →  { id, url }
GET  /api/sessions/{id}/result  (long-poll)  →  { id, status, typed_notes, annotation_images }
```

This is functional but thin. An agent calling this today must:
1. Parse `typed_notes` as free text to understand intent.
2. Decide whether to read `annotation_images` via a separate `Read` tool call.
3. Handle 204 (timeout) by retrying in a loop.
4. Infer context from the session it created — there is no tagging so parallel sessions from different contexts are indistinguishable.

### 2.2 Recommended result schema

```json
{
  "id": "abc12345",
  "status": "submitted",
  "verdict": "changes",
  "typed_notes": "The caching layer seems overengineered.",
  "has_annotation": true,
  "annotation_images": ["/path/to/img.png"],
  "meta": {
    "review_duration_seconds": 142,
    "tags": { "type": "code-review", "pr": "123" }
  }
}
```

The `verdict` field (feature 1.6) is the single most important addition for agent integration. It converts a free-text review into a branch condition. `has_annotation` (feature 1.12) tells the agent whether to issue a `Read` tool call. `meta.review_duration_seconds` lets the agent decide whether to retry with a shorter document next time.

### 2.3 Webhook / callback mode

An agent integration that avoids long-polling looks like:

```bash
# Agent registers a local callback server on an ephemeral port, then:
eink-review push plan.md --callback http://localhost:$LOCAL_PORT/hook
# Agent continues doing other work; when the hook fires, it reads the result
```

For Claude Code specifically, the `/eink` skill currently blocks. With webhooks, it could use `--async` to push, register a file-based "result inbox" (`~/.local/state/eink-bridge/pending/{id}`), and poll that file from a background timer — without holding a long HTTP connection.

### 2.4 Session tagging for routing

An agent that manages multiple review workflows should tag sessions at creation:

```bash
eink-review push diff.md --tag type=code-review --tag pr=123 --tag urgency=high
```

The Android app could then show a visual indicator (border color or badge) per tag type — making it immediately clear to the reviewer what kind of document they are about to read. The server's list endpoint gains a `?tag=type:code-review` filter so Claude Code can query only its own sessions.

### 2.5 Minimal viable agent integration checklist

Priority order for making this easy to integrate with Claude Code or similar:

1. `verdict` field in result JSON — agents can branch without NLP.
2. `has_annotation` flag — saves one `Read` tool call when there is nothing to read.
3. Session tags — lets agents filter their own sessions from shared servers.
4. Webhook callback — eliminates the long-poll TCP hold.
5. Structured error on cancel — distinguish "user rejected" from "timeout" from "server error" via distinct status codes or a `cancel_reason` field.

---

## 3. Testability Analysis

| Feature | Testability (1–5) | Notes |
|---|---|---|
| 1.1 Webhook mode | 5 | Pure HTTP; second axum handler as mock; in-process; < 50ms |
| 1.2 Session tags | 5 | Data model change only; trivial unit + integration tests |
| 1.3 Live content PATCH | 5 | Two oneshot calls; render output is deterministic |
| 1.4 Diff block rendering | 5 | String assertion on `to_eink_html()`; < 1ms per test |
| 1.5 Checklist block | 5 | Same as diff; JS localStorage logic requires browser but block generation is server-side |
| 1.6 Verdict parsing | 5 | Pure function `parse_verdict(s: &str)`; no IO |
| 1.7 Auto-retry on cancel | 4 | Needs mock HTTP server; achievable in-process with axum oneshot |
| 1.8 Watch mode | 3 | File events are async; need `tokio::time::timeout` wrapper; race conditions possible |
| 1.9 Android offline queue | 3 | Robolectric covers most paths; drain logic needs mock OkHttp |
| 1.10 Android sound/auto-open | 2 | Sound requires audio HAL; extract pure decision logic; test that in JUnit |
| 1.11 Metrics endpoint | 5 | Response is a string; assert substrings; in-process |
| 1.12 Annotation presence | 5 | `annotation_is_empty(png)` is pure; integration test via multipart submit |

**Hard to test without a real device:**
- Onyx SDK drawing (pressure, latency, EMR stylus detection). The `rawDrawingAction` function in `PenOverlay.kt` is deliberately extracted as a pure function to enable unit testing without the SDK. More logic should follow this pattern.
- E-ink refresh artifacts (ghosting on partial refresh). These require visual inspection on the physical display — no mock possible.
- Haptics and sound (features 1.10). Both require hardware or a full Android emulator.

**Mock strategy for Android features:** extract all decision logic into top-level pure functions (as `rawDrawingAction` demonstrates). Keep side effects (sound, vibration, network) behind single-call sites that can be replaced with no-op implementations in tests.

---

## 4. Priority Recommendations

Ranked by `impact × ease × testability`:

### #1 — Verdict field in result JSON (feature 1.6) ★★★★★

Impact is very high: it is the most common thing an AI agent needs from a human review — approve or reject. Ease is high: it is a pure parsing function + one new JSON field. Testability is maximal. Implementation is one new file (`verdict.rs`) and minor changes to two existing ones. This should be the first thing built.

### #2 — Diff block rendering (feature 1.4) ★★★★★

Impact is high: AI agents almost always produce diffs, and right now they render as unstyled text. Ease is high: all changes are in `render.rs` with no new dependencies. Testability is maximal — every test is a string assertion on `to_eink_html()`. Yields immediate visible value on the next `eink-review push` of a diff.

### #3 — Session tags (feature 1.2) ★★★★☆

Impact is high for teams and multi-agent setups. Ease is moderate: requires extending the `Session` struct, `CreateParams`, and the list filter. Testability is maximal. Unlocks all future routing and observability features, making it a force multiplier.

### #4 — Webhook / callback mode (feature 1.1) ★★★★☆

Impact is high for AI agents that submit many sessions in parallel or have unreliable long-poll connections. Ease is moderate: requires spawning a background HTTP call. Testability is very high. The design is clean (fire-and-forget `tokio::spawn`). Eliminates the "agent holds a TCP connection for 30 minutes" problem at scale.

### #5 — Session timing metrics endpoint (feature 1.11) ★★★☆☆

Impact is moderate but grows over time: once you have metrics, you can measure whether new features actually improve the review loop. Ease is high: computing from `SessionManager::list()` on demand avoids global metric state. Testability is maximal. Small investment for compounding value.

---

### What to deprioritize

- **Watch mode (1.8):** useful but file-watching edge cases (debounce, editor atomic writes) make it fragile and the testability score is low.
- **Android offline queue (1.9):** important for reliability but Android testing infra (Robolectric setup, CI) is expensive relative to the server-side features.
- **Auto-open / sound (1.10):** nice UX but zero AI integration value; human-reviewer convenience only.
