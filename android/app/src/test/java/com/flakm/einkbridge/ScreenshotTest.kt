package com.flakm.einkbridge

import android.graphics.Color
import android.view.View
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Request
import okhttp3.Response
import okhttp3.ResponseBody.Companion.toResponseBody
import okio.Buffer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/**
 * The agent's window onto the tablet.
 *
 * Three things can go wrong quietly here: the picture goes to a URL the server does not
 * serve (a 404 the agent reads as "no tablet"), a blank page is uploaded and the agent
 * concludes the document is empty, or a failed capture is swallowed and the agent waits
 * out its timeout. Each is pinned.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class ScreenshotTest {

    private class Recording {
        var request: Request? = null
        val client: OkHttpClient = OkHttpClient.Builder()
            .addInterceptor { chain ->
                request = chain.request()
                Response.Builder()
                    .request(chain.request())
                    .protocol(Protocol.HTTP_1_1)
                    .code(204)
                    .message("No Content")
                    .body("".toResponseBody(null))
                    .build()
            }
            .build()

        fun body(): String = Buffer().also { request!!.body!!.writeTo(it) }.readUtf8()
    }

    private fun view(colour: Int? = null) =
        View(RuntimeEnvironment.getApplication()).apply {
            if (colour != null) {
                layout(0, 0, 200, 200)
                setBackgroundColor(colour)
            }
        }

    @Test
    fun `a screenshot is sent to the request the server named`() {
        val recording = Recording()
        val png = byteArrayOf(0x89.toByte(), 'P'.code.toByte(), 'N'.code.toByte(), 'G'.code.toByte())

        val code = uploadScreenshot(recording.client, "http://10.0.0.5:3333/", "abc", "0f1e2d", png)

        assertEquals(204, code)
        val request = recording.request!!
        assertEquals("PUT", request.method)
        assertEquals("http://10.0.0.5:3333/api/sessions/abc/screenshot/0f1e2d", request.url.toString())
        assertEquals("image/png", request.body?.contentType().toString())
    }

    @Test
    fun `a failed capture is reported under the same request, as json with the reason`() {
        val recording = Recording()

        val code = reportScreenshotFailure(recording.client, "http://10.0.0.5:3333", "abc", "0f1e2d", "blank page")

        assertEquals(204, code)
        val request = recording.request!!
        assertEquals("PUT", request.method)
        assertEquals("http://10.0.0.5:3333/api/sessions/abc/screenshot/0f1e2d", request.url.toString())
        assertTrue(request.body?.contentType().toString().startsWith("application/json"))
        assertEquals("""{"error":"blank page"}""", recording.body())
    }

    @Test
    fun `a view that was never laid out is not a screenshot`() {
        assertNull(captureShowing(view(), null))
    }

    @Test
    fun `an untouched white page is not sent`() {
        assertNull(captureShowing(view(Color.WHITE), null))
    }

    @Test
    fun `a page with content in it is sent as a png`() {
        val bitmap = captureShowing(view(Color.BLACK), null)
        val png = bitmap!!.toPng()
        assertTrue(png.size > 8)
        assertEquals(0x89.toByte(), png[0])
        assertEquals('P'.code.toByte(), png[1])
    }

    @Test
    fun `ink is drawn over the page`() {
        val inkOnly = captureShowing(view(Color.BLACK), null)!!.toPng()
        // The ink is a capture of its own: a black page with nothing drawn, decoded and
        // laid over a white one, must leave the white page black where the ink was.
        val bitmap = captureShowing(view(Color.WHITE), inkOnly)!!
        assertEquals(Color.BLACK, bitmap.getPixel(10, 10))
        bitmap.recycle()
    }
}
