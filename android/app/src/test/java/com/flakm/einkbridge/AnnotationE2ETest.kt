package com.flakm.einkbridge

import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.*
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import java.util.concurrent.TimeUnit

/**
 * End-to-end annotation round-trip:
 *   1. Start the Rust server on a random port.
 *   2. Create a session via HTTP.
 *   3. Simulate stylus strokes with [MockPenController].
 *   4. Export strokes to PNG via [renderStrokesToPng].
 *   5. Submit the session with the PNG annotation.
 *   6. Fetch the result and assert the annotation was received.
 *   7. (Optional) Feed the PNG to the `claude` CLI and assert it describes the annotation.
 *
 * Requirements:
 *   - The `eink-serve` binary must be on PATH or set via EINK_SERVE_BIN.
 *   - Step 7 requires the `claude` CLI to be on PATH (or CLAUDE_BIN env var); skipped otherwise.
 */
class AnnotationE2ETest {

    private lateinit var serverProcess: Process
    private lateinit var serverUrl: String
    private val client = OkHttpClient.Builder()
        .connectTimeout(10, TimeUnit.SECONDS)
        .readTimeout(15, TimeUnit.SECONDS)
        .build()

    private val servebin: String by lazy {
        val fromEnv = System.getenv("EINK_SERVE_BIN")
        if (fromEnv != null) return@lazy fromEnv
        val cargo = listOf(
            "${System.getProperty("user.home")}/.cache/cargo-build/ae/d5abd8d27e4eee/debug/eink-serve",
            "server/target/debug/eink-serve",
        ).firstOrNull { java.io.File(it).canExecute() }
        cargo ?: "eink-serve"
    }

    private val claudeBin: String? by lazy {
        System.getenv("CLAUDE_BIN")?.takeIf { java.io.File(it).canExecute() }
            ?: (System.getenv("PATH") ?: "").split(":")
                .map { java.io.File(it, "claude") }
                .firstOrNull { it.canExecute() }
                ?.absolutePath
    }

    @Before
    fun startServer() {
        assumeTrue("eink-serve binary not found at $servebin", java.io.File(servebin).canExecute())
        val tempDir = createTempDir("eink-e2e-")
        val socket = java.net.ServerSocket(0)
        val port = socket.localPort
        socket.close()

        serverUrl = "http://127.0.0.1:$port"
        serverProcess = ProcessBuilder(
            servebin,
            "--host", "127.0.0.1",
            "--port", port.toString(),
            "--state-dir", tempDir.absolutePath,
        )
            .redirectErrorStream(true)
            .start()

        repeat(20) {
            try {
                val req = Request.Builder().url("$serverUrl/api/health").build()
                if (client.newCall(req).execute().isSuccessful) return
            } catch (_: Exception) {}
            Thread.sleep(200)
        }
    }

    @After
    fun stopServer() {
        if (::serverProcess.isInitialized) serverProcess.destroyForcibly()
    }

    // ── Core round-trip ────────────────────────────────────────────────────────

    @Test
    fun annotationPngIsReceivedByServer() {
        val sessionId = createSession("E2E Annotation Test")

        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.simulateStroke(listOf(50f to 50f, 50f to 150f, 50f to 250f))
        mock.simulateStroke(listOf(50f to 250f, 100f to 250f, 150f to 250f))
        assertEquals(2, buf.size)

        val png = renderStrokesToPng(400, 400, buf.strokes)
        assertTrue("PNG must be non-empty", png.isNotEmpty())

        submitSession(sessionId, typedNotes = "", annotationPng = png)

        val result = fetchResult(sessionId)
        assertEquals("Submitted", result.getString("status"))
        assertTrue("Server must record the annotation image",
            result.getJSONArray("annotation_images").length() > 0)
    }

    @Test
    fun submissionWithoutAnnotationIsAccepted() {
        val sessionId = createSession("Plain Notes Test")
        submitSession(sessionId, typedNotes = "LGTM: looks good", annotationPng = null)
        val result = fetchResult(sessionId)
        assertEquals("Submitted", result.getString("status"))
        assertEquals("lgtm", result.optString("verdict"))
        assertEquals("LGTM: looks good", result.optString("typed_notes"))
    }

    // ── Claude CLI vision step (skipped when `claude` is not on PATH) ──────────

    @Test
    fun claudeCliCanDescribeAnnotation() {
        assumeTrue("claude CLI not found on PATH, skipping vision step", claudeBin != null)

        val sessionId = createSession("Claude CLI Vision Test")

        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.simulateStroke(listOf(100f to 50f, 100f to 150f))
        mock.simulateStroke(listOf(50f to 100f, 150f to 100f))
        val png = renderStrokesToPng(200, 200, buf.strokes)

        submitSession(sessionId, typedNotes = "", annotationPng = png)
        val result = fetchResult(sessionId)
        val images = result.getJSONArray("annotation_images")
        assertTrue(images.length() > 0)

        val pngBytes = client.newCall(
            Request.Builder().url("$serverUrl${images.getString(0)}").build()
        ).execute().body!!.bytes()

        val description = askClaudeViaCliWith(pngBytes)
        assertNotNull("claude CLI must return a description", description)
        assertTrue("Description should be non-empty", description!!.isNotBlank())
        println("claude says: $description")
    }

    // ── Helpers ────────────────────────────────────────────────────────────────

    private fun createSession(title: String): String {
        val body = "# $title\n\nE2E test content".toRequestBody("text/plain".toMediaType())
        val req = Request.Builder()
            .url("$serverUrl/api/sessions?title=${urlEncode(title)}")
            .post(body)
            .build()
        val resp = client.newCall(req).execute()
        assertTrue("Create session must succeed, got ${resp.code}", resp.isSuccessful)
        return JSONObject(resp.body!!.string()).getString("id")
    }

    private fun submitSession(sessionId: String, typedNotes: String, annotationPng: ByteArray?) {
        val builder = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("typed_notes", typedNotes)
        if (annotationPng != null) {
            builder.addFormDataPart(
                "annotation", "strokes.png",
                annotationPng.toRequestBody("image/png".toMediaType()),
            )
        }
        val req = Request.Builder()
            .url("$serverUrl/api/sessions/$sessionId/submit")
            .post(builder.build())
            .build()
        val resp = client.newCall(req).execute()
        assertTrue("Submit must succeed, got ${resp.code}", resp.isSuccessful)
    }

    private fun fetchResult(sessionId: String): JSONObject {
        val req = Request.Builder().url("$serverUrl/api/sessions/$sessionId/result").build()
        val resp = client.newCall(req).execute()
        assertTrue("Fetch result must succeed, got ${resp.code}", resp.isSuccessful)
        return JSONObject(resp.body!!.string())
    }

    private fun askClaudeViaCliWith(imageBytes: ByteArray): String? {
        val tmp = kotlin.io.path.createTempFile("annotation-", ".png").toFile()
        return try {
            tmp.writeBytes(imageBytes)
            val proc = ProcessBuilder(
                claudeBin!!,
                "-p", "Describe what you see drawn in this image in one sentence: ${tmp.absolutePath}",
            )
                .redirectErrorStream(true)
                .start()
            val output = proc.inputStream.bufferedReader().readText()
            proc.waitFor(30, TimeUnit.SECONDS)
            output.trim().takeIf { it.isNotBlank() }
        } finally {
            tmp.delete()
        }
    }

    private fun urlEncode(s: String): String = java.net.URLEncoder.encode(s, "UTF-8")
}
