package com.flakm.einkbridge

import android.util.Log
import com.google.mlkit.vision.digitalink.DigitalInkRecognition
import com.google.mlkit.vision.digitalink.DigitalInkRecognitionModel
import com.google.mlkit.vision.digitalink.DigitalInkRecognitionModelIdentifier
import com.google.mlkit.vision.digitalink.DigitalInkRecognizerOptions
import com.google.mlkit.vision.digitalink.Ink
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.tasks.await
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

private const val TAG = "OcrClient"

internal class OcrClient(private val getServerUrl: () -> String) {
    private val http = OkHttpClient.Builder()
        .connectTimeout(3, TimeUnit.SECONDS)
        .readTimeout(30, TimeUnit.SECONDS)
        .build()

    suspend fun recognize(strokes: List<Stroke>): String? {
        if (strokes.isEmpty()) return null
        val pts = strokes.sumOf { it.points.size }
        val t0 = System.currentTimeMillis()
        val serverResult = recognizeViaServer(strokes)
        val serverMs = System.currentTimeMillis() - t0
        if (serverResult != null) {
            Log.i(TAG, "server OCR ok ($pts pts, ${serverMs}ms): \"${serverResult.take(40)}\"")
            return serverResult
        }
        Log.w(TAG, "server OCR failed ($pts pts, ${serverMs}ms), trying MLKit")
        val t1 = System.currentTimeMillis()
        val localResult = recognizeLocally(strokes)
        val localMs = System.currentTimeMillis() - t1
        if (localResult != null) {
            Log.i(TAG, "MLKit OCR ok ($pts pts, ${localMs}ms): \"${localResult.take(40)}\"")
        } else {
            Log.w(TAG, "MLKit OCR failed ($pts pts, ${localMs}ms)")
        }
        return localResult
    }

    private suspend fun recognizeViaServer(strokes: List<Stroke>): String? =
        withContext(Dispatchers.IO) {
            val strokesArr = JSONArray()
            val pressuresArr = JSONArray()
            var anyPressures = false
            for (stroke in strokes) {
                val pts = JSONArray()
                for ((x, y) in stroke.points) {
                    pts.put(JSONArray().apply { put(x.toDouble()); put(y.toDouble()) })
                }
                strokesArr.put(pts)
                val ps = JSONArray()
                if (stroke.pressures.size == stroke.points.size) {
                    for (p in stroke.pressures) ps.put(p.toDouble())
                    anyPressures = true
                }
                pressuresArr.put(ps)
            }
            try {
                val payload = JSONObject().put("strokes", strokesArr)
                if (anyPressures) payload.put("pressures", pressuresArr)
                val body = payload.toString().toRequestBody("application/json".toMediaType())
                val request = Request.Builder()
                    .url("${getServerUrl()}/api/ocr")
                    .post(body)
                    .build()
                val response = http.newCall(request).execute()
                if (response.isSuccessful) {
                    val text = JSONObject(response.body?.string() ?: return@withContext null)
                        .optString("text")
                    text.takeIf { it.isNotBlank() }
                } else {
                    Log.w(TAG, "server OCR returned ${response.code}")
                    null
                }
            } catch (e: Exception) {
                Log.w(TAG, "server OCR failed: ${e.javaClass.simpleName}: ${e.message}")
                null
            }
        }

    private suspend fun recognizeLocally(strokes: List<Stroke>): String? {
        val modelId = DigitalInkRecognitionModelIdentifier.fromLanguageTag("en-US")
            ?: return null
        val model = DigitalInkRecognitionModel.builder(modelId).build()
        return try {
            val inkBuilder = Ink.builder()
            for (stroke in strokes) {
                val sb = Ink.Stroke.builder()
                for ((x, y) in stroke.points) sb.addPoint(Ink.Point.create(x, y))
                inkBuilder.addStroke(sb.build())
            }
            val recognizer = DigitalInkRecognition.getClient(
                DigitalInkRecognizerOptions.builder(model).build()
            )
            val result = recognizer.recognize(inkBuilder.build()).await()
            result.candidates.firstOrNull()?.text?.takeIf { it.isNotBlank() }
        } catch (e: Exception) {
            Log.w(TAG, "MLKit OCR failed: ${e.message}")
            null
        }
    }
}
