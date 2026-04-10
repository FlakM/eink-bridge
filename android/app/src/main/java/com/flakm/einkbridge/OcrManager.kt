package com.flakm.einkbridge

import android.util.Log
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import java.util.concurrent.atomic.AtomicInteger

private const val TAG = "EinkOCR"

/**
 * Schedules and runs OCR tasks for bind groups and unbound stroke clusters.
 *
 * @param recognize   Suspending function that sends strokes to the OCR backend.
 * @param scope       Coroutine scope (expected to dispatch on Main).
 * @param onGroupRecognized   Called on Main when a bind group is recognized.
 * @param onUnboundResults    Called on Main when all unbound clusters finish.
 * @param onPendingChanged    Called on Main whenever the in-flight count changes.
 *                            Arguments: (remaining, total). total==0 means idle.
 */
internal class OcrManager(
    private val recognize: suspend (List<Stroke>) -> String?,
    private val scope: CoroutineScope,
    private val onGroupRecognized: (groupId: Int, text: String) -> Unit,
    private val onUnboundResults: (List<OcrResult>) -> Unit,
    private val onPendingChanged: (remaining: Int, total: Int) -> Unit,
    private val onError: ((message: String) -> Unit)? = null,
) {
    private var lastErrorMs = 0L
    private val semaphore = Semaphore(PARALLELISM)
    private var debounceJob: Job? = null
    private val clusterCache = mutableMapOf<Set<Stroke>, String>()

    fun schedule(bindGroups: List<BindGroup>, allStrokes: List<Stroke>) {
        debounceJob?.cancel()
        debounceJob = scope.launch {
            delay(DEBOUNCE_MS)
            runOcr(bindGroups.toList(), allStrokes.toList())
        }
    }

    fun cancel() {
        debounceJob?.cancel()
        debounceJob = null
    }

    fun clearCache() {
        clusterCache.clear()
    }

    fun evictCacheFor(strokes: Collection<Stroke>) {
        val set = strokes.toSet()
        clusterCache.keys.removeAll { key -> key.any { it in set } }
    }

    private data class Task(
        val label: String,
        val strokes: List<Stroke>,
        val onResult: (String) -> Unit,
    )

    private suspend fun runOcr(bindGroups: List<BindGroup>, allStrokes: List<Stroke>) {
        val tasks = mutableListOf<Task>()

        for (group in bindGroups) {
            if (group.recognizedText != null) continue
            val strokes = group.strokeIndices.mapNotNull { allStrokes.getOrNull(it) }
            if (strokes.isEmpty()) continue
            tasks += Task("group#${group.id}", strokes) { text ->
                onGroupRecognized(group.id, text)
            }
        }

        val boundIndices = bindGroups.flatMapTo(mutableSetOf()) { it.strokeIndices }
        val unboundIndexed = allStrokes.mapIndexedNotNull { i, s ->
            if (i !in boundIndices) i to s else null
        }
        val clusters = clusterStrokes(unboundIndexed.map { it.second })
        val clusterResults = arrayOfNulls<OcrResult>(clusters.size)

        // build per-cluster global index sets using the same union-find ordering
        val clusterGlobalIndices: List<Set<Int>> = run {
            // clusterStrokes returns groups in the same relative order as the input list
            // we need to match each cluster's strokes back to global indices
            val remaining = unboundIndexed.toMutableList()
            clusters.map { clusterStrokes ->
                val indices = mutableSetOf<Int>()
                for (stroke in clusterStrokes) {
                    val pos = remaining.indexOfFirst { it.second === stroke }
                    if (pos >= 0) { indices += remaining[pos].first; remaining.removeAt(pos) }
                }
                indices
            }
        }

        for ((ci, cluster) in clusters.withIndex()) {
            val pts = cluster.flatMap { it.points }
            val cx = pts.map { it.first }.average().toFloat()
            val cy = pts.map { it.second }.average().toFloat()
            val minY = pts.minOf { it.second }
            val globalIdx = clusterGlobalIndices[ci]
            val cacheKey = cluster.toSet()
            val cached = clusterCache[cacheKey]
            if (cached != null) {
                Log.d(TAG, "cached: cluster#$ci -> \"$cached\"")
                clusterResults[ci] = OcrResult(cx, cy, cached, minY, globalIdx)
            } else {
                tasks += Task("cluster#$ci", cluster) { text ->
                    clusterCache[cacheKey] = text
                    clusterResults[ci] = OcrResult(cx, cy, text, minY, globalIdx)
                }
            }
        }

        if (tasks.isEmpty()) {
            Log.d(TAG, "runOcr: nothing to do")
            if (clusters.isNotEmpty()) {
                onUnboundResults(clusterResults.filterNotNull())
            }
            return
        }

        val total = tasks.size
        val remaining = AtomicInteger(total)
        onPendingChanged(total, total)
        Log.d(TAG, "runOcr: $total tasks, parallelism=$PARALLELISM")

        coroutineScope {
            for (task in tasks) {
                launch {
                    semaphore.withPermit {
                        val pts = task.strokes.sumOf { it.points.size }
                        Log.d(TAG, "start: ${task.label} ($pts pts)")
                        val t0 = System.currentTimeMillis()
                        try {
                            val text = recognize(task.strokes)
                            val ms = System.currentTimeMillis() - t0
                            if (text != null) {
                                Log.i(TAG, "done: ${task.label} ($pts pts) -> \"$text\" [${ms}ms]")
                                MetricsReporter.recordOcrDuration(ms)
                                task.onResult(text)
                            } else {
                                Log.w(TAG, "empty: ${task.label} [${ms}ms]")
                                val now = System.currentTimeMillis()
                                if (now - lastErrorMs > ERROR_THROTTLE_MS) {
                                    lastErrorMs = now
                                    onError?.invoke("OCR empty for ${task.label} (${ms}ms)")
                                }
                            }
                        } catch (e: Exception) {
                            val ms = System.currentTimeMillis() - t0
                            Log.e(TAG, "error: ${task.label} [${ms}ms]: ${e.javaClass.simpleName}: ${e.message}")
                            val now = System.currentTimeMillis()
                            if (now - lastErrorMs > ERROR_THROTTLE_MS) {
                                lastErrorMs = now
                                onError?.invoke("OCR failed (${ms}ms): ${e.message?.take(60)}")
                            }
                        } finally {
                            val r = remaining.decrementAndGet()
                            onPendingChanged(r, total)
                        }
                    }
                }
            }
        }

        if (clusters.isNotEmpty()) {
            onUnboundResults(clusterResults.filterNotNull())
        }
    }

    companion object {
        const val PARALLELISM = 4
        const val DEBOUNCE_MS = 3000L
        const val ERROR_THROTTLE_MS = 10_000L
    }
}
