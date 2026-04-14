---
name: eink-webview
description: Render the eink WebView in local Chrome via DevTools MCP. Spawns a kitchen-sink or realistic-scenario session at the tablet viewport, optionally against a fresh `cargo run` server for iterating on `render.rs`. Includes a follow-up flow for verifying the same session on the actual Boox WebView via remote debugging.
---

# E-Ink WebView in Chrome

The Boox tablet's WebView loads `GET /session/{id}` from the eink server (`server/src/app.rs:159` → `render::to_eink_html()`). The HTML is self-contained, so any browser hitting that URL sees what the tablet sees.

This skill wires that URL into headed Chrome through the `chrome-devtools` MCP, posts a session built from the render golden fixtures (or a more realistic scenario), and verifies the result on the actual tablet WebView via Chrome DevTools Protocol.

## Usage

```
/eink-webview                   # kitchen-sink doc, deployed server on 3333
/eink-webview --scenario        # realistic code-review scenario instead
/eink-webview --dev             # fresh `cargo run` on 3344 against current source
/eink-webview --dev --release   # release build (slower start, faster render)
/eink-webview --tablet          # also push session to the Boox WebView and snap
```

Default mode is fast — useful when you only need to look at server output you've already deployed. `--dev` is for iterating on `server/src/render.rs` without restarting the systemd service: each rebuild picks up the latest source. `--tablet` extends the flow with the on-device verification step described below — combine it with the others as needed.

## Steps

### 1. Pick the server

**Default (`/eink-webview`)**: use `http://localhost:3333`. Verify it answers:

```bash
curl -fsS http://localhost:3333/api/sessions >/dev/null \
  || { echo "eink-serve not running; try: systemctl --user start eink-serve"; exit 1; }
SERVER=http://localhost:3333
```

**Dev mode (`/eink-webview --dev`)**: start a fresh `cargo run` on port `3344` with an isolated state dir, in the background. Keep the systemd instance untouched.

```bash
DEV_PORT=3344
DEV_HOME=/tmp/eink-webview-dev
mkdir -p "$DEV_HOME/config/eink-bridge" "$DEV_HOME/data/eink-bridge"
cat > "$DEV_HOME/config/eink-bridge/config.toml" <<EOF
[server]
host = "127.0.0.1"
port = $DEV_PORT
state_dir = "$DEV_HOME/data/eink-bridge"
session_timeout_minutes = 60
long_poll_seconds = 30
EOF

# kill any previous dev instance on this port
fuser -k ${DEV_PORT}/tcp 2>/dev/null || true

cd /home/flakm/programming/flakm/eink-bridge/server
XDG_CONFIG_HOME="$DEV_HOME/config" XDG_DATA_HOME="$DEV_HOME/data" \
  cargo run ${RELEASE:+--release} -p eink-bridge --bin eink-serve \
  > /tmp/eink-webview-dev.log 2>&1 &
disown

# wait for it to bind
for i in $(seq 1 60); do
  curl -fsS http://localhost:$DEV_PORT/api/sessions >/dev/null 2>&1 && break
  sleep 0.5
done
SERVER=http://localhost:$DEV_PORT
```

Pass `--release` through as `RELEASE=1` if requested. Tail `/tmp/eink-webview-dev.log` if startup fails.

### 2. Build / pick the document

**Kitchen-sink (default)**: concatenate every fixture under `server/tests/fixtures/eval/render/` so the page exercises every render branch in one scroll. Best for CSS regressions and per-element verification.

```bash
cd /home/flakm/programming/flakm/eink-bridge/server/tests/fixtures/eval/render
{
  for f in 00-prose.md 01-headings.md 02-inline-code.md 03-blockquote.md \
           04-table.md 05-list-nested.md 06-diff.md \
           10-code-rust.md 11-code-python.md 12-code-no-lang.md \
           20-mermaid-valid.md 21-mindmap-valid.md \
           22-graph-valid.md 23-graph-labeled-edges.md 24-mindmap-colors.md; do
    printf '\n\n---\n\n'
    cat "$f"
  done
} > /tmp/eink-webview-doc.md
```

**Realistic scenario (`--scenario`)**: a hand-written Postgres-migration code review at `harness/skills/eink-webview/scenarios/code-review.md`. It exercises the same render paths in a document that looks like real review content — useful for catching layout problems that only show up with realistic prose density (line wrap, table widths, mindmap notes that span multiple paragraphs).

```bash
cp /home/flakm/programming/flakm/eink-bridge/harness/skills/eink-webview/scenarios/code-review.md /tmp/eink-webview-doc.md
```

If the user supplies a path, use that instead.

### 3. Create the session

```bash
SID=$(curl -fsS -X POST "$SERVER/api/sessions" \
  -H 'Content-Type: text/plain' \
  --data-binary @/tmp/eink-webview-doc.md \
  | python3 -c 'import sys,json;print(json.load(sys.stdin)["id"])')
URL="$SERVER/session/$SID"
echo "$URL"
```

### 4. Launch headed Chrome wired to MCP

The `chrome-devtools` MCP is started with `--autoConnect`, so it reads `~/.config/google-chrome/DevToolsActivePort` to find an existing Chrome. Start headed Chrome with `--remote-debugging-port=9222`, then write the port file. (Same recipe as the `chrome-devtools` skill's "Headed Mode on NixOS" section.)

```bash
# Reuse an already-running headed Chrome if one exists.
if ! curl -fsS http://localhost:9222/json/version >/dev/null 2>&1; then
  DISPLAY=:0 WAYLAND_DISPLAY=wayland-1 \
    google-chrome-stable \
    --remote-debugging-port=9222 \
    --user-data-dir=/tmp/chrome-headed-profile \
    --no-first-run --no-default-browser-check \
    "$URL" > /tmp/chrome.log 2>&1 &
  disown
  for i in $(seq 1 40); do
    curl -fsS http://localhost:9222/json/version >/dev/null 2>&1 && break
    sleep 0.25
  done
fi

WS_PATH=$(curl -fsS http://localhost:9222/json/version \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['webSocketDebuggerUrl'].replace('ws://localhost:9222',''))")
mkdir -p ~/.config/google-chrome
printf "9222\n%s" "$WS_PATH" > ~/.config/google-chrome/DevToolsActivePort
```

### 5. Drive Chrome via MCP — match the tablet viewport

Boox physical display: **1860 × 2480 @ 300 dpi**, devicePixelRatio **1.875**. So the device's CSS pixel viewport is **992 × 1322**.

The HTML uses `<meta name="viewport" content="width=device-width, ...">`, but `body { width: 2400px }` makes the WebView grow the layout viewport to 2400 CSS px (per `useWideViewPort=true` / `loadWithOverviewMode=false` in `MainActivity.kt:225-226`). `#content` is a 992-wide white "page" centered in the body — at default zoom the WebView aligns the visible viewport's right edge with the content border-box, so the visible window lands on the page exactly. Pinch-zoom-out reveals 704 CSS px of gray canvas margin on each side for stylus annotation.

Use Chrome mobile emulation at the device CSS dimensions:

```
mcp__chrome-devtools__list_pages
mcp__chrome-devtools__navigate_page(type="url", url="<URL from step 3>")
mcp__chrome-devtools__emulate(viewport="992x1322,mobile,touch")
```

Verify the layout matches the device:

```
mcp__chrome-devtools__evaluate_script(function="() => ({
  innerWidth: window.innerWidth,
  bodyW: document.body.offsetWidth,
  contentL: document.getElementById('content').getBoundingClientRect().left,
  meta: document.querySelector('meta[name=viewport]').content,
})")
```

Expect `innerWidth: 2401` (the 2400 body forces layout growth), `bodyW: 2400`, `contentL: 704`. **Chrome will not auto-position the visible viewport like the WebView does** — it shows the leftmost 992 CSS px (i.e. the left canvas), so use `take_screenshot(fullPage=true)` to verify the *entire* document instead of relying on the visible slice. Layout differences vs the tablet are noted in the "Tablet vs Chrome differences" section below.

### 6. Inspect

Useful MCP calls once the page is loaded:

- `take_snapshot` — accessibility tree, gives `uid`s for every element.
- `take_screenshot(fullPage=true, format="jpeg", quality=70, filePath="/tmp/eink-render.png")` — what the user sees.
- `evaluate_script` — read `window.__einkElementMap` (built by the element-map JS in `render.rs:1227+`), inspect computed CSS, run query selectors.
- `list_console_messages` — JS errors from diagram rendering, etc.

### 7. Verify on the actual tablet (`--tablet`)

Chrome catches *content* problems (parsing, console errors, missing CSS rules) but the WebView's **visual viewport positioning** is unique to Android — it auto-aligns the visible viewport's right edge with the content border-box, and clamps the minimum pinch-zoom to fit the layout viewport. Always confirm canvas/zoom changes on the device.

Two prerequisites:

- The Boox is plugged in, USB debugging is on, and `adb devices` lists it.
- `MainActivity.kt` calls `WebView.setWebContentsDebuggingEnabled(true)` (already wired in this repo).

```bash
# 1) Find the WebView devtools socket and forward it to a host port.
adb shell "cat /proc/net/unix | grep webview_devtools"
# → @webview_devtools_remote_<pid>

PID=$(adb shell pidof com.flakm.einkbridge | tr -d '\r')
adb forward --remove tcp:9223 2>/dev/null || true
adb forward tcp:9223 localabstract:webview_devtools_remote_$PID

# 2) Resolve the WebSocket debugger URL of the active WebView page.
WS=$(curl -fsS http://localhost:9223/json | \
  python3 -c 'import sys,re;t=sys.stdin.read();m=re.search(r"\"webSocketDebuggerUrl\"\s*:\s*\"([^\"]+)\"",t);print(m.group(1) if m else "")')

# 3) Drive the tablet WebView via raw CDP. Navigate it to the new session.
PAYLOAD=$(python3 -c "import json,sys;print(json.dumps({'id':1,'method':'Page.navigate','params':{'url':sys.argv[1]}}))" "http://amd-pc:3333/session/$SID")
echo "$PAYLOAD" | nix-shell -p websocat --run "websocat -n1 $WS"

# 4) Query the WebView's actual visual viewport state.
PAYLOAD=$(python3 -c 'import json;print(json.dumps({"id":2,"method":"Runtime.evaluate","params":{"expression":"JSON.stringify({iw:innerWidth,vvw:visualViewport.width,vvox:visualViewport.offsetLeft,bw:document.body.offsetWidth,dpr:devicePixelRatio})","returnByValue":True}}))')
echo "$PAYLOAD" | nix-shell -p websocat --run "websocat -n1 $WS"

# 5) Snap the device screen.
adb shell screencap -p /sdcard/s.png
adb pull /sdcard/s.png /tmp/tablet-screenshot.png
adb shell rm /sdcard/s.png
# Then `Read /tmp/tablet-screenshot.png` to look at it.
```

Expected `Runtime.evaluate` result for the current layout:
- `dpr: 1.875`
- `iw: 2400` (layout viewport grew to fit the body's fixed width)
- `vvw: 992`, `vvox: 704` — visible viewport is exactly the centered `#content` border-box
- `bw: 2400`

To test pinch-zoom out without finger gestures, use `Emulation.setPageScaleFactor`:

```bash
PAYLOAD=$(python3 -c 'import json;print(json.dumps({"id":3,"method":"Emulation.setPageScaleFactor","params":{"pageScaleFactor":0.41}}))')
echo "$PAYLOAD" | nix-shell -p websocat --run "websocat -n1 $WS"
```

The minimum scale the WebView accepts is `vvw / iw` (≈ `992 / 2400 = 0.41`); below that the page would have to render whitespace beyond the layout, which the WebView refuses.

#### Stale-cache trap

`MainActivity.openSession()` always calls `webView.loadUrl()` now (since commit `e8acbab`), but the on-disk cache at `files/session_cache/<id>.html` is still re-saved by `onPageFinished`. If the device shows old HTML after a server change, clear the cache and reopen:

```bash
adb shell "run-as com.flakm.einkbridge sh -c 'rm -f files/session_cache/*.html'"
```

### 8. Iteration loop (dev mode)

When the user is editing `render.rs`:

1. Save the file.
2. Restart the dev server: `fuser -k 3344/tcp` then re-run the `cargo run` command from step 1. (The build is incremental — typically a few seconds for `render.rs`.)
3. Re-create the session (step 3) — the old session still exists on the new server only if `state_dir` is preserved; safer to push a fresh one.
4. `mcp__chrome-devtools__navigate_page(type="reload", ignoreCache=true)`.
5. (`--tablet`) re-run step 7's `Page.navigate` against the new session URL.

For tighter loops, just edit the markdown in `/tmp/eink-webview-doc.md` and POST a new session — no rebuild needed.

## Tablet vs Chrome differences

| Behaviour | Chrome mobile emulation | Boox WebView |
|---|---|---|
| Honours `meta[name=viewport]` | Yes (with `mobile` flag) | Yes |
| `body { width: 2400px }` grows layout viewport | Yes | Yes |
| Visible viewport offset within layout | `(0, 0)` always | Right-aligned to `#content` border-box |
| Pinch-zoom minimum scale | `0.25` (CDP default) | `vvw / iw` (≈ 0.41 here) |
| Fonts in physical px | `cssPx * deviceScaleFactor` | `cssPx * 1.875` |

So Chrome is the right place to verify *content* (parse errors, missing styles, broken diagrams) and the tablet is the right place to verify *layout* (canvas margins, default visible position, zoom limits).

## Notes

- **Don't stop the systemd service** unless the user asks. Default mode shares port 3333 with it; dev mode runs on 3344 alongside.
- **Mobile emulation matters.** Without `,mobile`, desktop Chrome ignores `meta[name=viewport]` and lays the page out at desktop width, which is *not* what the tablet shows.
- **Cleanup**: the headed Chrome and dev server stay running across invocations. Tell the user so they can kill them with `fuser -k 9222/tcp 3344/tcp` when done. Tablet WebView CDP forwarding cleans up with `adb forward --remove tcp:9223`.
- **Why two documents**: the kitchen-sink doc exercises every render branch in isolation (good for CSS regressions). The code-review scenario exercises real-content density (good for catching layout problems that only appear with realistic prose, table widths, multi-paragraph mindmap notes).
