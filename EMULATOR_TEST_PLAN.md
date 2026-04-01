# Automated Android Emulator Testing Plan

Goal: close the validation loop with instrumented tests running on an Android
emulator, including stylus/pen event injection.

## Current state

| Layer | What exists | Runner |
|-------|-------------|--------|
| Unit (pure Kotlin) | StrokeBuffer, ViewTransform, touch routing, annotations | JUnit on JVM |
| Component (Robolectric) | MainActivity states, PenOverlay + MockPenController, SessionAdapter | Robolectric (JVM) |
| Integration (HTTP) | AnnotationE2ETest: real Rust server + OkHttp round-trip | JUnit + spawned server |
| Render goldens | Server-side HTML output snapshots | cargo test |
| **Instrumented (emulator)** | **Nothing yet** | -- |

The gap: no tests exercise the real Android touch pipeline, WebView JS bridge,
or on-screen rendering. MockPenController bypasses the MotionEvent path entirely.

---

## Phase 1: Emulator on NixOS

### 1.1 Update `flake.nix`

```nix
androidComposition = pkgs.androidenv.composeAndroidPackages {
  buildToolsVersions = [ "35.0.0" "34.0.0" ];
  platformVersions = [ "35" "34" ];
  includeEmulator = true;
  includeSystemImages = true;
  systemImageTypes = [ "google_apis" ];
  abiVersions = [ "x86_64" ];
  includeNDK = false;
};
```

### 1.2 Wrap emulator binary for NixOS

The emulator binary expects `/lib64/ld-linux-x86-64.so.2` which does not exist
on NixOS. Two options:

**Option A -- `buildFHSEnv` (recommended, matches existing aapt2 workaround):**

```nix
emulatorFHS = pkgs.buildFHSEnv {
  name = "android-emulator";
  targetPkgs = pkgs: with pkgs; [
    androidSdk
    xorg.libX11 xorg.libXext xorg.libXrender
    libGL vulkan-loader
    pulseaudio
    zlib
  ];
  runScript = "${androidSdk}/libexec/android-sdk/emulator/emulator";
};
```

Add `emulatorFHS` to `devShells.default.packages`.

**Option B -- `androidenv.emulateApp` (declarative AVD):**

```nix
emulator = pkgs.androidenv.emulateApp {
  name = "eink-test-avd";
  platformVersion = "34";
  systemImageType = "google_apis";
  abiVersion = "x86_64";
  avdHomeDir = "$HOME/.android/avd";
};
```

### 1.3 KVM

Verify in NixOS system config:

```nix
users.users.flakm.extraGroups = [ "kvm" ];
```

Check: `ls -la /dev/kvm` should show the user has access.

### 1.4 AVD creation (one-time)

```bash
avdmanager create avd \
  --name eink-test \
  --package "system-images;android-34;google_apis;x86_64" \
  --device "pixel_tablet"
```

### 1.5 Justfile recipes

```makefile
emu-create:
    avdmanager create avd --name eink-test \
      --package "system-images;android-34;google_apis;x86_64" \
      --device "pixel_tablet" --force

emu-start:
    emulator -avd eink-test -no-snapshot -no-audio -gpu swiftshader_indirect &

emu-wait:
    adb wait-for-device
    adb shell 'while [[ -z $(getprop sys.boot_completed) ]]; do sleep 1; done'

test-instrumented: emu-wait
    cd android && ./gradlew connectedDebugAndroidTest
```

---

## Phase 2: Gradle setup for instrumented tests

### 2.1 Dependencies in `build.gradle.kts`

```kotlin
androidTestImplementation("androidx.test:core:1.6.1")
androidTestImplementation("androidx.test:runner:1.6.2")
androidTestImplementation("androidx.test:rules:1.6.1")
androidTestImplementation("androidx.test.ext:junit:1.2.1")
androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
androidTestImplementation("androidx.test.espresso:espresso-web:3.6.1")
androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
```

### 2.2 Test source set

```
android/app/src/androidTest/java/com/flakm/einkbridge/
  StylusInputTest.kt
  WebViewBridgeTest.kt
  FullFlowTest.kt
  util/
    StylusInjector.kt
    TestServer.kt
```

---

## Phase 3: Stylus event injection

`adb shell input` **cannot** inject `TOOL_TYPE_STYLUS` -- all events arrive as
`TOOL_TYPE_FINGER`. The app's `rawDrawingAction()` explicitly checks tool type
and routes finger vs stylus events differently. So we must use
`Instrumentation.sendPointerSync()`.

### 3.1 StylusInjector helper

```kotlin
// androidTest/util/StylusInjector.kt
class StylusInjector(private val instrumentation: Instrumentation) {

    fun stroke(points: List<Pair<Float, Float>>, pressure: Float = 0.8f) {
        require(points.size >= 2)
        val props = MotionEvent.PointerProperties().apply {
            id = 0
            toolType = MotionEvent.TOOL_TYPE_STYLUS
        }
        val downTime = SystemClock.uptimeMillis()
        for ((i, point) in points.withIndex()) {
            val action = when (i) {
                0 -> MotionEvent.ACTION_DOWN
                points.lastIndex -> MotionEvent.ACTION_UP
                else -> MotionEvent.ACTION_MOVE
            }
            val coords = MotionEvent.PointerCoords().apply {
                x = point.first; y = point.second
                this.pressure = pressure
                size = 0.1f
            }
            val event = MotionEvent.obtain(
                downTime, SystemClock.uptimeMillis(), action,
                1, arrayOf(props), arrayOf(coords),
                0, 0, 1f, 1f, 0, 0,
                InputDevice.SOURCE_STYLUS, 0
            )
            instrumentation.sendPointerSync(event)
            event.recycle()
            SystemClock.sleep(8) // ~120Hz sample rate
        }
    }

    fun tap(x: Float, y: Float) = stroke(listOf(x to y, x to y))
}
```

### 3.2 Finger touch injection (for scroll/toolbar)

Same pattern but with `TOOL_TYPE_FINGER` and `SOURCE_TOUCHSCREEN`. Espresso
`perform(click())` handles toolbar buttons; use manual injection only for
scroll gestures.

---

## Phase 4: Test scenarios

### 4.1 WebView JS bridge (`WebViewBridgeTest.kt`)

Validates the JS functions injected by `render.rs` are callable and return
expected shapes.

| Test | What it does |
|------|-------------|
| `elementMapPopulated` | Load a session page, wait for DOMContentLoaded, call `window.__einkElementMap`, assert non-empty array with expected fields (i, tag, t, b, l, r) |
| `findElementsReturnsMatchingBbox` | Call `__einkFindElements(left, top, right, bottom)` with known coords, verify correct elements returned |
| `highlightAllTogglesOutlines` | Call `__einkHighlightAll`, screenshot, call `__einkUnhighlightAll`, screenshot, compare |
| `applyBindGroupsStylesElements` | Call `__einkApplyBindGroups([{color:"#e74c3c", indices:[0,1]}])`, verify elements have outline style |

Uses `Espresso-Web`:
```kotlin
onWebView()
    .forceJavascriptEnabled()
    .perform(script("JSON.stringify(window.__einkElementMap)"))
    .check(webMatches(getText(), not(equalTo("[]"))))
```

### 4.2 Stylus drawing (`StylusInputTest.kt`)

Validates the real touch pipeline: MotionEvent -> rawDrawingAction ->
OnyxPenController/TouchHelper -> StrokeBuffer -> StrokeView.

**Note:** On the emulator without Onyx SDK hardware, `TouchHelper.create()` will
likely throw or no-op. Two strategies:

- **Strategy A (recommended):** Add a build flavor `emulatorDebug` that replaces
  `OnyxPenController` with `MockPenController` via DI/factory. The touch routing
  still works; only the SDK call is bypassed.
- **Strategy B:** Catch `TouchHelper` init failure gracefully and fall back to a
  software pen controller that reads `TOOL_TYPE_STYLUS` MotionEvents directly.

| Test | What it does |
|------|-------------|
| `stylusStrokeAddsToBuffer` | Inject stylus DOWN/MOVE/UP via StylusInjector, assert `buf.size == 1` |
| `fingerTouchDoesNotDraw` | Inject finger DOWN/MOVE/UP, assert `buf.isEmpty` |
| `fingerScrollsWebView` | Inject finger swipe, assert `webView.scrollY` changed |
| `eraserModeRemovesStrokes` | Draw stroke, switch to eraser, draw over it, assert stroke removed |
| `undoButtonRemovesLastStroke` | Draw two strokes, tap undo, assert `buf.size == 1` |

### 4.3 Full flow (`FullFlowTest.kt`)

End-to-end: start server, create session, open in app, draw, submit, verify.

| Test | What it does |
|------|-------------|
| `fullReviewCycle` | 1. Start Rust server on random port. 2. Set server URL in UI. 3. Create session via HTTP. 4. Wait for session to appear in list. 5. Tap session. 6. Draw 3 strokes with StylusInjector. 7. Tap "Done". 8. Verify server received submission with PNG and annotations. |
| `offlineCacheShowsSessions` | 1. Open session (online). 2. Go back. 3. Kill server. 4. Verify session list still shows with "Offline" label. 5. Tap cached session, verify content loads. |
| `bindModeLassoCaptures` | 1. Open session. 2. Draw strokes near heading. 3. Enter bind mode. 4. Draw lasso around strokes + heading. 5. Verify bind group created. 6. Submit. 7. Verify annotations JSON contains explicit anchor. |

### 4.4 Visual regression (optional, Phase 5)

Capture `StrokeView` bitmap after drawing known strokes, compare against golden PNG.

```kotlin
val bitmap = Bitmap.createBitmap(width, height, ARGB_8888)
Canvas(bitmap).also { strokeView.draw(it) }
// Compare against golden with pixel tolerance
```

---

## Phase 5: CI integration

### 5.1 GitHub Actions with hardware acceleration

```yaml
jobs:
  instrumented-tests:
    runs-on: ubuntu-latest  # or macos-latest for better KVM support
    steps:
      - uses: actions/checkout@v4
      - uses: cachix/install-nix-action@v30
      - uses: reactivecircus/android-emulator-runner@v2
        with:
          api-level: 34
          target: google_apis
          arch: x86_64
          script: |
            cd android && ./gradlew connectedDebugAndroidTest
```

The `android-emulator-runner` action handles AVD creation, boot wait, and
hardware acceleration on GitHub-hosted runners. It bypasses the NixOS emulator
entirely (runs Ubuntu's emulator in CI).

### 5.2 Local NixOS workflow

```bash
just emu-start          # boots emulator in background
just emu-wait           # blocks until boot_completed
just test-instrumented  # runs connectedDebugAndroidTest
```

---

## Dependency graph

```
Phase 1 (emulator setup)
  |
  v
Phase 2 (gradle deps + source set)
  |
  +---> Phase 3 (StylusInjector)
  |       |
  |       v
  |     Phase 4.2 (stylus tests)
  |       |
  |       v
  |     Phase 4.3 (full flow tests)
  |
  +---> Phase 4.1 (WebView bridge tests -- no stylus needed)
  |
  v
Phase 5 (CI)
```

Phase 4.1 (WebView tests) can start as soon as Phase 2 is done -- no stylus
injection needed, just Espresso-Web. This is the quickest win.

## Key risk: Onyx SDK on emulator

The production code imports `com.onyx.android.sdk.pen.TouchHelper`. This class
talks to Onyx e-ink hardware drivers that don't exist on a standard emulator.

**Mitigation:** The `PenInputController` interface already abstracts this. Create
a factory that checks for Onyx hardware at runtime:

```kotlin
fun createPenController(...): PenInputController =
    if (isOnyxDevice()) OnyxPenController(...) else SoftwarePenController(...)
```

`SoftwarePenController` would read `TOOL_TYPE_STYLUS` MotionEvents directly
from the View's `OnTouchListener`, bypassing the Onyx SDK entirely. This makes
the same APK testable on any emulator while preserving Onyx fast-path on real
hardware.

## Effort estimate

| Phase | Scope | Rough size |
|-------|-------|-----------|
| 1 | flake.nix + AVD + justfile | Small -- config only |
| 2 | gradle deps + source dirs | Small |
| 3 | StylusInjector | ~80 lines |
| 4.1 | WebView bridge tests | ~150 lines, 4 tests |
| 4.2 | Stylus drawing tests | ~200 lines, 5 tests |
| 4.3 | Full flow tests | ~300 lines, 3 tests |
| 5 | CI pipeline | Small -- yaml config |

Start with Phase 1 + 2 + 4.1 (WebView tests) for quickest value.
