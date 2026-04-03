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

### 2. Ask for updated content

Use the AskUserQuestion tool to ask: **"What should the updated content be?"**

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
eink-review update <session-id> /tmp/eink-update-XXXXX.md
```

On success, the server responds with the new version number. Tell the user the tablet has been updated.

On error:
- 404 → session not found
- 409 → session is not active (already submitted or cancelled)
- Connection refused → `systemctl --user start eink-serve`
