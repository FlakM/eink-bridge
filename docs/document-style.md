# Document Style Guide

How documents produced for e-ink review should look and feel.

This is the reference for the `/eink` skill and any skill that generates
content destined for the Boox tablet.

## Principles

1. **Dense but scannable** — pack information tightly, but use headings and whitespace to create visual rhythm. No filler prose.
2. **Diagrams over prose** — if a relationship can be shown, show it. A 6-node mermaid diagram replaces 3 paragraphs of "A depends on B which calls C".
3. **Concrete before abstract** — lead with the specific example, then generalize. Show the code first, explain the pattern second.
4. **Color is semantic** — red means danger/risk, green means done/safe, blue means information/todo, amber means needs-attention/review.

## Document Structure

Every review document should follow this skeleton, omitting sections that don't apply:

```
# Title (what is this about, 5 words max)

## Context
Why does this exist? 2-3 sentences max. Link to the ticket/issue if relevant.

## The Actual Content
The thing being reviewed — code explanation, architecture proposal,
implementation plan, blog draft, etc.

## Diagram (if applicable)
One diagram that captures the core structure or flow.
Prefer: mindmap for plans, mermaid sequence for API flows,
mermaid flowchart for decision logic, graph for architecture.

## Tradeoffs / Open Questions
Bullet list. What was considered and rejected? What's still unclear?
This is where the pen annotations are most valuable.
```

## Diagram Preferences

### When to use which

| Situation | Diagram type | Why |
|-----------|-------------|-----|
| Implementation plan / task decomposition | `mindmap` | Collapsible nodes, side index for navigation |
| API call sequence between systems | `mermaid` sequenceDiagram | Shows time ordering and who calls who |
| Decision tree / conditional logic | `mermaid` flowchart | Shows branching clearly |
| State transitions | `mermaid` stateDiagram-v2 | Matches the session state machine pattern |
| System architecture / dependencies | `graph` | ELK layout handles complex topologies |
| Class relationships | `mermaid` classDiagram | Rare — only for inheritance-heavy designs |

### Mindmap conventions

- Root label: the goal or deliverable, not a category
- Colors: blue=info/todo, green=done/module, red=risk/blocker, amber=review/question
- Kinds: `todo`, `module`, `function`, `risk`, `decision`
- Collapse nodes that are resolved or low-priority
- Include `file:` and `symbol:` when referencing code

### Mermaid conventions

- Keep diagrams under 12 nodes — split into multiple diagrams if larger
- Use `LR` (left-right) for flows, `TB` (top-bottom) for hierarchies
- Label edges — an unlabeled arrow is ambiguous
- Use participant aliases in sequence diagrams for readability

### Graph conventions

- Use `kind:` badges for non-color semantic cues (backend, frontend, tool, database)
- Layout direction: `RIGHT` for data flow, `DOWN` for dependency chains
- Edge labels should be verbs: "calls", "reads from", "publishes to"

## When a picture beats a diagram

A diagram shows the structure you already understand. A picture shows the artefact itself,
including the thing you are asking about — the misaligned label, the wrong colour, the
broken layout. Use one when the subject *is* the appearance:

- the rendered page, app screen or report under review
- a photograph of a whiteboard, a sketch on paper, a hand-drawn flow
- an artefact that cannot be reconstructed in text without losing the point

Use a `mermaid`, `graph` or `mindmap` block instead when the point is relationships,
ordering or hierarchy — a diagram stays sharp at any zoom, is searchable, and can be
annotated line by line. A picture is a raster: at reading size on e-ink the detail is
limited, and the reader has to pinch-zoom to make out fine text.

Rules:

- One or two pictures per document, no more. They cost a page of scrolling each.
- Always caption it with an italic line underneath, saying what to look at and why.
  The alt text is not shown on the page.
- Do not paste a screenshot of code or of a terminal. A `snippet` or `diff` block is
  sharper, selectable and annotatable per line; a screenshot of code is the one case where
  the picture is reliably worse.
- Crop to the part that matters. A full desktop screenshot on a 992px column is mostly
  chrome.
- PNG for UI and screenshots, JPEG for photographs. The server accepts `.png`, `.jpg`,
  `.jpeg`, `.webp` and `.gif`, and checks that the bytes match the extension.

In markdown, reference the file as `image/<name>` and let the CLI upload it:

```markdown
![the error page as it renders today](image/error-page.png)

*Figure 1 — the banner overlaps the heading at 992px; the fix should move it below.*
```

```bash
eink-review push brief.md --image error-page.png
```

## Typography and Layout

The renderer uses Georgia serif at 28px on a 2800px-wide canvas.
Documents should be written assuming:

- Headings are large and bold — use them for navigation, not decoration
- Inline code (`backticks`) has a visible border — use it for identifiers, not emphasis
- Tables render well — prefer tables over bullet-list comparisons
- Diff blocks are syntax-highlighted — use them when showing before/after changes

## What NOT to do

- Don't produce documents longer than ~800 lines — the tablet scrolls poorly beyond that
- Don't use nested bullet lists deeper than 2 levels — they're hard to read on e-ink
- Don't use pictures as decoration — one or two per document, and only when the artefact itself is the point (see above)
- Don't add "Summary" sections at the end — the reviewer will write their own summary with the pen
- Don't use emoji — they render inconsistently on e-ink displays
