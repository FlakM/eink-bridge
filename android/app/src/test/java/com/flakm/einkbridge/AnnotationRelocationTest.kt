package com.flakm.einkbridge

import org.json.JSONObject
import org.junit.Assert.*
import org.junit.Test

/**
 * The reader's ink is relocated by the *client* after the document changes; the server
 * only supplies the element targets. The rules below are the ones a reader feels: a note
 * travels with the text it was written against, and one that cannot be placed is
 * archived rather than drawn over unrelated paragraphs.
 */
class AnnotationRelocationTest {
    private fun element(i: Int, top: Float, text: String, bottom: Float = top + 40f) =
        ElementEntry(i, "P", null, "section-1", top, bottom, 20f, 500f, text)

    @Test
    fun movesInkWithTextAndPreservesRelativeOffset() {
        val stroke = Stroke(listOf(540f to 110f, 550f to 120f), 3f)
        val ink = relocateInk(listOf(stroke), emptyList(), listOf(element(1, 100f, "original")),
            listOf(element(8, 400f, "slightly changed")), listOf(8))
        assertEquals(listOf(540f to 410f, 550f to 420f), ink.strokes.single().points)
        assertEquals(0, ink.orphanCount)
    }

    @Test
    fun deletedTargetDoesNotLeaveInkAtOldCoordinates() {
        val ink = relocateInk(listOf(Stroke(listOf(500f to 110f), 3f)), emptyList(),
            listOf(element(1, 100f, "removed")), listOf(element(1, 100f, "unrelated")), listOf(null))
        assertTrue(ink.strokes.isEmpty())
        assertEquals(1, ink.orphanCount)
    }

    @Test
    fun movesBoundGroupAndReindexesAfterOrphanRemoval() {
        val strokes = listOf(Stroke(listOf(500f to 10f), 3f), Stroke(listOf(540f to 110f), 3f))
        val group = BindGroup(42, 0, setOf(1), listOf(1), emptyList(), 530f, 100f)
        val ink = relocateInk(strokes, listOf(group), listOf(element(0, 0f, "gone"), element(1, 100f, "kept")),
            listOf(element(8, 400f, "kept")), listOf(null, 8))
        assertEquals(setOf(0), ink.groups.single().strokeIndices)
        assertEquals(listOf(8), ink.groups.single().elementIndices)
        assertEquals(400f, ink.groups.single().markerDocY)
        assertEquals(410f, ink.strokes.single().points.single().second)
    }

    @Test
    fun inkFollowsTheTopOfTheParagraphWhenItGrows() {
        // The top-left is the anchor, not the centre or the bottom: a margin note must
        // stay beside the line it was written next to when a paragraph is edited to be
        // twice as tall.
        val stroke = Stroke(listOf(540f to 110f), 3f)
        val ink = relocateInk(listOf(stroke), emptyList(), listOf(element(1, 100f, "text", bottom = 140f)),
            listOf(element(8, 400f, "text", bottom = 520f)), listOf(8))
        assertEquals(
            "the ink moved by the top's delta (300), not by the height's",
            listOf(540f to 410f),
            ink.strokes.single().points
        )
    }

    @Test
    fun theArchivedCountIsTheTruthAboutWhatTheReaderCanNoLongerSee() {
        // A stroke bound to a paragraph that is gone is archived rather than drawn
        // somewhere it does not belong.
        val boundAndMoved = Stroke(listOf(500f to 110f), 3f)
        val boundToDeleted = Stroke(listOf(500f to 210f), 3f)
        val groups = listOf(
            BindGroup(1, 0, setOf(0), listOf(1), emptyList(), 500f, 100f),
            BindGroup(2, 0, setOf(1), listOf(2), emptyList(), 500f, 200f),
        )
        val ink = relocateInk(
            listOf(boundAndMoved, boundToDeleted), groups,
            listOf(element(1, 100f, "kept"), element(2, 200f, "removed")),
            listOf(element(8, 400f, "kept")),
            listOf(8, null),
        )
        assertEquals(1, ink.strokes.size)
        assertEquals(listOf(500f to 410f), ink.strokes.single().points)
        assertEquals("the note for the deleted paragraph is archived", 1, ink.orphanCount)
        assertEquals("the survivor keeps its note", listOf(8), ink.groups.single().elementIndices)
    }

    @Test
    fun aFreeNoteBindsToTheNearestParagraphHoweverFarItIs() {
        // Proximity is the only rule for a stroke the reader never bound, and there is no
        // distance cap: a note written in the margin or below the last paragraph belongs
        // to the paragraph nearest it, so it travels with that paragraph rather than
        // being archived on every edit.
        val far = Stroke(listOf(1500f to 3000f), 3f)
        val ink = relocateInk(
            listOf(far), emptyList(),
            listOf(element(1, 100f, "kept")),
            listOf(element(8, 400f, "kept")),
            listOf(8),
        )
        assertEquals("a free note is kept", 1, ink.strokes.size)
        assertEquals(listOf(1500f to 3300f), ink.strokes.single().points)
        assertEquals(0, ink.orphanCount)
    }

    @Test
    fun aFreeNoteLandsOnTheNearestParagraphWithTheSameText() {
        // A stroke drawn beside the page without binding: proximity decides, and among
        // identical paragraphs it must be the one it was actually written beside.
        val stroke = Stroke(listOf(540f to 510f), 3f)
        val ink = relocateInk(
            listOf(stroke), emptyList(),
            listOf(element(1, 100f, "Identical boilerplate"), element(2, 500f, "Identical boilerplate")),
            listOf(element(1, 150f, "Identical boilerplate"), element(2, 560f, "Identical boilerplate")),
            listOf(1, 2),
        )
        assertEquals(
            "the note should follow the paragraph it was beside (delta 60), not the other one",
            listOf(540f to 570f),
            ink.strokes.single().points
        )
        assertEquals(0, ink.orphanCount)
    }

    @Test
    fun aGroupWhoseInkIsAllArchivedIsDroppedEntirely() {
        val strokes = listOf(Stroke(listOf(500f to 110f), 3f))
        val group = BindGroup(7, 0, setOf(0), listOf(1), emptyList(), 530f, 100f)
        val ink = relocateInk(strokes, listOf(group), listOf(element(1, 100f, "removed")),
            listOf(element(1, 100f, "something else")), listOf(null))
        assertTrue("a group with no ink left must not survive", ink.groups.isEmpty())
        assertEquals(1, ink.orphanCount)
    }

    @Test
    fun aStrokeWithNoPointsIsNotDrawnAndCountsAsArchived() {
        val ink = relocateInk(listOf(Stroke(emptyList(), 3f)), emptyList(),
            listOf(element(1, 100f, "text")), listOf(element(8, 400f, "text")), listOf(8))
        assertTrue(ink.strokes.isEmpty())
        assertEquals(1, ink.orphanCount)
    }

    @Test
    fun movedGroupsPointAtTheNewElementsNotTheOldOnes() {
        // The submitted annotation names its target by text; pointing at the old text
        // would describe a paragraph that is no longer on the page.
        val strokes = listOf(Stroke(listOf(540f to 110f), 3f))
        val group = BindGroup(5, 0, setOf(0), listOf(1),
            listOf(ElementRef("section-1", "P", "before the edit")), 530f, 100f)
        val ink = relocateInk(strokes, listOf(group), listOf(element(1, 100f, "before the edit")),
            listOf(element(9, 400f, "after the edit")), listOf(9))
        val moved = ink.groups.single()
        assertEquals(listOf(9), moved.elementIndices)
        assertEquals("after the edit", moved.elementRefs.single().text)
        assertEquals(listOf(540f to 410f), ink.strokes.single().points)
    }

    @Test
    fun theElementMapJsonIsWhatTheServerReads() {
        // The server reads `i`, `tag` and `text` to identify a paragraph; a rename here
        // would silently drop the location and leave the note unplaceable.
        val payload = org.json.JSONArray(
            elementMapJson(listOf(ElementEntry(4, "P", "para-4", "section-1", 120f, 160f, 20f, 500f, "the retry loop")))
        )
        val json = payload.getJSONObject(0)
        val keys = json.keys().asSequence().toSet()
        for (required in listOf("i", "tag", "text", "t", "b", "l", "r", "section")) {
            assertTrue("$required is missing from $keys", required in keys)
        }
        assertEquals(4, json.getInt("i"))
        assertEquals("P", json.getString("tag"))
        assertEquals("the retry loop", json.getString("text"))
        assertEquals("para-4", json.getString("id"))
        assertEquals("section-1", json.getString("section"))
        assertEquals(120.0, json.getDouble("t"), 0.001)
        assertEquals(160.0, json.getDouble("b"), 0.001)
        assertTrue("geometry must be numbers, not strings", json.get("t") is Number)
    }

    @Test
    fun aMissingIdIsLeftOutRatherThanSentAsNull() {
        // org.json drops a null value, so the key simply is not there. The server tolerates
        // that -- it reads i, tag and text -- but the shape is worth pinning so nobody
        // makes the server depend on a key that never arrives.
        val json = org.json.JSONArray(elementMapJson(listOf(element(1, 10f, "no id")))).getJSONObject(0)
        assertFalse("id should be omitted when unknown", json.has("id"))
        assertTrue(json.has("section"))
        assertEquals("no id", json.getString("text"))
    }

    @Test
    fun theElementMapJsonSurvivesAnEmptyPage() {
        assertEquals("[]", elementMapJson(emptyList()))
    }
}
