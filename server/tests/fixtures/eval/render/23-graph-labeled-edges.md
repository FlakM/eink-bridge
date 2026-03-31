# Labeled Graph
```graph
layout:
  algorithm: layered
  direction: RIGHT
nodes:
  - id: api
    label: API Server
    kind: backend
  - id: db
    label: Postgres
    kind: service
  - id: cache
    label: Redis
    kind: service
edges:
  - from: api
    to: db
    label: writes
    kind: writes
  - from: api
    to: cache
    label: reads
    kind: reads
```
