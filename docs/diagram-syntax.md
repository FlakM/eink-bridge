# Diagram Syntax

The review renderer supports normal Markdown plus three fenced diagram blocks:

- `mermaid`
- `mindmap`
- `graph`

Use regular Markdown for prose, lists, code blocks, and review notes. Use fenced diagram blocks when you want structured visuals for coding tasks.

## Mermaid

Use Mermaid for standard engineering diagrams such as:

- flowcharts
- sequence diagrams
- state diagrams
- class diagrams
- C4 diagrams

Example:

```md
```mermaid
flowchart LR
  A[Editor] --> B[CLI]
  B --> C[Server]
  C --> D[Boox WebView]
```
```

Notes:

- Mermaid source is passed through directly.
- If Mermaid fails to render, the raw source is shown as a fallback.

## Mind Map

Use `mindmap` for coding-task plans, architecture overviews, and problem decomposition.

Example:

```md
```mindmap
root: LSP Formatting
index: true
nodes:
  - id: reproduce
    label: Reproduce
    color: red
  - id: inspect
    label: Inspect
    color: blue
    children:
      - id: parser
        label: Parser
      - id: renderer
        label: Renderer
  - id: patch
    label: Patch
    color: green
    collapsed: true
    children:
      - label: Add tests
      - label: Verify build
```
```

Supported document keys:

- `root`: optional root label
- `index`: optional boolean, defaults to `true`
- `nodes`: top-level node list

Supported node keys:

- `id`: optional stable node identifier
- `label`: required visible node text
- `color`: optional palette name
- `collapsed`: optional boolean, defaults to `false`
- `href`: optional URL to open on focus
- `kind`: optional semantic kind such as `module`, `file`, `function`, `decision`, `risk`, `todo`
- `file`: optional file path metadata
- `symbol`: optional symbol metadata
- `line`: optional line number metadata
- `notes`: optional freeform note metadata
- `children`: optional child node list

Supported colors:

- `blue`
- `green`
- `red`
- `amber`
- `purple`
- `slate`
- `gray`

Behavior:

- side index is generated from visible nodes
- tapping a node focuses and centers it
- nodes with children get a collapse/expand toggle
- collapsed branches are excluded from layout and index output
- selected nodes show metadata in a details panel
- color is paired with dashed or dotted stroke patterns so hierarchy still works on grayscale e-ink
- node `kind` is shown as a small badge in the node itself

## Graph

Use `graph` for architecture diagrams, dependency maps, and subsystem relationships.

Example:

```md
```graph
layout:
  algorithm: layered
  direction: RIGHT
  node_spacing: 48
  layer_spacing: 96
nodes:
  - id: cli
    label: CLI
    kind: tool
  - id: server
    label: Server
    kind: backend
  - id: android
    label: Android
    kind: client
edges:
  - from: cli
    to: server
    label: push markdown
  - from: android
    to: server
    label: poll sessions
```
```

Supported layout keys:

- `algorithm`: optional, defaults to `layered`
- `direction`: optional, defaults to `RIGHT`
- `node_spacing`: optional numeric spacing
- `layer_spacing`: optional numeric spacing

Supported node keys:

- `id`: required stable identifier
- `label`: optional display label
- `kind`: optional semantic kind such as `tool`, `client`, `backend`, `service`, `module`, `file`, `function`
- `file`: optional file path metadata
- `symbol`: optional symbol metadata
- `line`: optional line number metadata

Supported edge keys:

- `from`: required source node id
- `to`: required target node id
- `id`: optional edge identifier
- `label`: optional edge label
- `kind`: optional semantic edge kind

Behavior:

- ELK computes layout before rendering
- nodes are tappable and centered on focus
- edge labels render when present
- selected nodes show metadata in a details panel
- arrowheads are rendered for directed relationships
- semantic edge kinds use different dash patterns so they remain readable on e-ink
- node kind badges and border patterns provide non-color cues for grayscale screens

## Render Features

The renderer is optimized for e-ink review, not glossy color displays.

Implemented features:

- [x] local vendored Mermaid and ELK assets, so rendering does not depend on network access
- [x] e-ink-safe light fills with dark borders
- [x] grayscale-safe stroke patterns so meaning does not rely on color alone
- [x] node kind badges for fast visual scanning
- [x] metadata panel for selected `mindmap` and `graph` nodes
- [x] side index for visible mind-map branches
- [x] branch collapse and expand in mind maps
- [x] graph arrowheads and edge labels
- [x] parse-error fallback that preserves the original source block

Recommended authoring style for e-ink:

- keep node labels short
- prefer semantic `kind` values over relying on color only
- use `notes`, `file`, `symbol`, and `line` for extra detail instead of overloading labels
- use edge `kind` when relationship semantics matter

## Validation Rules

- Diagram blocks must use fenced code blocks with a supported language tag.
- `mindmap` and `graph` blocks use YAML syntax.
- Invalid `mindmap` or `graph` YAML shows a parse error block and preserves the original source.
- Unsupported block kinds are left alone as normal Markdown code blocks.

## Best Use In Coding Tasks

- Use `mermaid` for procedural flows and sequence explanations.
- Use `mindmap` for implementation plans, code-review branches, and feature decomposition.
- Use `graph` for component relationships, data flow, and dependency structure.
