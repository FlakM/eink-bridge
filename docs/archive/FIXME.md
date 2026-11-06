# FIXME

What is worth fixing next, in the order it should be done. Each item says what hurts, where
it lives, what done looks like, and how to prove it. Items in one phase are independent of
each other; a later phase may lean on an earlier one where noted.

Baseline (2026-10-19): server 221 lib tests + 20 test binaries green, clippy clean,
66 mutations caught, 280 Android tests, 69 harness tests. Keep every one of those green;
add a mutation case for every new invariant (`server/scripts/mutation-check.py`).

## Phase 1 -- the feedback path cannot lose a note

### 1. One channel server per Claude instance

- Hurts: `harness/channels/eink-channel/channel.ts:156` binds port 8789 with no `error`
  handler, so a second Claude Code instance crashes or is never reached. `deliver()`
  (`:31`) skips sessions it did not subscribe to but still answers `200 ok` (`:149`), so
  the server believes the feedback landed.
- Done: each instance picks a free port and passes it to `eink-review push
  --callback-url`; an unknown session answers `404`, which the server counts as a failed
  delivery and keeps the feedback pending; every delivery and every skip is logged to
  stderr with the session id.
- Prove: `harness/tests/test_feedback_delivery.py` posts a webhook for an unsubscribed
  session and asserts non-2xx; two channels start on the same machine without an error.

### 2. Feedback survives a Claude restart

- Hurts: subscriptions live in memory (`channel.ts:11`); after a restart every webhook is
  silently acknowledged until something re-subscribes. `/eink continue`
  (`harness/skills/eink/SKILL.md:34-50`) never re-calls `subscribe_session`.
- Done: `/eink continue` re-subscribes and then reads `GET /api/sessions/{id}/feedback`
  for anything pending; the server retries a failed webhook three times with backoff and
  exposes the count in `eink_webhook_total{result="retried"}`.
- Prove: a webhook test with a receiver that fails twice then succeeds; a skill asset test
  that the continue flow mentions `subscribe_session` and `/feedback`.

### 3. The CLI does the whole round trip

Superseded by `PLAN-cli-first.md`, which widens this into every procedure the skills
carry; the flags below are its Phase 1.

- Hurts: `update` takes only `<id> <file>` (`server/src/cli.rs:61-66`); no
  `--expected-version`, `--feedback-id`, `--consumed`, no `content` or `source` command,
  `push` has no `--tags`, `result` exits 0 for both pending and lgtm (`:497-500`), `list`
  has no `--json` and never checks the HTTP status (`:548`), the server URL is a flag only
  (`:15`).
- Done: `update --expected-version --feedback-id --consumed a,b`, `content <id>`,
  `source <id> <path:line>`, `push --tag k=v`, `list --json`, exit code 3 for "still
  active", `--server` defaulting to `$EINK_SERVER_URL`. Skills call the CLI instead of
  curl; `harness/skills/eink-update/SKILL.md:59-63` loses its JSON fallback.
- Prove: `tests/cli_integration_test.rs` for each command; a harness test that extracts
  every `eink-review` invocation from `harness/skills/*/SKILL.md` and checks each flag
  against `eink-review <cmd> --help`.

### 4. The agent narrates an update while it happens (feature)

- Hurts: after the reader taps UPDATE the tablet shows "Waiting for agent…" and nothing
  else until the whole document is replaced, often a minute or more. The reader cannot
  tell whether the notes were read, which section is being rewritten, or whether the
  agent is stuck, and a rewrite that lands all at once is hard to relate to the notes
  that caused it.
- Done: the agent sends short progress lines while it works and the tablet shows them
  in place.
  - Server: `POST /api/sessions/{id}/progress` with `{"feedback_id", "message",
    "step", "of"}` (message ≤ 120 chars, step/of optional); stored as the session's
    `current_progress`, cleared when the content update that names the same
    `feedback_id` lands; broadcast over `/ws/{id}` as `agent_progress`; included in the
    durable WS snapshot so a reconnecting tablet sees the latest line; metric
    `eink_agent_progress_total`.
  - CLI: `eink-review progress <id> "<message>" [--step n --of m]`.
  - Channel: a `report_progress` MCP tool beside `update_session`, taking the same
    fields, so the agent never writes curl.
  - Skill (`harness/skills/eink/SKILL.md`): after `annotation_result` arrives the agent
    posts one line per step, in the reader's words, not the agent's: "Read 4 notes on
    *Reading order* and *Checks*", "Rewriting *Important pieces* B and C", "Adding the
    missing test you asked for", "Rendering". One line per step, no more than one every
    few seconds, and the final content update carries `consumed_annotations` as today.
    The `ihaveadhd` agent is not needed for these lines; they are status, not prose.
  - Tablet: `agent_progress` replaces the static "Waiting for agent…" text on the UPDATE
    button with the latest line and, when `step/of` is present, a thin segmented bar
    under the toolbar (no animation, e-ink). The processing overlay shows the same line.
    When the `version_updated` arrives the line is replaced by "Document updated" as
    today, and the pending notes the update consumed are flashed once on the new page so
    the reader sees which notes became which change.
  - Order of work: server + WS + CLI first (testable without a tablet), then the channel
    tool and the skill text, then the Android view.
- Prove: `ws_test`: a progress post reaches a connected watcher and a late watcher
  gets the latest line in its snapshot; `api_edge_test`: progress for an unknown
  feedback id is 409, an over-long message is 400, a content update clears it;
  `cli_integration_test`: the `progress` command; `test_feedback_delivery.py`: the MCP
  tool posts the right body; Android: a `MainActivityTest` that an `agent_progress`
  message changes the UPDATE button text and that `version_updated` resets it.

## Phase 2 -- trust boundary

### 5. A bearer token for non-loopback callers

- Hurts: anyone on the LAN can read, edit, submit or remove a review
  (`CLAUDE.md` Security notes). File reads and callbacks are fenced; the API is not.
- Done: `EINK_TOKEN` (or `token` in `config.toml`). Loopback callers need nothing.
  Everything else sends `Authorization: Bearer`; `/session/{id}`, `/ws/{id}` and
  `/api/sessions/{id}/annotation/{i}` accept `?token=` for the WebView. The tablet gets a
  token field beside the server URL, stored in `SessionRepository`, sent by
  `SessionViewModel`, `WebSocketClient`, `SubmissionManager`, `OcrClient` and the
  WebView via `loadUrl(url, headers)`. Unset token keeps today's open behaviour.
- Prove: `api_edge_test`: LAN caller without token is 401 on every mutating route and on
  the list, loopback caller without token passes, wrong token is 401; Android test that
  every HTTP client attaches the header when a token is stored.

### 6. Finished sessions leave the disk, and the disk is measured

- Hurts: unstarred Cancelled/Expired sessions are not reloaded (`server/src/session.rs:283`)
  but their directories stay forever; `eink_sessions_stored` counts memory only, so the
  "Disk filling up" row in `server/METRICS.md` under-reports. `remove_dir_all` still runs
  under the write lock in `delete` and `purge_finished`.
- Done: a retention sweep (default 30 days, `EINK_RETENTION_DAYS`) deletes directories of
  terminal unstarred sessions on startup and daily, off the lock; a gauge
  `eink_session_dirs_on_disk`; `delete`/`purge` collect paths under the lock and remove
  them after it.
- Prove: session unit tests for the sweep (keeps starred, keeps active, removes expired
  older than the window); `metrics::` test that the new gauge is in `METRICS.md`.

## Phase 3 -- build, deploy, CI

### 7. CI checks everything the justfile does

- Hurts: `.github/workflows/ci.yml` runs fmt, clippy, test, eval. Not run: harness tests,
  `just lint-assets`, Android tests, `just test-mutations`, `nix build`.
- Done: jobs `harness` (python unittest + `node --check`), `android`
  (`./gradlew testDebugUnitTest` with the SDK from `flake.nix`), `mutations`, `nix-build`.
- Prove: a PR that breaks a Kotlin test goes red.

### 8. Reproducible, versioned Android build

- Hurts: `versionCode = 1`, `versionName = "0.1.0"` fixed
  (`android/app/build.gradle.kts:15-16`); no signing config, so a second machine's debug
  key cannot install over the first; `android/shell.nix` uses unpinned `<nixpkgs>` and
  duplicates the flake shell; `nix develop` rewrites `~/.gradle/gradle.properties`
  (`flake.nix:123-136`).
- Done: version from `git describe` (code = commit count), shown in the list header and
  sent as `User-Agent: eink-bridge-android/<version>`; a committed debug keystore;
  `shell.nix` deleted; the APK built by a flake package.
- Prove: `just apk` from a clean clone installs over the existing app; health or metrics
  show the tablet build.

### 9. One deploy story that works on a second machine

- Hurts: the service override that runs `./result/bin/eink-serve` lives in another repo
  (`CLAUDE.md` Deployment gap); `harness/skills/deploy-eink/SKILL.md:14-43` hard-codes
  `/home/flakm`, `nix_dots`, `#amd-pc` and uses `git add -p`; README packages, skills,
  API and state-dir paths are stale (`README.md:82,212,456,565,603`); `~/.claude/skills`
  is read-only in the Nix store but README says to create skills there (`:592`).
- Done: a home-manager module exported from this flake that installs the server unit,
  the CLI, the skills, the agents, the output style and the channel MCP registration; a
  README "New machine" section (server, MCP, APK, `just boox-setup`, app server URL);
  `deploy-eink` reads paths from env; `verify-deploy` passes on a module-installed host.
- Prove: follow the README on a fresh user account.

## Phase 4 -- the app and the renderer

### 10. Split the Android god objects

- Hurts: `MainActivity.kt` 1144 lines, `PenOverlay.kt` 1489; the `onPageFinished`
  wiring that gives the pen back is untested; 3 `evaluateJavascript` calls bypass
  `WebViewBridge`; 6 prefs accesses bypass `SessionRepository`.
- Done: `SessionScreen` (toolbar, popovers, mode state) and `PageLifecycle`
  (`onPageStarted`/`onPageFinished`/reconcile) extracted with constructor-injected
  collaborators; `OcrManager` callbacks become a `SharedFlow`; `MainActivity` under 400
  lines.
- Prove: a Robolectric test drives `onPageFinished` with a bridge whose element-map
  callback never answers and asserts the pen, ink view and UPDATE button come back.

### 11. Rendered pages are cached and the page scripts are tested

- Hurts: every `GET /session/{id}` re-renders the document (`server/src/app.rs`,
  `rendered_page`); `server/assets/element-map.js` and `bootstrap.js` are ~1400 lines of
  hit-testing and viewport logic covered only through goldens.
- Done: an LRU of rendered HTML keyed by `(id, version)` invalidated on content update; a
  node test runner for the pure functions (element lookup, nearest paragraph, viewport
  report) under `server/assets/tests/`, run by `just lint-assets`.
- Prove: a render-cache hit metric; `just lint-assets` fails on a broken hit-test.

## Smaller items, any time

- `harness/skills/eink-pr/SKILL.md:31,106,123,297` vs `scripts/registry.py:33-40`: three
  socket-path spellings, two worktree layouts, a quoted `~` that is never expanded,
  `herdr tab close` for a workspace it created, the registry written by hand. Make
  `registry.py` the CLI the skill calls; add a file lock and a unique temp name
  (`:87-93`); warn instead of silently resetting a corrupt file (`:82`).
- `harness/output-styles/eink-review.md:8-19` and `harness/skills/eink/SKILL.md:239`
  disagree on verdict casing (server sends lowercase plus `reject`/`question`,
  `server/src/verdict.rs:4-25`), diagram count, bullets vs tables and what amber means.
  One source, the skill points to it.
- `harness/skills/eink-update/SKILL.md:48-56` writes a fixed `/tmp/eink-update-XXXXX.md`;
  use `mktemp`, then fold the skill into `eink` once item 3 lands.
- `harness/skills/eink/scripts/watch-feedback.py:93` `json.loads` is unguarded; a bad
  `EVENT:` line kills the watcher without its `WATCH_ENDED` receipt.
- `channel.ts:113` does not URL-encode `session_id`; `deliveredFeedback` grows without
  bound; the shebang needs `env -S`; `@modelcontextprotocol/sdk` is `^1.0.0`.
- `harness/skills/eink-webview/SKILL.md:54,75,91,192` hard-code `/home/flakm` and
  `amd-pc`; use `$EINK_BRIDGE_ROOT` and `$EINK_SERVER_URL`.
- `harness/tests/test_eink_pr_reader.py:285` skips without a server, so CI never runs it;
  `harness/skills/eink/evals/evals.json` is run by nothing.
- OCR bounds are per process (`EINK_OCR_TIMEOUT_SECONDS`); a per-request `timeout_seconds`
  on `/api/ocr` and `request_update`, capped by the env bound, would let a dense page ask
  for more.
- `flake.lock` inputs date from 2026-03; a monthly `nix flake update` job.
- The mutation harness can abort on a transient Kotlin incremental-compile failure
  (`BUILD FAILED in 2s`); retry the gradle target once before declaring `NoTestsRan`.
