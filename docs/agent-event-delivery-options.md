# Pushing review feedback into an agent session

How feedback from the Boox reaches a coding agent, and what other mechanisms exist
in **OpenCode** and **Claude Code**.

## Problem

The server durably stores a feedback batch and exposes it over WebSocket (`/ws/<id>`)
and REST (`GET /api/sessions/<id>/feedback`). Something must turn that server-side
event into something the agent *reads in its conversation*. Delivery quality is
judged by three properties:

- **Persistent** — no re-arming, no gap where feedback is silently missed.
- **Pushed** — the agent is woken, not asked to poll.
- **Attributed** — the right agent session receives the right review.

The shipped mechanism (below) is a **portable floor**, not the ceiling.

## Shipped today

| Client | Mechanism | Persistent? | Pushed? | Notes |
|---|---|---|---|---|
| Claude Code | MCP `notifications/claude/channel` via `eink-channel` server | yes | yes | Needs `--dangerously-load-development-channels` (a dev flag). Subscription recovers pending feedback. |
| Any | Background `watch-feedback.py` (CLI WebSocket) | **no — rearm each event** | yes | Bounded job; harness completion wakes the agent. Works everywhere; PATH shadowing must be avoided (fixed). |

Limits: the portable watcher must be re-armed after every event and can time out
mid-review; both paths live outside the agent runtime, so neither can inject a turn,
only deliver text into an already-active session.

## OpenCode options

OpenCode plugins are TypeScript modules (`Plugin.define({ id, setup(ctx) })`) loaded
from `.opencode/plugins/` or the `plugins` array, with a real client API.

### A. Plugin tool + session push (recommended)
A plugin registers a tool the skill calls to bind the review, then pushes on arrival:

```ts
// setup(ctx)
await ctx.tool.transform((editor) => editor.add({
  name: "eink_watch",
  description: "Bind this session to an eink review and receive feedback as messages",
  input: { type: "object", properties: { review: { type: "string" } }, required: ["review"] },
  execute: async ({ review }, context) => {
    bind.set(review, context.sessionID)   // context.sessionID is the caller's session
    return { content: `watching ${review}` }
  },
}))

// on WebSocket feedback:
await ctx.session.synthetic({ sessionID, text: `Boox feedback: ${text}` })
// or, to start a real turn: ctx.session.prompt({ sessionID, text })
```

- **Push into the conversation without re-arming** — a live plugin survives the
  session. Directly fixes the "must rearm / timed out" failure mode.
- `ctx.session.synthetic` adds context; `ctx.session.prompt` starts a turn.
- The plugin owns the WebSocket, so no background shell job is needed.

### B. Plugin `ctx.event.subscribe` (complementary)
`ctx.event.subscribe({ signal })` yields OpenCode's own event stream. Useful to know
when a session is idle/busy before injecting, or to auto-arm on session start — but
it is not a channel for our feedback.

### C. External bridge over the OpenCode server API
The plugin context *is* an OpenCode server client, and the server also exposes an
HTTP API. A small bridge (or the eink server itself) can POST a prompt into a session
by id. No plugin install required, but the bridge must know the session id and the
agent must have been started as a server client.

### D. MCP server in OpenCode
OpenCode can run MCP servers, but MCP server *notifications* are not injected as
conversation turns the way Claude channels are; MCP is a tool surface. So MCP alone
does not solve push delivery here.

### E. Background watcher (current floor)
Keep as the fallback for one-off/local use.

## Claude Code options

### A. Mods (recommended for parity)
A **mod** is a plugin whose hooks are JavaScript functions that run inside Claude
Code and can also draw in the UI:

```js
on('prompt.submit', async ($, e, next) => {
  const pending = await fetchDue()
  if (!pending.length) return next(e)
  return next({ ...e, context: [...(e.context ?? []), `Boox feedback: ${pending.join('; ')}`] })
})
```

- `prompt.submit` can add text **only Claude reads**, after the prompt.
- Mods can register **tools and commands** and **run work on a timer**, so a mod can
  own the WebSocket, bind via a tool (same pattern as OpenCode A), and inject.
- Higher fidelity than the dev-flags channel, and not behind a dev flag.

### B. `asyncRewake` settings hook (push, no plugin)
A background hook that blocks on the review and wakes Claude on exit code 2:

```json
{ "hooks": { "Stop": [ { "matcher": "", "hooks": [
  { "type": "command", "command": "eink-wait-feedback", "args": ["<id>"], "async": true, "asyncRewake": true, "timeout": 1800 }
] } ] } }
```

The hook's stderr becomes a **system reminder** Claude can act on. Simple, but the
wake is tied to a hook event and each run must be re-armed by a Stop/other event.

### C. `FileChanged` hook (file-drop trigger)
Claude Code watches named files and fires `FileChanged`. An external process can drop
`feedback.json`; the hook reads it and injects. No MCP, no WebSocket from the agent
side — a good lowest-common-denominator if a client blocks sockets.

### D. HTTP hooks
Claude Code POSTs event JSON to a URL. Useful for outbound telemetry, not for inbound
push (the direction is Claude → server).

### E. Channels (shipped)
`notifications/claude/channel` already gives true push; keep it where the dev flag is
acceptable.

### F. Headless injection
`claude -p --resume <session-id> "..."` starts a new turn in an existing session from
outside. Works for automation, but it is a heavy, separate process and can race the
live session — treat as a last resort.

## Recommendation

Two layers:

1. **Portable floor (shipped):** background watcher with ready-file handshake, plus
   the Claude channel when available. Works in any client, degrades safely.
2. **Native push per client:** a webview-free plugin that binds the review with a tool
   and injects on arrival — **OpenCode plugin** (`ctx.session.synthetic`) and **Claude
   Code mod** (`prompt.submit` context or a tool + timer). This removes re-arming and
   the timeout gap, which is the current weak point.

Both native integrations consume the **same** server contract already implemented
(`/feedback`, `/ws`, `feedback_id` + `expected_version` + `consumed_annotations`), so
the server needs no change to support them.

## Substrate that makes this possible

- Durable pending batch + `feedback_id` (already shipped) — so a late or reconnecting
  consumer never loses feedback, and exactly-once acknowledgement is provable.
- A WebSocket stream per session (already shipped).
- An idempotent bind step (a tool call) so the agent session ↔ review id mapping is
  explicit and survives the session.

## Open questions to verify before implementing

- Does `ctx.session.synthetic` surface as a user-visible turn or only model context?
  Test in a live OpenCode session.
- Claude mod distribution: how a locally-built mod is installed and loaded, and
  whether hooks run without a marketplace entry.
- Whether either runtime allows a long-lived socket from the plugin host process
  (vs. spawning a child) and how that interacts with plugin unload.
- Which injection is cheapest on tokens: `synthetic` (context only) vs `prompt` (full
  turn) — prefer `synthetic` for a nudge, `prompt` when the agent must act.

## Status

Researched from primary docs: OpenCode V2 plugins (`/build/plugins`), Claude Code
hooks reference, and Claude Code mods events. The portable floor is implemented and
tested; the native per-client integrations are designed here but not yet built.
