# Android Refactoring Plan

## Goal

Make the codebase testable and maintainable for AI-assisted development by eliminating God Objects, separating concerns, and establishing clear boundaries between UI, business logic, and I/O.

---

## Current Architecture Snapshot

```
MainActivity (705 lines)
  ├── HTTP polling loop
  ├── WebSocket handling
  ├── SharedPreferences I/O
  ├── Toolbar + color picker UI
  ├── WebView management
  └── Submission workflow (coordinate transforms, grouping, multipart upload)

PenOverlay (770 lines)
  ├── Pen input routing (Onyx SDK)
  ├── Bind gesture state machine (touch → polygon → elements)
  ├── BindGroup CRUD + undo stack
  ├── OCR scheduling + result display
  ├── WebView JS evaluation
  ├── Coordinate transforms
  └── StrokeView invalidation

StrokeBuffer (485 lines) — ✅ clean, pure Kotlin, well-tested
StrokeView (287 lines)   — ✅ clean, pure rendering
OcrClient (87 lines)     — ✅ clean, testable
OcrManager (141 lines)   — ✅ mostly clean, some callback coupling
WebSocketClient (76 lines) — ✅ thin, clean wrapper
DocumentCache (89 lines) — ✅ clean file cache
SessionAdapter (91 lines) — ✅ clean list adapter
```

---

## Pain Points

### 1. MainActivity is a God Object

**File:** `MainActivity.kt` (~705 lines)

Every concern is crammed into one class:

- **Networking:** `fetchSessions()` does HTTP polling, offline fallback, adapter updates, error toasts
- **Persistence:** SharedPreferences keys are string literals scattered across `openSession()`, `submitAndGoBack()`, `onDestroy()`, `fetchSessions()`
- **Submission business logic:** `submitAndGoBack()` does ~80 lines of coordinate math, stroke grouping (`groupStrokesWithProximity`), annotation JSON building, PNG export, and multipart HTTP upload — all inline
- **UI construction:** `setupPenToolbar()` (~60 lines), `showColorPicker()` (~45 lines) build views imperatively with no separation
- **WebView lifecycle:** HTML caching, JS injection, scroll/scale state all managed in-line

**Impact:** Any change to submission, polling, or persistence requires reading the full 700-line class to understand context. Cannot test `submitAndGoBack()` without a real Activity.

---

### 2. PenOverlay is a God Object

**File:** `PenOverlay.kt` (~770 lines)

Three unrelated state machines share a single class:

- **Pen input state machine:** raw drawing mode enable/disable, Onyx SDK lifecycle, stroke width/style changes
- **Bind gesture state machine:** touch point accumulation → polygon → `__einkFindElements` JS call → `BindGroup` creation → undo stack — all inside a `setupScaleTracker()` lambda that is itself 100+ lines
- **OCR result display state machine:** badge tap detection, popup dialog, unbound result list, group text update

These interleave through shared mutable fields (`bindPoints`, `isBindMode`, `_bindGroups`, `expandedGroupIds`) making any single path hard to follow or test.

`setupScaleTracker()` is the worst offender: it is a single function that contains a nested touch listener with sub-gesture states, WebView JS calls, coroutine launches, and canvas invalidations — all mixed together.

---

### 3. No ViewModel / No Lifecycle-Aware State

State lives in:
- Activity fields (lost on rotation)
- `PenOverlay` fields (not observable)
- SharedPreferences (accessed imperatively, not reactively)

There is no ViewModel, no `StateFlow`, no observable state. Any UI update requires knowing which object holds the relevant field, then calling `invalidate()` or updating a TextView manually.

**Impact:** Hard to reason about what the UI should look like given the current state. Any refactor that moves fields risks silent regressions.

---

### 4. WebView JavaScript Coupling is Scattered

`PenOverlay.kt` calls `webView.evaluateJavascript(...)` in at least 6 different methods:
- `syncBindGroupsToWebView()` — apply highlight CSS
- `completeBindGesture()` — find elements in bbox
- `exitBindMode()` — clear highlights
- `expandGroup()` — flash group elements
- `openSession()` in MainActivity — inject toolbar JS

The JS function names (`__einkHighlightAll`, `__einkFindElements`, `__einkFlashGroup`, `__einkApplyBindGroups`) are bare string literals repeated in multiple places with no shared constants or type-safe wrappers.

**Impact:** Renaming a JS function requires grep across Kotlin files. JS call failures are silent (evaluateJavascript has no error callback). Cannot test the Kotlin side without a real WebView.

---

### 5. Persistence is Strewn Across Multiple Classes

SharedPreferences access uses hand-crafted string keys in MainActivity:

```kotlin
prefs.getString("strokes_$sessionId", null)
prefs.getString("bind_groups_$sessionId", null)
prefs.getString("ocr_results_$sessionId", null)
prefs.edit().putString("strokes_$sessionId", json).apply()
```

Same pattern repeated in `openSession()`, `submitAndGoBack()`, and `PenOverlay` callbacks. If the key format changes or a field is added, every callsite must be updated.

**Impact:** No single place to see all persisted state. Easy to forget to clear one key on session reset. Cannot test persistence without SharedPreferences.

---

### 6. Submission Logic is Untestable

`submitAndGoBack()` in MainActivity:
1. Calls `StrokeBuffer.groupStrokesWithProximity()` with element map
2. Merges bind-group annotations with proximity annotations
3. Exports PNG via `StrokeView.drawToBitmap()`
4. Builds multipart HTTP body
5. Uploads, shows toast, navigates back

Steps 1–3 are pure business logic but live inside a method that requires a running Activity, a real StrokeView, and a real OkHttp client. No unit test can reach them.

---

### 7. OcrManager Callbacks Create Tight Coupling

`OcrManager` constructor takes three lambdas:
```kotlin
val ocrManager = OcrManager(
    onGroupRecognized = { id, text -> /* mutate PenOverlay state */ },
    onUnboundResults  = { results -> /* mutate PenOverlay state */ },
    onPendingChanged  = { rem, tot -> /* update MainActivity UI */ },
)
```

The `onPendingChanged` callback updates a UI element in MainActivity. The `onGroupRecognized` callback mutates `PenOverlay._bindGroups`. This creates a fan of invisible dependencies across three classes held together only by lambda captures.

---

## Proposed Refactoring

### Phase 1 — Extract SessionRepository (high impact, low risk)

Create `SessionRepository.kt`:
- Owns all SharedPreferences access for a session
- Provides typed load/save for strokes, bind groups, OCR results
- Owns the polling loop (returns `Flow<List<SessionInfo>>`)
- Owns DocumentCache delegation

**Before:**
```kotlin
// scattered in MainActivity
prefs.getString("strokes_$sessionId", null)
prefs.getString("bind_groups_$sessionId", null)
```

**After:**
```kotlin
class SessionRepository(context: Context) {
    fun loadStrokes(sessionId: String): List<Stroke>
    fun saveStrokes(sessionId: String, strokes: List<Stroke>)
    fun loadBindGroups(sessionId: String): List<BindGroup>
    fun saveBindGroups(sessionId: String, groups: List<BindGroup>)
    fun loadOcrResults(sessionId: String): List<OcrResult>
    fun saveOcrResults(sessionId: String, results: List<OcrResult>)
    fun sessionListFlow(serverUrl: String): Flow<List<SessionInfo>>
}
```

Testable without Activity. SharedPreferences injected as constructor argument.

---

### Phase 2 — Extract SubmissionManager (high impact, medium risk)

Create `SubmissionManager.kt`:
- Owns the logic currently inside `submitAndGoBack()`
- Takes `StrokeBuffer`, `List<BindGroup>`, `List<OcrResult>`, element map
- Returns a `SubmissionPackage` (strokes JSON, annotations JSON, PNG bitmap)
- Uploads via injected `OkHttpClient`

**Before:**
```kotlin
// 80 lines inside MainActivity.submitAndGoBack()
val groups = groupStrokesWithProximity(...)
val annotations = annotationsToJson(...)
// ... build multipart, upload
```

**After:**
```kotlin
class SubmissionManager(private val client: OkHttpClient) {
    fun buildPackage(buffer: StrokeBuffer, bindGroups: List<BindGroup>, ...): SubmissionPackage
    suspend fun upload(sessionId: String, pkg: SubmissionPackage, serverUrl: String): Result<Unit>
}
```

`buildPackage()` is pure and fully unit-testable. `upload()` can be tested with a mock server.

---

### Phase 3 — Extract BindGestureHandler from PenOverlay (medium impact, medium risk)

Create `BindGestureHandler.kt`:
- Owns touch point accumulation during bind mode
- Owns polygon intersection logic (`pointInPolygon`)
- Owns element lookup (via injected `ElementLookup` interface)
- Emits `BindGestureResult` (selected stroke indices + found elements) via callback

`PenOverlay` becomes a coordinator that delegates to `BindGestureHandler` rather than containing the logic inline.

**Interface for testability:**
```kotlin
interface ElementLookup {
    suspend fun findElements(docRect: RectF): List<FoundElement>
}
// Real impl: evaluates JS on WebView
// Test impl: returns hardcoded list
```

This breaks the WebView dependency for testing bind gesture logic.

---

### Phase 4 — Extract WebViewBridge (low risk, high clarity)

Create `WebViewBridge.kt`:
- Wraps all `webView.evaluateJavascript(...)` calls
- Defines typed methods with documented contracts
- Centralizes JS function name constants

```kotlin
class WebViewBridge(private val webView: WebView) {
    fun highlightAll()
    fun clearHighlights()
    fun findElements(docRect: RectF, callback: (List<FoundElement>) -> Unit)
    fun applyBindGroups(groups: List<BindGroup>)
    fun flashGroup(elementIndices: List<Int>, color: Int)
}
```

JS function name strings exist in one place only. Mockable for testing `PenOverlay` without a real WebView.

---

### Phase 5 — ViewModel + StateFlow for UI State (medium risk)

Introduce `SessionViewModel`:
- Holds `StateFlow<SessionUiState>` (current session, bind mode flag, annotation mode flag, OCR pending count)
- Moves polling job into ViewModel scope (survives rotation)
- MainActivity becomes a thin observer

```kotlin
data class SessionUiState(
    val sessions: List<SessionInfo> = emptyList(),
    val currentSession: SessionInfo? = null,
    val isBindMode: Boolean = false,
    val isAnnotationMode: Boolean = false,
    val ocrPending: Int = 0,
    val ocrTotal: Int = 0,
)
```

---

### Phase 6 — OcrManager: Replace Callbacks with Flow (low risk)

Replace three callback lambdas with a single `Flow<OcrEvent>`:

```kotlin
sealed class OcrEvent {
    data class GroupRecognized(val groupId: Int, val text: String) : OcrEvent()
    data class UnboundResults(val results: List<OcrResult>) : OcrEvent()
    data class PendingChanged(val remaining: Int, val total: Int) : OcrEvent()
}

class OcrManager(...) {
    val events: Flow<OcrEvent>
}
```

Collectors (PenOverlay, MainActivity) subscribe independently. No invisible lambda capture dependencies.

---

## Refactoring Priority

| Phase | Value | Risk | Effort | Start Here? |
|-------|-------|------|--------|-------------|
| 1 — SessionRepository | High | Low | Small | Yes |
| 2 — SubmissionManager | High | Medium | Medium | Yes |
| 4 — WebViewBridge | Medium | Low | Small | Yes |
| 3 — BindGestureHandler | High | Medium | Medium | After Phase 4 |
| 6 — OcrManager Flow | Medium | Low | Small | Anytime |
| 5 — ViewModel | Medium | Medium | Large | After Phase 1+2 |

---

## What NOT to Touch

- `StrokeBuffer.kt` — already well-separated and tested
- `StrokeView.kt` — clean rendering layer, no logic changes needed
- `OcrClient.kt` — clean dual-backend strategy
- `SessionAdapter.kt` — trivial adapter, fine as-is
- `WebSocketClient.kt` — thin wrapper, fine as-is
- `DocumentCache.kt` — clean, tested

---

## Test Strategy After Refactoring

| Component | Test Type | What to Test |
|-----------|-----------|--------------|
| `SessionRepository` | Unit | load/save roundtrip, key isolation per session |
| `SubmissionManager.buildPackage()` | Unit | grouping + JSON output matches golden |
| `BindGestureHandler` | Unit | polygon intersection, empty result, overlapping strokes |
| `WebViewBridge` | Integration | JS evaluated with correct args (use WebView test harness) |
| `OcrManager` | Unit | debounce, caching, Flow emission order |
| `SessionViewModel` | Unit | state transitions, polling lifecycle |
| `MainActivity` | Instrumented | thin — only verify UI reacts to ViewModel state |
