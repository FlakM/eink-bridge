use eink_bridge::render::to_eink_html;

const OUT_DIR: &str = "/tmp/eink-diagrams";

fn write_diagram(name: &str, md: &str) {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let assets = manifest.join("assets/diagram");
    let mermaid = String::from_utf8(std::fs::read(assets.join("mermaid.min.js")).unwrap()).unwrap();
    let elk = String::from_utf8(std::fs::read(assets.join("elk.bundled.js")).unwrap()).unwrap();

    let html = to_eink_html(md, name);
    let html = html
        .replace(
            "<script src=\"/assets/diagram/mermaid.min.js\"></script>",
            &format!("<script>{mermaid}</script>"),
        )
        .replace(
            "<script src=\"/assets/diagram/elk.bundled.js\"></script>",
            &format!("<script>{elk}</script>"),
        );
    std::fs::write(format!("{OUT_DIR}/{name}.html"), html).unwrap();
}

#[test]
fn render_all_diagrams() {
    std::fs::create_dir_all(OUT_DIR).unwrap();

    write_diagram("graph", GRAPH_MD);
    write_diagram("mindmap", MINDMAP_MD);
    write_diagram("mermaid", MERMAID_MD);
    write_diagram("all", ALL_MD);

    eprintln!("\n  open {OUT_DIR}/all.html in Firefox\n");
}

const GRAPH_MD: &str = r#"# Graph Test

```graph
layout:
  algorithm: layered
  direction: RIGHT
nodes:
  - id: android
    label: Android App
    kind: client
    color: red
  - id: server
    label: Axum Server
    kind: backend
    color: blue
  - id: tablet
    label: Boox Tablet
    kind: client
    color: red
  - id: render
    label: render.rs
    kind: tool
    color: slate
  - id: ws
    label: WebSocket
    kind: service
    color: amber
edges:
  - from: android
    to: server
    label: annotations + color
    kind: submits
  - from: server
    to: render
    label: HTML + JS
  - from: render
    to: tablet
    label: colored diagrams
  - from: server
    to: ws
    label: AnnotationResult
    kind: invokes
  - from: ws
    to: android
    label: OCR + color
    kind: reads
```
"#;

const MINDMAP_MD: &str = r#"# Mindmap Test

```mindmap
root: Kaleido 3
index: true
nodes:
  - id: root
    label: Kaleido 3 Improvements
    color: purple
    children:
      - id: diagrams
        label: Colored Borders
        color: green
        kind: done
        children:
          - label: darkenColor helper
            color: green
          - label: Stroke borders on nodes
            color: green
      - id: color-wire
        label: Color Pipeline
        color: green
        kind: done
      - id: epd
        label: EPD Refresh
        color: amber
        kind: wip
        children:
          - label: GC on load
            color: green
          - label: DU on pen-down
            color: blue
          - label: Device testing needed
            color: red
```
"#;

const ALL_MD: &str = r#"# All Diagrams

## Graph

```graph
layout:
  algorithm: layered
  direction: RIGHT
nodes:
  - id: android
    label: Android App
    kind: client
    color: red
  - id: server
    label: Axum Server
    kind: backend
    color: blue
  - id: tablet
    label: Boox Tablet
    kind: client
    color: red
  - id: render
    label: render.rs
    kind: tool
    color: slate
  - id: ws
    label: WebSocket
    kind: service
    color: amber
edges:
  - from: android
    to: server
    label: annotations + color
    kind: submits
  - from: server
    to: render
    label: HTML + JS
  - from: render
    to: tablet
    label: colored diagrams
  - from: server
    to: ws
    label: AnnotationResult
    kind: invokes
  - from: ws
    to: android
    label: OCR + color
    kind: reads
```

## Mindmap

```mindmap
root: Kaleido 3
index: true
nodes:
  - id: root
    label: Kaleido 3 Improvements
    color: purple
    children:
      - id: diagrams
        label: Colored Borders
        color: green
        kind: done
        children:
          - label: darkenColor helper
            color: green
          - label: Stroke borders
            color: green
      - id: epd
        label: EPD Refresh
        color: amber
        kind: wip
        children:
          - label: GC on load
            color: green
          - label: DU pen-down
            color: blue
          - label: Device testing
            color: red
```

## Mermaid

```mermaid
flowchart LR
    A[Push markdown] --> B{Server}
    B --> C[Render HTML]
    B --> D[OCR Engine]
    C --> E[Boox Tablet]
    D --> E
    E --> F[Annotations]
    F --> B
```
"#;

const MERMAID_MD: &str = r#"# Mermaid Test

```mermaid
flowchart LR
    A[Push markdown] --> B{Server}
    B --> C[Render HTML]
    B --> D[OCR Engine]
    C --> E[Boox Tablet]
    D --> E
    E --> F[Annotations]
    F --> B
```

## Sequence

```mermaid
sequenceDiagram
    participant CLI as eink-review
    participant S as Server
    participant T as Tablet
    CLI->>S: POST /sessions (markdown)
    S-->>CLI: session_id
    S->>T: WebSocket push
    T->>T: User annotates
    T->>S: POST /request_update
    S-->>T: AnnotationResult (OCR)
    T->>S: POST /submit
    S-->>CLI: GET /result (verdict)
```
"#;
