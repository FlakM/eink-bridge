package com.flakm.einkbridge

import android.view.InputDevice
import android.view.MotionEvent
import androidx.test.platform.app.InstrumentationRegistry

/**
 * The app, driven from outside, against a real server.
 *
 * These are the tests that answer "is anything broken?" rather than "is this function
 * right": they launch the real activity, load a real page from the real server in a real
 * WebView, and press the real toolbar. Everything below Robolectric is exercised — the
 * only thing that is not real is the microphone.
 *
 * A session is created for each test and removed afterwards, so the suite can run against
 * a server that already has the reader's reviews on it.
 */
internal object App {

    /**
     * The activity under test, handed over by the scenario that launched it.
     *
     * Asking the lifecycle registry instead looked equivalent and was not: it answers from
     * a monitor that the scenario does not always feed, so every helper saw "no activity"
     * while the app was on screen.
     */
    @Volatile
    private var current: MainActivity? = null

    fun attach(activity: MainActivity) {
        current = activity
    }

    fun activity(): MainActivity =
        current ?: throw AssertionError("no activity has been launched")

    /** Run something on the UI thread and wait for it, unlike `Activity.runOnUiThread`. */
    fun onMain(block: () -> Unit) {
        InstrumentationRegistry.getInstrumentation().runOnMainSync(block)
    }

    /** A WebView property, read on the thread it insists on. */
    fun webViewUrl(webView: android.webkit.WebView): String? {
        var url: String? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync { url = webView.url }
        return url
    }

    /** Point the app at the server under test before the activity reads it. */
    fun useServer(url: String) {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        SessionRepository(context).apply {
            saveServerUrl(url)
            saveToken("")
        }
    }

    /**
     * A tap delivered to the page's own touch handling.
     *
     * `UiAutomation.injectInputEvent` puts the event into the window, where the topmost
     * overlay in the layout took it before the WebView saw it. Dispatching to the view is
     * what the framework does after hit-testing, and it exercises the same listener the
     * reader's finger does — the app's router, not the window manager.
     */
    fun tapPage(webView: android.webkit.WebView, screenX: Float, screenY: Float) {
        val origin = IntArray(2)
        onMain { webView.getLocationOnScreen(origin) }
        val x = screenX - origin[0]
        val y = screenY - origin[1]
        val down = android.os.SystemClock.uptimeMillis()
        onMain {
            view(webView, down, down, MotionEvent.ACTION_DOWN, x, y)
            view(webView, down, down + 40, MotionEvent.ACTION_UP, x, y)
        }
    }

    private fun view(
        webView: android.webkit.WebView,
        downTime: Long,
        whenTime: Long,
        action: Int,
        x: Float,
        y: Float,
    ) {
        val event = MotionEvent.obtain(downTime, whenTime, action, x, y, 0)
        event.source = InputDevice.SOURCE_TOUCHSCREEN
        webView.dispatchTouchEvent(event)
        event.recycle()
    }

    /**
     * A finger tap at screen coordinates, the way a reader would touch the page.
     *
     * Through UiAutomation rather than `Instrumentation.sendPointerSync`: that one is
     * refused with "not directed at a window owned by uid", because the injected event has
     * no window to target. UiAutomation injects the way the system does.
     */
    fun tap(x: Float, y: Float) {
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        val down = android.os.SystemClock.uptimeMillis()
        val events = listOf(
            MotionEvent.obtain(down, down, MotionEvent.ACTION_DOWN, x, y, 0),
            MotionEvent.obtain(down, down + 40, MotionEvent.ACTION_UP, x, y, 0),
        )
        events.forEach { event ->
            event.source = InputDevice.SOURCE_TOUCHSCREEN
            automation.injectInputEvent(event, true)
            event.recycle()
            android.os.SystemClock.sleep(20)
        }
    }
}
