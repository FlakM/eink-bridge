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
        assertEquals(
            "the element map skipped blocks the page is showing: a script threw partway",
            Page.blockCount(activity.webView),
            count,
        )

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

    /**
     * A note spoken at a block lands on that block, all the way to the server.
     *
     * This is the path that was silently dead: the page's search function did not exist,
     * so the tap produced nothing and the tray said "nothing to talk about there". It also
     * pins the payload — the anchor is the paragraph the reader aimed at, and the group
     * keeps the words, the severity and the voice provenance through the multipart submit.
     *
     * The tray is not told about the target: it would open a microphone, and what the
     * microphone does is not what this test is asking.
     */
    @Test
    fun aVoiceNoteLandsOnTheBlockItWasSpokenAt() {
        val id = newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        Page.awaitElementMap(activity.webView)

        val index = Page.elementTexts(activity.webView).indexOfFirst {
            it.contains("second paragraph")
        }
        assertTrue("the second paragraph is not on the page", index >= 0)
        val box = Page.elementAt(activity.webView, index)
        val cx = ((box.getDouble("l") + box.getDouble("r")) / 2).toFloat()
        val cy = ((box.getDouble("t") + box.getDouble("b")) / 2).toFloat()

        val overlay = activity.overlayForTest()
            ?: throw AssertionError("the reader has no overlay")
        App.onMain { activity.setToolMode(ToolMode.VOICE) }
        App.onMain {
            overlay.onVoiceTarget = null // no microphone in this test
            overlay.pickVoiceTargetAtForTest(cx, cy)
        }

        val groups = awaitGroups(activity)
        assertEquals("the tap bound ${groups.size} blocks", 1, groups.size)
        val group = groups.single()
        assertTrue(
            "bound to the wrong block: ${group.elementRefs.map { it.text }}",
            group.elementRefs.any { it.text.contains("second paragraph") },
        )

        // What the tray hands back when the server answers `done`.
        val note = "the second paragraph needs an example"
        App.onMain {
            overlay.setVoiceNote(
                group.id,
                note,
                "blocker",
                """{"raw":"$note","seconds":2.4,"severity":"blocker"}""",
            )
        }
        // The card is the reader-visible half: the words have to reach the page, not just
        // the submission. It is drawn by the same JS that draws every other block.
        val deadline = System.currentTimeMillis() + 10_000
        var cards = emptyList<String>()
        while (System.currentTimeMillis() < deadline && cards.none { it.contains("needs an example") }) {
            cards = Page.voiceCards(activity.webView)
            if (cards.none { it.contains("needs an example") }) Thread.sleep(200)
        }
        assertTrue("the page never drew the note's card: $cards", cards.any { it.contains("needs an example") })

        App.onMain { activity.submitForTest() }

        val annotations = awaitAnnotations(id)
        assertEquals("the server stored ${annotations.length()} notes", 1, annotations.length())
        val stored = annotations.getJSONObject(0)
        assertEquals("the words changed on the way", note, stored.optString("recognized_text"))
        assertEquals("the note lost its provenance", "voice", stored.optString("source"))
        // Severity is not a field of the annotation: it rides inside the voice payload,
        // which is what an agent reads to know whether the reader was stopping the work.
        val voice = stored.getJSONObject("voice")
        assertEquals("what was heard was not kept", note, voice.optString("raw"))
        assertEquals("a blocker became a note", "blocker", voice.optString("severity"))
        val anchored = stored.getJSONObject("anchor").getJSONArray("elements")
        assertTrue(
            "the anchor is not the block the reader aimed at: $anchored",
            (0 until anchored.length()).any {
                anchored.getJSONObject(it).getString("text").contains("second paragraph")
            },
        )
    }

    /**
     * A picture in the brief is a picture the page shows.
     *
     * Three things have to line up for this: the create request carrying bytes, the
     * renderer pointing `image/<name>` at the session's own route (a bare relative path
     * resolved against `/session/{id}` and 404ed), and that route serving the bytes back.
     * A broken URL still leaves an `img` element, so only the decoded width proves it.
     */
    @Test
    fun aPictureInTheBriefIsServedFromTheSession() {
        val name = "shot.png"
        val id = ServerUnderTest.createSessionWithImage(
            title = "A picture",
            markdown = "# A picture\n\nBelow is the artefact itself.\n\n![$name](image/$name)\n",
            name = name,
            bytes = pixelPng,
        ).also { sessions += it }

        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, "A picture"))
        open(activity, "A picture")
        Page.awaitElementMap(activity.webView)

        val deadline = System.currentTimeMillis() + 20_000
        var images = emptyList<Page.DocumentImage>()
        while (System.currentTimeMillis() < deadline && images.none { it.naturalWidth > 0 }) {
            images = Page.images(activity.webView)
            if (images.none { it.naturalWidth > 0 }) Thread.sleep(200)
        }
        assertEquals("the page shows ${images.size} pictures", 1, images.size)
        val image = images.single()
        assertEquals(
            "the picture is not served from its own session",
            "/api/sessions/$id/image/$name",
            image.src,
        )
        assertTrue("the picture never decoded: $image", image.naturalWidth > 0)
    }

    /**
     * A diagram is drawn, not left as an empty box.
     *
     * Mermaid runs in the page: the server sends a payload and `bootstrap.js` turns it into
     * an SVG. No server-side test can prove that happened, and when it fails the reader is
     * left with a blank rectangle where the picture should be.
     */
    @Test
    fun aDiagramIsDrawnInThePage() {
        ServerUnderTest.createSession(
            "A diagram",
            "# A diagram\n\n\u0060\u0060\u0060mermaid\nflowchart LR\n  A[Start] --> B[Done]\n\u0060\u0060\u0060\n",
        ).also { sessions += it }

        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, "A diagram"))
        open(activity, "A diagram")
        Page.awaitElementMap(activity.webView)

        val deadline = System.currentTimeMillis() + 20_000
        var diagrams = emptyList<Pair<Int, Int>>()
        while (System.currentTimeMillis() < deadline && diagrams.isEmpty()) {
            diagrams = Page.drawnDiagrams(activity.webView)
            if (diagrams.isEmpty()) Thread.sleep(200)
        }
        assertEquals("the page drew ${diagrams.size} diagrams", 1, diagrams.size)
        val (width, height) = diagrams.single()
        assertTrue("the diagram was drawn at ${width}x$height", width > 0 && height > 0)
    }

    /**
     * An agent that rewrites the brief while the reader has it open is seen.
     *
     * The update arrives as a WebSocket `version_updated` and the app reloads the page. If
     * either half breaks, the reader keeps reviewing the old document and nobody notices
     * -- which is worse than a page that refuses to load, because it looks fine.
     */
    @Test
    fun anAgentUpdateReachesTheOpenPage() {
        val id = newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        Page.awaitElementMap(activity.webView)

        // The push can only arrive once the app's socket is up; the server's own count of
        // tablet clients is the honest way to know, and it is the same signal the
        // screenshot test needs.
        awaitTabletListening(id)

        val added = "The agent added this paragraph."
        ServerUnderTest.updateContent(id, "# $title\n\n$added\n")

        val deadline = System.currentTimeMillis() + 20_000
        var texts = emptyList<String>()
        while (System.currentTimeMillis() < deadline && texts.none { it.contains(added) }) {
            // The page is being replaced while this runs: a script evaluated into the old
            // document can be dropped, which is not a failure of the update.
            texts = runCatching { Page.elementTexts(activity.webView) }.getOrElse { emptyList() }
            if (texts.none { it.contains(added) }) Thread.sleep(200)
        }
        assertTrue("the page never showed the update: $texts", texts.any { it.contains(added) })
    }

    /**
     * A review that ends somewhere else takes the tablet off the page.
     *
     * The agent submits from its own machine; the tablet is holding a page whose review is
     * over. Leaving it there invites more notes into a session that will refuse them.
     */
    @Test
    fun aSessionSubmittedElsewhereSendsTheTabletBackToTheList() {
        val id = newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        Page.awaitElementMap(activity.webView)
        awaitTabletListening(id)

        ServerUnderTest.submit(id)

        val deadline = System.currentTimeMillis() + 20_000
        while (System.currentTimeMillis() < deadline && activity.showingReaderForTest()) {
            Thread.sleep(200)
        }
        assertTrue("the tablet kept a finished page open", !activity.showingReaderForTest())
        assertTrue(
            "the list came back empty: ${titles(activity)}",
            activity.sessionsForTest().any { it.title == title },
        )
    }

    /**
     * The agent can ask the tablet what it is showing, and gets a picture.
     *
     * This runs the whole return path: the server's ask over the WebSocket, the app
     * compositing the page and the pen overlay off the main thread, the upload, and the
     * route back. Two failures it is aimed at: `delivered` counting the agent's own
     * observer instead of a tablet, and a blank page sent as a picture.
     */
    @Test
    fun theAgentCanAskTheTabletForAPicture() {
        val id = newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        Page.awaitElementMap(activity.webView)

        val ask = awaitTabletListening(id)
        val requestId = ask.getString("id")
        val deadline = System.currentTimeMillis() + 25_000
        var png: ByteArray? = null
        while (System.currentTimeMillis() < deadline && png == null) {
            png = ServerUnderTest.screenshot(id, requestId)
            if (png == null) Thread.sleep(300)
        }
        val picture = png ?: throw AssertionError("the tablet never sent a picture")
        assertEquals("the picture is not a PNG", 0x89.toByte(), picture[0])
        assertEquals("the picture is not a PNG", 'P'.code.toByte(), picture[1])
        assertEquals("the picture is not a PNG", 'N'.code.toByte(), picture[2])
        assertEquals("the picture is not a PNG", 'G'.code.toByte(), picture[3])
        assertTrue("the picture is ${picture.size} bytes, which is not a page", picture.size > 1_000)
    }

    /**
     * A server that is not there is said so, not crashed on.
     *
     * The reader's tablet is a tablet: it gets turned off, moved to another network and
     * pointed at a machine that is asleep. Every one of those used to be an exception in
     * a coroutine; the connection line is what the reader is left with.
     */
    @Test
    fun aServerThatIsNotThereIsSaidSo() {
        App.useServer("http://10.0.2.2:1")
        val activity = launch()

        val deadline = System.currentTimeMillis() + 25_000
        var status = ""
        while (System.currentTimeMillis() < deadline && !status.startsWith("Unreachable")) {
            status = activity.statusForTest()
            if (!status.startsWith("Unreachable")) Thread.sleep(250)
        }
        assertTrue(
            "the app never said the server was unreachable; it said \"$status\"",
            status.startsWith("Unreachable"),
        )
        assertTrue("the list should be empty, not stale", activity.sessionsForTest().isEmpty())
    }

    /** The back button is how a reader leaves the page; it has to land on the list. */
    @Test
    fun goingBackFromTheReaderReturnsToTheList() {
        newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        assertTrue("the reader never showed", activity.showingReaderForTest())

        App.onMain { activity.onBackPressedDispatcher.onBackPressed() }

        val deadline = System.currentTimeMillis() + 10_000
        while (System.currentTimeMillis() < deadline && activity.showingReaderForTest()) {
            Thread.sleep(100)
        }
        assertTrue("back did not leave the reader", !activity.showingReaderForTest())
        assertTrue(
            "the list came back empty after leaving: ${titles(activity)}",
            activity.sessionsForTest().any { it.title == title },
        )
    }

    /**
     * The tray's controls are buttons that can be seen.
     *
     * The bug this pins: the tray used plain `<Button>`s, which AppCompat inflates as
     * `MaterialButton`, took the dark Material background and painted the label black on
     * black. Robolectric sees the class; a device sees the class too, and this is the
     * layer that runs on every push.
     */
    @Test
    fun theVoiceTrayShowsRealButtons() {
        newSession()
        val activity = launch()
        assertTrue("the session list did not show it", awaitSession(activity, title))
        open(activity, title)
        Page.awaitElementMap(activity.webView)
        App.onMain { activity.setToolMode(ToolMode.VOICE) }

        var states = emptyList<Pair<String, android.view.View>>()
        App.onMain {
            states = listOf(
                "Done" to activity.trayButtonForTest(R.id.voiceDone),
                "Discard" to activity.trayButtonForTest(R.id.voiceDiscard),
            )
        }
        for ((name, view) in states) {
            assertTrue(
                "$name is a ${view.javaClass.name}, and a MaterialButton paints itself " +
                    "dark with a dark label",
                view is androidx.appcompat.widget.AppCompatButton,
            )
            assertTrue("$name is not shown", view.isShown)
            assertTrue("$name has no background", view.background != null)
        }
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

    /**
     * Wait until the server can see this app's socket, which is when a push can arrive.
     *
     * The socket connects asynchronously after the page opens. The server counts the
     * sockets that connected with `?client=tablet`, so asking for a screenshot is a
     * truthful probe: a "not delivered" answer is a retry, not a failure, and the server
     * reuses the pending request's id.
     */
    private fun awaitTabletListening(id: String, timeoutMs: Long = 20_000): org.json.JSONObject {
        val deadline = System.currentTimeMillis() + timeoutMs
        var ask = ServerUnderTest.requestScreenshot(id)
        while (!ask.optBoolean("delivered")) {
            if (System.currentTimeMillis() > deadline) {
                throw AssertionError("no tablet ever listened for the session: $ask")
            }
            Thread.sleep(300)
            ask = ServerUnderTest.requestScreenshot(id)
        }
        return ask
    }

    /** The groups the overlay is holding, once the page's search has answered. */
    private fun awaitGroups(activity: MainActivity, timeoutMs: Long = 10_000): List<BindGroup> {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            val groups = activity.boundGroupsForTest()
            if (groups.isNotEmpty()) return groups
            Thread.sleep(100)
        }
        return emptyList()
    }

    /**
     * The annotations the server ends up holding, once it has the submission.
     *
     * `/result` blocks while the session is still active, so this waits for the submit to
     * land first and then asks once; there is no retry of a blocked poll in here.
     */
    private fun awaitAnnotations(id: String, timeoutMs: Long = 25_000): org.json.JSONArray {
        val deadline = System.currentTimeMillis() + timeoutMs
        var last = ""
        while (System.currentTimeMillis() < deadline) {
            val result = try {
                ServerUnderTest.result(id)
            } catch (error: Throwable) {
                last = error.message ?: error.toString()
                Thread.sleep(300)
                continue
            }
            val annotations = result.optJSONArray("annotations") ?: org.json.JSONArray()
            if (annotations.length() > 0) return annotations
            last = result.optString("status")
            Thread.sleep(300)
        }
        throw AssertionError("no annotations ever arrived; the session said \"$last\"")
    }

    /** A one-pixel PNG: enough for the server to accept it and the WebView to decode it. */
    private val pixelPng: ByteArray = android.util.Base64.decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
        android.util.Base64.NO_WRAP,
    )
}
