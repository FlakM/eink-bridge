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
- [x] image inclusion in the rendered document
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
just test             # unit + integration tests
just test-render      # render tests only (~0.02s)
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
~/.claude/skills/eink/SKILL.md         /eink workflow (push, wait, read annotations)
~/.claude/skills/coverage/SKILL.md     /coverage workflow (llvm-cov + JaCoCo)
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
| `/eink [file]` | Push content for review | Sends markdown to the Boox, blocks until pen annotations come back |
| `/coverage [component]` | Measure test coverage | Runs `cargo llvm-cov` (Rust) or JaCoCo (Android), reports gaps |

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
