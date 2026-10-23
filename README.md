# E-Ink Bridge

Hack on an idea, plan, or codebase in an AI tool or vim. Send it to an e-ink tablet for review — read it with fresh eyes, scribble thoughts with the stylus, tap Done. The annotations land back in whatever sent them. Iterate until it feels right.

## Architecture

```
+---------------------------+          LAN          +------------------------------+
|  Dev Machine              |                       |  Boox Tab Ultra C Pro        |
|                           |                       |                              |
|  ┌─────────────────────┐  |  GET /session/{id}    |  ┌────────────────────────┐  |
|  │      eink-serve     │◄─┼───────────────────────┤  │     Android App        │  |
|  │      (Rust/axum)    │  |                       |  │                        │  |
|  │                     │◄─┼───────────────────────┤  │  · session list        │  |
|  └──────────┬──────────┘  |  POST /submit         |  │  · read content        │  |
|             │             |  (typed notes + PNG)  |  │  · annotate with pen   │  |
|  ┌──────────┴──────────┐  |                       |  │  · submit review       │  |
|  │  Claude Code        │  |  /eink → blocks       |  └────────────────────────┘  |
|  │  Neovim             │  |  <leader>ep → async   |                              |
|  │  Shell              │  |  eink-review push     |                              |
|  └─────────────────────┘  |                       |                              |
+---------------------------+                       +------------------------------+
```

### Flow

1. Caller creates a session — `POST /api/sessions` with markdown
2. Android app polls for new sessions, loads the e-ink HTML
3. User reads content, annotates with the Wacom pen (native Onyx SDK, ~20ms latency)
4. User taps Done — strokes exported as PNG, POSTed to server
5. Server notifies the blocking caller via long-poll
6. Caller receives typed notes + annotation image paths

### Render features

- [x] canvas style rendering (4000px wide body, pen-friendly layout)
- [x] attractive styles optimized for color e-ink (light mode, high contrast, large fonts)
- [x] beautiful typography (Georgia serif, 1.7 line-height, spaced headings)
- [x] mind map with navigation to different sections of the document
- [x] diagram blocks with Mermaid, ELK graph layout, and custom mindmap syntax
- [x] code blocks with syntax highlighting (syntect, e-ink-tuned theme)
- [x] rich meta information rendering (hidden by default, revealed on tap)
- [x] image inclusion in the rendered document: a screenshot, sketch or photo travels with
  the brief (`image/<name>`) and is served from the session that owns it, so no other host
  has to stay up
- [x] use of colors for emphasis and organization (e.g. red for warnings, green for good parts, blue for questions)

## Usage

Project workflow docs:

- `AGENTS.md`
- `CONTRIBUTING.md`
- `docs/api.md`

### CLI

```bash
# blocking — waits until Boox user submits
eink-review push document.md

# stdin
cat notes.md | eink-review push -

# non-blocking — prints session ID
eink-review push --async document.md

# a picture travels with the brief; ![x](shot.png) beside the document is uploaded too
eink-review push --async brief.md --image shot.png

# add one to a review that is already running; prints the markdown to paste
eink-review image <session-id> sketch.png --alt "whiteboard sketch"

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
/eink [file]  — push to Boox, iterative review with live annotation events
```

Mid-conversation: push an explanation to the Boox, read with a pen, scribble thoughts, tap "Request Update". Claude receives the annotations via the `eink-channel` MCP server, rewrites the document, and the tablet reloads — without you leaving the conversation.

#### Starting Claude Code with channel support

```bash
claude --dangerously-load-development-channels server:eink-channel
```

**Why `--dangerously-load-development-channels server:eink-channel`**
Channels are an experimental Claude Code extension that lets an MCP server push unsolicited events into the model's context. The `eink-channel` server (`harness/channels/eink-channel/channel.ts`) declares the `claude/channel` experimental capability, which is what makes this flag meaningful. Without it, Claude has no way to receive tablet events asynchronously — it would have to poll or block.

#### How skill ↔ model communication works

```
/eink invoked
      │
      ▼  eink-review push --async --callback-url http://127.0.0.1:8789/ doc.md
  eink-serve (port 3333)
      │
      │  user annotates on Boox, taps "Request Update"
      │
      ▼  POST http://127.0.0.1:8789/   (webhook body = annotation JSON)
  eink-channel HTTP server (port 8789)
      │
      │  mcp.notification("notifications/claude/channel", { event_type, session_id, content })
      ▼
  Claude Code session  ←  receives <channel source="eink-channel" event_type="annotation_result" session_id="...">
      │
      │  reads annotations[].recognized_text + anchor.elements
      │  rewrites document markdown
      │
      ▼  update_session(session_id, content)   ← MCP tool registered in eink-channel
  eink-channel → PUT /api/sessions/{id}/content → eink-serve
      │
      ▼
  Tablet reloads, user sees updated document
```

The `update_session` tool is registered in the same MCP server — so the same process that receives webhook events from the tablet also forwards `update_session` calls back to `eink-serve`. Claude is never blocked waiting; annotation events arrive as context injections and can be interleaved with other conversation turns.

**Environment variables**

| Variable | Default | Purpose |
|---|---|---|
| `EINK_CHANNEL_PORT` | `8789` | Port the channel HTTP server listens on (must match `--callback-url`) |
| `EINK_SERVER_URL` | `http://localhost:3333` | eink-serve base URL used by `update_session` |

## Document Syntax

The review page supports normal Markdown plus diagram blocks for coding tasks.

- `mermaid` for flowcharts, sequence diagrams, state diagrams, class diagrams, and C4
- `mindmap` for implementation plans, code-review branches, and task decomposition
- `graph` for architecture, dependency, and subsystem maps

Syntax reference:

- `docs/diagram-syntax.md`
- `docs/diagram-showcase.md`
- `docs/render-features-showcase.md`

Render features:

- [x] local vendored Mermaid and ELK assets
- [x] Mermaid rendering for standard engineering diagrams
- [x] mind map rendering with side index
- [x] mind map collapse and expand
- [x] mind map metadata/details panel
- [x] graph rendering with ELK layout
- [x] graph arrowheads and edge labels
- [x] graph metadata/details panel
- [x] grayscale-safe border and edge patterns for e-ink screens
- [x] node kind badges for non-color semantic cues
- [x] parse-error fallback for invalid structured diagram blocks
- [x] unsupported fenced blocks remain normal Markdown code blocks

Quick example:

```md
# Review

```mermaid
flowchart LR
  A[Prompt] --> B[Renderer]
  B --> C[Boox]
```

```mindmap
root: Fix bug
nodes:
  - label: Reproduce
  - label: Patch
  - label: Verify
```

```graph
nodes:
  - id: cli
    label: CLI
  - id: server
    label: Server
edges:
  - from: cli
    to: server
```
```

Notes:

- `mindmap` and `graph` blocks use YAML syntax.
- Invalid structured diagrams show a parse error block with the original source preserved.
- On device, open the session in `Read` mode to navigate diagrams and switch to `Annotate` mode when you want to draw.

## Nix Integration

The flake exports two packages:

| Package | Contents |
|---------|----------|
| `default` / `eink-bridge` | Server, CLI, and mock device binaries |
| `harness` | Claude Code skills (`/eink`, `/coverage`) and output styles |

### 1. Add the flake input

```nix
# flake.nix
{
  inputs.eink-bridge = {
    url = "github:FlakM/eink-bridge";
    inputs.nixpkgs.follows = "nixpkgs";
  };
}
```

Pass `inputs` through to your home-manager modules (via `extraSpecialArgs` or similar).

### 2. Create a home-manager module

```nix
# home-manager/modules/eink-bridge.nix
{ config, lib, pkgs, inputs, ... }:
let
  eink-bridge = inputs.eink-bridge.packages.${pkgs.system}.default;
  harness = inputs.eink-bridge.packages.${pkgs.system}.harness;

  # Auto-deploy each skill directory to ~/.claude/skills/
  skillFiles = builtins.listToAttrs (map (name: {
    name = ".claude/skills/${name}";
    value = { source = "${harness}/skills/${name}"; };
  }) (builtins.attrNames (builtins.readDir "${harness}/skills")));

  # Auto-deploy each output style to ~/.claude/output-styles/
  styleFiles = builtins.listToAttrs (map (name: {
    name = ".claude/output-styles/${name}";
    value = { source = "${harness}/output-styles/${name}"; };
  }) (builtins.attrNames (builtins.readDir "${harness}/output-styles")));
in
{
  # Binaries: eink-serve, eink-review, eink-mock-device
  home.packages = [ eink-bridge ];

  # Claude Code skills and output styles
  home.file = skillFiles // styleFiles;

  # Systemd user service for the server
  systemd.user.services."eink-serve" = lib.mkIf pkgs.stdenv.isLinux {
    Unit = {
      Description = "eink-bridge review server";
      After = [ "network-online.target" ];
    };
    Service = {
      ExecStart = "${eink-bridge}/bin/eink-serve";
      Restart = "on-failure";
      RestartSec = 5;
      Environment = [ "RUST_LOG=info" ];
    };
    Install.WantedBy = [ "default.target" ];
  };
}
```

### 3. Import the module

```nix
# home-manager config (e.g. home.nix or amd-pc.nix)
{
  imports = [
    ./modules/eink-bridge.nix
  ];
}
```

### 4. Deploy

```bash
sudo nixos-rebuild switch --flake ~/programming/flakm/nix_dots#amd-pc
```

After deployment you get:

| What | Where |
|------|-------|
| `eink-serve` | Systemd user service (auto-starts) |
| `eink-review` | CLI in `$PATH` |
| `/eink` skill | `~/.claude/skills/eink/` (symlink to nix store) |
| `/coverage` skill | `~/.claude/skills/coverage/` (symlink to nix store) |
| E-Ink Review style | `~/.claude/output-styles/eink-review.md` (symlink to nix store) |

Manage the service:

```bash
systemctl --user status eink-serve
systemctl --user restart eink-serve
journalctl --user -u eink-serve -f
```

### Adding new skills or output styles

Add files to `harness/skills/<name>/SKILL.md` or `harness/output-styles/<name>.md` in this repo. They auto-deploy on the next `nixos-rebuild switch` with no changes to the nix module.

### Neovim integration

The neovim plugin lives in [nix_dots](https://github.com/FlakM/nix_dots) at `home-manager/modules/nvim/config/eink-bridge.lua`. It provides:

| Keymap | Mode | Action |
|---|---|---|
| `<leader>ep` | normal | push entire buffer |
| `<leader>ep` | visual | push selection |
| `<leader>ec` | normal | cancel last session |
| `<leader>ea` | normal | list sessions in vertical split |

## OCR: caching and optional OpenAI race

OCR runs locally with Ollama (`qwen2.5vl:7b`) by default. No OpenAI requests are
made merely because an API key exists. Server-side caching and racing work with
the native app without another APK update.

### Caching

All OCR entry points share an in-memory cache of successful recognitions, keyed
by the exact rendered PNG. Unchanged handwriting skips inference; overlapping
requests for the same image share one recognition attempt. Changed images miss
the cache. Failures and empty results are not cached.

The cache keeps at most 128 entries and 16 MiB of image keys, evicts least-recently
used entries, and expires entries after 10 minutes of inactivity. It is cleared
on server restart, so model/prompt changes cannot reuse old results. Under cache
pressure an evicted in-flight entry can be recomputed. The cache does not write
handwriting to disk; `EINK_OCR_DEBUG` still explicitly enables diagnostic PNGs.

### Enable cloud OCR explicitly

Create a private file containing only your OpenAI API key (restrict it with
`chmod 600`). Keep it outside this repository. Configure the **server**, not the
Android app:

```bash
export EINK_OCR_OPENAI_ENABLED=true
export EINK_OPENAI_API_KEY_FILE="$HOME/.config/eink-bridge/openai-api-key"
just run
```

For the systemd user service, add these settings with
`systemctl --user edit eink-serve`, then restart `eink-serve`:

```ini
[Service]
Environment=EINK_OCR_OPENAI_ENABLED=true
Environment=EINK_OPENAI_API_KEY_FILE=%h/.config/eink-bridge/openai-api-key
```

| Server environment variable | Default | Purpose |
|---|---|---|
| `EINK_OCR_OPENAI_ENABLED` | unset/off | Set to `true` or `1` to opt into the race |
| `EINK_OPENAI_API_KEY_FILE` | unset | Read the API key from a private file; takes precedence over environment keys |
| `EINK_OPENAI_API_KEY` / `OPENAI_API_KEY` | unset | Alternative server-side API key; the EINK-prefixed value takes precedence |
| `EINK_OPENAI_MODEL` | `gpt-4.1-mini` | Vision model for the Responses API |
| `EINK_OPENAI_TIMEOUT_SECONDS` | `15` | Cloud deadline, 1–120 seconds; invalid values use 15 |
| `EINK_OLLAMA_MODEL` | `qwen2.5vl:7b` | Local vision model |
| `EINK_LLAMA_OCR_URL` | unset | Select a dedicated local llama-server instead of shared Ollama (e.g. `http://127.0.0.1:11435`) |

On `amd-pc`, the `eink-ocr` system service runs the existing Qwen GGUF and
projector with a 64–256 image-token budget, 16 CPU threads and one inference
slot. This avoids Ollama's forced 1,024-image-token minimum for Qwen-VL. The
service listens on loopback port 11435, shares the existing model files
read-only, and leaves shared Ollama unchanged. The five small handwriting
fixtures produced identical outputs at a 2.25 s median versus 12.40 s with
the original token budget. This lower visual-detail budget is scoped to small
handwriting crops, not general document understanding.

The application cache and cloud race also apply to this backend. If a dedicated
URL is configured but that service is unavailable, the local attempt fails and
OpenAI may still win; it does not silently switch to shared Ollama. Unset the
variable to restore the original local backend. For an explicit comparison:

```bash
EINK_LLAMA_OCR_URL=http://127.0.0.1:11435 just ocr-bench --compare
```

On an uncached image, local OCR and OpenAI start together. The first nonempty,
bounded, completed transcription wins. An error, refusal, incomplete response,
or empty text does **not** beat a still-running provider. If both fail, the
request fails and can be retried. Missing/invalid cloud credentials disable the
cloud provider with a startup warning while keeping local OCR available.

The loser is cancelled by dropping its HTTP request, but remote computation and
billing may continue. Each cache miss can incur a paid OpenAI call even when
local OCR wins. Only the rendered handwriting crop and transcription prompt are
sent, not the document, session metadata, or API key in an app payload. Requests
use the Responses API with `store: false`; this is **not** a promise of zero data
retention—OpenAI's applicable data policies still apply. Keys and remote error
bodies are not logged. This server has no authentication: keep it on a trusted
LAN, especially when cloud OCR can consume your API quota.

"Valid" here means a completed, nonempty response, not verified correctness.
Either model can still misread handwriting or invent text in a non-text mark.

### Measure instead of guessing

```bash
just ocr-bench             # initial recognition, then concurrent cache hits
just ocr-bench --compare   # uncached per-provider timings and text for five fixtures
```

`--compare` deliberately waits for both providers instead of cancelling the
loser. With cloud OCR enabled it makes **five paid OpenAI requests**. Without
cloud configuration it reports local-only results. Compare the text against
the printed expected words, not just the timing. Fixture content is printed;
normal server race logs record provider, result and latency rather than handwriting.

One five-fixture comparison on this workstation (2026-10-15, CPU-only local
Qwen vs. `gpt-4.1-mini`) measured:

| Provider | Mean | Range |
|---|---:|---:|
| Local | 14.06 s | 12.34–18.25 s |
| OpenAI | 1.47 s | 0.75–2.58 s |

All calls completed. Both providers misread the ambiguous cursive “now?” as
“mow?”, so speed is not a correctness guarantee. This is a small single run,
not a production latency percentile; provider-internal caching can still apply
even when the application cache is bypassed.

Prometheus metrics at `/metrics` include:

- `eink_ocr_cache_total{result="hit|miss|coalesced"}`
- `eink_ocr_provider_requests_total{provider="local|openai",result="ok|error|cancelled"}`
- `eink_ocr_provider_duration_seconds{provider,result}`
- `eink_ocr_provider_wins_total{provider}`
- `eink_ocr_provider_tokens_total{provider,kind="input|output"}`

Token counts cover responses received, not unknown usage after cancellation,
and are not an authoritative billing report. Existing aggregate OCR metrics
remain available.

## API

The full API reference lives in `docs/api.md`. The machine-readable OpenAPI document is available at `GET /api/openapi.json`.

```
POST   /api/sessions              create session (JSON or plain markdown body)
GET    /api/sessions              list sessions (?status= filter, sorted newest first)
GET    /api/sessions/{id}         session metadata
GET    /api/sessions/{id}/result  long-poll until submitted
DELETE /api/sessions/{id}         cancel session
POST   /api/sessions/{id}/submit  submit review (JSON or multipart)
GET    /session/{id}              e-ink optimized HTML
GET    /api/health                health check
GET    /api/openapi.json          OpenAPI contract
```

## Development

Enter the dev shell with `nix develop`, then use `just` for all common tasks:

```bash
just                  # list all recipes
just test             # server + tablet + harness (~15s, the whole gate)
just test-server      # server tests only (~3s)
just test-render      # render tests only (~0.5s)
just eval             # render + contract goldens
just lint             # format + clippy
just build            # cargo dev build
just run              # run server locally (debug logging)
just deploy           # nix build + restart systemd service
just apk-install      # build + install android APK
just doctor           # check service, connectivity, state
just manual-e2e       # full E2E with mock device
```

### Debugging

```bash
just run-trace        # max verbosity server
just doctor           # connectivity + state overview
just inspect-session ID  # view persisted session JSON
just show-config      # show config file
just logs             # follow server logs
just logs-recent 100  # last 100 log lines
just health           # quick health check
```

Server log levels: `RUST_LOG=debug` shows request traces, session lifecycle, and long-poll events. `RUST_LOG=trace` adds tower-http request/response details.

### Building

```bash
just build            # cargo dev build
just nix-build        # nix build (runs all tests)
just apk              # android debug APK
```

Or directly: `nix build`, `cd server && cargo build --release`.

### Deployment

```bash
just deploy           # nix build + restart systemd service
just status           # check service status
just logs             # follow server logs
```

Full NixOS deployment via nix_dots:

```bash
sudo nixos-rebuild switch --flake ~/programming/flakm/nix_dots#amd-pc
```

This installs all binaries, starts `eink-serve` as a systemd user service, wires the neovim plugin, and deploys the `/eink` Claude Code skill.

Note: the Boox requires `hidden_api_policy` set to 1 for the Onyx Pen SDK (`just boox-setup`).

## AI Harness

The project is set up for short-cycle AI-assisted development. Three layers define how AI tools generate and review content:

```
~/.claude/CLAUDE.md                    global preferences (style, git, tooling)
    |
AGENTS.md                              test commands, architecture, coverage
    |
docs/document-style.md                 visual language and document structure
    |
~/.claude/output-styles/eink-review.md how Claude writes review documents
    |
~/.claude/skills/eink/SKILL.md         /eink workflow (push, receive events, rewrite)
~/.claude/skills/coverage/SKILL.md     /coverage workflow (llvm-cov + JaCoCo)
    |
harness/channels/eink-channel/         MCP channel server (tablet → Claude events)
  channel.ts                           HTTP webhook receiver + MCP stdio server
```

### Document style (`docs/document-style.md`)

Defines the aesthetic preferences for content generated for e-ink review:

- **Structure**: Context (2-3 sentences) -> Content -> Diagram -> Tradeoffs/Questions
- **Diagrams over prose**: mindmap for plans, mermaid sequence for API flows, flowchart for decisions, graph for architecture
- **Color is semantic**: red=risk, green=done, blue=info, amber=review
- **Concrete before abstract**: show the code, then explain the pattern
- **Anti-patterns**: no emoji, no trailing summaries, no deep nesting, max 800 lines

### Output style (`~/.claude/output-styles/eink-review.md`)

A compact derivative of the style guide that modifies Claude's system prompt. Activate with `/output-style E-Ink Review` to shape all output for tablet reading.

### Skills

| Skill | Trigger | What it does |
|-------|---------|-------------|
| `/eink [file]` | Push content for review | Sends markdown to the Boox, receives live annotation events via `eink-channel`, rewrites document iteratively |
| `/coverage [component]` | Measure test coverage | Runs `cargo llvm-cov` (Rust) or JaCoCo (Android), reports gaps |

### Channels (`harness/channels/eink-channel/`)

The `eink-channel` is an MCP server with two roles running in the same process:

- **HTTP server** (default port 8789) — receives webhook POSTs from `eink-serve` when the tablet submits or requests an update
- **MCP server over stdio** — connected to Claude Code; forwards those events as `notifications/claude/channel` injections and exposes the `update_session` tool

Claude Code must be started with `--dangerously-load-development-channels server:eink-channel` for channel events to reach the model. See the [Claude Code section](#claude-code) for the full invocation and flag explanations.

### Agent guidance (`AGENTS.md`)

Operational reference for Claude Code and CI agents:

- Session state machine (Active -> Submitted/Cancelled/Expired)
- Long-poll architecture (server holds 30s, CLI retries for 30m)
- Test selection matrix (which command for which changed file)
- Coverage commands (`just coverage`, `just coverage-android`)
- Build and deploy recipes

### Adding to the harness

To define a new reusable workflow:

1. Create `~/.claude/skills/<name>/SKILL.md` with frontmatter (`name`, `description`) and step-by-step instructions
2. If the skill produces documents, reference `docs/document-style.md` for conventions
3. If the skill needs test validation, add a `just` recipe and document it in `AGENTS.md`

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
