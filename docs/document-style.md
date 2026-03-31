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
- Don't use images unless they're essential — the e-ink screen has limited color depth
- Don't add "Summary" sections at the end — the reviewer will write their own summary with the pen
- Don't use emoji — they render inconsistently on e-ink displays
