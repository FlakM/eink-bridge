---
name: eink-pr
description: Review a GitHub pull request on the Boox — check the PR out into a git worktree, open the diff in Neovim in a new herdr tab, analyze the wider impact of the changes, then send a distilled review to eink and mark the key locations with Neovim global named marks. Use for "/eink-pr", "review this PR on eink", "put this PR on the tablet", or any request to review a pull request (URL, number, or branch) on the e-ink reader.
---

# E-Ink PR Review

Turn a pull request into something the user can read and annotate on the Boox, with
the code open beside them in Neovim so a handwritten note can be acted on immediately.

This skill is the *setup + impact analysis* half. All e-ink delivery, document
authoring, and feedback handling live in the **`eink`** skill — read it and follow its
contract (request purpose when unclear, background observer with a confirmed
handshake, `feedback_id`/`expected_version`/`consumed_annotations` on every rewrite,
rearm before drafting). Do not duplicate or diverge from that protocol.

## Preconditions

Check once and report honestly rather than half-doing the setup:

```bash
test "${HERDR_ENV:-}" = 1 && echo herdr-ok || echo herdr-missing
command -v gh nvim git
```

- **Not inside herdr** (`HERDR_ENV != 1`): still create the worktree and analyze the
  PR, but skip the tab and the marks, and tell the user the Neovim/ marks step needs a
  herdr-managed pane. Per the herdr skill, do not drive herdr from outside.
- **No `gh`**: ask for the PR URL and a base ref, or accept a local branch.
- **No PR given**: ask which PR, or offer to review the current branch against its
  base. Do not guess a number.
- **No `nvim`**: skip marks; keep everything else.

## 1. Resolve the PR and its base

```bash
gh pr view <pr> --json number,title,url,baseRefName,headRefName,additions,deletions,changedFiles,author
gh pr diff <pr> --name-only
```

Compute the **merge base** with the target branch (`origin/<baseRefName>`), not the
base tip: the review diff is `merge-base(base, head)..head`, so upstream commits that
are not part of the PR stay out.

## 2. Check the code out into a git worktree

Use `refs/pull/<n>/head`, which works for fork PRs and needs no remote setup:

```bash
git fetch origin pull/<n>/head
git worktree add --detach "<worktree-dir>" FETCH_HEAD
```

- **Stable, reviewable path**: `<repo-parent>/<repo>-worktrees/pr-<n>`.
- **Reuse**: if the worktree already exists, `git -C <dir> fetch origin pull/<n>/head`
  and `git -C <dir> checkout --detach FETCH_HEAD` instead of creating a second one.
- **Track the exact revision** you analyzed (the commit sha) so later feedback refers
  to a known state.
- Prefer a worktree over checking out in place: the user's own working tree is never
  disturbed, and `git worktree remove` cleans up.

## 3. Open the diff in Neovim in a new herdr tab

Only when `HERDR_ENV=1`. Read IDs from the JSON responses; never guess them.

```bash
herdr tab create --workspace "$HERDR_WORKSPACE_ID" --cwd "<worktree-dir>" \
  --label "PR <n>: <short title>"
# -> .result.root_pane.pane_id
herdr pane run <pane_id> "nvim --listen <socket> -c 'DiffviewOpen <merge-base>..HEAD'"
```

- **Always pass `--listen <socket>`** (e.g. `/tmp/eink-pr-<n>.sock`). Marks and later
  navigation need a reachable Neovim; without it step 5 cannot work.
- `DiffviewOpen <merge-base>..HEAD` shows the PR diff. Add changed paths as arguments
  when the user wants only specific files.
- Diff review only: `internal` is available as the base when the PR is already merged
  or the user asks for a plain diff.
- Wait for readiness (`herdr pane wait-output <pane_id> --match '<something nvim shows>'`
  or poll for the socket) before setting marks — do not send commands into a shell that
  has not started Neovim yet.
- Keep the user's focus: this is a new tab the user asked for; do not steal focus from
  their current pane without being asked.

## 4. Understand the wider impact

The diff alone is a weak review; the value here is what the change *touches*. Work the
diff hunk by hunk, and for every non-trivial change establish:

- **Callers and references** of changed public items (`rg`/grep for the symbol), so the
  review can say who is affected, not just what changed.
- **Interfaces and contracts** — public API, HTTP/proto schema, config keys, CLI flags,
  on-disk formats. These break silently and deserve their own callout.
- **Tests** that cover, or fail to cover, the new behavior.
- **Risk shape** — concurrency, migrations, error handling, feature flags, defaults.

For a large PR, delegate the survey to a subagent (`explore`/`general`) and keep only
its conclusions. Finish with a ranked shortlist of the **most important pieces** — the
few changes a reviewer must actually think about. Everything else is context.

## 5. Send the review to eink and mark the jump points

Map the shortlist to Neovim **global named marks** (uppercase `A`–`Z`): they are
jumpable across files and persist in shada, so the reader can note "B" on the tablet
and the user can press `'B` to land in the file.

```bash
python3 <skill-base>/scripts/nvim_marks.py --socket /tmp/eink-pr-<n>.sock \
  --marks "A:src/app.rs:42,B:src/session.rs:118,C:proto/api.proto:9"
```

- Use **at most ~6** marks tied to the shortlist; more than that is not a priority list.
- Point each mark at the line the reviewer must read, not the top of the file.
- The helper sets marks buffer-targeted (the open diff view is untouched) and **fails
  loudly** if a mark does not land — marks persist across sessions, so a silent failure
  would be read back as a stale mark from an earlier review. Treat a non-zero exit as a
  real failure and say so rather than claiming the marks are set.
- If the socket is gone, reopen Neovim with `--listen` (step 3) and retry.

Then write the document and hand off to the `eink` skill for delivery:

- **Title**: `PR <n>: <title>`.
- **Overview**: one or two bullets — what the PR does and why it matters.
- **Important pieces**: one bullet per marked location, each starting with its mark
  letter (e.g. `**B** — session expiry now also clears notifiers; check the miss path.`)
  and phrased as *what to scrutinize*, not a restatement of the diff.
- **Impact / risk**: callers and contracts from step 4, and anything you could not
  verify.
- **Marks legend**: a short line mapping `A`–`F` to `file:line`, so the marks are useful
  even without the tablet.
- Follow the `eink` skill's authoring rules (bullets over prose, at most one diagram)
  and its delivery contract (background observer, confirmed handshake, `feedback_id` +
  `expected_version` + `consumed_annotations`, rearm before drafting).
- Suggest a `tags` entry such as `pr=<n>` when creating the session, so the review is
  findable later.

## 6. When the review comes back

Handwritten notes arrive with an anchor to the nearby text (the mark letter is often
part of it) and an index into the annotations array. Act on the note against the
worktree checkout:

- Resolve "the thing marked B" through the marks legend, and read the line before
  editing.
- Make the requested change **in the worktree**, not the user's main checkout, unless
  they ask otherwise.
- Keep the same e-ink protocol: acknowledge only the annotations you actually handled,
  and record the revision (sha) the feedback applied to.

## Degradation summary

| Missing | Do |
|---|---|
| herdr pane | Worktree + analysis; print the exact nvim command; skip marks; say so. |
| nvim | Worktree + analysis + eink review; skip marks; say so. |
| socket | Reopen nvim with `--listen`, else skip marks. |
| `gh` | Ask for URL/base, or review the current branch. |
| eink server | Report the outage; the worktree and analysis still stand. |

Never claim a step succeeded because a command exited zero: read the JSON, the socket,
and the mark verification.
