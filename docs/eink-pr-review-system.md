# E-Ink PR review: system design

Goal: review one or many pull requests on the Boox, with feedback that is accurate,
actionable, and short enough to read on an e-ink screen — and a harness that proves it
keeps working.

## Why a pipeline and not "one smart prompt"

A single prompt that reads a diff and writes a summary produces uneven results: it
drowns the reader in nits, repeats itself, and misses contracts. Splitting the work
gives each stage one job and one test:

| Stage | Kind | Testable without a model |
|---|---|---|
| 1. Resolve PR | mechanical | yes |
| 2. Worktree + Neovim diff | mechanical | yes |
| 3. Parallel lens review | **model** (subagents) | fixtures + live eval |
| 4. Filter to relevant feedback | mechanical (+ optional synthesis) | **yes, unit-tested** |
| 5. Brief + marks | mechanical | **yes, unit-tested** |
| 6. `ihaveadhd` human pass | **model** | fixtures + live eval |
| 7. Deliver to eink, handle feedback | mechanical | yes (real server) |

The model stages emit a strict JSON schema, so their output is checkable, cacheable,
and replayable. The mechanical stages are pure functions, so the rules that keep the
tablet clean — dedupe, ranking, caps, brevity — are enforced by code and covered by
tests rather than trusted to prose.

## Findings schema

Every reviewer subagent emits the same shape; `review.py` validates it.

```json
{
  "lens": "correctness",
  "severity": "critical|major|minor|nit",
  "confidence": 0.0,
  "file": "server/src/app.rs",
  "line": 170,
  "symbol": "update_content",
  "tags": ["behavior-change", "public-api"],
  "claim": "one factual problem statement",
  "evidence": "what in the diff shows it",
  "action": "what to change"
}
```

A finding without a file, a line, a claim, and an action is dropped: it is not
actionable, so it does not belong on the tablet.

## Filter rules (deterministic, in `review.py`)

1. Drop structurally invalid findings (missing file/line/claim/action).
2. Drop findings whose line does not exist in the checkout (stale anchors).
3. Drop `nit` unless nothing else survives or it is tagged `public-api`.
4. Drop `confidence < 0.5` unless `severity == critical`.
5. Deduplicate: same file, lines within 3, similar claim (token overlap) → keep the
   higher severity, then confidence.
6. Cap per file (2) so one messy file cannot dominate the brief.
7. Rank by severity, confidence, and blast radius (public-api / behavior-change /
   migration / security tags).
8. Cap the total (default 6, matching the usable mark letters).

## Brevity and humanness

Two layers, because "brief and human" needs both a floor and taste:

- **Floor (deterministic lint, `review.py lint`)**: word budget per bullet and per
  sentence, banned filler ("basically", "note that", "in order to", "robust"), no
  trailing summary, no raw diff dumps, and every important piece must carry an
  action verb. A failing lint blocks delivery — this is the rule the tests pin.
- **Taste (`ihaveadhd` agent)**: rewrites the text so it reads like a terse human
  wrote it, and reports what it cut. It cannot be unit-tested, so it is evaluated
  against fixtures with recorded before/after and an opt-in live run.

## Concurrency

Multiple PRs at once means every per-PR resource must be namespaced and independent:

| Resource | Keyed by |
|---|---|
| worktree | `pr-<n>` |
| Neovim socket | `/tmp/eink-pr-<n>.sock` |
| herdr tab label | `PR <n>: <title>` |
| mark letters | per review (A–F), read from the brief, not global |
| eink session | one per PR, one observer each |

A registry file records live reviews so a second run reattaches instead of duplicating.

## How it is wired

| Piece | Path | Role |
|---|---|---|
| orchestrator | `harness/skills/eink-pr/SKILL.md` | scenarios, subagent dispatch, gates |
| filter / render / lint | `harness/skills/eink-pr/scripts/review.py` | the deterministic rules |
| per-review keys + registry | `harness/skills/eink-pr/scripts/registry.py` | concurrency, reattach |
| marks / jump / open | `harness/skills/eink-pr/scripts/nvim_marks.py` | editor side |
| navigation skill | `harness/skills/jump/SKILL.md` | references ↔ editor |
| brevity editor | `harness/agents/ihaveadhd.md` | the taste pass |
| scenarios | `harness/eink-pr/scenarios/*.json`, `briefs/*.md` | fixtures |
| runner | `harness/eink-pr/run_scenarios.py` | offline + `--live` |
| reader loop | `harness/eink-pr/reader_loop.py` | tablet → agent, real server |
| multi-PR | `harness/eink-pr/multi_pr.py` | isolation + cleanup |

## Test harness

```sh
just harness-test          # unit tests: filter, render, lint, registry, marks, delivery
just eink-pr-scenarios     # deterministic pipeline scenarios
just eink-pr-live          # agent-level scenarios (drives the claude CLI)
just eink-pr-reader        # tablet -> agent path against the running server
just eink-pr-multi         # 3 reviews at once, isolation and cleanup
```

Coverage today: 7 deterministic scenarios (noisy self-review, teammate PR with a
contract change, a 30-finding large PR, docs-only, clean, and two agent-output lint
fixtures), one agent-level scenario, a live reader round trip, and a 3-way concurrency
run that also opens real herdr tabs. The agent-level and reader runs are opt-in and
report separately because they depend on the environment.

## Deliberate non-goals

- Not a replacement for human review; it ranks what to look at.
- No attempt to make the model deterministic — only its interface is fixed. The
  agent-level scenario asserts the *shape* of the output, not its wording.
- The brevity gate is a floor, not taste; `ihaveadhd` supplies the taste.

## Found by the harness, not by guessing

- A live agent wrote `- **A** New field is required… Make it optional`, and the lint
  gate rejected it because "make" was not in the action-verb list. The gate now accepts
  natural imperative and necessity phrasing, with a regression test taken from that
  real output.
- Duplicate detection missed `notifiers` vs `notifier`; token normalisation now stems a
  trailing plural, so the same finding worded twice collapses.
- `nvim --server` exits 0 even when it cannot connect, and marks persist in shada, so a
  silent miss read back as a stale mark from an earlier review. The helper verifies
  every mark it sets.

