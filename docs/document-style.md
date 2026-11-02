# Document Style Guide

How documents produced for e-ink review should look and feel.

This is the reference for the `/eink` skill and any skill that generates
content destined for the Boox tablet. The executable version of the shape lives in
`harness/skills/eink/references/review-document.md`; this file is the short form, and
the reasoning behind it.

## Principles

1. **The answer comes first.** Verdict, then evidence. A reader who trusts the author can
   stop after one line; a reader who does not has the rest of the page to argue with.
2. **Comprehension before defects.** In a study of professional reviews, finding defects
   was only the fourth most frequent kind of comment (14%). Understanding the change is the
   hard part, so the page spends its words on the shape of the change and the one thing
   that decides it — not on a tour of the diff.
3. **Size decides value.** Under 200 changed lines is the target, 400 the ceiling, 300
   lines an hour the fastest rate that still finds defects, 60–90 minutes a session. A page
   that cannot fit that budget is telling you to split the change; say so as a finding.
4. **Silence must be visible.** Name what you checked, where you stopped, and what you
   examined and cleared. An oversight and a considered non-issue look identical otherwise.
5. **Concrete before abstract** — lead with the specific example, then generalise. Show the
   code first, explain the pattern second.
6. **Colour is semantic** — red means danger/risk, green means done/safe, blue means
   information/todo, amber means needs-attention/review. On a greyscale panel it is a
   bonus, never the only signal.

## Document Structure

Three shapes; pick by what the reader has to do. Full skeletons are in the skill
reference above.

| Shape | The reader's job | Use for |
|---|---|---|
| **decide** | judge a change or a choice | a diff, a PR, a design with options |
| **understand** | build a model they will reuse | a concept, an architecture, a plan |
| **round** | see what happened to their notes | round two and later of a live review |

The review (`decide`) page, in order:

```
# The change, in the reader's words
**Ready?** <Yes | No | With fixes> — why.
<files · ±lines · minutes · lines to judge, mechanical>

**What changed** — one sentence.
**Why** — quoted, or "not stated".
**Wider system** — what else this touches.
**The question** — the one thing the reader must answer.

## Read these first      <- 3-6 findings, ranked, labelled, path:line on the line
## What I checked        <- checkboxes, including what you did not reach
## Set aside             <- what you examined and cleared, one line each
## Change surface        <- only when contracts move
## If you only read one thing

Write on the page, or reply LGTM / CHANGES.
```

Rules that carry the shape:

- **Findings are ranked, labelled and placed.** A label from the review vocabulary
  (`issue (blocking)`, `suggestion`, `question`, `praise`, `nitpick`) is the skim layer;
  the `path:line` on the same line is what makes it tappable. No legend at the bottom.
- **Headings state the claim**, not the topic. "Expiry clears the notifier map but not the
  socket map" beats "Notifications".
- **One mode per section** — explaining and enumerating are different jobs; a section that
  does both does neither well.
- No "Summary" + "TL;DR" stacking. No restatement of the title. The first sentence is the
  answer.

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

The renderer uses Georgia serif on a 992px column. Documents should be written assuming:

- Headings are large and bold — use them for navigation, not decoration
- Inline code (`backticks`) has a visible border — use it for identifiers, not emphasis,
  and it is what a finding's label is rendered in, so a label reads as a chip
- Tables render well — prefer tables over bullet-list comparisons
- `diff` blocks are coloured — use them for the contract being judged, nothing else
- `<details>` keeps background out of the way until it is wanted

## What NOT to do

- Don't show a diff the reader already has in the editor. The page owes judgement; the
  editor has the code. One diff is allowed, and only when the contract is the subject.
- Don't produce documents longer than ~800 lines — the tablet scrolls poorly beyond that
- Don't use nested bullet lists deeper than 2 levels — they're hard to read on e-ink
- Don't use pictures as decoration — one or two per document, and only when the artefact
  itself is the point (see above); `snippet` beats a screenshot of code every time
- Don't add "Summary" sections at the end — the reviewer will write their own with the pen
- Don't use emoji — they render inconsistently on e-ink displays
- Don't put a URL where a `path:line` would do — the reader cannot type a link from e-ink
