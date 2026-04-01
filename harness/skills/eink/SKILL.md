---
name: eink
description: Push content to Boox e-ink tablet for iterative review. Supports multiple annotation rounds — user annotates, agent updates document, tablet reloads.
---

# E-Ink Review

Push content to the Boox for reading and annotation. Supports iterative rounds: the user can annotate and request an update, the agent rewrites the document, and the tablet reloads.

## Usage

```
/eink [file]
```

- If a file path is given, push it directly.
- If no argument, write the current conversation state to a temp markdown file.

## Steps

1. **Determine content:**
   - If the user provided a file path, use it directly.
   - Otherwise, write a context summary to `/tmp/eink-review-XXXXX.md`.

2. **Start the interactive push in the background**, capturing stdout to a temp log:
```bash
LOGFILE=$(mktemp /tmp/eink-events-XXXXX.log)
ERRFILE=$(mktemp /tmp/eink-err-XXXXX.log)
eink-review push --interactive --timeout 60 <file> >"$LOGFILE" 2>"$ERRFILE" &
echo "PID:$! LOG:$LOGFILE"
```

3. **Get the session ID** (emitted first to stdout):
```bash
sleep 1
grep -m1 "^SESSION_ID:" "$LOGFILE"
```
Parse the session ID from `SESSION_ID:<id>`.

4. **Event loop** — repeat until `EVENT:SUBMITTED` appears:
```bash
cat "$LOGFILE"
```

   **On `EVENT:ANNOTATION_RESULT <json>`** (user tapped "Request Update"):
   - Parse the JSON: `annotations[]`, `version`
   - For each annotation:
     - `recognized_text` — OCR'd handwriting (may have errors; use judgement)
     - `anchor.elements[]` — `section_id`, `tag`, `text` of nearby document elements
   - Read any annotation image paths from the result (use **Read** tool to view PNGs)
   - Decide what to update based on the feedback
   - Write the updated markdown to `/tmp/eink-update-XXXXX.md`
     - **Preserve heading structure** so anchors survive across versions
   - Push the update:
   ```bash
   eink-review update <session-id> /tmp/eink-update-XXXXX.md
   ```
   - Check the log again for the next event

   **On `EVENT:SUBMITTED <json>`** (user tapped "Done"):
   - Parse the result JSON
   - Read annotation image paths from `result.annotation_images[]` using the **Read** tool
   - Break out of the event loop

5. **Summarize all feedback rounds** and continue the conversation informed by the annotations.

6. **On failure** (exit 1, timeout, server unreachable):
   - Report the error
   - If connection refused: `systemctl --user start eink-serve`

## Processing annotation feedback

When an `EVENT:ANNOTATION_RESULT` arrives:

- `recognized_text` is OCR output — trust it but it may misread letters.
- `anchor.elements[].section_id` maps the annotation to a specific document section. Use this to target the rewrite precisely.
- `anchor.elements[].text` gives a snippet of the anchored element's text for context.
- Annotations without `anchor` are global comments.

When rewriting:
- Keep sections that received no feedback unchanged.
- For annotated sections: apply the feedback (correct, expand, simplify, etc.).
- Preserve heading text and hierarchy — section anchoring depends on it.
- Prefer targeted edits over wholesale rewrites.

## Authoring Rich Review Documents

**Always use colors and diagrams.** The Boox is a color e-ink tablet — take advantage of it. Plain prose is fine for text, but any architecture, plan, status breakdown, or relationship should be a colored graph or mindmap.

The renderer supports normal Markdown plus three diagram types:

- `mermaid` for flowcharts, sequence diagrams, state machines
- `mindmap` for plans, review branches, task decomposition
- `graph` for component relationships, data flows, architecture maps

### Color semantics — use these consistently

| Color | Meaning |
|-------|---------|
| `red` | Problem, blocker, broken, needs attention |
| `green` | Good, done, healthy, approved |
| `amber` | Warning, uncertain, in progress |
| `blue` | Information, reference, external |
| `purple` | Key decision or design point |
| `slate` | Deprioritized, deferred, secondary |

### `graph` — architecture and data flow

```graph
layout:
  algorithm: layered
  direction: RIGHT
nodes:
  - id: user
    label: User
    kind: client
    color: blue
  - id: api
    label: API Server
    kind: backend
    color: green
edges:
  - from: user
    to: api
    label: HTTPS
```

### `mindmap` — plans and code review

```mindmap
root: Fix Auth Bug
index: true
nodes:
  - id: root
    label: Root Cause
    color: amber
    children:
      - label: Token expiry misconfigured
        color: red
```

### `mermaid` — flows and sequences

```mermaid
sequenceDiagram
  participant C as Client
  participant A as API
  C->>A: POST /login
  A-->>C: 200 JWT
```

## Guidance

- Push to the Boox whenever the user needs to review a plan, architecture, or diff.
- Prefer a graph or mindmap over a bullet list whenever structure or status matters.
- Color every node intentionally — `red` for what needs attention, `green` for what's good.
- The `eink-serve` systemd service must be running. Connection refused → `systemctl --user start eink-serve`
- Do NOT proceed with other work while waiting — the review is the user's input.
- After all rounds complete, summarize all feedback and continue informed by it.
