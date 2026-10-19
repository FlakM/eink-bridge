# Plan: a CLI-first harness

The agent writes documents. Everything else -- sessions, feedback, worktrees, editor
marks, filters, delivery, diagnostics -- is a deterministic command with a stable exit
code and `--json` output. A skill becomes a page: when to use it, the one question to
ask, the commands to run, and what to write.

This supersedes item 3 of `FIXME.md` and absorbs the "smaller items" there about skills.

## Why

Today the skills carry the procedure and the agent executes it by hand:

| Procedure the agent performs today | Where it is written | Lines |
|---|---|---|
| pick a session, watch it, arm the observer, confirm the handshake, rearm after each note | `harness/skills/eink/SKILL.md` + `scripts/watch-feedback.py` | 364 + 124 |
| PUT content with `expected_version`, `feedback_id`, `consumed_annotations`, handle 409 | eink skill (curl / `update_session`) | -- |
| resolve a PR, compute the merge base, add a worktree, write the registry JSON by hand | `harness/skills/eink-pr/SKILL.md` §1-2 | 330 |
| create a herdr workspace, start Neovim with `--listen`, wait for the socket | eink-pr §3 | -- |
| validate / filter / render / lint findings, set and verify marks, make permalinks | `review.py`, `nvim_marks.py`, `jump` | 594 + 171 |
| start a dev server, create a session, launch Chrome, forward the tablet's CDP socket | `harness/skills/eink-webview/SKILL.md` | 258 |
| commit, push, bump a flake input in another repo, rebuild, verify | `harness/skills/deploy-eink/SKILL.md` | 51 |

Each of those is a place where an agent improvises: a wrong flag, a literal `~`, a
forgotten rearm, a hand-written JSON file. The survey on 2026-10-19 found the skills
disagreeing with each other on socket paths, worktree layout, verdict casing and error
texts, and the CLI unable to do the feedback round trip at all. None of that is content
work. All of it is deterministic.

## Principles

1. **One binary, `eink-review`.** Every procedure above is a subcommand. Python helpers
   are ported, not wrapped: one language, one package in the flake, one test binary
   (`tests/cli_integration_test.rs`), no interpreter lookup on the agent's PATH.
2. **Idempotent by default.** Running a command twice yields the same state and says so
   (`pr prepare` reuses the registry entry, `watch` refuses a second observer for the
   same session, `editor open` finds the running socket).
3. **Machine-readable always.** `--json` on every command; without it, one line per
   fact. Exit codes are a contract (below). No prompts, no colour, no progress bars.
4. **Config over flags over nothing.** `~/.config/eink-bridge/config.toml` and
   `EINK_*` env carry server URL, token, registry path, worktree root, socket dir.
   Skills never spell out defaults.
5. **The agent sees compact facts, not transport.** Events are summarised (no stroke
   arrays) by the CLI; the channel server stays a thin pump that calls the same HTTP
   routes the CLI does.
6. **The skill is the manual, the CLI is the machine.** Every `eink-review` invocation
   in a skill is checked against `--help` by a harness test; a skill may not contain
   `curl`, `python3`, `jq` or `herdr` for anything the CLI does.

### Exit codes

| Code | Meaning |
|---|---|
| 0 | done |
| 1 | usage error (bad flag, missing file) |
| 2 | server unreachable |
| 3 | still pending (result / watch deadline reached, review active) |
| 4 | conflict (stale version, second submit, session not active) |
| 5 | refused (401/403/413, policy) |
| 6 | precondition missing (no herdr, no nvim socket, no gh, no device) |

## The command surface

```
eink-review
  doctor                                  # server, token, herdr, nvim, gh, adb, channel: each ok/missing
  push <file> [--title --tag k=v --timeout --async --callback-url]
  content <id> [--out file]               # current markdown + version
  update <id> <file> [--expected-version N] [--ack <feedback_id> --consumed 0,2]
  feedback <id>                           # pending feedback, compact
  progress <id> "<msg>" [--step n --of m] # FIXME item 4
  watch <id> [--timeout MIN --after-feedback ID --ready-file PATH --one-event]
  result <id> [--json]                    # exit 3 while active
  list [--status --starred --json]
  star | unstar | cancel | remove [--force] <id>
  source <id> <path:line> [--context N]

  pr prepare <ref>                        # PR url/number, branch, or "self": merge base, worktree, registry
  pr context <pr>                         # why/what/wider/checks -> context.json (facts only, agent fills prose)
  pr findings validate|filter|render|lint # review.py, ported; same flags, same reasons
  pr marks set|verify|jump|open           # nvim_marks.py, ported; socket from the registry
  pr links <pr> <file:start-end>          # permalink at the reviewed sha (jump github-link)
  pr deliver <pr> <brief.md>              # tags pr=/sha=, push, watch handshake, registry update
  pr status [<pr>]                        # registry view: worktree, sha, session, socket, workspace
  pr cleanup <pr>                         # only what prepare created: worktree, socket, workspace

  editor open <pr|path> [--diff base..head]   # herdr workspace + nvim --listen, or tmux, or report
  editor socket [<pr>]                        # the socket to use, in the skill's documented order

  dev serve [--port 3344 --release]       # isolated state dir, waits for bind, prints URL
  dev webview <file|--kitchen-sink> [--server URL --chrome]   # session + headed Chrome wired for CDP
  dev tablet screenshot [--out PATH]      # adb screencap, prints the path
  dev tablet cdp <session> [--scale 0.41] # forward webview_devtools, navigate, print viewport facts
  dev tablet logs [--since 5m]            # EinkDraw / MainActivity tags
  dev deploy [--server] [--harness]       # just deploy + the nix_dots bump, paths from config
```

Everything the agent needs from a session comes back as JSON with the same field names
the API uses (`version`, `feedback_id`, `annotations[].index`, `anchor`), so the skill
text can quote one shape.

## Phases

Each phase ships on its own and leaves every suite green: server tests, 66+ mutations,
280+ Android tests, harness tests. Add a `cli_integration_test` for every command and a
mutation case for every rule.

### Phase 0 -- the contract

- `doctor`, exit codes, `--json` on existing commands, `config.toml` + `EINK_SERVER_URL`
  / `EINK_TOKEN` fallback for `--server`, `list` checks HTTP status before parsing.
- A harness test (`harness/tests/test_skill_commands.py`) that extracts every
  `eink-review ...` invocation from `harness/skills/*/SKILL.md` and runs
  `eink-review <cmd> --help`, failing on an unknown command or flag. It also fails on
  `curl `, `python3 `, `jq ` or `herdr ` appearing in a skill once Phase 2 lands.
- Done when: `eink-review doctor --json` runs on a fresh machine and names what is
  missing; the asset test is green against the skills as they are.

### Phase 1 -- the session round trip without curl

- `content`, `feedback`, `update --expected-version --ack --consumed`, `progress`,
  `source`, `push --tag`; `result` exits 3 while active.
- `watch` absorbs `watch-feedback.py`: compact events (index, stroke_count, image count,
  no arrays), `--after-feedback`, `--ready-file` with the `.tmp`+rename receipt,
  `--one-event`, `EVENT:WATCH_ENDED` always written, one observer per session (a lock
  file under the state dir; a second `watch` exits 4 and prints the PID that owns it).
- The channel server's `update_session` and the new `report_progress` tool keep calling
  HTTP, but their request bodies are built by the same Rust types (`api.rs`) the CLI
  uses, via `/api/openapi.json` checked in a test, so the two cannot drift.
- Delete `watch-feedback.py`; the `eink` skill's delivery section shrinks to four
  commands (`push --async`, `watch --ready-file`, `update --ack`, `watch --after-feedback`).
- Done when: the eink skill contains no curl, no JSON PUT instructions and no
  `python3`; `test_feedback_delivery.py` drives the CLI end to end against a stub
  server; `eink-update/SKILL.md` is deleted (its one command is `update`).

### Phase 2 -- the PR pipeline as commands

- `pr prepare`: `gh pr view` or `git fetch origin <branch>` or `self`; merge base;
  worktree at `<worktree_root>/<org>/worktrees/pr-<n>/<repo>` from config (the single
  layout, `registry.py:33-40` and `SKILL.md:31,106` disagree today); registry entry
  written under a file lock with a unique temp name; `--json` returns everything the
  later commands need. Re-running returns the same entry and exit 0.
- `editor open`: herdr workspace create + `nvim --listen <socket>` + wait for the socket;
  tmux fallback; exit 6 with a message when neither is present. Socket path is
  `<socket_dir>/eink-pr-<n>.sock`, one spelling. `editor socket` implements the lookup
  order the `jump` skill documents.
- `pr context`: collects the *facts* `references/review-prompt.md` §1-2 asks for --
  PR body, linked issue, commit messages, changed files, interface files, callers via
  `git grep`, and starts the checks it names (`just test`, `cargo clippy`, whatever the
  repo's justfile has) in the background, writing `context.json` and `checks.json`.
  The agent writes the prose from those files; it does not gather them.
- `pr findings validate|filter|render|lint`: `review.py` ported to Rust behind the same
  flags and the same printed reasons. Acceptance: `harness/eink-pr/scenarios/*` become
  fixtures of a Rust test that asserts the same kept/dropped sets and the same lint
  verdicts as `run_scenarios.py` does today, before the Python is deleted.
- `pr marks set|verify|jump|open`: `nvim_marks.py` ported; the socket comes from the
  registry so the agent never passes it. `pr links` wraps `jump github-link` (or
  computes the permalink itself from the registry's sha and `gh repo view`).
- `pr deliver`: tags, push, confirmed `watch` handshake, registry update, one line of
  output for the user. `pr cleanup`: removes only what `prepare` and `editor open`
  created, by reading the registry.
- The `ihaveadhd` pass stays a model step, but its input and output are files the CLI
  reads back: `pr findings render --from filtered.json` after the agent edits
  `claim`/`action` in place.
- Done when: the eink-pr skill is under 120 lines and lists commands in order; `jump`
  skill is under 40 lines; `run_scenarios.py` is replaced by the Rust fixture test and
  a `--live` recipe in the justfile; `registry.py`, `review.py`, `nvim_marks.py` are gone.

### Phase 3 -- the developer loop

- `dev serve`, `dev webview`, `dev tablet screenshot|cdp|logs`: port the shell from
  `eink-webview/SKILL.md` and `tablet-screenshot/SKILL.md` (adb forward of
  `webview_devtools_remote_<pid>`, the `Page.navigate` and `Runtime.evaluate` payloads,
  `Emulation.setPageScaleFactor`). The skill keeps only what to look at.
- `dev deploy`: the `deploy-eink` steps with paths from config (`nix_dots` checkout,
  flake attribute), no `git add -p`, refuses to run with a dirty tree it did not stage,
  and ends with `just verify-deploy`. The skill becomes one command and a checklist.
- Done when: `eink-webview`, `tablet-screenshot` and `deploy-eink` skills are each
  under 40 lines, and `doctor` reports adb/Chrome/nix_dots availability.

### Phase 4 -- the skills, rewritten as pages

Each skill gets the same four sections and nothing else:

1. **Use when** -- one paragraph.
2. **Ask** -- the one question that changes the document (the reading contract), with
   the rule for when not to ask.
3. **Run** -- the commands, in order, each with the exit code that means "stop and say
   so".
4. **Write** -- what the document must contain (the authoring rules move to
   `docs/document-style.md`, which becomes the single source for the output style, the
   eink skill and the README; verdicts are quoted from `server/src/verdict.rs`).

`eink/evals/evals.json` runs from `just harness-test`. `test_harness_assets.py` checks
the four sections exist and the command test from Phase 0 checks every invocation.

## What stays with the model

- The reading contract: what to put on the tablet and why.
- The document itself, every rewrite, the progress lines, and the `ihaveadhd` pass.
- Lens reviews (subagents) and the synthesis of their findings into `findings.json`.
- Deciding which notes a rewrite consumed.

Everything else is `eink-review`.

## Risks and how to hold them

- **Porting `review.py` changes a verdict.** The scenario fixtures are the gate; port
  behind the same flags and run both implementations on every scenario until the
  Python is deleted.
- **herdr/nvim/gh differ per machine.** `doctor` and exit code 6 make the degradation
  explicit; the skill's table of fallbacks becomes the CLI's behaviour.
- **Another session is editing the channel while this plan lands.** On 2026-10-19 the
  tree carried uncommitted work adding `PUT /api/sessions/{id}/callback` and port-0
  binding in `channel.ts`; Phase 1 builds on that route rather than re-adding
  `--callback-url` plumbing.
- **Config sprawl.** One `config.toml` schema in `config.rs`, printed by `doctor`, with
  every key documented in `docs/api.md`'s sibling `docs/cli.md`.
