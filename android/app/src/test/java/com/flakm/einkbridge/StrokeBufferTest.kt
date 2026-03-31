package com.flakm.einkbridge

import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

class StrokeBufferTest {
    private lateinit var buf: StrokeBuffer

    @Before fun setUp() { buf = StrokeBuffer() }

    @Test fun emptyByDefault() {
        assertTrue(buf.isEmpty)
        assertEquals(0, buf.size)
        assertTrue(buf.strokes.isEmpty())
    }

    @Test fun recordsStrokeAfterBeginAddEnd() {
        buf.begin(0f, 0f)
        buf.addPoint(10f, 10f)
        buf.end(20f, 20f)

        assertEquals(1, buf.size)
        assertFalse(buf.isEmpty)
        val stroke = buf.strokes[0]
        assertEquals(3, stroke.points.size)
        assertEquals(0f to 0f, stroke.points[0])
        assertEquals(10f to 10f, stroke.points[1])
        assertEquals(20f to 20f, stroke.points[2])
    }

    @Test fun recordsStrokeFromBeginEndOnly() {
        // begin + end → 2 points → kept
        buf.begin(0f, 0f)
        buf.end(5f, 5f)
        assertEquals(1, buf.size)
        assertEquals(2, buf.strokes[0].points.size)
    }

    @Test fun accumulatesMultiplePointsInOneStroke() {
        buf.begin(0f, 0f)
        repeat(5) { i -> buf.addPoint(i.toFloat(), i.toFloat()) }
        buf.end(99f, 99f)
        // 1 (begin) + 5 (addPoint) + 1 (end) = 7
        assertEquals(7, buf.strokes[0].points.size)
    }

    @Test fun accumulatesMultipleStrokes() {
        repeat(3) { stroke(buf) }
        assertEquals(3, buf.size)
    }

    @Test fun undoRemovesLastStroke() {
        repeat(2) { stroke(buf) }
        buf.undo()
        assertEquals(1, buf.size)
    }

    @Test fun undoOnEmptyIsNoop() {
        buf.undo()
        assertTrue(buf.isEmpty)
    }

    @Test fun undoAllLeavesEmpty() {
        stroke(buf)
        buf.undo()
        assertTrue(buf.isEmpty)
    }

    @Test fun clearRemovesAllStrokes() {
        repeat(3) { stroke(buf) }
        buf.clear()
        assertTrue(buf.isEmpty)
    }

    @Test fun clearAfterEmptyIsNoop() {
        buf.clear()
        assertTrue(buf.isEmpty)
    }

    @Test fun newStrokeAfterClearWorks() {
        stroke(buf)
        buf.clear()
        stroke(buf)
        assertEquals(1, buf.size)
    }

    @Test fun strokesListIsImmutableSnapshot() {
        stroke(buf)
        val snapshot = buf.strokes
        stroke(buf)
        // snapshot should not grow
        assertEquals(1, snapshot.size)
        assertEquals(2, buf.size)
    }

    @Test fun strokeWidthIsRecorded() {
        buf.begin(0f, 0f, 7f)
        buf.addPoint(5f, 5f)
        buf.end(10f, 10f)
        assertEquals(7f, buf.strokes[0].width, 0.001f)
    }

    @Test fun defaultWidthIsThree() {
        buf.begin(0f, 0f)
        buf.end(10f, 10f)
        assertEquals(3f, buf.strokes[0].width, 0.001f)
    }

    @Test fun twoStrokesHaveDifferentWidths() {
        buf.begin(0f, 0f, 3f); buf.end(10f, 10f)
        buf.begin(0f, 0f, 10f); buf.end(20f, 20f)
        assertEquals(3f, buf.strokes[0].width, 0.001f)
        assertEquals(10f, buf.strokes[1].width, 0.001f)
    }

    @Test fun previousStrokeWidthUnchangedAfterNewBegin() {
        buf.begin(0f, 0f, 5f); buf.end(10f, 10f)
        buf.begin(0f, 0f, 15f)
        // first stroke width must remain 5f even though new begin with 15f started
        assertEquals(5f, buf.strokes[0].width, 0.001f)
    }

    @Test fun eraseRemovesStrokeWithinRadius() {
        buf.begin(50f, 50f); buf.end(100f, 50f)
        val removed = buf.erase(listOf(55f to 50f), 10f)
        assertTrue(removed)
        assertTrue(buf.isEmpty)
    }

    @Test fun eraseDoesNotRemoveStrokeOutsideRadius() {
        buf.begin(50f, 50f); buf.end(100f, 50f)
        val removed = buf.erase(listOf(200f to 200f), 10f)
        assertFalse(removed)
        assertEquals(1, buf.size)
    }

    @Test fun eraseEmptyPathReturnsFalse() {
        buf.begin(0f, 0f); buf.end(10f, 10f)
        assertFalse(buf.erase(emptyList(), 10f))
        assertEquals(1, buf.size)
    }

    @Test fun eraseReturnsFalseWhenNothingMatched() {
        buf.begin(0f, 0f); buf.end(10f, 10f)
        assertFalse(buf.erase(listOf(500f to 500f), 5f))
    }

    @Test fun eraseRemovesMultipleMatchingStrokes() {
        repeat(3) { buf.begin(50f, 50f); buf.end(100f, 50f) }
        assertTrue(buf.erase(listOf(55f to 50f), 10f))
        assertTrue(buf.isEmpty)
    }

    @Test fun eraseKeepsStrokesNotInRadius() {
        buf.begin(10f, 10f); buf.end(20f, 10f)  // far from eraser
        buf.begin(200f, 200f); buf.end(250f, 200f) // erased
        buf.erase(listOf(205f to 200f), 10f)
        assertEquals(1, buf.size)
        // remaining stroke should be the one at y=10
        assertTrue(buf.strokes[0].points.all { (_, y) -> y == 10f })
    }

    @Test fun pointsInStrokeMatchInsertionOrder() {
        buf.begin(1f, 1f)
        buf.addPoint(2f, 2f)
        buf.addPoint(3f, 3f)
        buf.end(4f, 4f)
        val pts = buf.strokes[0].points
        assertEquals(1f to 1f, pts[0])
        assertEquals(2f to 2f, pts[1])
        assertEquals(3f to 3f, pts[2])
        assertEquals(4f to 4f, pts[3])
    }

    @Test fun strokeIsImmutableAfterCommit() {
        buf.begin(0f, 0f); buf.end(10f, 10f)
        val snapshot = buf.strokes[0].points
        buf.begin(20f, 20f); buf.end(30f, 30f)
        // snapshot of first stroke's points must not grow
        assertEquals(2, snapshot.size)
    }

    @Test fun endWithoutBeginNotCommitted() {
        buf.end(10f, 10f)
        // current was empty, after end has 1 point — size 1 <= 1, not committed
        assertTrue(buf.isEmpty)
    }

    @Test fun clearDuringInProgressAlsoClearsCurrentPoints() {
        buf.begin(0f, 0f)
        buf.addPoint(5f, 5f)
        buf.clear()
        // After clear, a new begin+end should produce exactly 1 stroke
        buf.begin(1f, 1f); buf.end(2f, 2f)
        assertEquals(1, buf.size)
    }

    @Test fun listCallbackPointsNotDuplicated() {
        // Regression: onRawDrawingTouchPointListReceived was appending batch points
        // to the same buffer that already had individual move points, doubling them
        // and creating a closing line (batch included start point at end).
        buf.begin(10f, 10f)
        buf.addPoint(50f, 30f)
        buf.addPoint(90f, 50f)
        buf.addPoint(130f, 30f)
        buf.commit()
        assertEquals(1, buf.size)
        val stroke = buf.strokes[0]
        assertEquals(4, stroke.points.size)
        val first = stroke.points.first()
        val last = stroke.points.last()
        assertNotEquals("Stroke must not close", first, last)
    }

    @Test fun allStrokesIncludesInProgress() {
        buf.begin(0f, 0f)
        buf.addPoint(10f, 10f)
        val all = buf.allStrokes()
        assertEquals(1, all.size)
        assertEquals(2, all[0].points.size)
        buf.commit()
        assertEquals(1, buf.allStrokes().size)
        assertEquals(2, buf.allStrokes()[0].points.size)
    }

    @Test fun allStrokesEmptyWhenNoPoints() {
        assertTrue(buf.allStrokes().isEmpty())
    }

    private fun stroke(b: StrokeBuffer) {
        b.begin(0f, 0f)
        b.addPoint(5f, 5f)
        b.end(10f, 10f)
    }
}
