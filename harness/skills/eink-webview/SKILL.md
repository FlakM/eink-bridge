---
name: eink-webview
description: Render the eink WebView in local Chrome via DevTools MCP. Spawns a kitchen-sink session (every render element from the golden fixtures) at the exact tablet viewport, optionally against a fresh `cargo run` server for iterating on `render.rs`.
---

# E-Ink WebView in Chrome

The Boox tablet's WebView loads `GET /session/{id}` from the eink server (`server/src/app.rs:159` → `render::to_eink_html()`). The HTML is self-contained, so any browser hitting that URL sees what the tablet sees.

This skill wires that URL into headed Chrome through the `chrome-devtools` MCP, sized to match the tablet's actual viewport, and renders a kitchen-sink document built from the render golden fixtures so every element type (prose, headings, code, diff, table, lists, mermaid, mindmap, graph, ...) is on screen at once.

## Usage

```
/eink-webview              # use whatever server is running on 3333 (systemd)
/eink-webview --dev        # spawn a fresh `cargo run` on 3344 against current source
/eink-webview --dev --release  # release build (slower start, faster render)
```

Default mode is fast — useful when you only need to look at server output you've already deployed. `--dev` is for iterating on `server/src/render.rs` without restarting the systemd service: each rebuild picks up the latest source.

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

### 2. Build the kitchen-sink document

Concatenate every fixture under `server/tests/fixtures/eval/render/` so the page exercises all render paths in one scroll:

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
} > /tmp/eink-webview-kitchen-sink.md
```

If the user wants a different document, accept a file path argument and use that instead of the kitchen-sink build.

### 3. Create the session

```bash
SID=$(curl -fsS -X POST "$SERVER/api/sessions" \
  -H 'Content-Type: text/plain' \
  --data-binary @/tmp/eink-webview-kitchen-sink.md \
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

Boox physical display: **1860 × 2480 @ 300 dpi**. The HTML's `<meta name="viewport" content="width=2800, ...">` is honored by the Android WebView (`useWideViewPort=true`, `loadWithOverviewMode=false` in `MainActivity.kt:225-226`), so the layout viewport is 2800 CSS px wide and the device shows the leftmost slice at 1:1.

Use mobile emulation so Chrome obeys the meta viewport the same way:

```
mcp__chrome-devtools__list_pages
mcp__chrome-devtools__navigate_page(type="url", url="<URL from step 3>")
mcp__chrome-devtools__emulate(viewport="1860x2480x2,mobile,touch")
```

Verify the layout matches the device:

```
mcp__chrome-devtools__evaluate_script(function="() => ({
  innerWidth: window.innerWidth,
  scrollWidth: document.documentElement.scrollWidth,
  meta: document.querySelector('meta[name=viewport]').content,
})")
```

Expect `innerWidth: 2800` — that confirms the meta viewport was applied. If `innerWidth` is 1860, mobile emulation didn't take; re-run `emulate`.

### 6. Inspect

Useful MCP calls once the page is loaded:

- `take_snapshot` — accessibility tree, gives `uid`s for every element.
- `take_screenshot(fullPage=true, format="jpeg", quality=70, filePath="/tmp/eink-render.png")` — what the user sees.
- `evaluate_script` — read `window.__einkElementMap` (built by the element-map JS in `render.rs:1227+`), inspect computed CSS, run query selectors.
- `list_console_messages` — JS errors from diagram rendering, etc.

### 7. Iteration loop (dev mode)

When the user is editing `render.rs`:

1. Save the file.
2. Restart the dev server: `fuser -k 3344/tcp` then re-run the `cargo run` command from step 1. (The build is incremental — typically a few seconds for `render.rs`.)
3. Re-create the session (step 3) — the old session still exists on the new server only if `state_dir` is preserved; safer to push a fresh one.
4. `mcp__chrome-devtools__navigate_page(type="reload", ignoreCache=true)`.

For tighter loops, just edit the markdown in `/tmp/eink-webview-kitchen-sink.md` and POST a new session — no rebuild needed.

## Notes

- **Don't stop the systemd service** unless the user asks. Default mode shares port 3333 with it; dev mode runs on 3344 alongside.
- **Mobile emulation matters.** Without `,mobile`, desktop Chrome ignores `meta[name=viewport]` and lays the page out at 1860 px wide, which is *not* what the tablet shows.
- **Cleanup**: the headed Chrome and dev server stay running across invocations. Tell the user so they can kill them with `fuser -k 9222/tcp 3344/tcp` when done.
- **Why the kitchen-sink doc**: every golden fixture exercises a different render branch, so a single page surfaces regressions in any of them. If `just eval-update` would touch a fixture, this view shows the visual impact instantly.
