package com.flakm.einkbridge

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.util.Log
import android.view.View
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONObject
import java.io.ByteArrayOutputStream

private const val TAG = "EinkShot"

/**
 * The page as the reader has it, with whatever they have drawn on it.
 *
 * Two surfaces are composited because two surfaces are what they see: the WebView draws
 * the document, and the pen overlay draws the ink above it. Photographing only the page
 * would answer "where are they scrolled" and lose the part that usually matters: which
 * line the note is beside.
 *
 * Must run on the main thread, like any View draw. Returns the bitmap rather than its
 * PNG so the encoding, which takes a noticeable fraction of a second for a full
 * screen, can happen off the thread that owns the pen. Null means there was nothing to
 * send: the caller should say so rather than let the agent wait.
 */
internal fun captureShowing(view: View, ink: ByteArray?): Bitmap? {
    val width = view.width
    val height = view.height
    if (width <= 0 || height <= 0) return null
    val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
    val canvas = Canvas(bitmap)
    view.draw(canvas)
    ink?.let { bytes ->
        runCatching { BitmapFactory.decodeByteArray(bytes, 0, bytes.size) }
            .getOrNull()?.let { drawn ->
                canvas.drawBitmap(drawn, 0f, 0f, null)
                drawn.recycle()
            }
    }
    // A WebView that has not drawn yet gives a white rectangle, which the agent would
    // read as "the page is blank". That is a failed capture, not a picture.
    if (bitmap.isBlank()) {
        Log.w(TAG, "capture produced a blank page, not sending it")
        bitmap.recycle()
        return null
    }
    return bitmap
}

/** PNG bytes of a capture; recycles the bitmap. Safe off the main thread. */
internal fun Bitmap.toPng(): ByteArray {
    val out = ByteArrayOutputStream()
    try {
        compress(Bitmap.CompressFormat.PNG, 100, out)
    } finally {
        recycle()
    }
    return out.toByteArray()
}

/** A page nobody has drawn on and that never painted is white, which is not a screenshot. */
private fun Bitmap.isBlank(): Boolean {
    val step = 16
    for (y in 0 until height step step) {
        for (x in 0 until width step step) {
            if (getPixel(x, y) != Color.WHITE) return false
        }
    }
    return true
}

internal fun screenshotUrl(serverUrl: String, sessionId: String, requestId: String): String =
    "${serverUrl.trimEnd('/')}/api/sessions/$sessionId/screenshot/$requestId"

/**
 * Send the answer to a screenshot request. The id came from the server, so it is used as
 * given; an unknown one is a 404 rather than something to repair.
 */
internal fun uploadScreenshot(
    client: OkHttpClient,
    serverUrl: String,
    sessionId: String,
    requestId: String,
    png: ByteArray,
): Int = client.newCall(
    Request.Builder()
        .url(screenshotUrl(serverUrl, sessionId, requestId))
        .put(png.toRequestBody("image/png".toMediaType()))
        .build()
).execute().use { response ->
    Log.i(TAG, "screenshot uploaded request=$requestId code=${response.code}")
    response.code
}

/**
 * Tell the server the picture could not be taken, so the agent hears why at once
 * instead of waiting out its timeout.
 */
internal fun reportScreenshotFailure(
    client: OkHttpClient,
    serverUrl: String,
    sessionId: String,
    requestId: String,
    reason: String,
): Int = client.newCall(
    Request.Builder()
        .url(screenshotUrl(serverUrl, sessionId, requestId))
        .put(
            JSONObject().put("error", reason).toString()
                .toRequestBody("application/json".toMediaType())
        )
        .build()
).execute().use { response ->
    Log.w(TAG, "screenshot request=$requestId failed: $reason (reported, code=${response.code})")
    response.code
}
