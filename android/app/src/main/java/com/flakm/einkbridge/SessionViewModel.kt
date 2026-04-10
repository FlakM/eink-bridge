package com.flakm.einkbridge

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.TimeUnit

internal sealed class ConnectionStatus {
    object Unknown : ConnectionStatus()
    object Online : ConnectionStatus()
    object Offline : ConnectionStatus()
    object Unreachable : ConnectionStatus()
}

internal data class SessionListState(
    val sessions: List<SessionInfo> = emptyList(),
    val pendingStrokes: Set<String> = emptySet(),
    val cachedSessions: Set<String> = emptySet(),
    val connectionStatus: ConnectionStatus = ConnectionStatus.Unknown,
)

internal class SessionViewModel(app: Application) : AndroidViewModel(app) {
    private val sessionRepo = SessionRepository(app)
    private val docCache = DocumentCache(app)
    private val client = OkHttpClient.Builder()
        .connectTimeout(5, TimeUnit.SECONDS)
        .readTimeout(10, TimeUnit.SECONDS)
        .build()

    private val _state = MutableStateFlow(SessionListState())
    val state: StateFlow<SessionListState> = _state.asStateFlow()

    var serverUrl: String = sessionRepo.serverUrl()
        private set

    private var pollJob: Job? = null

    fun setServerUrl(url: String) {
        serverUrl = url
        sessionRepo.saveServerUrl(url)
    }

    fun startPolling() {
        pollJob?.cancel()
        pollJob = viewModelScope.launch {
            while (isActive) {
                fetchSessions()
                delay(5000)
            }
        }
    }

    fun stopPolling() {
        pollJob?.cancel()
        pollJob = null
    }

    suspend fun checkReachable(url: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val req = Request.Builder().url("$url/api/health").build()
            client.newCall(req).execute().isSuccessful
        } catch (_: Exception) { false }
    }

    suspend fun clearFinishedSessions(): Boolean = withContext(Dispatchers.IO) {
        try {
            val request = Request.Builder().url("$serverUrl/api/sessions").delete().build()
            client.newCall(request).execute().isSuccessful
        } catch (_: Exception) { false }
    }

    suspend fun deleteSession(id: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val request = Request.Builder().url("$serverUrl/api/sessions/$id").delete().build()
            client.newCall(request).execute().isSuccessful
        } catch (_: Exception) { false }
    }

    suspend fun toggleStar(id: String, starred: Boolean): Boolean = withContext(Dispatchers.IO) {
        try {
            val body = JSONObject().put("starred", starred).toString()
                .toRequestBody("application/json".toMediaType())
            val request = Request.Builder()
                .url("$serverUrl/api/sessions/$id/star")
                .put(body)
                .build()
            val ok = client.newCall(request).execute().isSuccessful
            if (ok) fetchSessions()
            ok
        } catch (_: Exception) { false }
    }

    private fun parseSession(obj: JSONObject): SessionInfo {
        val origin = obj.optJSONObject("origin")
        val cwd = origin?.let { if (it.has("cwd") && !it.isNull("cwd")) it.getString("cwd") else null }
        return SessionInfo(
            id = obj.getString("id"),
            title = obj.optString("title", "(untitled)"),
            status = obj.getString("status"),
            createdAt = obj.getString("created_at"),
            updatedAt = obj.optString("updated_at", obj.getString("created_at")),
            starred = obj.optBoolean("starred", false),
            originCwd = cwd,
        )
    }

    private fun sortStarredFirst(sessions: List<SessionInfo>): List<SessionInfo> =
        sessions.sortedWith(compareByDescending<SessionInfo> { it.starred }.thenByDescending { it.updatedAt })

    private suspend fun fetchSessions() {
        withContext(Dispatchers.IO) {
            try {
                val response = client.newCall(
                    Request.Builder().url("$serverUrl/api/sessions").build()
                ).execute()
                if (response.isSuccessful) {
                    val body = response.body?.string() ?: "[]"
                    val arr = JSONArray(body)
                    val sessions = sortStarredFirst(
                        (0 until arr.length()).map { i -> parseSession(arr.getJSONObject(i)) }
                    )
                    docCache.saveSessions(sessions)
                    val pending = sessions.filter { sessionRepo.hasStrokes(it.id) }.map { it.id }.toSet()
                    val cached = sessions.filter { docCache.hasCached(it.id) }.map { it.id }.toSet()
                    _state.value = SessionListState(sessions, pending, cached, ConnectionStatus.Online)
                }
            } catch (_: Exception) {
                val cached = sortStarredFirst(docCache.loadSessions())
                val cachedIds = cached.filter { docCache.hasCached(it.id) }.map { it.id }.toSet()
                val pending = cached.filter { sessionRepo.hasStrokes(it.id) }.map { it.id }.toSet()
                val status = if (cached.isNotEmpty()) ConnectionStatus.Offline else ConnectionStatus.Unreachable
                _state.value = SessionListState(cached, pending, cachedIds, status)
            }
        }
    }
}