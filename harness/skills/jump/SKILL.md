---
name: jump
description: Open a code reference in the running Neovim, or generate a GitHub permalink for a document. Use when asked to "jump to <file>:<line>", "open that reference", "go to mark B", "show me line 42", or when turning a review note into a location in the editor. Works with the socket Neovim the eink-pr skill starts.
---

# Jump

Two jobs: **go to a location** in the live editor, and **make a location linkable** for
a document going to the tablet.

The `jump` CLI resolves references and opens files. It supports two backends: **herdr**
when it runs inside a herdr pane (it finds the project's Neovim pane and drives it over
that editor's socket), and **tmux** otherwise. So `jump <ref>` works in a herdr desktop
without a tmux server. Check what is available with `jump verify`, which reports the
herdr backend alongside tmux and hyprland.

The eink-pr workflow also opens Neovim with an explicit `--listen` socket. When you
already know that socket, the marks helper is the more precise tool: it can set marks,
jump to a mark, and read back where the cursor landed. Prefer it inside a review;
use `jump` for one-off references and for link generation.

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
sets up the tab). The `jump` CLI needs no socket of its own: inside herdr it finds the
editor pane itself and falls back to tmux, so `jump <ref>` is a reasonable last resort.
If the editor is waiting at a prompt (a stale swap file leaves `E325 ATTENTION`), jump
reports that instead of typing the command into the prompt — resolve the prompt (open the
file read-only, or delete the stale swap file) and retry.

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
