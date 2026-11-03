package com.flakm.einkbridge

import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.filters.LargeTest
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The round trip a reader actually makes, on an emulator, against a real server.
 *
 * Deliberately coarse: each test is one thing a person does, and a failure means the app
 * is broken rather than that a function changed shape. Everything below Robolectric is
 * exercised — a real WebView, the real JS bridge, the real toolbar.
 *
 * The list and the page are driven through their own state and their own JS rather than
 * through Espresso view matchers: the list is a `RecyclerView` fed by a coroutine, and
 * matching on it made the suite fail for reasons that had nothing to do with the app.
 */
@RunWith(AndroidJUnit4::class)
@LargeTest
class AppFlowTest {

    private var scenario: ActivityScenario<MainActivity>? = null
    private val sessions = mutableListOf<String>()

    private val title = "Emulator round trip"
    private val document = """
        # $title

        The first paragraph is here to be tapped at.

        The second paragraph is the one a note is written on.
    """.trimIndent()

    @Before
    fun setUp() {
        assumeTrue(
            "no server at ${ServerUnderTest.url}; the harness starts one before this suite",
            ServerUnderTest.healthy(),
        )
        App.useServer(ServerUnderTest.url)
    }

    @After
    fun tearDown() {
        scenario?.close()
        scenario = null
        sessions.forEach(ServerUnderTest::remove)
    }

    /**
     * Launch the app with the server already configured — `onCreate` reads the URL, so it
     * has to be written before the activity exists.
     */
    private fun launch(): MainActivity {
        scenario = ActivityScenario.launch(MainActivity::class.java)
        var activity: MainActivity? = null
        scenario!!.onActivity { activity = it }
        return activity!!.also { App.attach(it) }
    }

    @Test
    fun theAppListsASessionTheServerHas() {
        newSession()
        val activity = launch()

        val listed = awaitSession(activity, title)
        assertTrue(
            "the session never appeared in the list; the app says " +
                "\"${statusLine(activity)}\" and lists ${titles(activity)}",
            listed,
        )
    }

    @Test
    fun openingASessionRendersTheDocumentInTheWebView() {
        newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)

        // The element map is built by the page's own JS once the document is laid out,
        // so a non-zero count means HTML, CSS and scripts all arrived and ran.
        val count = Page.awaitElementMap(activity.webView)
        assertTrue("the page reported $count elements", count >= 3)

        val texts = Page.elementTexts(activity.webView)
        assertTrue(
            "the rendered page does not contain the document: $texts",
            texts.any { it.contains("first paragraph") },
        )
    }

    /**
     * The bug this exists for: tapping a block in voice mode called a page function that
     * did not exist, so every tap silently did nothing and the tray never started. Nothing
     * below a real WebView could see it.
     */
    @Test
    fun everyBlockOnThePageResolvesToItself() {
        newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        val count = Page.awaitElementMap(activity.webView)

        for (index in 0 until count) {
            val element = Page.elementAt(activity.webView, index)
            // The map carries boxes, not centres: `cx`/`cy` are what the *search* returns.
            val cx = (element.getDouble("l") + element.getDouble("r")) / 2
            val cy = (element.getDouble("t") + element.getDouble("b")) / 2
            assertEquals(
                "the page picked the wrong block for its own centre: ${element.getString("text")}",
                index,
                Page.nearestAt(activity.webView, cx, cy),
            )
        }
    }

    /*
     * What is deliberately *not* here: tapping the page and watching a block bind.
     *
     * That path runs the finger's screen coordinates through the Onyx ink transform, and an
     * emulator has no Onyx SDK and no viewport report to transform from — the tap arrives,
     * the nearest-element search is asked about the wrong point, and the test fails for a
     * reason that has nothing to do with the app. The search itself is covered directly by
     * `everyBlockOnThePageResolvesToItself`, its input by `BindGestureHandlerTest`, and the
     * real finger on the real tablet by the device.
     */

    @Test
    fun aSubmittedNoteReachesTheServer() {
        val id = newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        Page.awaitElementMap(activity.webView)

        App.onMain { activity.submitForTest() }

        val deadline = System.currentTimeMillis() + 20_000
        var status = ""
        while (System.currentTimeMillis() < deadline && status != "submitted") {
            status = ServerUnderTest.sessionStatus(id)
            if (!status.equals("submitted", ignoreCase = true)) Thread.sleep(300)
        }
        assertEquals(
            "the server never saw the submission",
            "submitted",
            status.lowercase(),
        )
    }

    // ── helpers ──────────────────────────────────────────────────────────────

    private fun newSession(): String =
        ServerUnderTest.createSession(title, document).also { sessions += it }

    private fun titles(activity: MainActivity): List<String> =
        activity.sessionsForTest().map { it.title }

    private fun statusLine(activity: MainActivity): String = activity.statusForTest()

    /** Wait for a session with this title to reach the list. */
    private fun awaitSession(activity: MainActivity, title: String, timeoutMs: Long = 20_000): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (activity.sessionsForTest().any { it.title == title }) return true
            Thread.sleep(200)
        }
        return false
    }

    /** Open a session by title, the way the adapter's click would. */
    private fun open(activity: MainActivity, title: String) {
        val session = activity.sessionsForTest().first { it.title == title }
        activity.runOnUiThread { activity.openSessionForTest(session.id) }
        val deadline = System.currentTimeMillis() + 20_000
        while (System.currentTimeMillis() < deadline) {
            if (App.webViewUrl(activity.webView) != null) return
            Thread.sleep(200)
        }
        throw AssertionError("the reader never opened: ${App.webViewUrl(activity.webView)}")
    }

    /** The tray's line, once the UI thread has had a chance to write it. */
    private fun awaitVoiceState(activity: MainActivity, timeoutMs: Long = 5_000): String {
        val deadline = System.currentTimeMillis() + timeoutMs
        var text = ""
        while (System.currentTimeMillis() < deadline && text.isEmpty()) {
            text = activity.voiceStateForTest()
            if (text.isEmpty()) Thread.sleep(100)
        }
        return text
    }

    /** What the tablet itself has bound, which is the state the page is drawn from. */
    private fun awaitBound(activity: MainActivity, timeoutMs: Long = 10_000): List<String> {
        val deadline = System.currentTimeMillis() + timeoutMs
        var bound = emptyList<String>()
        while (System.currentTimeMillis() < deadline && bound.isEmpty()) {
            bound = activity.boundGroupsForTest().map { group ->
                group.elementRefs.joinToString(" | ") { it.text }
            }
            if (bound.isEmpty()) Thread.sleep(200)
        }
        return bound
    }
}
