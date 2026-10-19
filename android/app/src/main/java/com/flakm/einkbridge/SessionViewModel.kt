package com.flakm.einkbridge

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.currentCoroutineContext
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
    private val client = HttpClients.withToken(
        OkHttpClient.Builder()
            .connectTimeout(5, TimeUnit.SECONDS)
            .readTimeout(10, TimeUnit.SECONDS)
            .build()
    ) { sessionRepo.token() }

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
        pollJob = viewModelScope.launch { pollWithBackoff(fetch = { fetchSessions() }) }
    }

    fun stopPolling() {
        pollJob?.cancel()
        pollJob = null
    }

    suspend fun checkReachable(url: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val req = Request.Builder().url("$url/api/health").build()
            client.newCall(req).execute().use { it.isSuccessful }
        } catch (_: Exception) { false }
    }

    suspend fun clearFinishedSessions(): Boolean = withContext(Dispatchers.IO) {
        try {
            val request = Request.Builder().url("$serverUrl/api/sessions").delete().build()
            client.newCall(request).execute().use { it.isSuccessful }
        } catch (_: Exception) { false }
    }

    /** Forget a note and its files. A bare DELETE would only cancel it, leaving it listed. */
    suspend fun deleteSession(id: String): Boolean = withContext(Dispatchers.IO) {
        try {
            val request = Request.Builder()
                .url("$serverUrl/api/sessions/$id?remove=true&force=true")
                .delete().build()
            client.newCall(request).execute().use { it.isSuccessful }
        } catch (_: Exception) { false }
    }

    /**
     * Remove every note, pinned ones included.
     *
     * "Clear finished" leaves starred notes alone -- they are pinned precisely so that
     * purge, expiry and restarts keep them -- which is how a reader ends up with a list
     * they cannot clear.
     */
    suspend fun cleanNotes(): Boolean = withContext(Dispatchers.IO) {
        try {
            val listed = client.newCall(
                Request.Builder().url("$serverUrl/api/sessions").build()
            ).execute().use { response ->
                if (!response.isSuccessful) return@withContext false
                org.json.JSONArray(response.body?.string() ?: "[]")
            }
            var removed = 0
            for (index in 0 until listed.length()) {
                val id = listed.getJSONObject(index).optString("id")
                if (id.isBlank()) continue
                val ok = client.newCall(
                    Request.Builder()
                        .url("$serverUrl/api/sessions/$id?remove=true&force=true")
                        .delete().build()
                ).execute().use { it.isSuccessful }
                if (ok) removed++
            }
            fetchSessions()
            removed > 0 || listed.length() == 0
        } catch (_: Exception) {
            false
        }
    }

    suspend fun toggleStar(id: String, starred: Boolean): Boolean = withContext(Dispatchers.IO) {
        try {
            val body = JSONObject().put("starred", starred).toString()
                .toRequestBody("application/json".toMediaType())
            val request = Request.Builder()
                .url("$serverUrl/api/sessions/$id/star")
                .put(body)
                .build()
            val ok = client.newCall(request).execute().use { it.isSuccessful }
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

    /** How many pages a refresh will follow before giving up on a runaway list. */
    internal companion object {
        const val SESSION_PAGE_SIZE = 50
        const val SESSION_PAGE_LIMIT = 20
        const val POLL_INTERVAL_MS = 5_000L
        const val POLL_MAX_INTERVAL_MS = 60_000L
    }

    private fun showCachedList() {
        val cached = sortStarredFirst(docCache.loadSessions())
        val cachedIds = cached.filter { docCache.hasCached(it.id) }.map { it.id }.toSet()
        val pending = cached.filter { sessionRepo.hasStrokes(it.id) }.map { it.id }.toSet()
        val status = if (cached.isNotEmpty()) ConnectionStatus.Offline else ConnectionStatus.Unreachable
        _state.value = SessionListState(cached, pending, cachedIds, status)
    }

    private fun sortStarredFirst(sessions: List<SessionInfo>): List<SessionInfo> =
        sessions.sortedWith(compareByDescending<SessionInfo> { it.starred }.thenByDescending { it.updatedAt })

    /** Refresh the list. True when the server answered, whatever the list held. */
    private suspend fun fetchSessions(): Boolean = withContext(Dispatchers.IO) {
        try {
            var reached = false
            // Page rather than fetch everything: the list grows without bound, and
            // this runs on every refresh.
            val fetched = mutableListOf<SessionInfo>()
            var cursor: String? = null
            var pages = 0
            var complete = false
            while (pages < SESSION_PAGE_LIMIT) {
                pages++
                val page = client.newCall(
                    Request.Builder().url(sessionsPageUrl(serverUrl, cursor)).build()
                ).execute().use { response ->
                    if (!response.isSuccessful) return@use null
                    reached = true
                    val body = response.body?.string() ?: "[]"
                    val arr = JSONArray(body)
                    for (i in 0 until arr.length()) {
                        fetched.add(parseSession(arr.getJSONObject(i)))
                    }
                    // One connection per page would leak on every refresh.
                    response.header("x-next-cursor")
                }
                if (page == null) break
                cursor = page
                if (cursor == null) {
                    complete = true
                    break
                }
            }

            when (listOutcome(fetched.size, complete)) {
                // A transient failure on the first page must not blank the list: show
                // what we had, and say it is offline.
                ListOutcome.UseCached -> showCachedList()
                ListOutcome.UseFetchedOnly, ListOutcome.UseFetchedAndCache -> {
                    val sessions = sortStarredFirst(fetched)
                    if (listOutcome(fetched.size, complete) == ListOutcome.UseFetchedAndCache) {
                        docCache.saveSessions(sessions)
                    }
                    val pending = sessions.filter { sessionRepo.hasStrokes(it.id) }.map { it.id }.toSet()
                    val cached = sessions.filter { docCache.hasCached(it.id) }.map { it.id }.toSet()
                    _state.value = SessionListState(sessions, pending, cached, ConnectionStatus.Online)
                }
            }
            reached
        } catch (_: Exception) {
            showCachedList()
            false
        }
    }
}

/**
 * Call [fetch] forever, waiting [pollDelay] between calls: every 5 s while the server
 * answers, doubling up to a minute while it does not. Before this the tablet asked an
 * unreachable server every 5 s all night.
 */
internal suspend fun pollWithBackoff(
    fetch: suspend () -> Boolean,
    wait: suspend (Long) -> Unit = { delay(it) },
) {
    var failures = 0
    while (currentCoroutineContext().isActive) {
        failures = if (fetch()) 0 else failures + 1
        wait(pollDelay(failures))
    }
}

/** 5 s after a success or the first failure, then 10, 20, 40, 60 s. */
internal fun pollDelay(failures: Int): Long =
    (SessionViewModel.POLL_INTERVAL_MS shl (failures - 1).coerceIn(0, 4))
        .coerceAtMost(SessionViewModel.POLL_MAX_INTERVAL_MS)

/** The URL for one page of the session list. Kept pure so the paging policy is testable. */
internal fun sessionsPageUrl(serverUrl: String, cursor: String?): String {
    val base = "$serverUrl/api/sessions?limit=${SessionViewModel.SESSION_PAGE_SIZE}"
    return if (cursor == null) {
        base
    } else {
        "$base&cursor=${java.net.URLEncoder.encode(cursor, "UTF-8")}"
    }
}

/** What a fetch should do with what it read. */
internal enum class ListOutcome {
    /** Nothing arrived: keep the cached list and say so. */
    UseCached,

    /** The walk finished: show it and remember it. */
    UseFetchedAndCache,

    /** Some pages arrived and then one failed: show them, keep the old cache. A half-read
     *  list looks exactly like a real one, and would become the offline list too. */
    UseFetchedOnly,
}

internal fun listOutcome(fetchedCount: Int, complete: Boolean): ListOutcome = when {
    fetchedCount == 0 -> ListOutcome.UseCached
    complete -> ListOutcome.UseFetchedAndCache
    else -> ListOutcome.UseFetchedOnly
}
