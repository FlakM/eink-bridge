# Android Refactoring — Implementation Plan

This document is a step-by-step guide for an implementing agent.
Each phase is self-contained and mergeable independently.
Read the relevant source files before touching anything.

---

## Prerequisites

- All source under `android/app/src/main/java/com/flakm/einkbridge/`
- Tests under `android/app/src/test/` and `android/app/src/androidTest/`
- Run `just test-android` after each phase to verify nothing regressed

---

## Phase 1 — SessionRepository

**Goal:** Centralise all SharedPreferences access for session state.

### What to create

New file: `SessionRepository.kt`

```kotlin
package com.flakm.einkbridge

import android.content.SharedPreferences

internal class SessionRepository(private val prefs: SharedPreferences) {

    fun saveStrokes(sessionId: String, buf: StrokeBuffer) {
        val key = "strokes_$sessionId"
        if (buf.isEmpty) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, buf.toJson()).apply()
    }

    fun loadStrokes(sessionId: String, buf: StrokeBuffer) {
        val json = prefs.getString("strokes_$sessionId", null) ?: return
        try { buf.loadJson(json) } catch (_: Exception) {}
    }

    fun saveBindGroups(sessionId: String, groups: List<BindGroup>) {
        val key = "binds_$sessionId"
        if (groups.isEmpty()) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, bindGroupsToJson(groups)).apply()
    }

    fun loadBindGroups(sessionId: String): List<BindGroup> {
        val json = prefs.getString("binds_$sessionId", null) ?: return emptyList()
        return try { bindGroupsFromJson(json) } catch (_: Exception) { emptyList() }
    }

    fun saveOcrResults(sessionId: String, results: List<OcrResult>) {
        val key = "ocr_$sessionId"
        if (results.isEmpty()) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, ocrResultsToJson(results)).apply()
    }

    fun loadOcrResults(sessionId: String): List<OcrResult> {
        val json = prefs.getString("ocr_$sessionId", null) ?: return emptyList()
        return try { ocrResultsFromJson(json) } catch (_: Exception) { emptyList() }
    }

    fun clearSession(sessionId: String) {
        prefs.edit()
            .remove("strokes_$sessionId")
            .remove("binds_$sessionId")
            .remove("ocr_$sessionId")
            .apply()
    }

    fun hasStrokes(sessionId: String): Boolean =
        prefs.contains("strokes_$sessionId")
}
```

### What to change in `MainActivity.kt`

1. Add field: `private lateinit var repo: SessionRepository`
2. In `onCreate()`, after `prefs = ...`: `repo = SessionRepository(prefs)`
3. Delete the eight private methods: `saveStrokes`, `loadStrokes`, `saveBindGroups`,
   `loadBindGroups`, `saveOcrResults`, `loadOcrResults`, `clearSavedStrokes`, `hasStrokes`
4. Update all callsites (grep for each method name):

| Old call | New call |
|----------|----------|
| `saveStrokes(sessionId, buf)` | `repo.saveStrokes(sessionId, buf)` |
| `loadStrokes(sessionId, buf)` | `repo.loadStrokes(sessionId, buf)` |
| `saveBindGroups(sessionId, overlay.bindGroups)` | `repo.saveBindGroups(sessionId, overlay.bindGroups)` |
| `loadBindGroups(sessionId)` | `repo.loadBindGroups(sessionId)` |
| `saveOcrResults(sessionId, results)` | `repo.saveOcrResults(sessionId, results)` |
| `loadOcrResults(sessionId)` | `repo.loadOcrResults(sessionId)` |
| `clearSavedStrokes(sessionId)` | `repo.clearSession(sessionId)` |
| `hasStrokes(it.id)` | `repo.hasStrokes(it.id)` |

### Test to write

New file: `SessionRepositoryTest.kt` (unit test, no Android framework needed if you
inject a `MockSharedPreferences` — or use Robolectric with `ApplicationProvider`).

```kotlin
@Test fun `save and load strokes roundtrip`() {
    val prefs = ...  // in-memory SharedPreferences
    val repo = SessionRepository(prefs)
    val buf = StrokeBuffer().apply { begin(0f, 0f, 3f); addPoint(10f, 10f); commit() }
    repo.saveStrokes("s1", buf)
    val buf2 = StrokeBuffer()
    repo.loadStrokes("s1", buf2)
    assertEquals(buf.strokes, buf2.strokes)
}

@Test fun `clear session removes all keys`() { ... }
@Test fun `hasStrokes returns false when nothing saved`() { ... }
@Test fun `loadBindGroups returns empty on missing key`() { ... }
```

---

## Phase 2 — WebViewBridge

**Goal:** Centralise all `webView.evaluateJavascript(...)` calls behind a typed interface
so `PenOverlay` can be tested without a real WebView.

### What to create

New file: `WebViewBridge.kt`

```kotlin
package com.flakm.einkbridge

import android.webkit.WebView

/**
 * Typed wrapper over the JavaScript API injected by the server-rendered HTML.
 * All JS function names live here and nowhere else.
 */
internal interface WebViewBridge {
    fun highlightAll()
    fun unhighlightAll()
    fun findElements(
        left: Float, top: Float, right: Float, bottom: Float,
        callback: (List<FoundElement>) -> Unit,
    )
    fun applyBindGroups(groups: List<BindGroup>)
    fun flashGroup(elementIndices: List<Int>, colorHex: String)
    fun queryElementMap(callback: (List<ElementEntry>) -> Unit)
    fun computeStrokeLinks()
}

internal class LiveWebViewBridge(private val webView: WebView) : WebViewBridge {

    override fun highlightAll() {
        webView.evaluateJavascript(
            "window.__einkHighlightAll && window.__einkHighlightAll()", null
        )
    }

    override fun unhighlightAll() {
        webView.evaluateJavascript(
            "window.__einkUnhighlightAll && window.__einkUnhighlightAll()", null
        )
    }

    override fun findElements(
        left: Float, top: Float, right: Float, bottom: Float,
        callback: (List<FoundElement>) -> Unit,
    ) {
        webView.evaluateJavascript("window.__einkFindElements($left,$top,$right,$bottom)") { raw ->
            callback(parseFoundElements(raw))
        }
    }

    override fun applyBindGroups(groups: List<BindGroup>) {
        val json = buildBindGroupsJson(groups)
        webView.post {
            webView.evaluateJavascript(
                "window.__einkApplyBindGroups && window.__einkApplyBindGroups($json)", null
            )
        }
    }

    override fun flashGroup(elementIndices: List<Int>, colorHex: String) {
        val indicesJson = org.json.JSONArray(elementIndices).toString()
        webView.evaluateJavascript(
            "window.__einkFlashGroup($indicesJson,'$colorHex')", null
        )
    }

    override fun queryElementMap(callback: (List<ElementEntry>) -> Unit) {
        webView.evaluateJavascript("JSON.stringify(window.__einkElementMap || [])") { json ->
            val cleaned = json?.trim()?.removeSurrounding("\"")
                ?.replace("\\\"", "\"")?.replace("\\\\", "\\") ?: "[]"
            try { callback(parseElementMap(cleaned)) } catch (_: Exception) { callback(emptyList()) }
        }
    }

    override fun computeStrokeLinks() {
        webView.post {
            webView.evaluateJavascript(
                "window.__einkComputeStrokeLinks && window.__einkComputeStrokeLinks([],[])", null
            )
        }
    }

    // ---- helpers (moved from PenOverlay) ----

    private fun parseFoundElements(raw: String?): List<FoundElement> {
        val cleaned = raw?.trim()?.removeSurrounding("\"")
            ?.replace("\\\"", "\"")?.replace("\\\\", "\\") ?: "[]"
        return try {
            val arr = org.json.JSONArray(cleaned)
            (0 until arr.length()).map {
                val obj = arr.getJSONObject(it)
                FoundElement(
                    i = obj.getInt("i"),
                    tag = obj.getString("tag"),
                    id = obj.optString("id", null),
                    section = obj.optString("section", null),
                    text = obj.optString("text", ""),
                    cx = obj.optDouble("cx", 0.0).toFloat(),
                    cy = obj.optDouble("cy", 0.0).toFloat(),
                )
            }
        } catch (_: Exception) { emptyList() }
    }

    private fun buildBindGroupsJson(groups: List<BindGroup>): String {
        val arr = org.json.JSONArray()
        for (g in groups) {
            arr.put(org.json.JSONObject().apply {
                put("color", "#%06X".format(g.color and 0xFFFFFF))
                put("indices", org.json.JSONArray(g.elementIndices))
            })
        }
        return arr.toString()
    }
}
```

### What to change in `PenOverlay.kt`

1. Add constructor parameter: `private val bridge: WebViewBridge`
   - Default: `LiveWebViewBridge(webView)` — keep backward-compatible for now
2. Delete `parseFoundElements()` (lines 513–531) — it moves to `LiveWebViewBridge`
3. Delete `syncBindGroupsToWebView()` (lines 661–679) — replace with `bridge.applyBindGroups(_bindGroups)`
4. Delete `queryElementMap()` (lines 634–645) — replace with `bridge.queryElementMap(...)`
5. Delete `refreshStrokeLinks()` (lines 655–659) — replace with `bridge.computeStrokeLinks()`
6. Replace each bare `webView.evaluateJavascript(...)` call:

| Location | Old | New |
|----------|-----|-----|
| `setupScaleTracker()` ACTION_DOWN (line 308) | `webView.evaluateJavascript("window.__einkHighlightAll...")` | `bridge.highlightAll()` |
| `setupScaleTracker()` ACTION_UP (line 320) | `webView.evaluateJavascript("window.__einkUnhighlightAll...")` | `bridge.unhighlightAll()` |
| `completeBindGesture()` (line 474) | `webView.evaluateJavascript("window.__einkFindElements(...)")` | `bridge.findElements(left, top, right, bottom) { elements -> ... }` |
| `expandGroup()` (line 589) | `webView.evaluateJavascript("window.__einkFlashGroup(...)")` | `bridge.flashGroup(group.elementIndices, colorHex)` |
| `clearStrokes()` (line 432) | `webView.evaluateJavascript("window.__einkComputeStrokeLinks...")` | `bridge.computeStrokeLinks()` |
| `refreshStrokeLinks()` (lines 656–658) | direct call | `bridge.computeStrokeLinks()` |

7. Update `submitAndGoBack()` in `MainActivity` to call `overlay.bridge.queryElementMap(...)` → actually
   `PenOverlay` retains a public `queryElementMap` method that delegates to `bridge.queryElementMap`.

### Construction in `MainActivity.openSession()`

```kotlin
val bridge = LiveWebViewBridge(webView)
val overlay = PenOverlay(webView, penToolbar, strokeView, buf = buf,
    bridge = bridge,
    onStrokesChanged = { repo.saveStrokes(sessionId, buf) },
)
```

### Test to write

New file: `WebViewBridgeTest.kt` — test `buildBindGroupsJson` and `parseFoundElements` logic
in isolation by instantiating `LiveWebViewBridge` with a mock WebView stub or by
extracting the pure parsing helpers as package-level functions and testing those directly.

For `PenOverlay` tests, pass a `FakeWebViewBridge`:

```kotlin
internal class FakeWebViewBridge : WebViewBridge {
    val highlightAllCalls = mutableListOf<Unit>()
    var findElementsResult: List<FoundElement> = emptyList()
    val applyBindGroupsCalls = mutableListOf<List<BindGroup>>()

    override fun highlightAll() { highlightAllCalls += Unit }
    override fun unhighlightAll() {}
    override fun findElements(left: Float, top: Float, right: Float, bottom: Float,
                              callback: (List<FoundElement>) -> Unit) = callback(findElementsResult)
    override fun applyBindGroups(groups: List<BindGroup>) { applyBindGroupsCalls += groups }
    override fun flashGroup(elementIndices: List<Int>, colorHex: String) {}
    override fun queryElementMap(callback: (List<ElementEntry>) -> Unit) = callback(emptyList())
    override fun computeStrokeLinks() {}
}
```

---

## Phase 3 — SubmissionBuilder

**Goal:** Extract the pure annotation-building logic out of `submitAndGoBack()` so it
can be unit tested without an Activity or network.

### What to create

New file: `SubmissionBuilder.kt`

```kotlin
package com.flakm.einkbridge

/**
 * Pure builder — no Android framework dependencies.
 * Converts in-memory stroke state into the wire format expected by the server.
 */
internal object SubmissionBuilder {

    data class Package(
        val annotationsJson: String,
        val strokeJson: String?,
        val pngBytes: ByteArray?,
    )

    fun build(
        strokes: List<Stroke>,
        bindGroups: List<BindGroup>,
        elements: List<ElementEntry>,
        pngExporter: () -> ByteArray?,
        strokeJsonExporter: () -> String?,
    ): Package {
        val (explicitGroups, unanchored) = bindGroupsToAnnotations(strokes, bindGroups)
        val (proximityGroups, trulyUnanchored) = groupStrokesWithProximity(unanchored, elements)
        val annotationsJson = annotationsToJson(explicitGroups + proximityGroups, trulyUnanchored)
        return Package(
            annotationsJson = annotationsJson,
            strokeJson = strokeJsonExporter(),
            pngBytes = pngExporter(),
        )
    }
}
```

Then create a thin upload function (can live in `SubmissionBuilder.kt` or its own file):

```kotlin
internal class SubmissionUploader(private val client: OkHttpClient) {

    suspend fun upload(
        serverUrl: String,
        sessionId: String,
        pkg: SubmissionBuilder.Package,
    ): Result<Int> = withContext(Dispatchers.IO) {
        runCatching {
            val builder = MultipartBody.Builder()
                .setType(MultipartBody.FORM)
                .addFormDataPart("typed_notes", "")
                .addFormDataPart("annotations", pkg.annotationsJson)
            pkg.pngBytes?.let {
                builder.addFormDataPart("annotation", "strokes.png",
                    it.toRequestBody("image/png".toMediaType()))
            }
            pkg.strokeJson?.let { builder.addFormDataPart("stroke_data", it) }
            val request = Request.Builder()
                .url("$serverUrl/api/sessions/$sessionId/submit")
                .post(builder.build())
                .build()
            client.newCall(request).execute().code
        }
    }
}
```

### What to change in `MainActivity.submitAndGoBack()`

Replace the inline grouping + JSON building block (currently lines 471–484) with:

```kotlin
overlay.bridge.queryElementMap { elements ->
    scope.launch {
        try {
            val pkg = SubmissionBuilder.build(
                strokes = overlay.buf.strokes,
                bindGroups = overlay.bindGroups,
                elements = elements,
                pngExporter = overlay::exportToPng,
                strokeJsonExporter = overlay::exportStrokeJson,
            )
            val code = uploader.upload(serverUrl, sessionId, pkg).getOrThrow()
            withContext(Dispatchers.Main) {
                when (code) {
                    in 200..299 -> { repo.clearSession(sessionId); Toast...; showSessionList(); startPolling() }
                    409         -> { Toast...; showSessionList(); startPolling() }
                    else        -> Toast.makeText(this@MainActivity, "Submit failed: $code", ...)
                }
            }
        } catch (e: Exception) { ... }
    }
}
```

Add field in `MainActivity`:
```kotlin
private val uploader = SubmissionUploader(client)
```

### Test to write

New file: `SubmissionBuilderTest.kt` (pure JVM unit test, no Android):

```kotlin
@Test fun `build produces explicit group for bind-group strokes`() {
    val stroke = Stroke(listOf(0f to 0f, 10f to 10f), 3f)
    val ref = ElementRef("sec1", "p", "Some text")
    val bindGroup = BindGroup(0, Color.RED, setOf(0), listOf(0),
        listOf(ref), 5f, 5f)
    val elements = listOf(ElementEntry(0, "p", "sec1", "sec1", 0f, 20f, 0f, 20f, "Some text"))

    val pkg = SubmissionBuilder.build(
        strokes = listOf(stroke),
        bindGroups = listOf(bindGroup),
        elements = elements,
        pngExporter = { null },
        strokeJsonExporter = { null },
    )

    val json = org.json.JSONArray(pkg.annotationsJson)
    assertEquals(1, json.length())
    assertEquals("explicit", json.getJSONObject(0).getJSONObject("anchor").getString("type"))
}

@Test fun `build produces proximity group when no bind groups`() { ... }
@Test fun `build returns null png when buffer empty`() { ... }
```

---

## Phase 4 — BindGestureTracker

**Goal:** Extract the bind-mode touch state machine out of the 100-line lambda in
`PenOverlay.setupScaleTracker()` into a testable class.

### What to create

New file: `BindGestureTracker.kt`

```kotlin
package com.flakm.einkbridge

import android.graphics.PointF
import android.view.MotionEvent

/**
 * Stateful tracker for the bind-mode gesture (finger drag to select strokes + elements).
 *
 * Call [onTouchEvent] from the WebView touch listener when [active] is true.
 * The tracker fires [onGestureComplete] when the finger is lifted after a drag,
 * or [onTap] when the finger barely moved (tap on an existing marker).
 */
internal class BindGestureTracker(
    /** Called with the accumulated screen-space points when the gesture ends. */
    private val onGestureComplete: (List<PointF>) -> Unit,
    /** Called with the tap position when finger moved less than TAP_THRESHOLD_PX2. */
    private val onTap: (screenX: Float, screenY: Float) -> Unit,
    /** Called whenever the accumulating path changes (null = gesture ended/not started). */
    private val onPathChanged: (List<PointF>?) -> Unit,
) {
    var active: Boolean = false

    private var points: MutableList<PointF>? = null

    /** Returns true if the event was consumed. */
    fun onTouchEvent(event: MotionEvent): Boolean {
        if (!active || event.pointerCount != 1) return false
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                val pts = mutableListOf(PointF(event.x, event.y))
                points = pts
                onPathChanged(pts)
            }
            MotionEvent.ACTION_MOVE -> {
                points?.add(PointF(event.x, event.y))
                onPathChanged(points)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                val pts = points ?: emptyList<PointF>()
                points = null
                onPathChanged(null)
                val moved = pts.size > 1 && run {
                    val dx = pts.last().x - pts.first().x
                    val dy = pts.last().y - pts.first().y
                    dx * dx + dy * dy > TAP_THRESHOLD_PX2
                }
                if (moved && event.actionMasked == MotionEvent.ACTION_UP) {
                    onGestureComplete(pts)
                } else {
                    onTap(event.x, event.y)
                }
            }
        }
        return true
    }

    companion object {
        const val TAP_THRESHOLD_PX2 = 30f * 30f
    }
}
```

### What to change in `PenOverlay.kt`

1. Add field: `private val bindTracker = BindGestureTracker(...)`

2. In `setupScaleTracker()`, replace the entire `isBindMode && event.pointerCount == 1 -> when (...)` branch (lines 301–332) with:

```kotlin
if (isBindMode) {
    val consumed = bindTracker.onTouchEvent(event)
    if (consumed) return@OnTouchListener result || isBindMode
}
```

3. Construct `bindTracker` inline or in `init {}`:

```kotlin
private val bindTracker = BindGestureTracker(
    onGestureComplete = { pts -> completeBindGesture(pts) },
    onTap = { x, y ->
        if (!showOcrPopupNear(x, y)) flashMarkerNear(x, y)
    },
    onPathChanged = { pts ->
        strokeView?.setBindPath(pts)
        strokeView?.bindDrawingActive = (pts != null)
        strokeView?.invalidate()
        if (pts != null) bridge.highlightAll() else bridge.unhighlightAll()
    },
)
```

4. Remove `private val TAP_THRESHOLD_PX2` constant from `PenOverlay.Companion` — it moves to `BindGestureTracker.Companion`.

5. Remove `private var bindPoints: MutableList<PointF>?` field — it moves into `BindGestureTracker`.

6. `enterBindMode()` / `exitBindMode()` must set `bindTracker.active`:

```kotlin
fun enterBindMode() {
    isBindMode = true
    bindTracker.active = true
    controller.setEnabled(false)
}

fun exitBindMode() {
    isBindMode = false
    bindTracker.active = false
    strokeView?.setBindPath(null)
    controller.resetRenderBuffer()
}
```

### Test to write

New file: `BindGestureTrackerTest.kt` (pure JVM — MotionEvent can be created via
`MotionEvent.obtain(...)` or Robolectric):

```kotlin
@Test fun `drag fires onGestureComplete with accumulated points`() {
    val completed = mutableListOf<List<PointF>>()
    val tracker = BindGestureTracker(
        onGestureComplete = { completed += it },
        onTap = { _, _ -> },
        onPathChanged = {},
    )
    tracker.active = true

    tracker.onTouchEvent(motionEvent(ACTION_DOWN, 0f, 0f))
    tracker.onTouchEvent(motionEvent(ACTION_MOVE, 50f, 50f))  // move enough
    tracker.onTouchEvent(motionEvent(ACTION_UP, 100f, 100f))

    assertEquals(1, completed.size)
    assertEquals(3, completed[0].size)
}

@Test fun `tap fires onTap not onGestureComplete`() { ... }
@Test fun `inactive tracker ignores events`() { ... }
@Test fun `cancel clears path and fires onTap`() { ... }
```

---

## Phase 5 — OcrManager: Replace Callbacks with SharedFlow (optional)

**Goal:** Eliminate invisible lambda-capture dependencies across three classes.

### What to change in `OcrManager.kt`

Add a `SharedFlow`:

```kotlin
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow

internal sealed class OcrEvent {
    data class GroupRecognized(val groupId: Int, val text: String) : OcrEvent()
    data class UnboundResults(val results: List<OcrResult>) : OcrEvent()
    data class PendingChanged(val remaining: Int, val total: Int) : OcrEvent()
}

internal class OcrManager(
    private val recognize: suspend (List<Stroke>) -> String?,
    private val scope: CoroutineScope,
) {
    private val _events = MutableSharedFlow<OcrEvent>(extraBufferCapacity = 64)
    val events: SharedFlow<OcrEvent> = _events.asSharedFlow()

    // replace onGroupRecognized(id, text) → _events.tryEmit(OcrEvent.GroupRecognized(id, text))
    // replace onUnboundResults(results) → _events.tryEmit(OcrEvent.UnboundResults(results))
    // replace onPendingChanged(r, t) → _events.tryEmit(OcrEvent.PendingChanged(r, t))
    ...
}
```

### Wiring in `MainActivity.openSession()`

```kotlin
val ocrManager = OcrManager(recognize = ocrClient::recognize, scope = scope)
overlay.ocrManager = ocrManager

scope.launch {
    ocrManager.events.collect { event ->
        when (event) {
            is OcrEvent.GroupRecognized -> overlay.onGroupOcrResult(event.groupId, event.text)
            is OcrEvent.UnboundResults  -> overlay.onUnboundOcrResults(event.results)
            is OcrEvent.PendingChanged  -> updateOcrProgress(event.remaining, event.total)
        }
    }
}
```

Remove the three callback constructor parameters from `OcrManager`.

### Test benefit

`OcrManager` can be tested by collecting from `events` — no callback injection needed.

---

## Execution Order

Do phases in this order. Each should pass `just test-android` before the next begins.

```
Phase 1 → Phase 2 → Phase 3 → Phase 4 → Phase 5 (optional)
```

Phases 1, 2, 3 can be done in parallel by different agents on separate git branches,
since they touch disjoint parts of the code (SharedPreferences, JS calls, submission
logic). Phase 4 depends on Phase 2 (needs `WebViewBridge` to exist before
rewriting `setupScaleTracker`).

---

## Files Created / Modified Summary

| File | Action |
|------|--------|
| `SessionRepository.kt` | **Create** |
| `WebViewBridge.kt` | **Create** |
| `SubmissionBuilder.kt` | **Create** |
| `BindGestureTracker.kt` | **Create** |
| `MainActivity.kt` | **Modify** (remove 8 methods, wire repo + uploader + bridge) |
| `PenOverlay.kt` | **Modify** (remove parseFoundElements, syncBindGroupsToWebView, queryElementMap, refreshStrokeLinks, bindPoints field, TAP_THRESHOLD_PX2; wire bridge + bindTracker) |
| `OcrManager.kt` | **Modify** (Phase 5: replace 3 callbacks with SharedFlow) |
| `SessionRepositoryTest.kt` | **Create** |
| `WebViewBridgeTest.kt` | **Create** |
| `SubmissionBuilderTest.kt` | **Create** |
| `BindGestureTrackerTest.kt` | **Create** |

## Files NOT touched

- `StrokeBuffer.kt` — already clean
- `StrokeView.kt` — already clean
- `OcrClient.kt` — already clean
- `SessionAdapter.kt` — already clean
- `WebSocketClient.kt` — already clean
- `DocumentCache.kt` — already clean
- `PenInputController.kt` — already clean

---

## Definition of Done

- [ ] `just test-android` passes after each phase
- [ ] No `webView.evaluateJavascript(...)` calls exist outside `LiveWebViewBridge`
- [ ] No `prefs.getString("strokes_...")`/`prefs.edit()` calls exist outside `SessionRepository`
- [ ] `SubmissionBuilder.build()` has ≥3 unit tests covering explicit, proximity, and empty cases
- [ ] `BindGestureTracker` has ≥4 unit tests covering drag, tap, cancel, inactive
- [ ] `MainActivity.kt` is under 500 lines
- [ ] `PenOverlay.kt` is under 550 lines
