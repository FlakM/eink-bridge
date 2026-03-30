# E-Ink Bridge

Push content from a dev session to a Boox e-ink tablet, annotate with the Wacom pen, and get handwritten notes back into the exact process that sent them.

A **blocking review primitive**: Claude Code or neovim pushes markdown, the user reads and annotates on e-ink, taps Done, and the pen strokes flow back as PNG alongside any typed notes.

## Architecture

```
+---------------------------+          LAN          +------------------------------+
|  Dev Machine              |                       |  Boox Tab Ultra C Pro        |
|                           |                       |                              |
|  ┌─────────────────────┐  |  GET /session/{id}    |  ┌────────────────────────┐  |
|  │      eink-serve     │◄─┼───────────────────────┤  │     Android App        │  |
|  │      (Rust/axum)    │  |  POST /submit          |  │                        │  |
|  │                     │◄─┼───────────────────────┤  │  session list           │  |
|  └──────────┬──────────┘  |   (text + PNG)        |  │  read content           │  |
|             │             |                       |  │  annotate with pen      │  |
|  ┌──────────┴──────────┐  |                       |  │  submit review          │  |
|  │  Claude Code        │  |  /eink → blocks       |  └────────────────────────┘  |
|  │  Neovim             │  |  <leader>ep → async   |                              |
|  │  Shell              │  |  eink-review push     +------------------------------+
|  └─────────────────────┘  |
+---------------------------+
```

### Flow

1. Caller creates a session — `POST /api/sessions` with markdown
2. Android app polls for new sessions, loads the e-ink HTML
3. User reads content, annotates with the Wacom pen (native Onyx SDK, ~20ms latency)
4. User taps Done — strokes exported as PNG, POSTed to server
5. Server notifies the blocking caller via long-poll
6. Caller receives typed notes + annotation image paths

## Repository Structure

```
eink-bridge/
├── flake.nix                    Nix build (crane)
├── server/                      Rust: server + CLI + mock device
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs              eink-serve — HTTP server daemon
│   │   ├── cli.rs               eink-review — CLI (push/result/cancel/list)
│   │   ├── mock_device.rs       eink-mock-device — simulates Boox for testing
│   │   ├── lib.rs               library crate (shared modules)
│   │   ├── app.rs               axum routes, state, notify-based long-poll
│   │   ├── session.rs           session model, disk persistence, expiry
│   │   ├── config.rs            TOML config loading
│   │   └── render.rs            markdown → e-ink optimized HTML
│   └── tests/
│       ├── health_test.rs       API basics
│       ├── session_lifecycle_test.rs  persistence, expiry, filtering
│       ├── long_poll_test.rs    notify wake, timeout, cancel
│       ├── cli_integration_test.rs   CLI round-trips via spawned binary
│       └── e2e_test.rs          full loop: server + mock device + CLI
└── android/                     Kotlin: Boox client app
    ├── shell.nix                Nix shell for Android SDK
    ├── app/src/main/java/com/flakm/einkbridge/
    │   ├── MainActivity.kt      session list, WebView, pen controls
    │   ├── PenOverlay.kt        Onyx Pen SDK integration, stroke capture, PNG export
    │   └── SessionAdapter.kt    RecyclerView adapter with relative timestamps
    └── app/src/main/res/        layouts, themes, strings
```

### Related files in [nix_dots](https://github.com/FlakM/nix_dots)

- `home-manager/modules/eink-bridge.nix` — systemd user service + PATH
- `home-manager/modules/nvim/config/eink-bridge.lua` — neovim keybindings
- `home-manager/modules/claude/skills/eink/SKILL.md` — `/eink` Claude Code skill

## Usage

### CLI

```bash
# blocking — waits until Boox user submits
eink-review push document.md

# stdin
cat notes.md | eink-review push -

# non-blocking — prints session ID
eink-review push --async document.md

# check result
eink-review result <session-id>

# manage
eink-review list
eink-review cancel <session-id>
```

Output on submit:
```
--- review notes (session abc123) ---

## Typed Notes
The caching layer seems overengineered. Start with simple TTL.

## Attached Images
~/.local/state/eink-bridge/sessions/abc123/annotations/img_001.png
```

### Neovim

- `<leader>ep` — push current buffer, notes open in split on return
- `<leader>ec` — cancel active review
- `<leader>ea` — list sessions

### Claude Code

```
/eink [file]  — push to Boox, block until notes come back
```

Mid-conversation: push an explanation to the Boox, read with a pen, scribble thoughts, tap Done. Claude sees the typed notes and handwritten annotations and continues informed by your feedback.

## API

```
POST   /api/sessions              create session (markdown body, ?title= query param)
GET    /api/sessions              list sessions (?status= filter, sorted newest first)
GET    /api/sessions/{id}         session metadata
GET    /api/sessions/{id}/result  long-poll until submitted
DELETE /api/sessions/{id}         cancel session
POST   /api/sessions/{id}/submit  submit review (multipart: typed_notes + annotation PNGs)
GET    /session/{id}              e-ink optimized HTML
GET    /api/health                health check
```

## Testing

All server-side components are testable without the Boox device:

```bash
# unit + integration tests
cargo test

# manual E2E without device
eink-serve &
eink-mock-device --notes "LGTM" --once &
eink-review push document.md   # blocks, receives "LGTM"
```

17 integration tests cover session lifecycle, long-poll, CLI round-trips, and full E2E.

## Building

### Server (Nix)

```bash
nix build          # produces result/bin/{eink-serve,eink-review,eink-mock-device}
```

Or with cargo directly:

```bash
cd server && cargo build --release
```

### Android app

```bash
cd android
nix-shell shell.nix --run './gradlew assembleDebug'
# APK at app/build/outputs/apk/debug/app-debug.apk
```

Sideload to Boox:

```bash
adb install app/build/outputs/apk/debug/app-debug.apk
```

Note: the Boox requires `hidden_api_policy` set to 1 for the Onyx Pen SDK:

```bash
adb shell settings put global hidden_api_policy 1
```

### Deployment via nix_dots

```bash
# add eink-bridge to nix_dots flake inputs, then:
sudo nixos-rebuild switch --flake ~/programming/flakm/nix_dots#amd-pc
```

This installs all binaries, starts `eink-serve` as a systemd user service, wires the neovim plugin, and deploys the `/eink` Claude Code skill.

## Configuration

```toml
# ~/.config/eink-bridge/config.toml
[server]
host = "0.0.0.0"
port = 3333
state_dir = "~/.local/state/eink-bridge"
session_timeout_minutes = 30
```

The Android app defaults to `http://amd-pc:3333` (works over LAN and Tailscale).
