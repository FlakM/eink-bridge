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

    @Test fun toJsonAndLoadJsonRoundTrip() {
        buf.begin(10f, 20f, 5f)
        buf.addPoint(30f, 40f)
        buf.end(50f, 60f)
        buf.begin(100f, 200f, 2f)
        buf.addPoint(110f, 210f)
        buf.end(120f, 220f)

        val json = buf.toJson()
        val restored = StrokeBuffer()
        restored.loadJson(json)

        assertEquals(buf.size, restored.size)
        for (i in 0 until buf.size) {
            assertEquals(buf.strokes[i].width, restored.strokes[i].width, 0.001f)
            assertEquals(buf.strokes[i].points, restored.strokes[i].points)
        }
    }

    @Test fun loadJsonEmptyArray() {
        buf.begin(0f, 0f); buf.end(10f, 10f)
        buf.loadJson("[]")
        assertTrue(buf.isEmpty)
    }

    @Test fun toJsonEmptyBuffer() {
        assertEquals("[]", buf.toJson())
    }

    private fun stroke(b: StrokeBuffer) {
        b.begin(0f, 0f)
        b.addPoint(5f, 5f)
        b.end(10f, 10f)
    }

    // --- Proximity grouping tests ---

    private fun element(i: Int, tag: String, l: Float, t: Float, r: Float, b: Float, text: String = "el$i") =
        ElementEntry(i = i, tag = tag, id = "section-$i", t = t, b = b, l = l, r = r, text = text)

    @Test fun proximityGroupsStrokeNearElement() {
        val stroke = Stroke(listOf(100f to 100f, 110f to 110f), 3f)
        val el = element(0, "H2", 80f, 80f, 200f, 130f)
        val (groups, unanchored) = groupStrokesWithProximity(listOf(stroke), listOf(el))
        assertEquals(1, groups.size)
        assertTrue(unanchored.isEmpty())
        assertTrue(groups[0].anchor is Anchor.Proximity)
        assertEquals("H2", (groups[0].anchor as Anchor.Proximity).elements[0].tag)
    }

    @Test fun strokeFarFromAllElementsIsUnanchored() {
        val stroke = Stroke(listOf(500f to 500f, 510f to 510f), 3f)
        val el = element(0, "P", 10f, 10f, 50f, 30f)
        val (groups, unanchored) = groupStrokesWithProximity(listOf(stroke), listOf(el))
        assertTrue(groups.isEmpty())
        assertEquals(1, unanchored.size)
    }

    @Test fun multipleStrokesNearSameElementGrouped() {
        val s1 = Stroke(listOf(100f to 100f, 110f to 110f), 3f)
        val s2 = Stroke(listOf(105f to 105f, 115f to 115f), 3f)
        val el = element(0, "PRE", 80f, 80f, 200f, 130f)
        val (groups, unanchored) = groupStrokesWithProximity(listOf(s1, s2), listOf(el))
        assertEquals(1, groups.size)
        assertEquals(2, groups[0].strokes.size)
        assertTrue(unanchored.isEmpty())
    }

    @Test fun strokesGroupToNearestElement() {
        val s1 = Stroke(listOf(100f to 100f, 110f to 110f), 3f)
        val s2 = Stroke(listOf(300f to 300f, 310f to 310f), 3f)
        val el1 = element(0, "H2", 80f, 80f, 150f, 130f)
        val el2 = element(1, "P", 280f, 280f, 350f, 330f)
        val (groups, unanchored) = groupStrokesWithProximity(listOf(s1, s2), listOf(el1, el2))
        assertEquals(2, groups.size)
        assertTrue(unanchored.isEmpty())
    }

    @Test fun explicitBindingOverridesProximity() {
        val stroke = Stroke(listOf(100f to 100f, 110f to 110f), 3f)
        val el1 = element(0, "H2", 80f, 80f, 150f, 130f)
        val el2 = element(1, "PRE", 280f, 280f, 350f, 330f)
        val bindings = mapOf(0 to listOf(el2))
        val (groups, unanchored) = groupStrokesWithProximity(listOf(stroke), listOf(el1, el2), bindings)
        assertEquals(1, groups.size)
        assertTrue(groups[0].anchor is Anchor.Explicit)
        assertEquals("PRE", (groups[0].anchor as Anchor.Explicit).elements[0].tag)
        assertTrue(unanchored.isEmpty())
    }

    @Test fun emptyElementsAllUnanchored() {
        val stroke = Stroke(listOf(100f to 100f, 110f to 110f), 3f)
        val (groups, unanchored) = groupStrokesWithProximity(listOf(stroke), emptyList())
        assertTrue(groups.isEmpty())
        assertEquals(1, unanchored.size)
    }

    @Test fun parseElementMapRoundTrip() {
        val json = """[{"i":0,"tag":"H2","id":"sec-1","t":10,"b":50,"l":0,"r":100,"text":"Hello"}]"""
        val entries = parseElementMap(json)
        assertEquals(1, entries.size)
        assertEquals("H2", entries[0].tag)
        assertEquals("sec-1", entries[0].id)
        assertEquals("Hello", entries[0].text)
    }

    @Test fun annotationsToJsonProducesValidJson() {
        val ref = ElementRef(sectionId = "s1", tag = "H2", text = "Heading")
        val group = StrokeGroup(
            anchor = Anchor.Proximity(listOf(ref)),
            strokes = listOf(Stroke(listOf(1f to 2f, 3f to 4f), 3f)),
        )
        val json = annotationsToJson(listOf(group), emptyList())
        val arr = org.json.JSONArray(json)
        assertEquals(1, arr.length())
        val obj = arr.getJSONObject(0)
        assertEquals("proximity", obj.getJSONObject("anchor").getString("type"))
    }
}
