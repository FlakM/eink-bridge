# AGENTS.md

Guidance for automated agents (Claude Code, CI) working in this repository.

## Session state machine

```
POST /api/sessions
      |
      v
  +--------+  POST /submit    +-----------+
  | Active |------------------>| Submitted |
  +--------+                   +-----------+
      |
      +-- DELETE /{id} ------> Cancelled
      |
      +-- expire_stale() ----> Expired
```

- Only `Active` sessions accept submissions or cancellation.
- `Submitted`, `Cancelled`, and `Expired` are terminal states.
- Long-poll (`GET /result`) blocks on Active, returns immediately on terminal states.

## Long-poll architecture

The CLI (`eink-review push`) polls `GET /api/sessions/{id}/result` in a loop:
- Server holds each request for up to `long_poll_seconds` (default 30s).
- If the session transitions during that window, the server wakes the waiter via `tokio::sync::Notify` and responds immediately.
- On timeout, the server responds with 204 (no change); the CLI retries.
- The CLI has its own `--timeout` (default 30 minutes) bounding total wait time.

## OCR setup

Handwriting recognition uses **Ollama** with `qwen2.5vl:7b` (vision model). Tesseract is not used.

### Prerequisites

1. Install Ollama — on NixOS add to `configuration.nix`:
   ```nix
   services.ollama = {
     enable = true;
     package = pkgs-unstable.ollama;
     environmentVariables = {
       OLLAMA_NUM_PARALLEL = "4";  # allow concurrent annotation OCR
     };
   };
   ```
   Then `sudo nixos-rebuild switch`.

2. Pull the model (one-time):
   ```bash
   ollama pull qwen2.5vl:7b
   ```

3. Verify it's running:
   ```bash
   curl http://localhost:11434/api/ps
   ```

### Configuration

| Env var | Default | Description |
|---|---|---|
| `EINK_OLLAMA_URL` | `http://localhost:11434` | Ollama base URL |
| `EINK_OLLAMA_MODEL` | `glm-ocr` | Model name |
| `EINK_OCR_DEBUG` | unset | If set, saves PNG to `/tmp/eink-ocr-*.png` |

### Timing

Each OCR call logs wall time, prompt-eval time, and token-eval time at `INFO` level:
```
INFO OCR complete text="WRITE A TLDR" wall_ms=1823 prompt_eval_ms=312 token_eval_ms=271
```

Typical latency on CPU with `qwen2.5vl:7b`: 1–4 s per annotation group.

### OCR handwriting tests

```bash
cargo test --test ocr_handwriting_test -- --ignored
```

Tests use PNG fixtures cropped from a real Boox tablet screenshot (`tests/fixtures/ocr/`).
They require Ollama to be running with the vision model loaded.
A `tokio::sync::Mutex` serialises the calls so concurrent tests don't saturate the model.
Once `OLLAMA_NUM_PARALLEL=4` is active in the NixOS service, parallelism is handled by Ollama itself.

### Gotcha: coloured ink

The OCR engine renders strokes as **black on white** before sending to Ollama. The test
fixtures were converted to greyscale for the same reason — Ollama's vision model returns
empty output for brightly coloured (e.g. pink) handwriting.

## Security notes

The server binds to `0.0.0.0:3333` by default with no authentication.
Anyone on the LAN can read sessions, submit reviews, or cancel them.
This is acceptable for personal/home-lab use but must not be exposed to untrusted networks.
Request body size is limited to 10 MB.

## Task runner

Use `just` for all common operations. Run `just` to see all recipes.

```bash
just test          # all server tests
just test-render   # render unit tests only (~0.02s)
just test-one NAME # single test by name
just test-mutations # reintroduce each known render bug, expect a test to fail
just lint          # fmt + clippy
just deploy        # nix build + restart service + verify the restart took
just verify-deploy # prove the running server is the build in ./result
just ci            # full CI check
```

## Quick test loop -- use this for validation

Most changes to `render.rs` can be validated instantly without a server or device:

```bash
just test-render                        # all render unit tests
just test-one toc_shows_for_single_heading  # single test by name
just test                               # all fast unit + integration tests
```

## Eval harness (golden tests)

Snapshot-based tests that freeze API contracts and render output:

```bash
just eval            # check render + contract goldens match
just eval-update     # regenerate goldens after intentional changes
```

### Render goldens (`tests/golden/render/`)

Each `.md` fixture in `tests/fixtures/eval/render/` is rendered through `to_eink_html()` and compared against the corresponding `.html` golden. Covers: prose, headings, inline code, blockquotes, tables, lists, diffs, code highlighting (Rust, Python, no-lang), mermaid, mindmap (valid, colored, invalid), graph (simple, labeled edges, invalid), and mixed documents.

### Contract goldens (`tests/golden/contracts/`)

Each API scenario produces a JSON response that is normalized (random IDs/timestamps replaced) and compared against a golden `.json`. Covers: create session (JSON and plaintext), list sessions (all and filtered), get session detail, submitted result (LGTM and CHANGES verdicts), cancelled result (410 GONE), and OpenAPI spec.

### Adding new goldens

1. For render: add a `.md` fixture to `tests/fixtures/eval/render/`, run `just eval-update`.
2. For contracts: add a scenario function in `eval_contract_test.rs`, add it to the `scenarios` vec, run `just eval-update`.
3. Run `just eval` to verify, then commit the new golden files.

## Test layers

### 1. Render unit tests -- `src/render.rs` (fastest)

Live in `#[cfg(test)] mod tests` inside `render.rs`. Call `to_eink_html()` directly -- no HTTP, no disk, no async. Each runs in < 1ms.

```bash
cargo test render::tests
```

### 2. HTTP integration tests -- `tests/` (fast)

Use `tower::ServiceExt::oneshot()` on an in-process axum app. No real server started. Tempdir for state.

```bash
cargo test --test health_test
cargo test --test session_lifecycle_test
cargo test --test long_poll_test
cargo test --test render_validation_test
cargo test --test webhook_test
```

### 3. E2E tests -- `tests/e2e_test.rs` + `tests/cli_integration_test.rs` (slower)

Start a real TCP server on a random port, spawn `eink-review` and `eink-mock-device` as child processes.

```bash
cargo test --test e2e_test
cargo test --test cli_integration_test
```

## Coverage

```bash
just coverage            # Rust server + CLI (cargo llvm-cov, text summary)
just coverage-html       # Rust server (HTML report)
just coverage-android    # Android/Kotlin (JaCoCo HTML report)
just test-android        # Android unit tests only (no coverage)
```

Use `/coverage` skill for automated analysis of gaps.

## What to test when

| Change area | Run |
|---|---|
| `render.rs` CSS or HTML output | `just test-render` then `just eval` |
| Annotation / highlighting / tap behaviour | `just test-render` then `just test-mutations` |
| Snippet, diff, steps or fence handling | `cargo test render::tests::parse_chunks` / `a_snippet` / `a_line_mark` |
| Relocating notes after an edit | `cargo test reanchor::tests` |
| `render.rs` diagram parsing | `cargo test render::tests::graph` / `mindmap` / `mermaid` |
| `app.rs` handlers | `cargo test --test health_test` |
| `session.rs` lifecycle | `cargo test --test session_lifecycle_test` |
| Long-poll / notify | `cargo test --test long_poll_test` |
| CLI `push`/`result`/`cancel` | `cargo test --test cli_integration_test` |
| API response shapes | `just eval` |
| Full round-trip | `cargo test --test e2e_test` |
| After any `just deploy` | `just verify-deploy` (automatic at the end of `just deploy`) |

### Presentation invariants

Three classes of bug have bitten the e-ink view, and each now has a test aimed at the
class rather than the instance:

- **A clipping ancestor hides the annotation highlight.** The bind/flash highlight is an
  `outline` with `outline-offset`, drawn outside the element's box, so any ancestor with
  `overflow: hidden|auto|scroll` clips it and annotating that element looks broken. Only
  the pan/zoom viewports may clip; `tests::only_viewports_clip_their_contents` fails on
  any new clipping rule, and `tests::content_that_can_be_annotated_is_never_clipped`
  checks it from the reader's side.
- **The syntax palette goes unreadable.** `tests::every_syntax_colour_is_readable_on_paper`
  loads the shipped `tmTheme` the way the renderer does and requires a paper-light canvas
  and WCAG AA contrast for every ink. Edit the theme with `plistlib`, never by text
  substitution — a blanket `#ffffff` replacement once took the canvas with it.
- **A block cannot be annotated at all.** Every block kind must contain something the
  element map sees; `tests::every_block_kind_can_be_bound_by_a_tap` renders one of each.

Because a green test only shows the tests pass, `just test-mutations` reintroduces each
defect and asserts the named test goes red. Add a case there whenever you add an
invariant.

### Snippets, fences and line marks

The `snippet` block is a small format with several ways to lose information quietly, and
each has a test:

- **Fence length.** A block ends only at a fence at least as long as the one that opened
  it, so a snippet can quote a fence of its own (use four backticks). Reading any closer
  as the end truncated the snippet at the inner fence; rendering the code in a fence it
  could contain produced a nested `<pre>`.
- **Line marks.** Only the trailing `#L12,15` token is a mark. Taking the first `#L`
  anywhere truncated a caption that mentioned a line ("fixes #L42 regression") and
  highlighted the wrong line. `tests::a_line_mark_lands_on_the_line_it_names` pins the
  line itself, not merely that some line is marked: the renderer emits a newline between
  `<pre>` and the code, and counting it sent every reader one line too high.
- **Markup.** Unwinding a marked line used to close its `<mark>` before the `<span>` that
  carried the newline. Marked code must stay balanced, one rendered line per line of code.
- **Headers.** The first token is a language only if it can be one; a path with an
  annotation after it is a location, not a language, and reading it as one drops the file
  name from the caption.

Everything a reader sees in a block comes from a pasted brief, so caption, language badge
and step text are escaped: `tests::review_text_in_a_snippet_or_step_block_is_escaped`.

### Relocating notes after an edit (`reanchor.rs`)

`targets()` decides which paragraph an existing note follows when the document changes.
A note that lands on the *wrong* paragraph is worse than one that is left unplaced: the
author is told the wrong thing with confidence. The rules, each with a test:

- Exact matches are resolved before fuzzy guesses, and a guess may never take a paragraph
  an exact match owns. Otherwise a deleted paragraph similar to a surviving one stole the
  survivor's match and dropped its note.
- An unchanged paragraph that survives still gets its note back, and reordered paragraphs
  take their notes with them (position is not identity).
- Two evenly good candidates, repeated text, an empty source, a changed tag and a short
  label against a long paragraph are all left unplaced rather than guessed.

`tests::a_relocated_note_never_lands_on_another_paragraph` asserts the property over 200
deterministic document shapes: a note either follows its own paragraph or is left
unplaced, never a neighbour.

### Deployment gap

The `eink-serve` systemd **user override runs the repository's `result` binary**, so
`sudo nixos-rebuild switch` does **not** deploy server changes — the tests compile from
source and pass while the tablet is served by an older renderer. Server changes need
`just deploy`; server and harness changes need both. `just verify-deploy` (run
automatically by `just deploy`) compares `./result/bin/eink-serve` with the `exe` the
running server reports on `/api/health` and fails loudly on a mismatch.


## Build

```bash
just build         # cargo dev build
just nix-build     # nix build (also runs all tests)
just lint          # cargo fmt + clippy
```

## Deployment

```bash
just deploy        # nix build + restart systemd service
just status        # check service status
just logs          # follow server logs

just apk           # build android debug APK
just apk-install   # build + install on connected device
```

## Debugging

### Metrics

The `/metrics` endpoint (port 3333) exposes all Prometheus metrics in text format — useful for checking live state without logs:

```bash
curl -s http://localhost:3333/metrics | grep eink_
```

Key metrics for debugging:

| Symptom | Metric to check |
|---|---|
| Sessions not submitting | `eink_sessions_submitted_total`, `eink_sessions_active` |
| OCR failures | `eink_ocr_requests_total{result="error"}` |
| OCR slow | `eink_ocr_duration_seconds` buckets |
| WebSocket drops | `eink_ws_connections_active` |
| Long-poll timing out too fast | `eink_long_poll_total{result="timeout"}` |
| Webhook not firing | `eink_webhook_total{result="error"}` |
| Unexpected session expiry | `eink_sessions_expired_total` |

Grafana dashboard on odroid:3002 visualizes all of these over time — often faster to spot trends than grepping logs.

### Logs

```bash
just logs              # follow live server logs
just status            # systemd service status
EINK_OCR_DEBUG=1 ...   # saves OCR PNGs to /tmp/eink-ocr-*.png before sending to Ollama
```

## Adding render tests

When adding a new feature to `render.rs`:
1. Add a unit test in `src/render.rs` under `mod tests` that calls `render(markdown)` and asserts on the HTML string.
2. Add an HTTP-level test in `tests/render_validation_test.rs` if the feature involves session creation or asset serving.
3. Add a render fixture in `tests/fixtures/eval/render/` and run `just eval-update` to generate the golden.
4. Run `just test-render` to iterate quickly; `just ci` before committing to run the full suite.

## Known weak points

Areas that work today but could bite under load or after refactoring:

### Notifier / WS sender map leak

`AppState.notifiers` and `AppState.ws_senders` are cleaned up on submit and cancel
but **not** on session expiry. The expiry loop in `main.rs` only calls
`SessionManager::expire_stale()` — it never touches the notifier/sender maps.
Long-running servers will accumulate orphaned entries for every expired session.

Fix: have the expiry loop call `notify_and_cleanup` + `ws_cleanup` for each expired id.

### No server-side WebSocket heartbeat

The CLI sends pings every 30 s, but the server's `handle_ws` has no periodic
ping. Intermediate proxies or NAT can silently drop idle connections without
either side noticing until the next message fails.

### Disk I/O under write lock

`SessionManager::persist()` does synchronous `fs::write` inside the `RwLock`
write guard. Under high concurrency, every mutation (create, update, star,
submit) serialises on disk I/O. Acceptable at current scale (single user);
would need async write or write-behind buffer if concurrency grows.

### No pagination on list sessions

`GET /api/sessions` returns every session in one JSON array. Fine for dozens
of sessions; will blow up response size if sessions accumulate over months
without purging.

### Startup panics

`main.rs` uses `.unwrap()` on `TcpListener::bind` and `axum::serve`. A port
conflict gives a raw panic trace instead of a human-readable error.

### OCR timeout not tunable per session

The Ollama HTTP client has a global 120 s timeout. There's no way to set a
shorter timeout for fast-fail or a longer one for complex images.
