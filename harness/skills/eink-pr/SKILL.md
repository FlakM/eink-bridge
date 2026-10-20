---
name: eink-pr
description: Review a pull request on the Boox with a parallel subagent workflow — check the code out into a git worktree, open the diff in Neovim in a new herdr tab, run several reviewers in parallel, filter their findings down to the few that matter, pass them through the brevity editor, then send a short actionable review to eink and mark the key lines with global marks. Use for "/eink-pr", "review this PR on eink", "put this PR on the tablet", "self-review my change on the tablet", or reviewing a PR link on the e-ink reader. Handles several PRs at once.
---

# E-Ink PR Review

Turn a pull request into something short and worth reading on the Boox, with the code
open beside the reader so a handwritten note becomes an edit immediately.

Delivery, document protocol, and feedback acknowledgement belong to the **`eink`**
skill — read it and follow its contract (ask the review purpose when unclear, a
background observer with a confirmed handshake, `feedback_id` + `expected_version` +
`consumed_annotations` on every rewrite, rearm before drafting). Locations and marks
belong to the **`jump`** skill. This skill orchestrates them; it does not reimplement
either.

## Two ways in

1. **You are already in an agent session** (Claude or OpenCode) and want this review in
   it: run the skill. It joins the review instead of starting a second one.
2. **A request arrives** (a message plus a link): use the herdr widget — `prefix+shift+p`
   runs `eink-pr-launch`. It resolves the repo, creates the worktree and a workspace for
   the review, and starts OpenCode seeded with the request; the session then follows this
   skill.

Both paths share the same conventions, so they meet in the middle:

- **one workspace per review**, keyed by PR number, recorded in
  `~/.local/state/eink-bridge/pr-reviews.json` (`EINK_PR_REGISTRY` overrides it);
- **worktrees live at `~/programming/<org>/worktrees/pr-<n>/<repo>`**, matching the
  layout used for ticket work;
- the widget passes the pasted request by **file** (`EINK_REVIEW_SEED`), never inline.

So a review started from the widget is already set up: read the registry, reuse its
workspace and worktree, and skip steps 2 and 3 below.

## Why a pipeline

One prompt that reads a diff and writes a summary produces uneven reviews: many nits,
repeated points, missed contracts. Split the work, and each stage is checkable:

```
resolve PR -> worktree -> Neovim workspace + diff
                 |
      context: what / why / wider  +  background checks   <- read references/review-prompt.md
                 |
      parallel lens reviewers (subagents) -> findings.json
                 |
      review.py filter   (mechanical: only relevant, actionable findings)
                 |
      ihaveadhd          (model: brief and human)
                 |
      review.py lint     (mechanical gate: refuses verbose output)
                 |
      review.py render   (context opening + checks + marks) -> eink -> feedback -> jump
```

**The brief always opens with what changed, why, and how it fits the wider system**, and
it always states what was checked. `review.py lint` enforces both, so a brief cannot reach
the tablet without them. How to collect that context, which checks to run in the
background, and the calibrated writing prompt are in
`references/review-prompt.md` — read it before writing any brief.

Any stage can fail without failing the review: if the filter box keeps nothing, say the
PR looks clean rather than padding the page.

## Preconditions

```bash
test "${HERDR_ENV:-}" = 1 && echo herdr-ok || echo herdr-missing
command -v gh nvim git jq
```

- **Not inside herdr**: still do the worktree, the review, the filter and the delivery;
  skip the tab and the marks and say so. Do not drive herdr from outside a pane.
- **No `gh`**: take a PR URL and a base ref, or review the current branch.
- **No `nvim`**: skip marks; the brief still carries `file:line`.

## 1. Resolve the review target

Three shapes, same pipeline:

- **Self-review of work just made** (Claude Code just wrote the change): no PR yet.
  Use the current branch against its base — `git merge-base origin/HEAD HEAD` — and
  title it `Changes: <short description>`. This is the "did I get it right?" pass.
- **A PR link or number** (a teammate's): `gh pr view <pr> --json
  number,title,url,baseRefName,headRefName,additions,deletions,changedFiles,author`.
- **A branch name**: `git fetch origin <branch>` and review it against the merge base.

Compute the **merge base**, not the base tip: the diff is
`merge-base(base, head)..head`, so unrelated upstream commits stay out.

Confirm the review focus in one line when it is not obvious from the PR title; do not
re-interview the user. A PR title plus files changed is usually enough to state what to
look at.

## 2. Check the code out into a worktree

**First look in the registry** (`~/.local/state/eink-bridge/pr-reviews.json`): if this PR
already has an entry, reuse its `worktree`, `socket` and `pane_id` and skip to step 4 —
the editor is already open on the diff. The widget creates all three, and re-invoking the
skill must not make a second copy or a second editor.

```bash
git fetch origin pull/<n>/head          # PRs, including forks
git worktree add --detach "~/programming/<org>/worktrees/pr-<n>/<repo>" FETCH_HEAD
```

- **Path**: `~/programming/<org>/worktrees/pr-<n>/<repo>` (the org is the parent
  directory of the local checkout). `self-<branch>` for a review of your own work.
- **Reuse**: fetch and `checkout --detach` instead of adding a second worktree.
- **Record the sha** you reviewed and write the registry entry (`pr`, `sha`, `worktree`,
  `socket`, workspace ids) so the other entry point finds it.
- Never check out in the user's own working tree.

## 3. The diff is already open — do not start a second editor

When the widget launched this review it created the reading workspace, started
`nvim --listen <socket> -c "DiffviewOpen <merge-base>..HEAD"`, and recorded `socket`,
`pane_id` and `sha` in the registry. A PR with a registry entry needs **nothing** in this
step. `EINK_PR_SOCKET` is also in the reading workspace's environment.

Only when you had to create the workspace yourself, start the editor in a **shell** pane:

```bash
export EINK_PR_SOCKET=/run/user/$(id -u)/jump-pr-<n>-<repo>.sock
herdr workspace create --cwd "<worktree-dir>" --label "PR <n>: <short title>" --focus
# -> .result.root_pane.pane_id
herdr pane run <root_pane.pane_id> "nvim --listen $EINK_PR_SOCKET -c 'DiffviewOpen <merge-base>..HEAD'"
```

- **Never send `nvim ...` into a pane that already runs Neovim or an agent.** `pane run`
  delivers the command to the pane's foreground process, so it lands in the buffer as
  text. Check `herdr pane process-info <pane>`; if it is not a shell, create a tab
  (`herdr tab create --workspace <id> --cwd <worktree> --focus`) and run the command in
  that tab's pane.
- **One workspace per review**, not a tab in someone else's workspace: the review owns its
  editor, its marks and its agent, and closes cleanly. Reuse the registry's workspace if
  one exists.
- **Always pass `--listen`** and use the registry's socket. It is
  `jump-pr-<n>-<repo>.sock`, not `jump-<repo>.sock`: two PRs of one repository are
  reviewed side by side and must not share an editor socket.
- Wait for the socket (`until [ -S "$EINK_PR_SOCKET" ]`) before sending anything. Marks and
  the `jump` skill depend on it; `jump <ref>` also finds this editor on its own.

## 4. Collect context and start the checks

Read `references/review-prompt.md` and follow it. In short: gather **why** (description,
issue, commits), **what** (the diff), **wider system** (callers, contracts, migrations) and
**checks** — in parallel, before reading the diff hunk by hunk — and write `context.json`.
Start the background checks now so their results are ready when the brief is written; a
check that cannot run is reported as such, never implied green.

`why` is quoted from a human source, not invented. When the change is unexplained, that gap
is itself worth the reader's attention.

## 5. Run the lens reviewers in parallel

Dispatch **separate subagents in the same turn** so they run concurrently. Give each
one the worktree path, the diff range, and one lens. Cap at four lenses for a normal
PR; add the conditional ones only when the diff warrants it.

| Lens | Looks for | When |
|---|---|---|
| correctness | logic errors, off-by-one, wrong default, unhandled error path, swallowed result | always |
| contracts | public API/HTTP/proto/schema, config keys, CLI flags, on-disk formats, versioning | always for libraries/services |
| tests | behavior with no test, tests that would pass with the bug present | always |
| concurrency | locks across `await`, races, unbounded growth, retries without backoff | when async/shared state changes |
| security | authz checks, injection, secrets in logs, unsafe deserialization | when trust boundaries move |
| docs | stale doc comment, README/code drift | when docs change |

Each reviewer must return **only** findings in this shape, and nothing else:

```json
{"findings": [{
  "lens": "correctness",
  "severity": "critical|major|minor|nit",
  "confidence": 0.0,
  "file": "server/src/app.rs",
  "line": 170,
  "symbol": "expire_loop",
  "tags": ["behavior-change", "public-api"],
  "claim": "one factual problem statement",
  "evidence": "what in the diff shows it",
  "action": "what to change"
}]}
```

Instruct them explicitly: a finding without a real `file`, a `line` inside that file, a
claim and an action is not a finding — drop it rather than filling the quota. Inventing
a problem to seem useful is worse than returning none, and it is the fastest way to
lose the reader's trust. Give them the exact revision so the line numbers are stable.

Collect every reviewer's findings into one file:

```bash
python3 <skill-base>/scripts/review.py validate --findings findings.json   # schema check
```

## 5b. Put the contract in front of the reader

If the change touches a **contract** -- a proto, a schema, an IDL, an OpenAPI document --
or if the PR has a **companion PR in a proto/interface repo**, that is the part the reader
is being asked to agree with. Show it, ahead of the implementation:

- Find the interface diff: `git diff` for `.proto`/`.avsc`/`.thrift`/`.graphql`/OpenAPI
  files in this change, and any linked companion PR (the PR body usually names it). Read
  it from the sibling worktree, or fetch that PR's diff if it is not checked out.
- Present it as its own `## Interfaces` section: the messages, fields, methods, defaults,
  and what breaks or stays compatible. A short `snippet` with a `diff` block is the right
  form -- it is the contract, so it earns the space.
- Keep the implementation to one line per change. The reader can read code later; they
  cannot un-agree with an interface.

`review.py filter --changed-file <path>` (repeatable, or `--changed-files-from <list>`)
ranks findings on contract files first, and `review.py lint --changed-file <path>` **fails a
brief that touches a contract and has no `## Interfaces` section**.

## 6. Filter to only relevant feedback

This is the step that keeps the tablet readable. Run it before anything is written for
a human:

```bash
python3 <skill-base>/scripts/review.py filter \
  --findings findings.json --checkout "<worktree-dir>" --max 6 --out filtered.json
```

It drops, with a reason for each: invalid findings, anchors past the end of the file,
nits, low-confidence non-critical claims, high-severity claims with no evidence, and
near-duplicates (keeping the stronger one). It ranks by severity, then confidence, then
blast radius and caps per file and in total. Read the printed reasons: if it drops
something you believe is real, fix the finding (add evidence, correct the line) and
re-run — do not bypass the filter.

When several reviewers found the same theme in different words, a short synthesis
subagent may merge them into one finding **before** filtering; it must not add findings
of its own. Skip it for small PRs.

If nothing survives, the honest answer is "no actionable findings" — send a two-line
brief saying so, or tell the user and stop.

## 7. Make it brief and human

Two layers, and both must pass:

1. **`ihaveadhd`** — rewrite the findings text the way a terse human would say it. Give
   it the finding text, not the diff. It keeps every `file:line`, mark letter and
   number, and cuts hedging, filler, restatement and non-actionable lines.
2. **The lint gate** — mechanical, and it blocks delivery:

```bash
python3 <skill-base>/scripts/review.py render --findings filtered.json --out brief.md --marks-out marks.json
python3 <skill-base>/scripts/review.py lint --file brief.md    # non-zero = fix it, do not send
```

Wire the `ihaveadhd` output back into `filtered.json` (`claim`/`action`) and re-render,
so the renderer still owns letters and the legend. If lint fails, cut the words it
names and re-run; never send a failing brief because it "mostly reads fine".

## 8. Marks and jumpable links

Render assigns `A`–`F` to the kept findings. Set them in the running Neovim and verify:

```bash
python3 <skill-base>/scripts/nvim_marks.py --socket "$EINK_PR_SOCKET" \
  --marks "A:server/src/app.rs:170,B:proto/api.proto:12" --verify A,B
```

Then attach permalinks pinned to the reviewed sha, so the legend works away from the
tablet (the `jump` skill covers this):

```bash
jump github-link --file server/src/app.rs --start-line 170 --end-line 176
```

Re-render with `--links links.json` to put the URLs in the legend. If the socket is
gone, reopen Neovim with `--listen` and retry; if marks cannot be set, say so and still
send the legend as text.

## 9. Deliver, then hand the reader the wheel

Create the session and deliver the brief through the `eink` skill's contract — its
observer, its handshake, its version guard. Add `tags` with `pr=<n>` and `sha=<short>`
so the review is findable.

Tell the user, in one line: what is on the tablet, the session id, and that `A`–`F` jump
to the important lines in the tab you opened. Nothing else.

## 10. When feedback comes back

A note arrives anchored to nearby text and often names a mark letter.

1. Resolve it: `review.py`'s marks map, or `jump` — `nvim_marks.py --jump B` puts the
   cursor on the line the reader meant.
2. Read the line before changing anything.
3. Edit **in the worktree**, never the user's main checkout, unless asked.
4. Reply through `eink` (acknowledge only what you handled) and note the sha.
5. If the note asks for an explanation rather than a change, answer it in the next
   brief — still under `ihaveadhd`.

## 11. Collect the comments and post the review

Comments come from two places and become one GitHub review:

- **Neovim:** `<leader>gc` (or `:EinkComment`) at a line records `path`, `line`, `side` and
  text in `~/.local/state/eink-bridge/comments/pr-<n>.jsonl`; `:EinkComments` shows them.
  The reading workspace sets `EINK_PR_NUMBER`; in a `pr-<n>` worktree it is derived.
  Diffview's base side is recorded as LEFT and the new side as RIGHT, so the line numbers
  are the ones GitHub validates.
- **e-ink:** an annotation that comments on the *code* rather than on how the brief reads
  is a review comment. Use the `path:line` in its anchor or code snippet; if it has no
  place, it belongs in the review body. Record it with:

```bash
python3 <eink-pr-base>/scripts/pr_comments.py add \
  --pr <n> --path <file> --line <n> --body "<the note>" --source eink
```

At the end of the review:

```bash
python3 <eink-pr-base>/scripts/pr_comments.py list --pr <n>
```

Show that list in chat and ask how to post it: **pending review** (the default; the reader
finishes it in GitHub), **submit as a comment**, or **discard**. Do not post without the
answer.

```bash
# pending review, finished in the GitHub UI
python3 <eink-pr-base>/scripts/pr_comments.py post --pr <url> --yes
# or submit it now
python3 <eink-pr-base>/scripts/pr_comments.py post --pr <url> --submit --yes
```

A line outside the diff cannot take an inline comment; the script puts it in the review
body and reports how many. A successful post moves the JSONL aside, so a retry or a second
agent cannot duplicate the comments. Report the review URL.

Do not turn every e-ink note into a comment: "shorter", "add a diagram" changes the brief;
"this races with the cache" is a comment. When it is genuinely unclear, ask one short
question.

## Several PRs at once

Every per-PR resource is keyed by number, so reviews cannot collide:

| Resource | Key |
|---|---|
| worktree | `pr-<n>` |
| socket | `/run/user/<uid>/jump-pr-<n>-<repo>.sock` |
| herdr tab | `PR <n>: <title>` |
| eink session | one per PR, one observer each |

Run the pipeline per PR; the parallel lens reviewers can span PRs in one turn. Record
live reviews in `~/.local/state/eink-bridge/pr-reviews.json` (`pr`, `sha`, `session`,
`socket`, `worktree`) so a second run reattaches instead of duplicating, and so cleanup
knows what it owns. Clean up by removing only what you created: `herdr workspace close`
(only for a workspace you created), `git worktree remove`, and the socket file.

## Testing this workflow

```bash
python3 harness/eink-pr/run_scenarios.py            # deterministic, run on every change
python3 harness/eink-pr/run_scenarios.py --live     # agent-level, slower, opt-in
just harness-test                                   # all harness tests
```

The scenarios cover a noisy self-review, a teammate PR with a contract change, a large
noisy PR, a docs-only PR, a clean PR, and chatty agent output. `--list` shows them.
When a scenario fails, fix the pipeline (or the scenario), never the assertion.

## Degradation

| Missing | Do |
|---|---|
| herdr pane | worktree + review + delivery; skip tab and marks; say so |
| nvim / socket | skip marks; keep the legend as text |
| `gh` | ask for URL + base, or review the current branch |
| subagents | run the lenses sequentially, say the review is narrower |
| eink server | report the outage; leave the worktree and brief on disk |

Never claim a stage succeeded because a command exited zero: read the filter's reasons,
the lint's output, and the mark verification.
