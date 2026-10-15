# PR 7: Notifications

## Summary

This basically changes how notifications work. Simply read the important pieces.

## Important pieces

- **A** — The notifier map is never cleared on expiry. That is a behaviour change. It could possibly leak memory in long running processes. The WebSocket sender map is also never touched. Confirm both maps are cleared when a session expires so nothing accumulates over weeks of uptime.
- **B** — There are changes in the session handler behaviour.

```diff
- old behaviour
+ new behaviour
```

@@ -1,3 +1,4 @@
