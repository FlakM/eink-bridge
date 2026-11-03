package com.flakm.einkbridge

import androidx.test.platform.app.InstrumentationRegistry
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.util.concurrent.TimeUnit

/**
 * The server the emulator talks to, running on the host.
 *
 * An instrumented test cannot spawn a process inside the emulator, so the harness starts
 * the real `eink-serve` before the suite and points it at a port. `10.0.2.2` is how the
 * emulator reaches the host's loopback.
 *
 * Override with `-e serverUrl http://10.0.2.2:4123` when the runner picks a port.
 */
internal object ServerUnderTest {
    val url: String by lazy {
        val arguments = InstrumentationRegistry.getArguments()
        arguments.getString("serverUrl")?.trimEnd('/') ?: "http://10.0.2.2:3333"
    }

    private val client = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    /** Create a session and return its id. A title, as the CLI sends one. */
    fun createSession(title: String, markdown: String): String {
        val body = JSONObject().put("content", markdown).put("title", title)
        val request = Request.Builder()
            .url("$url/api/sessions")
            .post(body.toString().toRequestBody("application/json".toMediaType()))
            .build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "create session failed: ${response.code}" }
            val body = JSONObject(response.body!!.string())
            return body.getString("id")
        }
    }

    /**
     * Create a session that carries a picture, the way `push --image` does.
     *
     * The bytes travel in the request, never a path: the server will not open a file for a
     * caller. It writes them under the session and rewrites the markdown's `image/<name>`
     * reference to the session's own route.
     */
    fun createSessionWithImage(
        title: String,
        markdown: String,
        name: String,
        bytes: ByteArray,
    ): String {
        val body = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("content", markdown)
            .addFormDataPart("title", title)
            .addFormDataPart("image", name, bytes.toRequestBody("image/png".toMediaType()))
            .build()
        val request = Request.Builder().url("$url/api/sessions").post(body).build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "create session with image failed: ${response.code}" }
            return JSONObject(response.body!!.string()).getString("id")
        }
    }

    fun sessionStatus(id: String): String {
        val request = Request.Builder().url("$url/api/sessions/$id").build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "read session failed: ${response.code}" }
            return JSONObject(response.body!!.string()).getString("status")
        }
    }

    fun result(id: String): JSONObject {
        val request = Request.Builder().url("$url/api/sessions/$id/result").build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "read result failed: ${response.code}" }
            return JSONObject(response.body!!.string())
        }
    }

    /**
     * Ask the tablet for a picture of its page, the way `eink-review screenshot` does.
     *
     * `delivered` is the server's own answer to "is a tablet listening": it counts the
     * sockets that connected with `?client=tablet` and not the agent's observer.
     */
    fun requestScreenshot(id: String): JSONObject {
        val request = Request.Builder()
            .url("$url/api/sessions/$id/screenshot")
            .post(ByteArray(0).toRequestBody(null))
            .build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "screenshot request failed: ${response.code}" }
            return JSONObject(response.body!!.string())
        }
    }

    /** The picture, once the tablet has sent it; null while it is still working (404). */
    fun screenshot(id: String, requestId: String): ByteArray? {
        val request = Request.Builder()
            .url("$url/api/sessions/$id/screenshot/$requestId")
            .build()
        client.newCall(request).execute().use { response ->
            if (response.code == 404) return null
            check(response.isSuccessful) { "read screenshot failed: ${response.code}" }
            return response.body!!.bytes()
        }
    }

    /** End the review from elsewhere, the way the CLI or another agent would. */
    fun submit(id: String) {
        val body = JSONObject().put("annotations", org.json.JSONArray())
        val request = Request.Builder()
            .url("$url/api/sessions/$id/submit")
            .post(body.toString().toRequestBody("application/json".toMediaType()))
            .build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "submit failed: ${response.code}" }
        }
    }

    /** Replace the brief, the way `eink-review update` does. */
    fun updateContent(id: String, markdown: String) {
        val request = Request.Builder()
            .url("$url/api/sessions/$id/content")
            .put(markdown.toRequestBody("text/plain".toMediaType()))
            .build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "update content failed: ${response.code}" }
        }
    }

    /** The detail record, which is where a submitted session's annotations live. */    fun detail(id: String): JSONObject {
        val request = Request.Builder().url("$url/api/sessions/$id").build()
        client.newCall(request).execute().use { response ->
            check(response.isSuccessful) { "read session failed: ${response.code}" }
            return JSONObject(response.body!!.string())
        }
    }

    fun remove(id: String) {
        val cancel = Request.Builder().url("$url/api/sessions/$id").delete().build()
        runCatching { client.newCall(cancel).execute().close() }
        val gone = Request.Builder().url("$url/api/sessions/$id?remove=true").delete().build()
        runCatching { client.newCall(gone).execute().close() }
    }

    fun healthy(): Boolean = runCatching {
        val request = Request.Builder().url("$url/api/health").build()
        client.newCall(request).execute().use { it.isSuccessful }
    }.getOrDefault(false)
}
