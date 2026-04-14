# Colored Mindmap
```mindmap
root: Deploy Plan
index: true
nodes:
  - id: prep
    label: Preparation
    color: blue
    kind: todo
    notes: |
      Run the **full** test suite before tagging.

      Checklist:
      - `just test`
      - `just lint`
      - Verify staging smoke test
    children:
      - label: Run tests
      - label: Update changelog
  - id: deploy
    label: Deploy
    color: green
    kind: module
    collapsed: true
    children:
      - label: Build image
      - label: Push to registry
  - id: risk
    label: Risk
    color: red
    kind: risk
    notes: |
      Database migration could fail.

      **Rollback plan:**
      1. Revert the service to `previous-stable`.
      2. Run `just db-rollback` on the primary.
      3. Page the on-call via `#eink-oncall`.

      See the runbook for `migrations/0042` for details.
```
