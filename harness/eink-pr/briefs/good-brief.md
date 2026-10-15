# PR 7: Quiet notifications

## Important pieces

- **A** — Expiry skips notifier cleanup. Clear the map for each expired id.
- **B** — The version guard trusts the client. Verify the version before accept.

## Marks

- A → server/src/app.rs:170
- B → server/src/session.rs:118
