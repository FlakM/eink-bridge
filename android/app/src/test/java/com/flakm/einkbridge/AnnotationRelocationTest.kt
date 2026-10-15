package com.flakm.einkbridge

import org.junit.Assert.*
import org.junit.Test

class AnnotationRelocationTest {
    private fun element(i: Int, top: Float, text: String) = ElementEntry(i, "P", null, "section-1", top, top + 40, 20f, 500f, text)

    @Test fun movesInkWithTextAndPreservesRelativeOffset() {
        val stroke = Stroke(listOf(540f to 110f, 550f to 120f), 3f)
        val ink = relocateInk(listOf(stroke), emptyList(), listOf(element(1, 100f, "original")),
            listOf(element(8, 400f, "slightly changed")), listOf(8))
        assertEquals(listOf(540f to 410f, 550f to 420f), ink.strokes.single().points)
        assertEquals(0, ink.orphanCount)
    }

    @Test fun deletedTargetDoesNotLeaveInkAtOldCoordinates() {
        val ink = relocateInk(listOf(Stroke(listOf(500f to 110f), 3f)), emptyList(),
            listOf(element(1, 100f, "removed")), listOf(element(1, 100f, "unrelated")), listOf(null))
        assertTrue(ink.strokes.isEmpty())
        assertEquals(1, ink.orphanCount)
    }

    @Test fun movesBoundGroupAndReindexesAfterOrphanRemoval() {
        val strokes = listOf(Stroke(listOf(500f to 10f), 3f), Stroke(listOf(540f to 110f), 3f))
        val group = BindGroup(42, 0, setOf(1), listOf(1), emptyList(), 530f, 100f)
        val ink = relocateInk(strokes, listOf(group), listOf(element(0, 0f, "gone"), element(1, 100f, "kept")),
            listOf(element(8, 400f, "kept")), listOf(null, 8))
        assertEquals(setOf(0), ink.groups.single().strokeIndices)
        assertEquals(listOf(8), ink.groups.single().elementIndices)
        assertEquals(400f, ink.groups.single().markerDocY)
        assertEquals(410f, ink.strokes.single().points.single().second)
    }
}
