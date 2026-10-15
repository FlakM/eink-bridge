---
name: jump
description: Open a code reference in the running Neovim, or generate a GitHub permalink for a document. Use when asked to "jump to <file>:<line>", "open that reference", "go to mark B", "show me line 42", or when turning a review note into a location in the editor. Works with the socket Neovim the eink-pr skill starts.
---

# Jump

Two jobs: **go to a location** in the live editor, and **make a location linkable** for
a document going to the tablet.

The `jump` CLI resolves references and generates permalinks, but it *opens* files via a
tmux session and `nvr`, so it fails when no tmux server is running. The eink-pr workflow
runs Neovim in a herdr pane with an explicit `--listen` socket, which is the reliable
route. Prefer the socket; use the CLI for link generation.

## Going to a location

Pick the socket in this order, and say which one you used:

1. `$EINK_PR_SOCKET` if set (the eink-pr skill exports the socket for the review).
2. A socket passed by the caller.
3. The most recent `/tmp/eink-pr-*.sock` whose process is still alive.
4. `$NVIM` (Neovim's own socket for the current session).

```bash
# a mark letter from a review brief ("go to B")
python3 <eink-pr-base>/scripts/nvim_marks.py --socket "$EINK_PR_SOCKET" --jump B

# an explicit reference, absolute or relative to the worktree
python3 <eink-pr-base>/scripts/nvim_marks.py --socket "$EINK_PR_SOCKET" --open server/src/app.rs:170
```

Both commands verify the target first and exit non-zero with a clear message when the
mark is unset, the file is missing, or the line is past the end — read the message
instead of retrying blindly.

### Resolving the reference form

- `path:line`, `path:line:col` → strip the column, open `path:line`.
- `file:///path#L10` → path plus line.
- GitHub URL (`.../blob/<sha>/<path>#L10-L20`) → take the path and the first line, then
  map it under the review's worktree root.
- Markdown link `[label](url)` → unwrap to the URL or path first.
- A bare letter `A`–`F` that matches a mark in the brief → treat it as a mark jump.

Relative paths resolve against the worktree the review is using, not the current
directory, otherwise you jump into the wrong checkout.

### When there is no socket

Report it. Offer to reopen Neovim with `--listen` (the eink-pr skill does this when it
sets up the tab). Only fall back to the `jump` CLI when a tmux server is actually
running — check with `jump verify` first, since a bare `jump <ref>` fails with an
unhelpful `tmux command failed` otherwise.

## Making a location linkable

For a brief's marks legend, pin the link to the exact revision:

```bash
jump github-link --file server/src/app.rs --start-line 170 --end-line 176
# { "url": ".../blob/<sha>/server/src/app.rs#L170-L176", "markdown": "[...](...)", ... }
```

`jump github-link` and `jump copy-markdown` need git (and an LSP for `copy-markdown`),
not tmux, so they work offline in the review worktree. Use the `markdown` field in the
brief and the `url` when the reader may open it elsewhere.

## Reporting

Say where you went in one line — `B → server/src/app.rs:170` — and stop. Do not narrate
the resolution steps or re-explain the finding; the reader is already looking at it.
