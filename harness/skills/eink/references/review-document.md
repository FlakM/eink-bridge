# The page the reader gets

The tablet is not a place to read a diff. It is a place to **decide about a change**, with
the code one tap away and a pen in hand. This file is the recipe for that page.

Read this before writing any review document. `eink-pr` renders the same skeleton from
`eink-pr-review`; a harness test keeps the two from drifting apart.

## Why this shape and not a summary

A review page is judged as a *reading instrument*, and the research on that is old,
consistent and unflattering about the obvious design:

- **Comprehension, not defect-finding, is the bottleneck.** In a study of professional
  reviews, finding defects was only the fourth most frequent kind of comment — 14% of 78
  (Bacchelli & Bird, ICSE 2013). The reviewers' own words: understanding the change is
  the hard part. A page that lists nits and skips the shape of the change has optimised
  the easy 14%.
- **Size decides effectiveness before anything else.** Under 200 changed lines is the
  target, 400 is the ceiling, 300 lines an hour is the fastest rate that still finds
  defects, and 60–90 minutes is a session (SmartBear/Cisco, 2,500 reviews). Above ~1000
  changed lines nobody is reading; Danger warns at 600.
- **The same study's one line about documents**: authors who prepare the review with
  *annotations and explanations* have far better outcomes than authors who do not. That is
  this file. The document is the intervention.
- **Google's review order** is design, then functionality, complexity, tests, naming,
  comments, style, docs. The document should spend its words in that order too — a page
  that leads with naming has inverted the reader's priorities.
- **A prohibition about shape makes output worse than no instruction.** Matching form to
  failure means writing the page as a *recipe with named slots* ("the page is: this, then
  this"), not as a list of don'ts. This file is written as slots for that reason. Keep it
  that way when you edit it.

## Three shapes, one skeleton

Pick the shape from the job the reader is doing, not from what is convenient to write.

| Shape | The reader's job | Use when |
|---|---|---|
| **decide** | judge a change or a choice | a PR, a diff, a design with options, "should I merge this" |
| **understand** | learn something well enough to explain it | a concept, an architecture, a postmortem, a plan |
| **round** | see what happened to their notes | the second and later rounds of a live review |

Every shape starts with a **one-line answer** and ends with **what to do next**. A page
without those two is a dump, whatever its body looks like.

The reader is holding a pen. Every page ends by telling them what a pen stroke will reach.

---

## Shape: decide

The review. Copy this skeleton, keep the order, fill the slots.

```markdown
# <change, in the reader's words — not the ticket title>

**Ready?** <Yes | No | With fixes> — <the verdict in one clause>.

<12 files · +340 −96 · 4 min · 310 lines to judge, 30 mechanical>

**What changed** — <one sentence, plain terms>.

**Why** — <quoted from the description or issue, or "not stated">.

**Wider system** — <what else this touches or could break>.

**The question** — <the one thing the reader must answer, answerable in a sentence>.

## Read these first

- **A** `issue (blocking)` <claim, with the trigger named>. <the action, verb first>. `app.rs:170`
- **B** `question` <what is genuinely undecided>. <what would settle it>. `session.rs:118`

## What I checked

- [x] `cargo test`, `clippy` — green.
- [x] Every caller of `expire()` updated (4, all in `session.rs`).
- [ ] Not checked: the Lua plugin path — no test reaches it.

## Set aside

- Retry backoff — deliberate, matches the gateway's.
- The rename in `session.rs` — mechanical, no behaviour.

## Change surface

| surface | effect |
|---|---|
| HTTP API | none |
| `session.json` | one additive field |
| CLI flags | none |

## If you only read one thing

<the single paragraph that would let the reader approve or refuse without the rest>

---

Write on the page, or reply **LGTM** / **CHANGES**. I'll apply what you mark and send the
next round.
```

### The slots, in order

**Title** — the change in the reader's words: "Expiry leaks the notifier map", not
"[STS-882] fix cleanup". The ticket number belongs in a tag, not a title.

**Ready?** — the verdict, first, before evidence, because a reader who trusts you can stop
after one line and a reader who does not can read on. Pick one of the three and give the
reason in one clause. `With fixes — the leak is real, the guard is not` is right.
`Mostly fine` is not a verdict. Google's standard applies: approve once it *improves*
the code, not when it is perfect.

**Budget line** — files, added/removed, minutes, and the split between lines that need
judgement and lines that do not. The split is the most useful number on the page: a 900-line
diff that is 850 mechanical is a 50-line review, and saying so is the difference between a
reader who starts and one who closes the tablet. Count it with `git diff --stat` and
`git diff -w` (whitespace), `--diff-filter=R` (renames), and generated or vendored paths.

**The three opening lines** — what, why, wider, **each its own paragraph**. Consecutive
lines of markdown join into one block, and one block is one anchor: the reader cannot put a
note on *Why* without catching *What changed*. A blank line between them is not cosmetic. `why` is quoted from a human source; when
there is none, say "not stated" and put the gap in `Set aside`. A guessed rationale reads
as fact and misleads.

**The question** — one sentence, and the reason this shape exists. A review that ends
"looks good" teaches nothing; one that ends with a real question gets a considered answer.
Ask the biggest thing you would otherwise have to bury in a comment: "split the migration
out of this PR?", "is the extra round trip acceptable for login?". If there is genuinely
no question, write **The question** — none, and say what makes this one safe. That is
information too.

**Read these first** — three to six findings, ranked by what would hurt most if missed,
each one line, each with a label and a place. This is the whole review. Beyond six the
reader stops believing the ranking, and the tail costs attention that belongs to the head.
Label with the vocabulary a review already has (Conventional Comments), because it is the
skim layer: a reader can find every `issue (blocking)` without reading prose.

| label | means | one decoration |
|---|---|---|
| `issue (blocking)` | wrong, and must change before merge | `(blocking)` |
| `issue` | wrong, but survivable | `(non-blocking)` |
| `suggestion` | would be better | `(if-minor)` |
| `question` | needs an answer before this is safe | |
| `praise` | something done well, and why | |

Combine similar comments into one line that covers both. Twelve nitpicks are one
`nitpick` with a patch, or nothing at all. Never invent praise; one real `praise` line is
worth more than three polite ones, and its absence is noticed.

**What I checked** — the asymmetry, as checkboxes. What ran, what you read, and what you
did **not** reach. This is the highest-trust section on the page: a reader who knows where
you stopped can aim their own attention there instead of re-deriving your coverage.
"Not checked" is a valid, useful line. Implying green for something that did not run is
the fastest way to lose the reader.

**Set aside** — the things you looked at and deliberately did not raise, one line each with
the reason. This is the innovation worth stealing: silence becomes visible. Without it the
reader cannot tell a considered non-issue from an oversight, and every omitted nit sits in
their head as a doubt. Three to six lines. `Deliberate`, `mechanical`, `pre-existing`,
`out of scope for this change` are the usual reasons.

**Change surface** — a table, and only when the change touches something other code
depends on: an API, a schema, a config key, an on-disk format, a CLI flag. Columns are the
surface and the effect on a caller that has not changed yet. When the contract *is* the
thing being judged, put its diff under the table in a `snippet` or `diff` block — that is
the one diff that earns the page, because it is the thing that cannot be un-agreed with
later, and the implementation behind it can be read later.

**If you only read one thing** — one paragraph, at the end, for the reader who has
scrolled past everything. Not a summary of the summary: the single argument that decides
it. This is the inverted pyramid closing on itself: verdict, evidence, argument.

**The last line** — how to reply, in the two moves the hardware makes possible: write with
the pen, or write a verdict word. Never make the reader guess what happens next.

---

## Shape: understand

A concept, a design, a plan, a postmortem. The reader is not deciding yet; they are
building a model they will use later.

```markdown
# <the idea, stated as a claim>

**In one line** — <the thing to remember>.

**Why it matters** — <the consequence, for this reader>.

## The mental model

<the three or four moving parts, as a list, each one clause>

## One worked example

<a real case, end to end — the reader generalises from this, not from prose>

## Where it breaks

- <the gotcha>. <how it shows up>.

## Where to look next

- <file:line or document> — <what is there>
```

Same rules as `decide`: one line at the top, one thing to do at the bottom ("tap
`reanchor.rs:120` to read the matcher", "reply with what is still unclear"). The worked
example is doing the work that prose cannot — a reader can re-derive a rule from an example,
and cannot from a paragraph.

## Shape: round

The second and later round of a live review. The reader has already read once and marked
the page; their attention is on **what changed because of them**.

```markdown
# <same title as before> — round 2

**You asked for** — <the notes, compressed into one sentence each>.

## What I changed

- <note, in the reader's words> → <what you did>. `app.rs:170`

## What I did not change

- <note> → <why not, in one line>.

## New in this round

- <anything that appeared because of the edit — keep this short; new material is where
  mistakes enter>

## Still open

- <the question that survived, if any>
```

The order is the point: **their notes first**, then the diff your edit made, then new risk.
A round that opens with a fresh summary of the document makes the reader re-read what they
already agreed to.

---

## Whatever the shape: the writing rules

- **The heading states the claim.** "`expire()` clears the notifier map but not the socket
  map" beats "Notifications". A reader skimming only the headings should get the argument.
- **One idea per line.** If a line has two verbs, it is two lines.
- **Numbers, flags, error strings, identifiers verbatim.** Say `EINK_TOKEN`, not "the token
  env var", and `409`, not "a conflict". The reader will search the code for the string.
- **Verb first in every action**: "clear", "verify", "decide", "split". "It would be good
  if" is not an action.
- **One mode per section.** Explaining and enumerating are different modes; a section that
  does both does neither well.
- **Nothing the reader already knows.** No apology, no restatement, no "in this document I
  will". The first sentence is the answer.
- **No links longer than a line.** The reader cannot type a URL from e-ink; a reference
  they can tap (`path:line`) or a letter they can press in the editor is the way through.

## The arsenal

The renderer is a page, not a markdown preview. Pick the form that carries the idea with
the least reading.

| Form | Use when | Renders as |
|---|---|---|
| short bullets | the default | text |
| `table` | two or more things on the same axes: before/after, options, surfaces | ruled table |
| `snippet` | the reader must see code — a call site, the changed lines, a guard | captioned, highlighted, line-marked |
| `diff` | only the contract being judged | coloured lines |
| `steps` | a sequence over time — a request, a migration, a race | ordered list, tap to advance |
| `mermaid` | interaction or ordering between parts | diagram |
| `graph` | how components relate; blast radius | diagram |
| `mindmap` | decomposition of a plan or a chain of causes | collapsible tree |
| picture | the artefact itself: a rendered UI, a chart, a photo of a whiteboard | image |
| `<details>` | background a reader may want and most will not | collapsed |
| `- [ ]` | what was and was not checked | checkbox |

**One diagram per page**, two only when they answer different questions. A diagram is the
most expensive thing on the page: it does not zoom, it does not skim, and it reflows badly.
Bullets saying the same thing are better in every case.

A `snippet` beats a screenshot of code on every axis — sharper, searchable, annotatable per
line, and it costs kilobytes instead of megabytes. Screenshots are for what text cannot
say: the mis-rendered page, the layout bug, the chart.

```snippet
rust server/src/app.rs:170 | the expiry loop #L2
for id in expired {
    notifiers.remove(&id);
}
```

The first line is `LANG REF | CAPTION #L12,15`; every part is optional. `#L` marks the exact
line so the reader does not hunt.

Colour is available inline and means the same thing everywhere: `<span style="color:#b3261e">`
for broken, `#2e7d32` good, `#a15c00` uncertain, `#1a4d8f` reference. Use it on the word
that carries the meaning, never on a whole sentence, and never as the only signal — a
greyscale panel is the common case.

## The budget

| The page | Target | Ceiling |
|---|---|---|
| words | 350 | 450 |
| findings | 4 | 6 |
| diagrams | 0 | 1 |
| pictures | 0 | 1 |
| changed lines in scope | 200 | 400 |
| reading time | 4 min | 8 min |

Over budget, the answer is not "cut words" — it is **split the change**. A page that cannot
fit is telling you the PR is too big, which Google will also tell you, and which is a
finding in its own right: `issue (blocking) — 1,400 lines in one change; split the
migration out and this reviews in 20 minutes instead of 90`.

## What makes a page feel like a diff dump

Read this list as symptoms, not sins. Any one of them means the page is doing the editor's
job.

| Symptom | What is really wrong |
|---|---|
| the page shows a diff, or a second diff | the reader has a diff; the page owes them judgement |
| findings are ordered as the reviewer found them | nothing was ranked, so nothing is prioritised |
| every finding is `nitpick` | the review has no opinion |
| "looks good" with no `What I checked` | coverage is unknown, so the verdict is worthless |
| a summary heading, then a TL;DR | the page is stalling instead of answering |
| a link where a `path:line` would do | the reader has to leave the page to act |
| nothing under `Set aside` | either nothing was examined, or the examination is invisible |
| the question is missing | the page is an announcement, not a review |
