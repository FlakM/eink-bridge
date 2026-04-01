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
just lint          # fmt + clippy
just deploy        # nix build + restart service
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
| `render.rs` diagram parsing | `cargo test render::tests::graph` / `mindmap` / `mermaid` |
| `app.rs` handlers | `cargo test --test health_test` |
| `session.rs` lifecycle | `cargo test --test session_lifecycle_test` |
| Long-poll / notify | `cargo test --test long_poll_test` |
| CLI `push`/`result`/`cancel` | `cargo test --test cli_integration_test` |
| API response shapes | `just eval` |
| Full round-trip | `cargo test --test e2e_test` |

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

## Adding render tests

When adding a new feature to `render.rs`:
1. Add a unit test in `src/render.rs` under `mod tests` that calls `render(markdown)` and asserts on the HTML string.
2. Add an HTTP-level test in `tests/render_validation_test.rs` if the feature involves session creation or asset serving.
3. Add a render fixture in `tests/fixtures/eval/render/` and run `just eval-update` to generate the golden.
4. Run `just test-render` to iterate quickly; `just ci` before committing to run the full suite.
