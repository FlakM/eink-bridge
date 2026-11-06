package com.flakm.einkbridge

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
}
