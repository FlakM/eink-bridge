---
name: eink-update
description: Update the content of an active eink session on the Boox tablet. Use when iterating on a document mid-review without waiting for annotation events.
---

# E-Ink Update

Push updated content to an existing active session. The tablet reloads automatically (~25 ms).

## Usage

```
/eink-update [session-id]
```

## Steps

### 1. Find the session

If a session-id was provided as an argument, use it.

Otherwise, list active sessions and let the user pick:

```bash
eink-review list --status active
```

If there is exactly one active session, use it. If there are multiple, ask the user which one to update.

### 2. Establish what should change

If the requested change and review focus are already clear, use them. Otherwise ask
one short question about the intended change and desired depth. A file path or
“current context” alone is not a review goal; follow the `eink` skill's reading
contract before preparing content. Use the client's question tool if available,
otherwise ordinary chat—this workflow works in OpenCode and Claude Code.

Accept either:
- Raw markdown text pasted directly
- A file path (use it directly)
- "Current context" — synthesize a new markdown document from the conversation

### 3. Prepare the file

If the user provided raw markdown or asked for a synthesized document, write it to a temp file:

```bash
cat > /tmp/eink-update-XXXXX.md << 'EOF'
<content>
EOF
```

### 4. Push the update

```bash
eink-review update <session-id> /tmp/eink-update-XXXXX.md \
  [--expected-version <n>] [--feedback-id <id>] [--consumed 0,2] [--image picture.png]
```

A picture added in this round travels the same way it does on a new push: a
`![what to look at](shot.png)` next to the markdown file is uploaded, and `--image` adds a
file the document does not reference. The existing pictures stay in place — an
`image/<name>` reference in the new content resolves to the same session route, and
re-uploading under the same name replaces the bytes (the page is served `no-store`, so the
reader sees the new one on reload). Keep the alt text on a picture the reader wrote a note
on: the note arrives anchored to the text `[image] <alt>`.

On success, the server responds with the new version number. Retained native ink is
repositioned using server text matches; deleted/ambiguous targets are archived.

When the update answers a note, pass `--feedback-id` and the zero-based annotation
indices you incorporated in `--consumed`; those notes are then retired. Leave a note out
of `--consumed` and it follows the remaining text instead of disappearing. `--consumed`
without the matching `--feedback-id` is refused: an acknowledgement must name what it
consumes.
Attach the `eink` skill's background observer and confirm its handshake before
claiming further feedback will reach the conversation. When handling an annotation,
rearm immediately on receipt rather than waiting until the rewrite finishes.

On error:
- 404 → session not found
- 409 → session is not active (already submitted or cancelled)
- Connection refused → `systemctl --user start eink-serve`
