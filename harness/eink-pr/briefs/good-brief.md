# PR 7: Quiet notifications

**Ready?** With fixes — the leak is real, the version guard is not doing anything.

3 files · +34 −9 · 3 min · 40 lines to judge, 10 mechanical

**What changed** — Expiry now also clears the notifier and WebSocket maps.

**Why** — Notifier entries leaked for every expired session, so memory grew with uptime.

**Wider system** — The expiry loop also feeds metrics and the session purge path.

**The question** — Should the socket map clear on the same timer, or at handshake close?

## Read these first

- **A** `issue (blocking)` Expiry skips notifier cleanup on the cancelled path. Clear the map for each expired id. `server/src/app.rs:170`
- **B** `suggestion` The version guard trusts the client. Verify the version before accept. `server/src/session.rs:118`

## What I checked

- [x] All green: cargo test, clippy.
- [x] Every caller of `expire()` updated (4, all in `session.rs`).
- [ ] Not checked: the reconnect path in the tablet app.

## Set aside

- The rename in `notifier.rs` — mechanical, no behaviour.
- Metrics still count cancelled sessions — pre-existing, out of scope here.

Write on the page, or reply **LGTM** / **CHANGES**.
