package com.flakm.einkbridge

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE)
class OcrManagerTest {

    private fun stroke(vararg pts: Pair<Float, Float>): Stroke =
        Stroke(pts.toList(), width = 3f, color = android.graphics.Color.BLACK)

    private fun group(
        id: Int,
        strokeIndices: Set<Int>,
        recognizedText: String? = null,
    ) = BindGroup(
        id = id,
        color = android.graphics.Color.RED,
        strokeIndices = strokeIndices,
        elementIndices = emptyList(),
        elementRefs = emptyList(),
        markerDocX = 0f,
        markerDocY = 0f,
        strokeDocCenters = emptyList(),
        elementDocCenters = emptyList(),
        recognizedText = recognizedText,
    )

    private data class OcrEvents(
        val groupRecognized: MutableList<Pair<Int, String>> = mutableListOf(),
        val unboundResults: MutableList<List<OcrResult>> = mutableListOf(),
        val pendingChanged: MutableList<Pair<Int, Int>> = mutableListOf(),
    )

    private fun makeManager(
        scope: TestScope,
        recognize: suspend (List<Stroke>) -> String? = { "text" },
        events: OcrEvents = OcrEvents(),
    ): Pair<OcrManager, OcrEvents> {
        val mgr = OcrManager(
            recognize = recognize,
            scope = scope,
            onGroupRecognized = { id, text -> events.groupRecognized.add(id to text) },
            onUnboundResults = { results -> events.unboundResults.add(results) },
            onPendingChanged = { remaining, total -> events.pendingChanged.add(remaining to total) },
        )
        return mgr to events
    }

    @Test
    fun `groups without text get recognized`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, recognize = { "hello" }, events = ev)
        val strokes = listOf(stroke(0f to 0f, 10f to 10f))
        mgr.schedule(listOf(group(id = 1, strokeIndices = setOf(0))), strokes)
        advanceUntilIdle()

        assertEquals(listOf(1 to "hello"), ev.groupRecognized)
    }

    @Test
    fun `groups with existing text are skipped`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, recognize = { "text" }, events = ev)
        val strokes = listOf(
            stroke(0f to 0f, 10f to 10f),
            stroke(20f to 0f, 30f to 10f),
        )
        mgr.schedule(listOf(
            group(id = 1, strokeIndices = setOf(0), recognizedText = null),
            group(id = 2, strokeIndices = setOf(1), recognizedText = "already done"),
        ), strokes)
        advanceUntilIdle()

        assertEquals(listOf(1), ev.groupRecognized.map { it.first })
    }

    @Test
    fun `pending count starts at total and reaches zero`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, recognize = { "x" }, events = ev)
        mgr.schedule(listOf(group(id = 1, strokeIndices = setOf(0))), listOf(stroke(0f to 0f, 5f to 5f)))
        advanceUntilIdle()

        assertEquals(1 to 1, ev.pendingChanged.first())
        assertEquals(0 to 1, ev.pendingChanged.last())
    }

    @Test
    fun `null result does not crash and count still decrements`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, recognize = { null }, events = ev)
        mgr.schedule(listOf(group(id = 1, strokeIndices = setOf(0))), listOf(stroke(0f to 0f, 5f to 5f)))
        advanceUntilIdle()

        assertEquals(0, ev.pendingChanged.last().first)
    }

    @Test
    fun `cancel stops debounced job`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, recognize = { "x" }, events = ev)
        mgr.schedule(listOf(group(id = 1, strokeIndices = setOf(0))), listOf(stroke(0f to 0f, 10f to 10f)))
        mgr.cancel()
        advanceUntilIdle()

        assertTrue(ev.groupRecognized.isEmpty())
    }

    @Test
    fun `later schedule replaces earlier one`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, recognize = { "x" }, events = ev)
        val strokes = listOf(
            stroke(0f to 0f, 10f to 10f),
            stroke(20f to 0f, 30f to 10f),
        )
        mgr.schedule(listOf(group(id = 1, strokeIndices = setOf(0))), strokes)
        mgr.schedule(listOf(group(id = 2, strokeIndices = setOf(1))), strokes)
        advanceUntilIdle()

        val recognized = ev.groupRecognized.map { it.first }
        assertFalse(1 in recognized)
        assertTrue(2 in recognized)
    }

    @Test
    fun `empty groups list does nothing`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, events = ev)
        mgr.schedule(emptyList(), emptyList())
        advanceUntilIdle()

        assertTrue(ev.pendingChanged.isEmpty())
    }

    @Test
    fun `multiple groups run and all complete`() = runTest {
        val ev = OcrEvents()
        val (mgr, _) = makeManager(this, recognize = { "text" }, events = ev)
        val strokes = (0 until 6).map { i -> stroke(i * 10f to 0f, i * 10f + 5f to 5f) }
        val groups = (0 until 6).map { i -> group(id = i + 1, strokeIndices = setOf(i)) }
        mgr.schedule(groups, strokes)
        advanceUntilIdle()

        val recognized = ev.groupRecognized.map { it.first }.sorted()
        assertEquals((1..6).toList(), recognized)
    }
}
