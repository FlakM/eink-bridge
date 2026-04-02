package com.flakm.einkbridge

import android.content.Context
import android.content.SharedPreferences

internal class SessionRepository(context: Context) {
    private val prefs: SharedPreferences =
        context.getSharedPreferences("eink_bridge", Context.MODE_PRIVATE)

    fun serverUrl(): String =
        prefs.getString("server_url", "http://amd-pc:3333") ?: "http://amd-pc:3333"

    fun saveServerUrl(url: String) {
        prefs.edit().putString("server_url", url).apply()
    }

    fun pushgatewayUrl(): String =
        prefs.getString("pushgateway_url", "") ?: ""

    fun savePushgatewayUrl(url: String) {
        prefs.edit().putString("pushgateway_url", url).apply()
    }

    fun loadStrokes(sessionId: String, buf: StrokeBuffer) {
        val json = prefs.getString("strokes_$sessionId", null) ?: return
        try { buf.loadJson(json) } catch (_: Exception) {}
    }

    fun saveStrokes(sessionId: String, buf: StrokeBuffer) {
        val key = "strokes_$sessionId"
        if (buf.isEmpty) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, buf.toJson()).apply()
    }

    fun loadBindGroups(sessionId: String): List<BindGroup> {
        val json = prefs.getString("binds_$sessionId", null) ?: return emptyList()
        return try { bindGroupsFromJson(json) } catch (_: Exception) { emptyList() }
    }

    fun saveBindGroups(sessionId: String, groups: List<BindGroup>) {
        val key = "binds_$sessionId"
        if (groups.isEmpty()) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, bindGroupsToJson(groups)).apply()
    }

    fun loadOcrResults(sessionId: String): List<OcrResult> {
        val json = prefs.getString("ocr_$sessionId", null) ?: return emptyList()
        return try { ocrResultsFromJson(json) } catch (_: Exception) { emptyList() }
    }

    fun saveOcrResults(sessionId: String, results: List<OcrResult>) {
        val key = "ocr_$sessionId"
        if (results.isEmpty()) prefs.edit().remove(key).apply()
        else prefs.edit().putString(key, ocrResultsToJson(results)).apply()
    }

    fun clearSession(sessionId: String) {
        prefs.edit()
            .remove("strokes_$sessionId")
            .remove("binds_$sessionId")
            .remove("ocr_$sessionId")
            .apply()
    }

    fun hasStrokes(sessionId: String): Boolean =
        prefs.contains("strokes_$sessionId")
}
