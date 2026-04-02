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
) {
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
        val unbound = allStrokes.filterIndexed { i, _ -> i !in boundIndices }
        val clusters = clusterStrokes(unbound)
        val clusterResults = arrayOfNulls<OcrResult>(clusters.size)

        for ((ci, cluster) in clusters.withIndex()) {
            val pts = cluster.flatMap { it.points }
            val cx = pts.map { it.first }.average().toFloat()
            val cy = pts.map { it.second }.average().toFloat()
            val cacheKey = cluster.toSet()
            val cached = clusterCache[cacheKey]
            if (cached != null) {
                Log.d(TAG, "cached: cluster#$ci -> \"$cached\"")
                clusterResults[ci] = OcrResult(cx, cy, cached)
            } else {
                tasks += Task("cluster#$ci", cluster) { text ->
                    clusterCache[cacheKey] = text
                    clusterResults[ci] = OcrResult(cx, cy, text)
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
                        try {
                            val text = recognize(task.strokes)
                            if (text != null) {
                                Log.d(TAG, "done: ${task.label} -> \"$text\"")
                                task.onResult(text)
                            } else {
                                Log.w(TAG, "empty: ${task.label}")
                            }
                        } catch (e: Exception) {
                            Log.e(TAG, "error: ${task.label}: ${e.javaClass.simpleName}: ${e.message}")
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
        const val DEBOUNCE_MS = 2000L
    }
}
