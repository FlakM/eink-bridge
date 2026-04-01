package com.flakm.einkbridge

import android.content.Context
import android.content.SharedPreferences
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

internal class DocumentCache(private val context: Context) {
    private val prefs: SharedPreferences =
        context.getSharedPreferences("doc_cache", Context.MODE_PRIVATE)
    private val cacheDir = File(context.filesDir, "session_cache").also { it.mkdirs() }

    fun saveHtml(sessionId: String, html: String) {
        File(cacheDir, "$sessionId.html").writeText(html)
        touchAccess(sessionId)
        evictOldest()
    }

    fun loadHtml(sessionId: String): String? {
        val file = File(cacheDir, "$sessionId.html")
        if (!file.exists()) return null
        touchAccess(sessionId)
        return file.readText()
    }

    fun hasCached(sessionId: String): Boolean =
        File(cacheDir, "$sessionId.html").exists()

    fun saveSessions(sessions: List<SessionInfo>) {
        val arr = JSONArray()
        for (s in sessions) {
            arr.put(JSONObject().apply {
                put("id", s.id)
                put("title", s.title)
                put("status", s.status)
                put("created_at", s.createdAt)
                put("updated_at", s.updatedAt)
            })
        }
        prefs.edit().putString("sessions", arr.toString()).apply()
    }

    fun loadSessions(): List<SessionInfo> {
        val json = prefs.getString("sessions", null) ?: return emptyList()
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { i ->
                val o = arr.getJSONObject(i)
                SessionInfo(
                    id = o.getString("id"),
                    title = o.optString("title", "(untitled)"),
                    status = o.getString("status"),
                    createdAt = o.getString("created_at"),
                    updatedAt = o.optString("updated_at", o.getString("created_at")),
                )
            }
        } catch (_: Exception) { emptyList() }
    }

    private fun touchAccess(sessionId: String) {
        val order = accessOrder().toMutableList()
        order.remove(sessionId)
        order.add(0, sessionId)
        prefs.edit().putString("access_order", JSONArray(order).toString()).apply()
    }

    private fun accessOrder(): List<String> {
        val json = prefs.getString("access_order", null) ?: return emptyList()
        return try {
            val arr = JSONArray(json)
            (0 until arr.length()).map { arr.getString(it) }
        } catch (_: Exception) { emptyList() }
    }

    private fun evictOldest() {
        val order = accessOrder().toMutableList()
        while (order.size > MAX_CACHED) {
            val old = order.removeAt(order.lastIndex)
            File(cacheDir, "$old.html").delete()
        }
        prefs.edit().putString("access_order", JSONArray(order).toString()).apply()
    }

    companion object {
        const val MAX_CACHED = 10
    }
}
