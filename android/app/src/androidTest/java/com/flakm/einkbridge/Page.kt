package com.flakm.einkbridge

import android.webkit.WebView
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit

/**
 * Waiting for a WebView, which has no idling resource of its own.
 *
 * `evaluateJavascript` must run on the UI thread and answers asynchronously, so every
 * helper here posts, waits on a latch with a deadline, and fails with what it saw.
 */
internal object Page {

    /** Run an expression in the page and return its JSON-decoded value, or null on timeout. */
    fun eval(webView: WebView, script: String, timeoutMs: Long = 5_000): String? {
        val latch = CountDownLatch(1)
        var result: String? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            webView.evaluateJavascript(script) { value ->
                result = value
                latch.countDown()
            }
        }
        latch.await(timeoutMs, TimeUnit.MILLISECONDS)
        return result?.let(::unquote)
    }

    /** `evaluateJavascript` returns a JSON string; strip the outer quotes of a string result. */
    private fun unquote(raw: String): String {
        if (raw.length >= 2 && raw.startsWith("\"") && raw.endsWith("\"")) {
            return raw.substring(1, raw.length - 1)
                .replace("\\\"", "\"")
                .replace("\\\\", "\\")
                .replace("\\n", "\n")
        }
        return raw
    }

    /** Wait until the page has built its element map: the document is rendered and its JS ran. */
    fun awaitElementMap(webView: WebView, timeoutMs: Long = 15_000): Int {
        val deadline = System.currentTimeMillis() + timeoutMs
        var count = -1
        while (System.currentTimeMillis() < deadline) {
            count = eval(
                webView,
                "JSON.stringify(window.__einkElementMap ? window.__einkElementMap.length : -1)",
            )?.toIntOrNull() ?: -1
            if (count > 0) return count
            Thread.sleep(200)
        }
        throw AssertionError("the page never built an element map (last count $count)")
    }

    /** The text of every annotatable element, in page order. */
    fun elementTexts(webView: WebView): List<String> =
        elements(webView).let { array ->
            (0 until array.length()).map { array.getJSONObject(it).getString("text") }
        }

    /**
     * Every diagram on the page, as the size it was drawn at.
     *
     * A diagram is drawn by `bootstrap.js` with mermaid, in the page. A missing or broken
     * asset leaves the block in place with an empty body, which is what this count sees.
     */
    fun drawnDiagrams(webView: WebView): List<Pair<Int, Int>> {
        val raw = eval(
            webView,
            """
            JSON.stringify(Array.prototype.map.call(
              document.querySelectorAll('.diagram-block svg'),
              function (svg) {
                var r = svg.getBoundingClientRect();
                return [Math.round(r.width), Math.round(r.height)];
              }
            ))
            """.trimIndent(),
        ) ?: throw AssertionError("could not read the page's diagrams")
        val array = JSONArray(raw)
        return (0 until array.length()).map { index ->
            val size = array.getJSONArray(index)
            size.getInt(0) to size.getInt(1)
        }
    }

    /**
     * How many blocks the page shows, counted the way the element map counts them.
     *
     * If the map's own build throws partway, the count it reports is smaller than this and
     * the blocks it skipped can no longer be annotated — a failure that otherwise looks
     * like a page that simply has fewer paragraphs.
     */
    fun blockCount(webView: WebView): Int {
        val raw = eval(
            webView,
            """
            (function () {
              var content = document.getElementById('content');
              if (!content) return -1;
              return content.querySelectorAll(
                'h1, h2, h3, p, li, pre, blockquote, td, th, table, ul, ol, img, .diagram-block, .snippet-block, .steps-block, .diff-block'
              ).length;
            })()
            """.trimIndent(),
        ) ?: return -1
        return raw.toIntOrNull() ?: -1
    }

    /** The spoken-note cards the page has drawn, by the words they carry. */
    fun voiceCards(webView: WebView): List<String> {
        val raw = eval(
            webView,
            """
            JSON.stringify(Array.prototype.map.call(
              document.querySelectorAll('.voice-block .voice-text'),
              function (el) { return (el.textContent || '').trim(); }
            ))
            """.trimIndent(),
        ) ?: throw AssertionError("could not read the voice cards")
        val array = JSONArray(raw)
        return (0 until array.length()).map { array.getString(it) }
    }

    /** A picture the page is showing, and whether its bytes actually arrived. */
    data class DocumentImage(val src: String, val complete: Boolean, val naturalWidth: Int)

    /**
     * Every `img` on the page.
     *
     * `naturalWidth` is what makes this worth asking: a broken URL still leaves an `img`
     * element behind, so only a decoded picture proves the route served it.
     */
    fun images(webView: WebView): List<DocumentImage> {
        val raw = eval(
            webView,
            """
            JSON.stringify(Array.prototype.map.call(document.images, function (img) {
              return {
                src: img.getAttribute('src') || '',
                complete: !!img.complete,
                naturalWidth: img.naturalWidth || 0
              };
            }))
            """.trimIndent(),
        ) ?: throw AssertionError("could not read the page's images")
        val array = JSONArray(raw)
        return (0 until array.length()).map { index ->
            val image = array.getJSONObject(index)
            DocumentImage(
                src = image.getString("src"),
                complete = image.getBoolean("complete"),
                naturalWidth = image.getInt("naturalWidth"),
            )
        }
    }

    /** The elements of the page, as the element map sees them. */
    fun elements(webView: WebView): JSONArray {
        val raw = eval(webView, "JSON.stringify(window.__einkElementMap || [])")
            ?: throw AssertionError("could not read the element map")
        return JSONArray(raw)
    }

    /**
     * Where an element is on the device's screen, for a tap.
     *
     * The page is laid out in CSS pixels and drawn into a WebView of some other width, so
     * the mapping is device px per CSS px = `webView.width / viewport.width` — the same
     * number the app's ink transform uses. `visualViewport.scale` alone is not it: on this
     * emulator it reads 1 while the page is drawn at a third of a CSS pixel per device px.
     */
    fun centerOnScreen(webView: WebView, elementIndex: Int): Pair<Float, Float> {
        val raw = eval(
            webView,
            """
            (function () {
              var el = document.querySelectorAll(
                'h1, h2, h3, p, li, pre, blockquote, td, th, table, ul, ol, img, .diagram-block, .snippet-block, .steps-block, .diff-block'
              )[$elementIndex];
              if (!el) return JSON.stringify(null);
              var r = el.getBoundingClientRect();
              var vv = window.visualViewport;
              return JSON.stringify({
                x: (r.left + r.right) / 2,
                y: (r.top + r.bottom) / 2,
                vw: vv ? vv.width : window.innerWidth,
                vh: vv ? vv.height : window.innerHeight,
                scale: vv ? vv.scale : 1
              });
            })()
            """.trimIndent(),
        ) ?: throw AssertionError("could not locate element $elementIndex")
        val point = org.json.JSONObject(raw)
        var size = IntArray(2)
        var origin = IntArray(2)
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            webView.getLocationOnScreen(origin)
            size = intArrayOf(webView.width, webView.height)
        }
        val viewportWidth = point.optDouble("vw", size[0].toDouble()).toFloat().takeIf { it > 0f }
            ?: size[0].toFloat()
        val perCss = size[0].toFloat() / viewportWidth
        return (origin[0] + point.getDouble("x").toFloat() * perCss) to
            (origin[1] + point.getDouble("y").toFloat() * perCss)
    }

    /**
     * Which element the page would pick for a point, as the ink and the voice aim both do.
     *
     * This is the function that did not exist: `findNearestElement` called it by name and
     * got nothing, so every tap in voice mode silently missed. Asserting it directly is
     * what pins it — a coordinate-driven tap proves the wiring but not the answer.
     */
    fun nearestAt(webView: WebView, docX: Double, docY: Double): Int? {
        val raw = eval(webView, "window.__einkFindNearestElement($docX, $docY)") ?: return null
        val array = JSONArray(raw)
        if (array.length() == 0) return null
        return array.getJSONObject(0).getInt("i")
    }

    /** The element map, as raw JSON, for the coordinates the page itself computed. */
    fun elementAt(webView: WebView, index: Int): org.json.JSONObject = elements(webView).getJSONObject(index)

    /** Which elements the tablet has bound to a note, by the attribute the JS bridge sets. */
    fun boundElements(webView: WebView): List<String> {
        val raw = eval(
            webView,
            """
            JSON.stringify(Array.prototype.map.call(
              document.querySelectorAll('[data-bind-color]'),
              function (el) { return el.tagName + ': ' + (el.textContent || '').trim().slice(0, 40); }
            ))
            """.trimIndent(),
        ) ?: return emptyList()
        val array = JSONArray(raw)
        return (0 until array.length()).map { array.getString(it) }
    }
}
