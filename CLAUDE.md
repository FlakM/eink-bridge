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
| `EINK_OLLAMA_MODEL` | `qwen2.5vl:7b` | Model name |
| `EINK_OCR_DEBUG` | unset | If set, saves PNG to `/tmp/eink-ocr-*.png` |
| `EINK_OCR_TIMEOUT_SECONDS` | `120` (max 300) | Bound on one local OCR attempt |
| `EINK_OCR_INTERACTIVE_TIMEOUT_SECONDS` | `20` (max 120) | The shorter bound for a call a reader is waiting on |

Both bounds fall back to their default on an unusable value; a `0` would abandon every
attempt before it was sent, which reads as "the model never answers".

### Timing

Each OCR call logs the provider that won, whether the cache answered, and wall time at
`INFO`; prompt-eval and token-eval timings are at `DEBUG`:
```
INFO OCR complete provider="ollama" cache_hit=false wall_ms=1823
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
Request body size is limited to 10 MB; one OCR or update request may carry at most 5000
strokes and 500 000 points.

What a LAN caller cannot do: read files. `/source` only serves from a checkout the server
recorded itself, and it records one only when the session was created from a loopback
address (the CLI on the same host). `EINK_SOURCE_ROOTS` (colon-separated directories)
narrows that further. `callback_url` must be http or https and may not point at a
link-local address. Raw HTML in a brief is sanitised before it reaches the page, and
diagram payloads cannot close their own `<script>`, so a brief cannot run code in the
page's origin.

A picture in a brief is uploaded as bytes, never as a path for the server to open, and it
lands under that session's `images/` directory. The name must be one path segment with a
png/jpeg/webp/gif extension, the bytes must be that format, and the file is served back
from the session's own route with `nosniff` and `no-store`. SVG is not on the list: it is
a document that can carry script, and an image URL shares the page's origin.

## Task runner

Use `just` for all common operations. Run `just` to see all recipes.

```bash
just test          # all server tests, every binary in parallel (~3s)
just test-fast     # in-process tests only: no server, no sockets (~1s)
just test-render   # render unit tests only (~0.5s)
just test-one NAME # single test by name
just test-cargo    # the same suite through plain cargo (slower, no nextest)
just test-mutations # reintroduce each known bug, expect a test to fail
just lint          # fmt + clippy
just deploy        # nix build + restart service + verify the restart took
just verify-deploy # prove the running server is the build in ./result
just ci            # full CI check
```

`just test` is the default because it is fast enough to run before every commit: 382
tests across 25 binaries in about three seconds, against 28 s when `cargo test` ran the
binaries one after another. `server/.config/nextest.toml` leaves retries at zero on
purpose -- a flaky test is a bug worth seeing, not a nuisance to paper over.

## Quick test loop -- use this for validation

Most changes can be validated without a server or device:

```bash
just test-render                            # all render unit tests
just test-one toc_shows_for_single_heading  # single test by name
just test-fast                              # in-process tests only
just test                                   # everything (~3s)
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

### 1. Render unit tests -- `src/render/tests.rs` (fastest)

Call `to_eink_html()` directly -- no HTTP, no disk, no async. Each runs in < 1ms.

```bash
cargo test render::tests
```

The page's CSS and scripts are files in `server/assets/` (`eink.css`, `toc.js`,
`bootstrap.js`, `coderef.js`, `element-map.js`) pulled in with `include_str!`, so an edit
needs a rebuild and `just deploy` to reach the tablet. `just lint-assets` runs
`node --check` over the scripts.

Raw HTML in a brief is sanitised: `span`, `mark`, `kbd`, `sup`, `sub`, `details`,
`summary`, `br` and inline `color`/`background-color` survive; scripts, event handlers and
`javascript:` links do not. A brief without raw HTML renders byte-identically. Diagram
payloads are JSON with `<`, `>` and `&` written as `<`-style escapes, which is why a
golden shows `-->` where the source says `-->`.

### 2. HTTP integration tests -- `tests/` (fast)

Use `tower::ServiceExt::oneshot()` on an in-process axum app. No real server started. Tempdir for state.

```bash
cargo nextest run --test health_test
cargo nextest run --test session_lifecycle_test
cargo nextest run --test long_poll_test
cargo nextest run --test render_validation_test
cargo nextest run --test webhook_test
```

### 3. E2E tests -- `tests/e2e_test.rs` + `tests/cli_integration_test.rs` (slower)

Start a real TCP server on a random port, spawn `eink-review` and `eink-mock-device` as child processes.

```bash
cargo nextest run --test e2e_test
cargo nextest run --test cli_integration_test
```

`cargo test` still works for all of these; `nextest` is what `just` uses because it runs
every binary at once. A test that needs a real clock and a real socket (the WebSocket
heartbeat is the only one) is bounded by its own threshold, not by a sleep someone tuned
on a fast machine: the heartbeat is a `Duration` on `AppState`, so the test drives 200 ms
intervals instead of spending seven seconds at one second each.

## Coverage

```bash
just coverage            # Rust server + CLI (cargo llvm-cov, text summary)
just coverage-html       # Rust server (HTML report)
just coverage-android    # Android/Kotlin (JaCoCo HTML report)
just test-android        # Android unit tests only (no coverage)
```

Use `/coverage` skill for automated analysis of gaps.

Last measured: **91.6% of Rust lines** — and **~95%** of everything that is not an
entrypoint or a benchmark. Together with 280 Android unit tests and every mutation caught
by `just test-mutations`.

Two caveats when reading the number. Subprocess runs do count (the CLI and mock device are
built with instrumentation and their profiles are merged), so `cli.rs`'s 82% is real
rather than uncollected: what is missing there is the error branches — a refused tag, a
404 on the observer socket. And the two benchmark binaries are entrypoints, not behaviour.

Fully covered: `api.rs`, `metrics.rs`, `render.rs`, `render/chunks.rs`,
`render/browser.rs`, `render/html_utils.rs`, `render/models.rs`, `ocr/cache.rs`. Behind
them: `reanchor.rs` 99.0%, `render/diagrams.rs` 99.3%, `ocr/race.rs` 98.4%,
`session.rs` 97.6%, `ocr.rs` 95.2%, `ws.rs` 94.8%.

What is left uncovered is deliberately so. `main.rs`, `mock_device.rs` and the two
benchmark binaries are entrypoints. The rest is error-logging paths for a disk that cannot
be written, and `providers::OpenAi::from_env`, which reads process environment and cannot
be exercised in a unit test without racing every other test in the binary.

## What to test when

| Change area | Run |
|---|---|
| `assets/*.css`, `assets/*.js` or render HTML output | `just test-render`, `just lint-assets`, then `just eval` |
| Annotation / highlighting / tap behaviour | `just test-render` then `just test-mutations` |
| Snippet, diff, steps or fence handling | `cargo test render::tests::parse_chunks` / `a_snippet` / `a_line_mark` |
| Document images (`image/` refs, `--image`, upload) | `cargo test images::` then `cargo test --test api_edge_test` and `--test cli_integration_test` |
| Relocating notes after an edit | `cargo test reanchor::tests` |
| HTTP edge cases (remove, purge, ack, limits) | `cargo test --test api_edge_test` |
| CLI commands end to end | `cargo test --test cli_integration_test` |
| OCR: strokes, providers, the engine | `cargo test ocr::` then `cargo test --test ocr_handwriting_test -- --ignored` |
| WebSocket gauge leak | `cargo test --test ws_metrics_test` |
| The tablet's own logic | `just test-android` |
| WebSocket heartbeat / half-open sockets | `cargo test --test ws_heartbeat_test` |
| Startup failures (port in use) | `cargo test --test startup_test` |
| Pagination and the session list | `cargo test --test api_edge_test` (`pages_` / `a_page_size`) |
| Session write durability | `cargo test --lib session::write_queue` and the `api_edge_test` durability case |
| A file read by a test is in the nix source | `cargo test --test build_source_test` |
| Metric names and the metrics reference | `cargo test metrics::` |
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

### Pictures in a document (`images.rs`)

A brief may show a screenshot, a sketch or a photo — the artefact itself, where a diagram
would only show the structure around it. Four things can go wrong quietly, and each has a
test:

- **The picture is served from the wrong place.** The markdown says `image/shot.png`; the
  renderer must point that at `/api/sessions/{id}/image/shot.png`, because a bare relative
  path would resolve against `/session/{id}` and 404.
  `tests::a_document_image_is_served_from_the_session_that_owns_it` pins the URL, and
  `an_image_that_is_not_a_session_image_is_left_alone` keeps an external link external.
- **A name escapes the session directory.** A name is one path segment with an image
  extension; `..%2Fescape.png`, `sub%2Fdir.png` and `.hidden.png` are a 400, and nothing
  is written outside the session's `images/`. SVG is refused on purpose: it is a document
  that can carry script, and these images are served from the page's own origin.
- **The bytes are not the format the name promises.** A `.png` that is not a PNG is
  refused before it is stored, so the image route can never serve something else. Served
  images carry `nosniff` and `no-store` (a redrawn screenshot reuses its name).
- **The picture arrives after the page does.** A create request may carry its images
  (`multipart/form-data`, repeated `image` parts), so the first version of the document
  already shows them; uploading afterwards would render the page once broken and then
  reload it. Only an active session accepts a new image (409 otherwise).

No caller hands the server a path to read: the bytes travel in the request. The CLI hides
the route — `push` uploads a `![x](shot.png)` that sits next to the brief and rewrites the
reference, `--image` adds a file the brief does not reference, and `image <id> <file>`
prints the markdown to paste into an existing review.

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

### The HTTP surface and the CLI

`tests/api_edge_test.rs` covers the requests a client makes when something is unfinished,
misspelled or already over. The rules it pins, each one a place a wrong answer looks
harmless: `?remove=true` forgets a session and deletes its files while a bare DELETE
cancels and keeps them; purge takes finished reviews and leaves active and starred ones;
an acknowledgement must name the feedback it consumes and cannot be spent twice; a stale
writer gets 409 with its edit intact; an over-large element map is refused before the
quadratic comparison runs; a second submit is a conflict; a broken multipart field names
itself in the 400; an uploaded annotation lands on disk byte for byte; and without an OCR
engine `POST /api/ocr` answers "not available" rather than an empty transcription.

`tests/cli_integration_test.rs` runs the real binary: push, update, star, unstar, cancel,
remove and list, plus the observer's reconnect loop. `src/cli.rs` keeps its own decisions
pure so they can be tested without a server -- the verdict exit code a pipeline gates on,
the result text, the list row, and the metric body it pushes to the pushgateway (whose
label values are escaped: one malformed line and the whole batch is rejected, silently).

### The tablet (`android/`)

```bash
just test-android        # all JVM unit tests (280 of them)
just coverage-android    # JaCoCo report
```

`AnnotationRelocationTest` is the client half of the relocation problem `reanchor.rs`
solves on the server, and holds the same safety property: ink follows the top-left of its
paragraph (so a margin note stays beside its line when the paragraph grows), a bound note
whose paragraph is gone is archived rather than drawn over unrelated text, and the
archived count is the truth about what the reader can no longer see. A *free* stroke
always binds to the nearest paragraph however far it is -- there is no distance cap, which
is deliberate and worth knowing before "fixing" it.

### The mutation harness

`just test-mutations` reintroduces each known defect and asserts the named test goes red.
Three rules keep it honest, each learned the hard way:

- **Every case names its target.** A test in a binary crate is invisible to `--lib`; a
  filter that matches no test exits zero, which would make a missed mutation look caught.
  The harness now refuses to run a filter that selects nothing.
- **Reverts are by offset and verified byte for byte.** Restoring by searching for the
  replacement text once put a `?remove=true` line back into `cmd_cancel`, because a bare
  `DELETE /api/sessions/{id}` appears in both commands.
- **The final gate runs every target**, not just the library. The corruption above reached
  a commit because only `--lib` was checked at the end of the run.

Add a case whenever you add an invariant. `python3 server/scripts/mutation-check.py <word>` runs only
the cases whose label or test name contains the word, and skips the final whole-suite gate.

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

`GET /api/sessions` takes `limit` and `cursor` and answers with an `x-next-cursor`
header; without `limit` it returns the whole list, which is what older clients expect. The
order is total — starred, `updated_at`, id — so a cursor cannot duplicate or skip a
session. `GET /api/health` carries a `ready` block: OCR availability, whether the state
directory can be written (probed, not assumed), session counts, and the version.

The `/metrics` endpoint (port 3333) exposes all Prometheus metrics in text format — useful for checking live state without logs:

```bash
curl -s http://localhost:3333/metrics | grep eink_
```

Key metrics for debugging — the symptom-to-metric table, kept next to the code that
defines them so a test can hold it to account: [`server/METRICS.md`](server/METRICS.md).
The common ones:

| Symptom | Metric to check |
|---|---|
| Sessions not submitting | `eink_sessions_submitted_total`, `eink_sessions_active` |
| OCR failures | `eink_ocr_requests_total{result="error"}` |
| WebSocket drops | `eink_ws_connections_active` |
| Webhook not firing | `eink_webhook_total{result="error"}` |

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

### Authentication is opt-in

The server binds `0.0.0.0:3333`. Without `EINK_TOKEN` it has no auth (see Security
notes), which is acceptable only on a trusted network. With a token set, a LAN caller
cannot list, read, edit, submit or remove a review without presenting it; loopback callers
never need it. File reads and callbacks are fenced either way (loopback-only checkouts,
validated `callback_url`). What remains open by design: `/api/health` and `/metrics`, and
the tablet has to carry the token in its settings.

### A SIGKILL between the write and the response still loses that one write

Writes are queued, land atomically (temp file, fsync, rename), a failed write turns the
mutation into a 500, and SIGTERM drains the queue before exit. What is left is a SIGKILL
or power loss inside one handler's window: the change the client was about to be told had
landed. The window is one handler rather than the whole queue, but it is not zero.

### `annotation_images` are machine paths

The field is a filesystem path on the server. `annotation_urls` carries the same images as
addresses, so a remote consumer can fetch them; the path field stays for the local agent
and for compatibility, and nothing new should depend on it.

### Free strokes bind by proximity, with a cap

A stroke the reader never bound attaches to the nearest paragraph within 300 units and is
submitted unanchored beyond it. That cap trades one mistake for another: a note far past
the end of the page is no longer attributed to the last paragraph (it is still submitted
and still visible). If margin notes turn out to sit further away, the cap is a parameter,
not a constant buried in the logic.

### Long-poll waiters hold a task each

Every waiting request parks a task and a `Notify` for up to `long_poll_seconds` (30 s by
default), and the notifier is cleaned up on submit, cancel and expiry. Fine for a handful
of observers; it would need a rethink for hundreds.

### Directory removal still runs under the lock

Session writes are queued and land in `spawn_blocking`, but `remove_dir_all` in `delete`
and `purge_finished` still runs inside the write guard. A purge of hundreds of sessions
stalls every other mutation for its duration.

### OCR timeouts are per process, not per session

`EINK_OCR_TIMEOUT_SECONDS` and `EINK_OCR_INTERACTIVE_TIMEOUT_SECONDS` bound every call the
same way. A session cannot ask for a longer budget for a dense page or a shorter one for a
quick margin note.

### Document images have no quota beyond one request

One image is bounded by the request body (10 MB), and the whole set is bounded only by the
disk: a session may keep as many pictures as a caller uploads, and they are removed with
the session at retention. A server shared by several agents would want a per-session count
and a total-byte cap; for one reviewer on one tablet, the file system is the limit.
