# Collecting context and writing the review

Two artifacts, in order: **`context.json`** (collected once, shared by every reviewer) and
the **brief** (written from it). Skipping the first is what makes a review feel like a
diff dump: the reader has to re-derive why the change exists before they can judge it.

## 1. Collect the context the reader needs behind their eyes

Gather the four things below **in parallel** — they are independent lookups, not a
sequence — then write them down once.

| What | Where it comes from |
|---|---|
| **why** | the PR description, the linked issue, the commit messages, review threads |
| **what** | `git diff` against the merge base, the diffstat, changed symbols |
| **wider system** | callers of changed items, adjacent modules, contracts touched (proto/HTTP/config/CLI/on-disk), migrations |
| **checks** | CI status, build, tests, lint — run them in the background now, not at the end |

Reuse the helpers rather than re-implementing them:

```bash
bash ~/.claude/skills/aaa-pr-review/scripts/preflight.sh <n> --checkout   # CI, blockers, size
bash ~/.claude/skills/aaa-pr-review/scripts/pr-context.sh <n> <repo> <out> # description, threads
gh pr diff <n>            # or: git diff <merge-base>...HEAD
```

Then write `context.json`:

```json
{
  "what": "one sentence: the change in plain terms",
  "why": "one sentence: the problem it solves, from the description or linked issue",
  "wider": "one sentence: what else this touches or could break",
  "checks": [{"name": "cargo test", "status": "pass"}],
  "unknowns": ["what you could not determine"]
}
```

**Rules that keep this honest:**

- `why` is **quoted from a human source**, never invented. No description, no issue, and
  unhelpful commits is a real finding: put "not stated — infer from the diff" in `why`
  and list the gap in `unknowns`. A guessed rationale reads as fact and misleads.
- One sentence each, ≤ 25 words, concrete nouns. "Reworks the ingest pipeline" beats
  "various improvements to parsing and related components".
- `unknowns` is not failure: it becomes the question the reader answers on the tablet,
  which is the most valuable line in the brief.
- If the diff and the description disagree, say so — that is a finding, and often the
  most important one.

## 2. Checks run in the background

Start them when you start reading, so results are ready when you write. Derive extra
checks from the diff itself: touched a proto? check the schema lint. Changed a query?
check the migration. Touched concurrency? look for a test that exercises the race.

Report **failures and skips only**; when everything passes, one line names them. Never
imply green for something that did not run — "checks: not run" is a valid, useful line
and is exactly what the reader needs to weigh the review.

## 3. The brief

Hand this to the writer (or follow it directly). It is deliberately short: every extra
line is a rule the writer must hold in mind while reading code.

```
Write the tablet brief for <PR>. Output markdown only. Start from context.json.

Open with three lines, in this order, one sentence each (≤ 25 words):
  **What changed** — the change in plain terms
  **Why** — the problem, quoted from the description/issue, or say it is not stated
  **Wider system** — what else this touches

Then `## Checks` — failures and skips; one "All green: <names>." line when clean.

Then `## Important pieces` — at most 6, ranked, each:
  - **A** — <problem in one clause>. <what to do, verb first>.
Keep every file:line, number, flag and error string exactly as they are.
One idea per line. ≤ 25 words per line. Say "check"/"verify"/"decide" — the reader acts.

Then `## Marks` — `- A → path:line`. **No URLs**: the reader jumps by pressing the letter
in the editor, so a permalink is a line of noise they cannot use. Long links go in the PR.

Add at most one question the reader should answer, if the change leaves something genuinely open.
```

**Calibration — this is the part that goes wrong:**

| Findings after filtering | Brief |
|---|---|
| 0 | the three opening lines, the checks line, one sentence saying nothing actionable. No padding. |
| 1–3 | all of them; they are the review |
| 4–6 | all of them, ranked by blast radius |
| > 6 | the 6 that would hurt most if missed; drop the rest silently |

Not too much: if a piece does not change what the reader does, it is noise that costs them
attention on the *other* pieces. Not too little: the reader cannot re-derive a finding from
the diff mid-review, so a missing "why" or a bare file:line is a wasted round trip.

## 4. The presentation arsenal

Pick the form that carries the idea with the least reading. A brief is a presentation, not
a diff transcript.

| Use | When |
|---|---|
| short bullets | the default; most findings |
| **table** | two or more things compared on the same axes (before/after, options, cost) |
| **`snippet`** | the reader must see the code: a call site, the changed lines, the guard |
| **`steps`** | a sequence over time: a request lifecycle, a migration, a race |
| **`mermaid`** | interaction or ordering between parts (sequence, state, flow) |
| **`graph`** | how components relate; blast radius |
| **`mindmap`** | decomposition of a plan or a chain of causes |
| **`diff`** | a small, load-bearing diff only — the editor holds the rest |

Two rules keep this from becoming decoration: **one diagram per brief** (two only if they
answer different questions), and never a diagram where bullets say it faster.

### Code: highlighted text, not screenshots

Use a `snippet` block. Text stays crisp at any zoom, reflows to the column, is searchable,
costs bytes instead of megabytes, and needs no image decode on an e-ink device — a
screenshot of code is worse on every axis. Reserve images for what text cannot express: a
rendered UI, a chart, a layout.

```
```snippet
rust server/src/app.rs:170 | the expiry loop #L2
for id in expired {
    notifiers.remove(&id);
}
```
```

The first line is `LANG REF | CAPTION #L12,15` — all parts optional. The reference is what
the reader needs to find it; the `#L` markers point at the exact line so they do not hunt.

### Sequences: a tap-through, not an animation

E-ink cannot animate smoothly and should not try. A `steps` block reads as a plain ordered
list, and tapping advances which step is emphasised — the honest form of movement on this
device. Keep it to 3–6 steps; a longer sequence is a document, not a step.

## 5. The reader's question is the deliverable

A review that ends with "all good" teaches nothing; a review that ends with a real
question gets a considered answer. Prefer one sharp question over three soft ones, and
make it answerable in a sentence — the reader responds with a pen.
