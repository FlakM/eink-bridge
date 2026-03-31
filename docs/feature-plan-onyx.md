# Onyx SDK Feature Plan

Three features to add, in dependency order. Each builds on the previous.

## Phase 1: EPD Refresh Optimization

**Goal:** Faster pen input and cleaner page transitions.

**Effort:** 1-2 hours. Add dependency + ~20 lines in PenOverlay.kt and MainActivity.kt.

### Changes

Add `onyxsdk-device` to `android/app/build.gradle.kts`:

```kotlin
implementation("com.onyx.android.sdk:onyxsdk-device:1.2.26")
```

Modify `PenOverlay.kt`:

1. In `initTouchHelper()`, after `helper.openRawDrawing()`:
   ```kotlin
   EpdController.setDisplayScheme(EpdController.SCHEME_SCRIBBLE)
   EpdController.enableA2ForSpecificView(webView)
   EpdController.setEpdTurbo(100)
   ```

2. In `destroy()`, before nulling touchHelper:
   ```kotlin
   EpdController.setDisplayScheme(EpdController.SCHEME_NORMAL)
   EpdController.applyGCOnce()
   ```

Modify `MainActivity.kt`:

3. In `setupWebView()`:
   ```kotlin
   EpdController.setWebViewContrastOptimize(webView, true)
   ```

4. Add a "Refresh" button to the pen toolbar that calls `EpdController.applyGCOnce()` to clear ghosting on demand.

### Testing

- Manual: open a session, draw rapidly. Compare latency before/after.
- Manual: scroll the document, verify no permanent ghosting after refresh.
- Unit test: none needed (hardware-dependent).

### Acceptance criteria

- Drawing latency is perceptibly faster (A2 mode skips grayscale rendering).
- Leaving drawing mode does a clean GC refresh (no ghost strokes).
- WebView code blocks have better contrast.

---

## Phase 2: Rich Stroke Data (Pressure + Timestamps)

**Goal:** Pressure-sensitive rendering and timestamp data for HWR in Phase 3.

**Effort:** 3-4 hours. Refactor StrokeBuffer, update PenOverlay callbacks, update PNG export.

### Changes

#### 2a. New stroke point data class

Create `StrokePoint.kt`:

```kotlin
data class StrokePoint(
    val x: Float,
    val y: Float,
    val pressure: Float = 0f,
    val timestamp: Long = 0L,
)
```

#### 2b. Refactor StrokeBuffer

Change `StrokeBuffer` from `MutableList<List<Pair<Float, Float>>>` to `MutableList<List<StrokePoint>>`.

Update all methods:

```kotlin
fun begin(x: Float, y: Float, pressure: Float, timestamp: Long)
fun addPoint(x: Float, y: Float, pressure: Float, timestamp: Long)
fun end(x: Float, y: Float, pressure: Float, timestamp: Long)
```

Keep the existing `val strokes: List<List<StrokePoint>>` API shape.

#### 2c. Update PenOverlay callbacks

In `RawInputCallback`, pass pressure and timestamp from `TouchPoint`:

```kotlin
override fun onBeginRawDrawing(b: Boolean, tp: TouchPoint) {
    buf.begin(tp.x, tp.y, tp.pressure, System.currentTimeMillis())
}
override fun onRawDrawingTouchPointMoveReceived(tp: TouchPoint) {
    buf.addPoint(tp.x, tp.y, tp.pressure, System.currentTimeMillis())
}
override fun onEndRawDrawing(b: Boolean, tp: TouchPoint) {
    buf.end(tp.x, tp.y, tp.pressure, System.currentTimeMillis())
}
```

#### 2d. Pressure-sensitive PNG export

In `exportToPng()`, vary stroke width based on pressure:

```kotlin
val maxPressure = EpdController.getMaxTouchPressure().toFloat().coerceAtLeast(1f)
for (stroke in buf.strokes) {
    if (stroke.size < 2) continue
    val path = Path()
    path.moveTo(stroke[0].x, stroke[0].y)
    for (i in 1 until stroke.size) {
        val p = stroke[i]
        val width = currentWidth * (0.5f + 0.5f * (p.pressure / maxPressure))
        paint.strokeWidth = width
        canvas.drawLine(stroke[i-1].x, stroke[i-1].y, p.x, p.y, paint)
    }
}
```

#### 2e. Implement eraser callbacks

In `PenOverlay.callback`, implement the eraser methods:

```kotlin
override fun onBeginRawErasing(b: Boolean, tp: TouchPoint) {
    eraserActive = true
}
override fun onRawErasingTouchPointMoveReceived(tp: TouchPoint) {
    // Find strokes that intersect a circle around (tp.x, tp.y, radius=20)
    // Mark them for removal
}
override fun onEndRawErasing(b: Boolean, tp: TouchPoint) {
    eraserActive = false
    buf.removeMarked()
    // Trigger re-render
}
```

Eraser can be simpler: just remove any stroke where any point is within 20px of the eraser path. No need for precise intersection.

### Testing

Update `StrokeBufferTest.kt`:
- All existing tests updated to use `StrokePoint` instead of `Pair<Float, Float>`
- New test: `pressureIsRecorded` -- verify pressure values survive begin/add/end
- New test: `timestampsAreRecorded` -- verify timestamps survive
- New test: `eraserRemovesIntersectingStrokes` (if eraser added to StrokeBuffer)

### Acceptance criteria

- Pressing harder produces visibly thicker strokes in the exported PNG.
- Timestamps are present on every point (needed by Phase 3).
- Existing tests pass with the new data model.
- Pen flip erases strokes (nice-to-have, can defer).

---

## Phase 3: Handwriting Recognition (Onyx HWR via AIDL)

**Goal:** Convert pen strokes to text automatically before submission. Claude sees both the PNG and the recognized text.

**Effort:** 6-8 hours. AIDL integration, protobuf encoding, UI changes.

**Depends on:** Phase 2 (needs timestamps and pressure in StrokeBuffer).

### Architecture

```
StrokeBuffer (x, y, pressure, timestamp)
      |
      v
OnyxHWREngine.recognizeStrokes()
      |
      +-- encode strokes as protobuf
      +-- bind to com.onyx.android.ksync/.service.KHwrService
      +-- call batchRecognize(pfd, callback)
      +-- parse JSON result
      |
      v
recognized text string
      |
      v
POST /api/sessions/{id}/submit
  typed_notes = recognized text
  annotation = PNG (existing)
```

### Changes

#### 3a. Add AIDL files

Create `android/app/src/main/aidl/com/onyx/android/sdk/hwr/service/`:

- `IHWRService.aidl`:
  ```aidl
  package com.onyx.android.sdk.hwr.service;
  import com.onyx.android.sdk.hwr.service.HWRInputArgs;
  import com.onyx.android.sdk.hwr.service.HWROutputCallback;
  import com.onyx.android.sdk.hwr.service.HWRCommandArgs;

  oneway interface IHWRService {
      void init(in HWRInputArgs args, boolean forceReinit, HWROutputCallback callback);
      void batchRecognize(in ParcelFileDescriptor pfd, HWROutputCallback callback);
      void execCommand(in HWRInputArgs args, in HWRCommandArgs cmdArgs, HWROutputCallback callback);
      void closeRecognizer();
  }
  ```

- `HWRInputArgs.aidl`, `HWROutputArgs.aidl`, `HWROutputCallback.aidl`, `HWRCommandArgs.aidl` (Parcelable declarations)

#### 3b. Add Parcelable implementations

Create Kotlin Parcelable classes for `HWRInputArgs`, `HWROutputArgs`, `HWRCommandArgs` matching the AIDL contracts.

Key fields in `HWRInputArgs`:
```kotlin
var lang: String = "en_US"
var contentType: String = "Text"
var recognizerType: String = "MS_ON_SCREEN"
var viewWidth: Int = 0
var viewHeight: Int = 0
var isTextEnable: Boolean = true
var isShapeEnable: Boolean = false
var isGestureEnable: Boolean = false
```

#### 3c. Create OnyxHWREngine

Port from [aragonite's OnyxHWREngine.kt](https://github.com/jdkruzr/aragonite). Core flow:

```kotlin
class OnyxHWREngine(private val context: Context) {
    private var service: IHWRService? = null

    fun bind(): Boolean {
        val intent = Intent().apply {
            component = ComponentName(
                "com.onyx.android.ksync",
                "com.onyx.android.ksync.service.KHwrService"
            )
        }
        return context.bindService(intent, connection, Context.BIND_AUTO_CREATE)
    }

    suspend fun recognize(
        strokes: List<List<StrokePoint>>,
        canvasWidth: Int,
        canvasHeight: Int,
        lang: String = "en_US"
    ): String? {
        // 1. Init recognizer with canvas dimensions
        // 2. Encode strokes to protobuf (hand-rolled, ~60 lines)
        // 3. Write protobuf to MemoryFile -> ParcelFileDescriptor
        // 4. Call batchRecognize(pfd, callback)
        // 5. Read result JSON from callback's PFD
        // 6. Parse result.label from JSON
        // 7. Return recognized text
    }
}
```

#### 3d. Protobuf encoding (hand-rolled, no protobuf dependency)

Encode each point as a protobuf message with fields:
- field 1 (float): x
- field 2 (float): y
- field 3 (sint64): timestamp in ms
- field 4 (float): pressure
- field 5 (sint32): pointer ID (always 0)
- field 6 (enum): event type (0=DOWN, 1=MOVE, 2=UP)
- field 7 (enum): pointer type (0=PEN)

Wrap the points list in a container message. Use `MemoryFile` to pass to the service (requires `hiddenapibypass` for `MemoryFile.getFileDescriptor()`).

Add dependency:
```kotlin
implementation("org.lsposed.hiddenapibypass:hiddenapibypass:4.3")
```

#### 3e. UI changes in MainActivity

Add a "Recognize" step before submit:

```kotlin
private fun submitAndGoBack() {
    val sessionId = currentSessionId ?: return
    scope.launch {
        val pngData = penOverlay?.exportToPng()
        val strokes = penOverlay?.getStrokes() ?: emptyList()

        // Recognize handwriting if strokes exist
        var recognizedText = ""
        if (strokes.isNotEmpty()) {
            try {
                recognizedText = hwrEngine.recognize(
                    strokes, webView.width, webView.height
                ) ?: ""
            } catch (e: Exception) {
                Log.w("HWR", "recognition failed, submitting without text", e)
            }
        }

        // Submit both text and PNG
        withContext(Dispatchers.IO) {
            val builder = MultipartBody.Builder()
                .setType(MultipartBody.FORM)
                .addFormDataPart("typed_notes", recognizedText)
            if (pngData != null) {
                builder.addFormDataPart("annotation", "strokes.png",
                    pngData.toRequestBody("image/png".toMediaType()))
            }
            // ... existing submit logic
        }
    }
}
```

#### 3f. Expose strokes from PenOverlay

Add to `PenOverlay`:
```kotlin
fun getStrokes(): List<List<StrokePoint>> = buf.strokes
```

### Testing

- `OnyxHWREngineTest.kt` (unit): test protobuf encoding produces valid bytes for known input points
- `StrokeBufferTest.kt`: verify `strokes` returns `StrokePoint` data with all fields
- Manual on device: write "hello world" with pen, verify recognized text appears in submission
- Manual: write messy cursive, verify graceful fallback (empty string, not crash)
- Integration: submit with recognized text, verify it arrives in `typed_notes` on server

### Acceptance criteria

- Writing legible text with the pen produces readable `typed_notes` on the server.
- Claude sees both the PNG annotation and the recognized text in the review result.
- Recognition failure is silent (falls back to empty typed_notes + PNG only).
- Works offline with no model download.

---

## Phase 4 (Future): Typed Notes Input

**Goal:** Allow mixing pen annotations with typed text on the tablet. A text input field below the document where the user can type quick notes alongside pen drawings.

This is simpler than HWR and doesn't depend on any SDK:

1. Add an `EditText` to the review layout (collapsible, below the pen toolbar).
2. On submit, concatenate the typed text with HWR-recognized text.
3. Send both in `typed_notes`.

Low effort, high value for users who prefer typing short notes ("LGTM", "move this function") rather than writing them by hand.

---

## Dependency graph

```
Phase 1 (EPD refresh)
    |
    | (independent, do first for immediate win)
    v
Phase 2 (rich strokes)
    |
    | (StrokePoint with timestamps needed by Phase 3)
    v
Phase 3 (HWR recognition)
    |
    v
Phase 4 (typed input, independent)
```

## Files touched per phase

| Phase | New files | Modified files |
|-------|-----------|----------------|
| 1 | none | `build.gradle.kts`, `PenOverlay.kt`, `MainActivity.kt`, `activity_main.xml` |
| 2 | `StrokePoint.kt` | `StrokeBuffer.kt`, `PenOverlay.kt`, `StrokeBufferTest.kt` |
| 3 | `OnyxHWREngine.kt`, 5 AIDL files, 3 Parcelable classes, `OnyxHWREngineTest.kt` | `MainActivity.kt`, `PenOverlay.kt`, `build.gradle.kts` |
| 4 | none | `activity_main.xml`, `MainActivity.kt` |

## References

- [OnyxAndroidDemo](https://github.com/onyx-intl/OnyxAndroidDemo) -- official SDK examples
- [Aragonite HWR engine](https://github.com/jdkruzr/aragonite) -- reverse-engineered Onyx HWR service
- [ML Kit Digital Ink](https://developers.google.com/ml-kit/vision/digital-ink-recognition/android) -- fallback if Onyx HWR unavailable
- [Onyx Pen SDK docs](https://github.com/onyx-intl/OnyxAndroidDemo/blob/master/doc/Onyx-Pen-SDK.md)
- [onyxsdk-device on Maven](https://mvnrepository.com/artifact/com.onyx.android.sdk/onyxsdk-device)
