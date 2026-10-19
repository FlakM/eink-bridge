package com.flakm.einkbridge

import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.InputStream
import java.net.ServerSocket
import java.util.concurrent.atomic.AtomicInteger

/**
 * A minimal keep-alive HTTP responder.
 *
 * Hand-rolled rather than pulling in MockWebServer: the point of these tests is connection
 * reuse, so the server has to speak HTTP/1.1 well enough for OkHttp to keep the socket
 * open -- which is exactly what a response with a `Content-Length` and no `Connection:
 * close` does.
 */
private class TinyHttpServer {
    private val socket = ServerSocket(0)
    private val codes = ArrayDeque<Int>()
    val requests = AtomicInteger(0)
    val port: Int get() = socket.localPort

    private val thread = Thread {
        try {
            while (!socket.isClosed) {
                val connection = socket.accept()
                Thread {
                    connection.use {
                        val input = it.getInputStream()
                        while (true) {
                            val request = readRequest(input) ?: break
                            requests.incrementAndGet()
                            val code = synchronized(codes) { codes.removeFirstOrNull() ?: 200 }
                            val body = """{"version":2}"""
                            val response = buildString {
                                append("HTTP/1.1 $code ${if (code in 200..299) "OK" else "ERR"}\r\n")
                                append("Content-Type: application/json\r\n")
                                append("Content-Length: ${body.toByteArray().size}\r\n")
                                append("\r\n")
                                append(body)
                            }
                            it.getOutputStream().write(response.toByteArray())
                            it.getOutputStream().flush()
                        }
                    }
                }.start()
            }
        } catch (_: Exception) {
            // closed by the test
        }
    }.apply { isDaemon = true; start() }

    fun answer(code: Int) = synchronized(codes) { codes.addLast(code); Unit }

    /** Returns the request headers, or null when the connection closed. */
    private fun readRequest(input: InputStream): String? {
        val head = StringBuilder()
        var last = -1
        while (true) {
            val byte = input.read()
            if (byte == -1) return if (head.isEmpty()) null else head.toString()
            head.append(byte.toChar())
            if (head.endsWith("\r\n\r\n")) break
            last = byte
        }
        val headers = head.toString()
        val length = Regex("(?i)content-length:\\s*(\\d+)").find(headers)?.groupValues?.get(1)?.toInt()
        check(!"transfer-encoding" .toRegex(RegexOption.IGNORE_CASE).containsMatchIn(headers)) {
            "this stub does not read chunked bodies: $headers"
        }
        if (length != null) {
            var read = 0
            while (read < length) {
                val skipped = input.read(ByteArray(minOf(8192, length - read)))
                if (skipped == -1) break
                read += skipped
            }
        }
        return headers
    }

    fun stop() {
        try {
            socket.close()
        } catch (_: Exception) {
        }
    }

    @Suppress("unused")
    private val unusedLast = 0
}

/**
 * The submit path is where a review is won or lost, and none of it had a test.
 *
 * The dangerous mistake is not a failure -- it is a *success* reported when the server
 * never took the review: the caller clears the reader's ink on success, so a wrong answer
 * deletes the notes and saves nothing.
 */
class SubmissionManagerTest {
    private val server = TinyHttpServer()
    private lateinit var client: OkHttpClient
    private lateinit var manager: SubmissionManager
    private var serverUrl = ""

    private fun start(): SubmissionManagerTest {
        serverUrl = "http://127.0.0.1:${server.port}"
        client = OkHttpClient()
        manager = SubmissionManager(client)
        return this
    }

    @After
    fun stop() = server.stop()

    private fun submit() = kotlinx.coroutines.runBlocking {
        manager.submit(serverUrl, "abc", "[]", null, null)
    }

    private fun update() = kotlinx.coroutines.runBlocking {
        manager.requestUpdate(serverUrl, "abc", "[]", 2)
    }

    @Test
    fun aSubmitTheServerTookIsASuccess() {
        start(); server.answer(200)
        val result = submit()
        assertTrue("200 means the review is safe to clear locally: $result", result.isSuccessful)
        assertFalse(result.isConflict)
    }

    @Test
    fun aConflictIsNotASuccess() {
        // 409 is "already submitted or expired". Treating it as success would clear the
        // reader's ink for a review the server never took.
        start(); server.answer(409)
        val result = submit()
        assertFalse("409 must not clear local ink", result.isSuccessful)
        assertTrue(result.isConflict)
    }

    @Test
    fun aServerErrorIsNeitherSuccessNorConflict() {
        start(); server.answer(500)
        val result = submit()
        assertFalse(result.isSuccessful)
        assertFalse(result.isConflict)
        assertEquals(500, (result as SubmitResult.Http).code)
    }

    @Test
    fun theClassificationCoversTheWholeRangeOfCodes() {
        for (code in listOf(200, 204, 299)) {
            assertTrue("$code is a success", SubmitResult.Http(code).isSuccessful)
        }
        // 1xx is interim and 3xx is not the server taking the review: only 2xx clears ink.
        for (code in listOf(199, 300, 400, 404, 409, 500, 503)) {
            assertFalse("$code is not a success", SubmitResult.Http(code).isSuccessful)
        }
        assertTrue(SubmitResult.Http(409).isConflict)
        assertFalse(SubmitResult.Http(200).isConflict)
        assertFalse(SubmitResult.Failure(RuntimeException("boom")).isSuccessful)
    }

    @Test
    fun anUnreachableServerIsAFailureRatherThanAnException() {
        start()
        val result = kotlinx.coroutines.runBlocking {
            manager.submit("http://127.0.0.1:1", "abc", "[]", null, null)
        }
        assertTrue("a dead server is a Failure: $result", result is SubmitResult.Failure)
        assertFalse(result.isSuccessful)
    }

    @Test
    fun submitReusesItsConnection() {
        // A response body that is never closed holds its connection; five submits must end
        // with a pool of one, not five.
        start()
        repeat(5) { server.answer(200) }
        repeat(5) { submit() }
        val pooled = client.connectionPool.connectionCount()
        assertTrue("five submits leaked connections (pool=$pooled)", pooled <= 2)
        assertEquals(5, server.requests.get())
    }

    @Test
    fun requestUpdateReusesItsConnection() {
        // The hot path: the reader taps send for every annotation.
        start()
        repeat(5) { server.answer(200) }
        repeat(5) { update() }
        val pooled = client.connectionPool.connectionCount()
        assertTrue("five updates leaked connections (pool=$pooled)", pooled <= 2)
    }

    @Test
    fun aRequestUpdateReportsWhetherTheServerTookIt() {
        start(); server.answer(200)
        assertTrue(update())
        server.answer(422)
        assertFalse("a rejected update must not look sent", update())
    }

    @Test
    fun inkFarFromEveryParagraphIsSentWithoutAnAnchor() {
        // End to end for the proximity cap: the stroke is still submitted -- it is just
        // not attributed to text it was not written about.
        start()
        val elements = listOf(
            ElementEntry(0, "P", null, "s", 100f, 140f, 20f, 900f, "the retry loop")
        )
        val json = manager.buildAnnotationsJson(
            listOf(Stroke(listOf(5000f to 9000f), 3f)),
            emptyList(),
            elements,
        )
        val group = org.json.JSONArray(json).getJSONObject(0)
        assertFalse("no anchor for ink nobody bound", group.has("anchor"))
        assertEquals(1, group.getJSONArray("strokes").length())
    }

    @Test
    fun aBoundStrokeKeepsItsAnchorAndAddsAProximityOne() {
        start()
        val elements = listOf(
            ElementEntry(0, "P", null, "s", 100f, 140f, 20f, 900f, "the retry loop"),
            ElementEntry(1, "H2", null, "s", 1000f, 1040f, 20f, 900f, "the next section"),
        )
        val bound = BindGroup(1, 0, setOf(0), listOf(0), emptyList(), 200f, 120f)
        val json = manager.buildAnnotationsJson(
            listOf(Stroke(listOf(200f to 120f), 3f), Stroke(listOf(1020f to 1000f), 3f)),
            listOf(bound),
            elements,
        )
        val groups = org.json.JSONArray(json)
        assertEquals("one explicit and one by proximity", 2, groups.length())
        val types = (0 until groups.length()).map {
            groups.getJSONObject(it).getJSONObject("anchor").getString("type")
        }
        assertTrue("the bound stroke keeps its binding: $types", types.contains("explicit"))
        assertTrue("the free one is anchored by proximity: $types", types.contains("proximity"))
    }
}
