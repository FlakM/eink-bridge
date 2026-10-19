# Overnight plan — 2026-10-19

Implementing [`FIXME.md`](../FIXME.md) in its stated order. This file is the decision log:
each entry records what I chose *before* writing the code, what I rejected, and how the
choice is proved. Outcomes are appended as items land.

## How the night runs

- **Branch `overnight/fixme`.** One commit per FIXME item, plus docs and mutation cases.
  Merge to `main` only when a phase's gate is green (`just ci`, `just test-mutations`,
  Android tests). A half-finished item stays on the branch and is recorded under
  *Stalled*; `main` never goes red.
- **Definition of done per item:** code + the tests named in FIXME + a mutation case where
  `server/scripts/mutation-check.py` can express the defect + a decision entry here.
- **Stop rule:** three failed attempts at the same test means revert that item's working
  tree, record why, and move to the next item. A working Phase 1 item beats a broken
  Phase 4 one.
- **Deploy:** `just deploy` + `just verify-deploy` after Phases 1 and 2 merge (both change
  the server). Android APK after the Phase 4 view work lands.
- **Morning artifact:** a review session on the tablet summarising what landed, what
  stalled, and the two or three things only the user can decide. Readable in five minutes.

## Decisions

### D1 — Register the callback when a session is claimed, not when it is pushed

FIXME item 1 says each channel instance should pick a free port and pass it to
`eink-review push --callback-url`. That has a chicken-and-egg problem: `push` runs
*before* `subscribe_session`, so the agent does not yet know which channel instance owns
the session — and on a restart the stored URL points at a process that no longer exists.

Decision: `subscribe_session` registers the instance's own `http://127.0.0.1:<port>/…`
with the server (`POST /api/sessions/{id}/callback`), then subscribes locally, then reads
pending feedback. `push --callback-url` stays as an override for other clients. A restart
is cured by the same call, which is exactly what item 2 needs.

Rejected: a discovery file per instance (ambiguous with two instances); a fixed port with
a crash handler (does not let two instances coexist).

### D2 — An unsubscribed session is a failed delivery, not a success

`deliver()` returns `false` for an unknown session and the HTTP handler answers **404**.
The server already treats non-2xx as a failed webhook; with D3 it will retry. Every
delivery and every skip writes one stderr line with the session id (`stderr`, never
stdout — stdout is the MCP transport).

### D3 — Retry is a server concern, bounded and visible

Three attempts with 250 ms / 1 s / 4 s backoff, jittered, inside the existing
`send_webhook` task. Retry on connection error and on any non-2xx (including the 404 from
D2). Count `eink_webhook_total{result="retried"}`; keep `error` for the final failure.
The pending feedback stays on the session until a 2xx, so `/feedback` is the durable
queue. No retry when there is no `callback_url` at all — that is "not asked for", not
"failed".

### D4 — Exit codes are a contract, so they get names

`result`: `0` approved/lgtm, `1` changes/reject/question, `2` usage or transport error,
`3` still active (204). Today active exits `0`, which makes a pipeline believe an unreviewed
document passed. `list --json` prints the array unchanged; `list` checks status first and
fails with the body snippet rather than trying to parse an error page. `--server` reads
`EINK_SERVER_URL` when the flag is absent (clap `env` feature added to the dependency).

### D5 — CLI flags are extracted from the skills by a test, not by trust

`harness/tests/test_cli_flags.py` finds every `eink-review <cmd> …` invocation in
`harness/skills/*/SKILL.md`, collects `--flags`, and asserts each appears in
`eink-review <cmd> --help`. It builds the binary if missing. This is what makes "skills
call the CLI instead of curl" stay true after item 3.

### D6 — Progress is session state, delivered by the page's own channel

`agent_progress` is stored on the session (`progress: {feedback_id, message, step, of,
updated_at}`), included in the WS snapshot, and cleared only by the content update that
names the same `feedback_id` — not by time, so a late reconnect still sees the last line.
Validated at the route: unknown/stale feedback id → 409, message over 120 chars → 400,
`step > of` → 400. Metric `eink_agent_progress_total{result}`.

It is **not** rendered into the initial HTML; the page receives it over WS/JS. That keeps
the future render cache (item 11) safe: HTML stays a pure function of `(id, version)`.

Rejected: progress as a special feedback annotation (would be consumed and counted);
progress in a separate `/progress` poll (the tablet already holds a WS).

### D7 — Token: bearer for API clients, cookie for the WebView, `?token=` only where FIXME says

Loopback callers stay open. Non-loopback needs `Authorization: Bearer $EINK_TOKEN`
(constant-time compare, token from env or `config.toml`). The rendered page sets an
`HttpOnly; SameSite=Strict` cookie so `<img>` and page-JS fetches authenticate without a
token in the markup; the Android app sets that cookie via `CookieManager` after loading
the page with the header. `?token=` is accepted only on `GET /session/{id}`, `GET
/ws/{id}`, and the annotation image route — the three places a WebView or a deep link
cannot set a header. Unset token = today's behaviour, with a startup warning.

Rejected: token in every annotation URL (it lands in page source and logs); mTLS (not
worth the ceremony on a home LAN).

### D8 — Retention sweeps off the lock, and the gauge is honest

`EINK_RETENTION_DAYS` (default 30). A sweep at startup and every 24 h removes directories
of **terminal, unstarred** sessions older than the window. `delete` and `purge_finished`
stop calling `remove_dir_all` under the write guard: they collect paths, drop the guard,
then remove. The gauge `eink_session_dirs_on_disk` is set after every sweep, delete and
purge (a cheap directory count, not an estimate from memory).

### D9 — CI mirrors the justfile, with the expensive jobs bounded

New jobs: `harness` (python unittest + `node --check` on the channel and assets),
`android` (`./gradlew testDebugUnitTest` from the dev shell), `mutations`, `nix-build`.
`mutations` gets a 60-minute timeout and only runs on pull requests and `main` — it is
the job that catches a green-but-vacuous test, so it is not optional, just scheduled.
Nix and Gradle caches keep the Android job under ten minutes.

### D10 — Android version from git, signed by a committed *debug* key

`versionName` = `git describe --tags --always --dirty`, `versionCode` = commit count, with
a fallback to `1` when git is unavailable (Nix sandbox). Sent as
`User-Agent: eink-bridge-android/<version>`; shown in the list header. The debug keystore
is committed so a clean clone can install over the existing app — it is a **debug** key,
never a release key, and the module comment says so. `android/shell.nix` is deleted (the
flake is the only shell); the `~/.gradle/gradle.properties` rewrite is replaced by
`GRADLE_USER_HOME` inside the dev shell.

### D11 — The deploy story is exported from this flake

`nix/home-manager.nix` becomes `homeManagerModules.eink-bridge`: server unit, CLI, skills,
agents, output style, channel MCP registration. `nix_dots` imports it and loses its local
copy. `deploy-eink` reads `EINK_BRIDGE_ROOT`, `EINK_HOST`, `EINK_FLAKE` from the
environment with today's values as fallbacks. README gains a *New machine* section.
`verify-deploy` is unchanged but the README says why it matters.

### D12 — Split the pen wiring first, because it is the part that just broke

`PageLifecycle` is extracted before `SessionScreen`: it is the `onPageStarted` /
`onPageFinished` / reconcile path that left the pen dead, and the Robolectric test FIXME
asks for (a bridge whose element-map callback never answers) is the regression test for
last night's bug. Order: `PageLifecycle` → `OcrManager` flow → `SessionScreen` → bridge and
prefs cleanup. Each step compiles, tests and installs on its own.

### D13 — Cache rendered pages by `(id, version)`, invalidate on write

An LRU of 32 rendered documents in `AppState`, filled in `render_session`, cleared for an
id on any content update/removal. `eink_render_cache_total{result="hit"|"miss"}`. Safe
because D6 keeps per-request state out of the HTML. The JS unit tests run under
`node --test` from `just lint-assets`; the pure functions are exported from
`element-map.js` rather than copied.

### D14 — Smaller items are a final cleanup pass, not filler

Each gets its own commit in the order they appear in FIXME, with a test where one is
missing: registry CLI + lock, one verdict vocabulary, `mktemp` in the update skill, the
watcher's guarded parse, channel URL-encoding + bounded dedupe set + shebang, env vars in
`eink-webview`, un-skipped reader test, per-request OCR timeout, monthly flake update,
mutation-harness gradle retry.

## Execution order and proof

| # | Item | Proof | Mutation |
|---|---|---|---|
| 1 | Channel per instance | `test_feedback_delivery.py`: unsubscribed → 404; two instances start | python, n/a |
| 2 | Feedback survives restart | flaky receiver (fail, fail, succeed); skill asset test names `subscribe_session` | retry loop removed → flaky test red |
| 3 | CLI round trip | `cli_integration_test` per command; `test_cli_flags.py` | exit code 3 → 0; bare DELETE in update |
| 4 | Progress narration | `ws_test`, `api_edge_test`, CLI test, channel test, Android test | progress not cleared on update |
| 5 | Bearer token | `api_edge_test` 401 matrix; Android interceptor test | token check skipped |
| 6 | Retention + disk gauge | session unit tests; metrics reference test | sweep keeps unstarred terminal |
| 7 | CI parity | a Kotlin break goes red on the PR | n/a |
| 8 | Reproducible APK | clean-clone install over existing; User-Agent visible | n/a |
| 9 | Deploy on a second machine | README followed on a fresh account | n/a |
| 10 | Android split | Robolectric never-answering bridge test | wiring not called |
| 11 | Render cache + JS tests | cache hit metric; broken hit-test fails lint-assets | cache key drops version |
| s | Smaller items | one test each where missing | where expressible |

## Status

| Item | State | Commit |
|---|---|---|
| plan | landed | — |
| 1 | landed | item 1 commit |
| 2 | landed | item 2 commit |
| 3–11, s | queued | |

## Log

- **2026-10-19, evening.** Read FIXME against the code. Confirmed: channel binds 8789 with
  no error handler and answers 200 for unsubscribed sessions; `send_webhook` has no retry;
  `cmd_result` returns `Ok(())` on 204; `cmd_update` takes only id+file; ConnectInfo already
  exists (token work is cheap); `/feedback` already exists (item 2 is mostly a skill and
  retry change); the render route is uncached. Wrote D1–D14 above; starting item 1.

- **Item 1, landed.** The channel binds port 0, writes `listening on <url>` to stderr, and
  `subscribe_session` now registers that URL with the server before subscribing. Unknown
  sessions get 404 and are logged; deliveries and duplicates are logged with the session
  id. `PUT /api/sessions/{id}/callback` refuses a remote caller (403) and a non-loopback
  URL (400), and is covered in `api_edge_test`. Two instances and the 404 are covered in
  `test_feedback_delivery.py`. Deviation from FIXME recorded as D1: registering on claim
  beats passing the URL to `push`, which cannot know the owning instance's port.

- **Item 2, landed.** `send_webhook` retries three times (250 ms / 1 s / 4 s) on a
  connection error or any non-2xx, counts `result="retried"`, and keeps the feedback
  pending until a 2xx. The backoff is a field on `AppState` so `common::test_app` can zero
  it — otherwise the existing metric test would wait five seconds to see a failure, and a
  test that slow gets deleted. `/eink continue` now says, in the channel path, to call
  `subscribe_session` again (which re-registers the callback and re-reads `/feedback`);
  the portable path keeps the watcher. Proved by a receiver that refuses twice and then
  accepts, plus an asset test that the continue section names both.
