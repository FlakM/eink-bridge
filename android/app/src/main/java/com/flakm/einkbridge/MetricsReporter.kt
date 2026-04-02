package com.flakm.einkbridge

import android.util.Log
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicLong

private const val TAG = "EinkMetrics"

internal object MetricsReporter {
    private val ocrOk = AtomicLong(0)
    private val ocrErr = AtomicLong(0)
    private val submitOk = AtomicLong(0)
    private val submitErr = AtomicLong(0)
    private val sessionsViewed = AtomicLong(0)
    private val wsConnections = AtomicLong(0)

    private val client = OkHttpClient.Builder()
        .connectTimeout(3, TimeUnit.SECONDS)
        .readTimeout(3, TimeUnit.SECONDS)
        .build()

    fun recordOcr(success: Boolean) {
        if (success) ocrOk.incrementAndGet() else ocrErr.incrementAndGet()
    }

    fun recordSubmission(success: Boolean) {
        if (success) submitOk.incrementAndGet() else submitErr.incrementAndGet()
    }

    fun recordSessionView() {
        sessionsViewed.incrementAndGet()
    }

    fun recordWsConnection() {
        wsConnections.incrementAndGet()
    }

    suspend fun push(pushgatewayUrl: String) {
        if (pushgatewayUrl.isBlank()) return
        withContext(Dispatchers.IO) {
            try {
                val body = buildMetricsText()
                val url = "$pushgatewayUrl/metrics/job/eink-android/instance/android"
                val request = Request.Builder()
                    .url(url)
                    .post(body.toRequestBody("text/plain".toMediaType()))
                    .build()
                val resp = client.newCall(request).execute()
                Log.d(TAG, "metrics pushed: HTTP ${resp.code}")
            } catch (e: Exception) {
                Log.w(TAG, "metrics push failed: ${e.message}")
            }
        }
    }

    private fun buildMetricsText() = buildString {
        append("# HELP eink_android_ocr_requests_total OCR requests from Android app\n")
        append("# TYPE eink_android_ocr_requests_total counter\n")
        append("eink_android_ocr_requests_total{result=\"ok\"} ${ocrOk.get()}\n")
        append("eink_android_ocr_requests_total{result=\"error\"} ${ocrErr.get()}\n")
        append("# HELP eink_android_submissions_total Submission attempts from Android app\n")
        append("# TYPE eink_android_submissions_total counter\n")
        append("eink_android_submissions_total{result=\"ok\"} ${submitOk.get()}\n")
        append("eink_android_submissions_total{result=\"error\"} ${submitErr.get()}\n")
        append("# HELP eink_android_sessions_viewed_total Sessions opened in Android app\n")
        append("# TYPE eink_android_sessions_viewed_total counter\n")
        append("eink_android_sessions_viewed_total ${sessionsViewed.get()}\n")
        append("# HELP eink_android_ws_connections_total WebSocket connections initiated by Android\n")
        append("# TYPE eink_android_ws_connections_total counter\n")
        append("eink_android_ws_connections_total ${wsConnections.get()}\n")
    }
}
