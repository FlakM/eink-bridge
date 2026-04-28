package com.flakm.einkbridge

import android.util.Log
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody

private const val TAG = "EinkSubmit"

internal sealed class SubmitResult {
    data class Http(val code: Int) : SubmitResult()
    data class Failure(val exception: Exception) : SubmitResult()
    val isSuccessful get() = this is Http && code in 200..299
    val isConflict get() = this is Http && code == 409
}

internal class SubmissionManager(private val client: OkHttpClient) {

    fun buildAnnotationsJson(
        strokes: List<Stroke>,
        bindGroups: List<BindGroup>,
        elements: List<ElementEntry>,
    ): String {
        val (explicitGroups, unanchoredStrokes) = bindGroupsToAnnotations(strokes, bindGroups)
        val (proximityGroups, trulyUnanchored) = groupStrokesWithProximity(unanchoredStrokes, elements)
        return annotationsToJson(explicitGroups + proximityGroups, trulyUnanchored)
    }

    suspend fun submit(
        serverUrl: String,
        sessionId: String,
        annotationsJson: String,
        pngData: ByteArray?,
        strokeJson: String?,
    ): SubmitResult {
        val builder = MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("typed_notes", "")
            .addFormDataPart("annotations", annotationsJson)
        pngData?.let {
            builder.addFormDataPart("annotation", "strokes.png", it.toRequestBody("image/png".toMediaType()))
        }
        strokeJson?.let { builder.addFormDataPart("stroke_data", it) }

        val url = "$serverUrl/api/sessions/$sessionId/submit"
        Log.i(TAG, "submit POST $url")
        return try {
            val response = client.newCall(Request.Builder().url(url).post(builder.build()).build()).execute()
            Log.i(TAG, "submit response code=${response.code}")
            SubmitResult.Http(response.code)
        } catch (e: Exception) {
            Log.e(TAG, "submit exception", e)
            SubmitResult.Failure(e)
        }
    }

    suspend fun requestUpdate(
        serverUrl: String,
        sessionId: String,
        annotationsJson: String,
    ): Boolean {
        val url = "$serverUrl/api/sessions/$sessionId/request_update"
        val body = """{"annotations":$annotationsJson}"""
        Log.i(TAG, "requestUpdate POST $url")
        return try {
            val request = Request.Builder()
                .url(url)
                .post(body.toRequestBody("application/json".toMediaType()))
                .build()
            val response = client.newCall(request).execute()
            Log.i(TAG, "requestUpdate response code=${response.code}")
            response.isSuccessful
        } catch (e: Exception) {
            Log.e(TAG, "requestUpdate exception: ${e.javaClass.simpleName}: ${e.message}")
            false
        }
    }
}
