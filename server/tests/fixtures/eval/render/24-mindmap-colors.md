# Colored Mindmap
```mindmap
root: Deploy Plan
index: true
nodes:
  - id: prep
    label: Preparation
    color: blue
    kind: todo
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
    notes: Database migration could fail
```
