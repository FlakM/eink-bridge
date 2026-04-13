# Late Night Hacking: Kaleido 3 Color Improvements

Session: 2026-04-13

## What was done

### Server (render.rs, api.rs)

**Diagram colors instead of hatching:**
- Replaced `einkStrokePattern()` (SVG stroke-dasharray patterns) with `darkenColor(hex, 0.65)` helper
- Mindmap nodes: colored fill + 65% darkened stroke border
- Mindmap edges: colored to match child node
- Graph nodes: same colored fill + darkened border
- Kept `edgePattern()` for semantic edge types (invokes, depends, etc.)
- Legend swatches: solid colored borders instead of hatched patterns

**Node kind badges (graph + mindmap):**
- Removed dark background pills that overflowed node borders
- Replaced with plain italic text at bottom of node (class `graph-node-kind` / `mindmap-node-kind`)
- CSS: `fill: #888; font-size: 9px; font-style: italic`

**Edge labels:**
- Increased from 13px to 14px, added italic style, darker fill (#444)

**Anchor highlights:**
- Changed stroke-link styling from `outline: 4px dashed` to `borderLeft: 6px solid`
- Background alpha 0.05 -> 0.08
- Badge border changed from dashed to solid

**Annotation color pipeline:**
- Added `color: Option<String>` to `AnnotationGroup` in api.rs
- OpenAPI spec updated
- Contract golden regenerated

### Android

**Color picker:**
- Replaced inline color strip with drawer approach: single color indicator button in toolbar, tapping toggles a floating row of 8 color dots above the toolbar
- Auto-dismisses on color selection, mode switch, or session exit

**Annotation color serialization:**
- `StrokeGroup` gets `color: String?` field
- `bindGroupsToAnnotations()` converts `BindGroup.color` (Int) to hex string
- `annotationsToJson()` serializes the color field

**OCR result boxes:**
- Group OCR pills get white undercoat + 12% alpha tint of bind group color
- Unbound clusters stay white/gray

**EPD refresh modes:**
- `EpdController.setViewDefaultUpdateMode()` for GC/DU/REGAL
- Pen-down: DU (fast draw), pen-up: GC (full color)
- Scroll: REGAL with 800ms debounced restore to GC
- All wrapped in try/catch for non-Onyx device safety

**Submission overlay:**
- Added cancel button (X) to processing overlay
- Both `requestUpdate` and `submitAndGoBack` wrapped in `withTimeout(30_000)`
- Track coroutine as `processingJob` for cancel support
- Shows overlay with status text for both operations

## Visual test harness

`server/tests/diagram_visual_test.rs` dumps graph, mindmap, and mermaid diagrams
to `/tmp/eink-diagrams/` with inlined JS assets. Open in browser to iterate on
diagram rendering without deploying.

```bash
cargo test --test diagram_visual_test -- --nocapture
# then open /tmp/eink-diagrams/graph.html in Firefox/Chrome
```

## Files changed

- `server/src/render.rs` - diagram colors, badges, edge labels, anchor highlights
- `server/src/api.rs` - AnnotationGroup.color + OpenAPI
- `server/tests/golden/contracts/openapi.json` - regenerated
- `server/tests/diagram_visual_test.rs` - visual test harness (new)
- `android/.../MainActivity.kt` - color drawer, EPD modes, cancel button, timeout
- `android/.../PenOverlay.kt` - EPD fast-draw/full-color switching
- `android/.../StrokeBuffer.kt` - color field on StrokeGroup
- `android/.../StrokeView.kt` - tinted OCR boxes
- `android/.../activity_main.xml` - color drawer layout, cancel button
