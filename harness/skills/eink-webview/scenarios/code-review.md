# Review: switch session storage to Postgres

**PR**: `feature/postgres-sessions`
**Author**: claude
**Files**: 12 changed, +480 / −210

## Why

We're outgrowing the JSON-on-disk session store. Three pain points:

1. **Concurrency**: `SessionManager::persist()` holds the write lock during `fs::write`, serialising every mutation.
2. **Crash safety**: `state.json` can be torn on power loss; we've already lost two sessions this month.
3. **Discovery**: there's no way to query "all sessions tagged X" without scanning every file.

Postgres solves all three with a single dependency we already run for the metrics dashboard.

## Architecture

```graph
layout:
  algorithm: layered
  direction: RIGHT
nodes:
  - id: cli
    label: eink-review CLI
    kind: client
  - id: srv
    label: Axum server
    kind: backend
  - id: pool
    label: deadpool-postgres
    kind: tool
  - id: pg
    label: Postgres 16
    kind: service
  - id: cache
    label: in-memory LRU
    kind: tool
edges:
  - from: cli
    to: srv
    label: HTTP/1.1
    kind: invokes
  - from: srv
    to: pool
    label: acquire
  - from: pool
    to: pg
    label: SQL
  - from: srv
    to: cache
    label: read-through
    kind: reads
```

## Schema

```sql
CREATE TABLE sessions (
    id          TEXT PRIMARY KEY,
    title       TEXT,
    status      TEXT NOT NULL,
    starred     BOOLEAN NOT NULL DEFAULT FALSE,
    tags        JSONB NOT NULL DEFAULT '{}',
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_sessions_status ON sessions (status);
CREATE INDEX idx_sessions_tags ON sessions USING GIN (tags);
```

## The hot path

The `submit` handler is the most-called write path. New implementation:

```rust
#[derive(Clone)]
pub struct AppState {
    pub pool: PgPool,
    pub cache: Arc<LruCache<String, Session>>,
    pub notifiers: Arc<DashMap<String, Arc<Notify>>>,
}

pub async fn submit(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SubmitRequest>,
) -> Result<StatusCode, AppError> {
    let mut tx = state.pool.begin().await?;
    let session = sqlx::query_as::<_, Session>(
        "SELECT * FROM sessions WHERE id = $1 FOR UPDATE",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|_| AppError::NotFound)?;

    session.transition_to_submitted(req.verdict)?;

    sqlx::query(
        "UPDATE sessions SET status = 'Submitted', updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(&id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    state.cache.invalidate(&id);
    state.notifiers.get(&id).map(|n| n.notify_waiters());
    Ok(StatusCode::OK)
}
```

Relative to the current code, the diff is:

```diff
--- a/server/src/app.rs
+++ b/server/src/app.rs
@@ -120,15 +120,12 @@ pub async fn submit(
     State(state): State<AppState>,
     Path(id): Path<String>,
     Json(req): Json<SubmitRequest>,
 ) -> Result<StatusCode, AppError> {
-    let mut sessions = state.sessions.write().await;
-    let session = sessions.get_mut(&id).ok_or(AppError::NotFound)?;
-    session.transition_to_submitted(req.verdict)?;
-    state.persist(&sessions).await?;
+    let mut tx = state.pool.begin().await?;
+    let session = sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = $1 FOR UPDATE")
+        .bind(&id).fetch_one(&mut *tx).await?;
+    session.transition_to_submitted(req.verdict)?;
+    sqlx::query("UPDATE sessions SET status='Submitted', updated_at=NOW() WHERE id=$1")
+        .bind(&id).execute(&mut *tx).await?;
+    tx.commit().await?;
     Ok(StatusCode::OK)
 }
```

Notice the row-level lock (`FOR UPDATE`) replaces the global `RwLock` — concurrent submits to different sessions no longer serialise on each other.

## Migration

```python
import psycopg
import json
from pathlib import Path

def migrate(state_dir: Path, dsn: str):
    with psycopg.connect(dsn) as conn, conn.cursor() as cur:
        for f in state_dir.glob("*.json"):
            doc = json.loads(f.read_text())
            cur.execute(
                "INSERT INTO sessions (id, title, status, starred, tags, "
                "created_at, updated_at) VALUES (%s, %s, %s, %s, %s, %s, %s)",
                (doc["id"], doc.get("title"), doc["status"],
                 doc.get("starred", False), json.dumps(doc.get("tags", {})),
                 doc["created_at"], doc["updated_at"]),
            )
        conn.commit()
```

Run as `migrate(Path("/var/lib/eink-bridge"), os.environ["DATABASE_URL"])`. Roughly 30 seconds for the 1,800 sessions on prod.

## Trade-offs

| Concern | JSON files | Postgres |
|---|---|---|
| Setup cost | None | One systemd unit, one schema migration |
| Concurrency | Global write lock | Row-level locks |
| Crash safety | Vulnerable to torn writes | WAL + fsync |
| Backups | `tar -czf` | `pg_dump` |
| Memory footprint | Loaded entirely on start | LRU cache + connection pool |
| Failure modes | Corrupt JSON file | Network blip, schema drift |

## Open questions

> Is the LRU cache actually worth it, or are we just adding an invalidation
> bug we'll spend weeks debugging? The original `RwLock<HashMap>` was already
> a cache by accident.

```mindmap
root: Decisions
nodes:
  - id: cache
    label: LRU cache
    color: amber
    kind: open
    notes: |
      Two camps:

      - **For**: avoids round-trip per `GET /session/{id}`, which is hit
        on every WebView reload.
      - **Against**: invalidation across multiple processes is hard.
        Risk of stale reads after `submit`.

      Decision needed by **Friday** so the migration can land before
      next week's freeze.
  - id: pool
    label: deadpool vs sqlx pool
    color: blue
    kind: closed
    notes: |
      Going with **deadpool-postgres**. Reasons:

      1. Same crate we use for the dashboard.
      2. Better health-check semantics on idle.
      3. Connection pinning works around the
         `pg_stat_statements` quirk we hit last quarter.
  - id: schema
    label: JSONB tags or join table?
    color: green
    kind: closed
```

## Risk

```mindmap
root: Rollout risk
nodes:
  - id: down
    label: Postgres unreachable
    color: red
    kind: blocker
    notes: |
      `eink-serve` currently fails fast on startup. Need a backoff
      retry loop OR a feature flag to fall back to the old JSON store.
  - id: slow
    label: Schema migration window
    color: amber
    notes: |
      30s offline window. Acceptable if announced. Otherwise need to
      use logical replication or a dual-write phase.
  - id: ok
    label: Index hits in pg_stat
    color: green
```

## Next steps

- [ ] Land schema migration in `migrations/0042_sessions.sql`
- [ ] Wire deadpool config in `config.toml`
- [ ] Backfill existing sessions
- [ ] Update CLAUDE.md "Known weak points" section
- [ ] Cut `v0.4.0` once `just ci` is green
