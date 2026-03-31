# PRD: Anchored Annotations

## Problem

Today, pen annotations are submitted as a flat PNG overlay and optional raw stroke JSON. Neither carries information about *which part of the document* the user annotated. Claude sees a screenshot with scribbles but has no way to associate specific marks with specific headings, code blocks, or paragraphs.

This makes the review loop lossy. The user circles a code block and writes "extract this" next to it, but Claude only sees pixels — it has to visually correlate the annotation position with the rendered HTML, which is unreliable and fails completely when the document is long or has been scrolled.

## Goal

Every annotation submitted from the tablet should carry a structured anchor linking it to the HTML element(s) it refers to. Claude receives annotations as:

```
"The user circled the code block under '## Data Flow' and wrote: extract this to a helper"
```

instead of:

```
"Here is an annotation PNG" [opaque image]
```

## Concepts

### Element map

A spatial index of every content element in the rendered HTML, built at page load time by JavaScript running inside the WebView. Each entry records the element's bounding box in document coordinates, its tag name, section ID (from the TOC), and a text preview.

### Stroke group

A set of strokes that belong together. The user draws multiple strokes near the same element — a circle around a code block plus an arrow plus some handwriting. These should be treated as a single annotation targeting that code block.

### Lasso anchor

A special pen gesture where the user draws a closed shape (circle, rectangle, or any loop) around one or more HTML elements. The system detects the loop, identifies which elements are enclosed, and creates an explicit anchor binding all nearby strokes to those elements. This is the primary anchoring mechanism — it's intentional and unambiguous.

### Proximity anchor (fallback)

When the user draws near an element without lassoing it, the system infers the target by proximity. Less precise than lasso but covers the common case of writing a note in the margin next to a paragraph.

## User experience

### Drawing mode (current behavior, unchanged)

The user draws freely with the pen. Strokes appear in real-time via the Onyx SDK hardware renderer. The toolbar has pencil, brush, eraser, undo, clear, and submit buttons.

### Lasso gesture

1. The user draws a roughly closed shape around a piece of the document (a heading, a code block, a table row, a diagram).
2. The system detects that the stroke forms a closed loop (first and last points within a threshold distance, and the stroke has enough area to not be a dot).
3. Visual feedback: the enclosed region gets a subtle highlight (thin dashed border or light tint). The lassoed elements appear briefly listed at the top of the screen ("Anchored to: ## Data Flow, code block (rust)").
4. Subsequent strokes drawn near the lasso region are automatically grouped with it until the user starts drawing in a different area of the document.

### No mode switch required

The lasso is detected automatically from stroke shape — no button or mode toggle. Drawing a circle around something is the natural pen gesture for "I'm talking about this thing." Everything else (arrows, underlines, margin notes) is a regular stroke.

### Submit

On submit, each stroke group is tagged with its anchor. The payload sent to the server includes:

```json
{
  "typed_notes": "",
  "annotations": [
    {
      "anchor": {
        "type": "lasso",
        "elements": [
          {"section_id": "section-3", "tag": "PRE", "text": "fn render(markdown: &str) -> String {"}
        ],
        "lasso_bbox": {"x": 120, "y": 1450, "w": 600, "h": 200}
      },
      "strokes": [[[120,1450],[125,1455],...], ...],
      "recognized_text": "extract this to a helper"
    },
    {
      "anchor": {
        "type": "proximity",
        "elements": [
          {"section_id": "section-1", "tag": "H2", "text": "Components"}
        ]
      },
      "strokes": [...],
      "recognized_text": "add a cache column"
    }
  ],
  "unanchored_strokes": [...],
  "annotation_png": "<base64 or multipart part>"
}
```

## Lasso detection algorithm

### Is this stroke a lasso?

Given a completed stroke (list of points), classify it as a lasso if ALL of:

1. **Closed loop**: distance between first and last point < `CLOSE_THRESHOLD` (40px).
2. **Minimum size**: bounding box area > `MIN_LASSO_AREA` (2500 px^2, roughly 50x50).
3. **Not a scribble**: the stroke's bounding box width and height are both > 30px (not a dot or tap).
4. **Reasonable point count**: the stroke has > 15 points (not a quick flick).

```kotlin
fun isLasso(stroke: List<Pair<Float, Float>>): Boolean {
    if (stroke.size < 15) return false
    val (x0, y0) = stroke.first()
    val (xn, yn) = stroke.last()
    val closeDist = sqrt((xn - x0).pow(2) + (yn - y0).pow(2))
    if (closeDist > CLOSE_THRESHOLD) return false
    val minX = stroke.minOf { it.first }
    val maxX = stroke.maxOf { it.first }
    val minY = stroke.minOf { it.second }
    val maxY = stroke.maxOf { it.second }
    val w = maxX - minX
    val h = maxY - minY
    return w > 30f && h > 30f && w * h > MIN_LASSO_AREA
}
```

### Which elements are inside the lasso?

Convert the lasso to document coordinates (add scroll, divide by scale). Then check each element in the element map:

An element is "inside" if the lasso polygon contains the element's center point, OR if the lasso bounding box overlaps > 50% of the element's bounding box.

The polygon containment test uses ray casting (point-in-polygon). For the overlap test, use simple rectangle intersection area / element area > 0.5.

### Stroke grouping

After a lasso is detected:

1. Compute the lasso's bounding box expanded by `GROUP_MARGIN` (80px).
2. All subsequent strokes whose centroid falls within this expanded box are grouped with the lasso.
3. Grouping ends when a stroke's centroid is > `GROUP_BREAK_DISTANCE` (200px) from the expanded box, or when a new lasso is drawn.

Ungrouped strokes that are near an element (centroid within 60px of an element's bounding box) get a proximity anchor. Strokes far from any element go into `unanchored_strokes`.

## Element map

### Built at page load (JavaScript in render.rs)

```javascript
window.__einkElementMap = [];
var content = document.getElementById('content');
var elements = content.querySelectorAll(
    'h1, h2, h3, p, pre, blockquote, table, ul, ol, .diagram-block, img'
);
elements.forEach(function(el, i) {
    var r = el.getBoundingClientRect();
    var sy = window.pageYOffset;
    var sx = window.pageXOffset;
    window.__einkElementMap.push({
        i: i,
        tag: el.tagName,
        id: el.id || null,
        t: r.top + sy,
        b: r.bottom + sy,
        l: r.left + sx,
        r: r.right + sx,
        text: el.textContent.substring(0, 150).replace(/\s+/g, ' ').trim()
    });
});
```

### Queried from Kotlin

```kotlin
fun queryElementMap(callback: (List<ElementEntry>) -> Unit) {
    webView.evaluateJavascript(
        "JSON.stringify(window.__einkElementMap || [])"
    ) { json ->
        callback(parseElementEntries(json))
    }
}
```

The map is queried once before submit (not on every stroke) to keep things simple. The map is rebuilt on page resize or navigation.

## Data model changes

### Android (`StrokeBuffer.kt`)

```kotlin
data class StrokeGroup(
    val anchor: Anchor?,
    val strokes: List<List<Pair<Float, Float>>>,
)

sealed class Anchor {
    data class Lasso(
        val elements: List<ElementRef>,
        val lassoBbox: Rect,
        val lassoStroke: List<Pair<Float, Float>>,
    ) : Anchor()

    data class Proximity(
        val elements: List<ElementRef>,
    ) : Anchor()
}

data class ElementRef(
    val sectionId: String?,
    val tag: String,
    val text: String,
)
```

### Server (`api.rs`)

Add to `SubmitReviewRequest` and `SessionResultResponse`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationGroup {
    pub anchor: Option<AnnotationAnchor>,
    pub strokes: Vec<Vec<[f64; 2]>>,
    #[serde(default)]
    pub recognized_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnnotationAnchor {
    #[serde(rename = "lasso")]
    Lasso {
        elements: Vec<ElementRef>,
        lasso_bbox: BBox,
    },
    #[serde(rename = "proximity")]
    Proximity {
        elements: Vec<ElementRef>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementRef {
    pub section_id: Option<String>,
    pub tag: String,
    pub text: String,
}
```

## How Claude consumes anchored annotations

The `/eink` skill and the CLI `eink-review result` output change from:

```
## Attached Images
/path/to/annotation.png
```

to:

```
## Annotations

### On "## Data Flow" (code block)
> extract this to a helper
[annotation image cropped to lasso region]

### On "## Components" (heading)
> add a cache column

### Unanchored
[full annotation PNG]
```

Claude sees structured, per-section feedback instead of a single opaque image. The `/eink` skill reads each cropped region with the Read tool, giving Claude both the recognized text and the visual context.

## Phasing

### Phase A: Element map + proximity anchoring

- Add element map JS to the rendered HTML
- Query map on submit
- Assign proximity anchors to strokes based on centroid-to-element distance
- Include anchors in submit payload
- Server stores and returns anchors in result
- CLI formats anchored output

No Android UI changes. No lasso detection. Just spatial matching.

### Phase B: Lasso detection

- Add `isLasso()` to StrokeBuffer
- Detect lasso on `end()`, provide visual feedback (toast or highlight)
- Group subsequent nearby strokes with the lasso
- Include lasso anchor in submit payload

### Phase C: HWR integration

- Recognize text within each stroke group separately (Phase 3 from the Onyx plan)
- Include `recognized_text` per annotation group
- CLI/skill formats: "On [section]: [recognized text]"

### Phase D: Cropped annotation images

- For each lasso anchor, crop the annotation PNG to the lasso bounding box
- Submit cropped images as separate parts
- Claude sees per-section crops instead of one big PNG

## Out of scope

- Multi-page documents (scroll-and-annotate is sufficient)
- Annotation editing after drawing (undo is enough)
- Collaborative annotations (single reviewer)
- Anchoring to individual words or characters (element-level is sufficient)
- Custom lasso shapes beyond closed loops (rectangle select, etc.)

## Success criteria

1. Annotations on a 5-section document produce 3+ anchored groups with correct section IDs.
2. A lasso drawn around a code block correctly identifies the `<pre>` element and its section heading.
3. Unanchored strokes (margin doodles) are preserved but separated from anchored feedback.
4. Claude, given the structured output, can respond to each annotation by section without needing to interpret the PNG spatially.
5. Round-trip latency (draw -> submit -> result) increases by < 200ms from the element map query + grouping.
