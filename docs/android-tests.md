# Android Test Suite

JVM unit tests under `android/app/src/test`, run with `just test-android`
(`./gradlew testDebugUnitTest` in `android/`). Robolectric stands in for the device;
no emulator or tablet is needed. Total: **280 tests**, 0 failures, 3 skipped (the E2E
suite, which needs a live server and `ANTHROPIC_API_KEY`).

| Class | Tests | What it holds |
|-------|------:|---------------|
| `StrokeBufferTest` | 65 | The pure stroke accumulator: begin/add/end, undo, clear, erase by radius, per-stroke width, proximity grouping. |
| `PenOverlayTest` | 72 | The overlay around the Onyx controller: toolbar exclusion, undo/clear syncing the StrokeView and the hardware layer, scroll ghost lines, style and width, export, re-arming the pen after a resize or page load. |
| `MainActivityTest` | 25 | Robolectric: which screen shows on launch, tool-mode transitions, `drawControls` and `contextBar` visibility, the clear confirmation, the quiet notification channel. |
| `PenToolbarLayoutTest` | 3 | The toolbar laid out at the tablet's 992dp portrait width fits with slack in DRAW and in MOVE mode with the context bar, DONE is on screen, and the colour drawer opens over its button. |
| `RenderGoldenTest` | 18 | Golden PNGs for `renderStrokesToPng`; regenerate with `just golden-android`. |
| `AnnotationRelocationTest` | 16 | The client half of relocation: ink follows its paragraph's top-left, a bound note whose paragraph is gone is archived, and the archived count is truthful. |
| `TouchRoutingTest` | 15 | `rawDrawingAction`: a finger disables raw drawing, a stylus leaves it alone. |
| `BindGestureHandlerTest` | 13 | The lasso state machine from touch to polygon to elements: tap, cancel, endpoint fallback, which strokes a lasso captures. |
| `SessionAdapterTest` | 13 | Status icon and relative time text for a list row (`StatusIconTest`, `FormatSessionTimeTest`). |
| `SubmissionManagerTest` | 10 | Submit and update over HTTP: which codes count as success or conflict, and that one connection is reused. |
| `OcrManagerTest` | 8 | OCR scheduling: only groups without text are recognised, pending counts, debounce and cancel. |
| `MetricsReporterTest` | 5 | The pushgateway body: label escaping, since one malformed line rejects the whole batch. |
| `SessionPagingTest` | 5 | Page URL and cursor escaping, a bounded walk, and what a partial or empty fetch may replace. |
| `SessionPollingTest` | 3 | List polling backs off 5 s to 60 s while the server is unreachable and resets on the first answer. |
| `ViewportTransformTest` | 4 | The viewport-to-view mapping in both orientations, with numbers measured on the tablet. |
| `AnnotationE2ETest` | 3 | Starts the Rust server, pushes a session, submits a PNG annotation, asks Claude to describe it. Skipped without the environment. |
| `OcrClientTest` | 2 | The OCR HTTP client's timeouts outlast the server's model timeout and are bounded. |

## Related recipes

```bash
just test-android      # the whole suite
just coverage-android  # JaCoCo report
just golden-android    # regenerate RenderGoldenTest PNGs after an intentional change
just test-android-e2e  # the E2E suite against a built eink-serve
```
