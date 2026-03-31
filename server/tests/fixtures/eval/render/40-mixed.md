# Architecture Review

The system uses a layered approach with three main components.

## Components

| Component | Language | Role |
|-----------|----------|------|
| Server    | Rust     | HTTP API and rendering |
| CLI       | Rust     | Push/pull interface |
| App       | Kotlin   | Tablet UI |

## Data Flow

```mermaid
sequenceDiagram
  participant C as CLI
  participant S as Server
  participant A as App
  C->>S: POST /api/sessions
  A->>S: GET /session/{id}
  A->>S: POST /submit
  S-->>C: 200 result
```

## Key Code

```rust
pub fn render(markdown: &str) -> String {
    to_eink_html(markdown, "preview")
}
```

> Note: The renderer handles mermaid, mindmap, and graph blocks natively.
