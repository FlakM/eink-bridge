# PR 7: Quiet notifications

**What changed** — Expiry now also clears the notifier and WebSocket maps.
**Why** — Notifier entries leaked for every expired session, so memory grew with uptime.
**Wider system** — The expiry loop also feeds metrics and the session purge path.

## Checks

- All green: cargo test, clippy.

## Important pieces

- **A** — Expiry skips notifier cleanup. Clear the map for each expired id.
- **B** — The version guard trusts the client. Verify the version before accept.

## Marks

- A → server/src/app.rs:170
- B → server/src/session.rs:118
