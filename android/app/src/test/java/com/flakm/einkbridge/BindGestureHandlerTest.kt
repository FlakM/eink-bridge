package com.flakm.einkbridge

import org.junit.Assert.*
import org.junit.Before
import org.junit.Test

class BindGestureHandlerTest {

    private lateinit var buf: StrokeBuffer
    private val identity = ViewTransform()

    // Fixed fake element at (200, 100)
    private val fakeElement = FoundElement(i = 0, tag = "P", id = "s-0", section = "s-0", text = "hello", cx = 200f, cy = 100f)

    private fun makeHandler(
        onComplete: (BindGestureResult) -> Unit,
        onTap: (Float, Float) -> Unit = { _, _ -> },
        endpointElements: (Float, Float, Float, Float) -> List<FoundElement> = { _, _, _, _ -> emptyList() },
        nearestElements: (Float, Float) -> List<FoundElement> = { _, _ -> listOf(fakeElement) },
    ): BindGestureHandler {
        val lookup = object : ElementLookup {
            override fun findElements(l: Float, t: Float, r: Float, b: Float, callback: (List<FoundElement>) -> Unit) =
                callback(endpointElements(l, t, r, b))
            override fun findNearestElement(cx: Float, cy: Float, callback: (List<FoundElement>) -> Unit) =
                callback(nearestElements(cx, cy))
        }
        return BindGestureHandler(buf, { identity }, lookup, onComplete, onTap)
    }

    @Before fun setUp() {
        buf = StrokeBuffer()
        // Stroke at doc (100, 100)
        buf.begin(100f, 100f); buf.end(110f, 100f)
    }

    // --- Tap vs gesture ---

    @Test fun shortMovementCallsOnTap() {
        var tapped = false
        val handler = makeHandler(onComplete = { fail("should not complete") }, onTap = { _, _ -> tapped = true })
        handler.onDown(50f, 50f)
        handler.onMove(51f, 50f) // < TAP_THRESHOLD_PX2
        handler.onUp(51f, 50f, cancelled = false)
        assertTrue(tapped)
    }

    @Test fun cancelledGestureCallsOnTap() {
        var tapped = false
        val handler = makeHandler(onComplete = { fail("should not complete") }, onTap = { _, _ -> tapped = true })
        handler.onDown(0f, 0f)
        handler.onMove(500f, 500f) // large movement
        handler.onUp(500f, 500f, cancelled = true)
        assertTrue(tapped)
    }

    // --- Endpoint hit: line-to-element ---

    @Test fun endpointOnElementUsesEndpointElement() {
        // onUp at (200, 100) — should detect element at that position
        val target = FoundElement(i = 5, tag = "H2", id = "s-5", section = "s-5", text = "target", cx = 200f, cy = 100f)
        var result: BindGestureResult? = null
        val handler = makeHandler(
            onComplete = { result = it },
            endpointElements = { l, _, r, _ ->
                if (l <= 200f && r >= 200f) listOf(target) else emptyList()
            },
        )
        // Draw lasso, end stylus lift at (200, 100)
        handler.onDown(50f, 50f)
        handler.onMove(50f, 150f)
        handler.onMove(150f, 150f)
        handler.onMove(150f, 50f)
        handler.onUp(200f, 100f, cancelled = false)

        assertNotNull(result)
        assertEquals(1, result!!.elements.size)
        assertEquals(5, result!!.elements[0].i)
        assertEquals("H2", result!!.elements[0].tag)
    }

    @Test fun endpointOffElementFallsBackToNearest() {
        val nearest = FoundElement(i = 3, tag = "P", id = "s-3", section = "s-3", text = "nearest", cx = 105f, cy = 100f)
        var result: BindGestureResult? = null
        val handler = makeHandler(
            onComplete = { result = it },
            endpointElements = { _, _, _, _ -> emptyList() }, // nothing at endpoint
            nearestElements = { _, _ -> listOf(nearest) },
        )
        handler.onDown(50f, 50f)
        handler.onMove(50f, 150f)
        handler.onMove(150f, 150f)
        handler.onMove(150f, 50f)
        handler.onUp(50f, 50f, cancelled = false)

        assertNotNull(result)
        assertEquals(1, result!!.elements.size)
        assertEquals(3, result!!.elements[0].i)
    }

    // --- Stroke capture ---

    @Test fun strokeInsideLassoIsCaptured() {
        var result: BindGestureResult? = null
        val handler = makeHandler(onComplete = { result = it })
        // Lasso enclosing (100,100)–(110,100)
        handler.onDown(80f, 80f)
        handler.onMove(80f, 130f)
        handler.onMove(140f, 130f)
        handler.onMove(140f, 80f)
        handler.onUp(80f, 80f, cancelled = false)

        assertNotNull(result)
        assertTrue(result!!.strokeIndices.contains(0))
    }

    @Test fun strokeOutsideLassoNotCaptured() {
        var result: BindGestureResult? = null
        val handler = makeHandler(onComplete = { result = it })
        // Lasso far from stroke at (100,100)
        handler.onDown(300f, 300f)
        handler.onMove(300f, 400f)
        handler.onMove(400f, 400f)
        handler.onMove(400f, 300f)
        handler.onUp(300f, 300f, cancelled = false)

        assertNotNull(result)
        assertTrue(result!!.strokeIndices.isEmpty())
    }

    @Test fun emptyLassoWithNoStrokesReturnsEmpty() {
        buf.clear()
        var result: BindGestureResult? = null
        val handler = makeHandler(onComplete = { result = it })
        handler.onDown(0f, 0f)
        handler.onMove(0f, 100f)
        handler.onMove(100f, 100f)
        handler.onMove(100f, 0f)
        handler.onUp(0f, 0f, cancelled = false)

        assertNotNull(result)
        assertTrue(result!!.strokeIndices.isEmpty())
        // No strokes → no nearest element call, empty elements
        assertTrue(result!!.elements.isEmpty())
    }

    // --- Polygon accumulation ---

    @Test fun currentPointsNullBeforeGesture() {
        val handler = makeHandler(onComplete = {})
        assertNull(handler.currentPoints())
    }

    @Test fun currentPointsAvailableDuringGesture() {
        val handler = makeHandler(onComplete = {})
        handler.onDown(10f, 20f)
        handler.onMove(30f, 40f)
        val pts = handler.currentPoints()
        assertNotNull(pts)
        assertEquals(2, pts!!.size)
        assertEquals(10f, pts[0].first, 0.01f)
        assertEquals(40f, pts[1].second, 0.01f)
    }

    @Test fun currentPointsNullAfterGestureCompletes() {
        val handler = makeHandler(onComplete = {})
        handler.onDown(0f, 0f)
        handler.onMove(500f, 500f)
        handler.onUp(500f, 500f, cancelled = false)
        assertNull(handler.currentPoints())
    }

    // --- Edge cases ---

    @Test fun multipleSequentialGestures() {
        var count = 0
        val handler = makeHandler(onComplete = { count++ })
        repeat(3) {
            handler.onDown(0f, 0f)
            handler.onMove(200f, 200f)
            handler.onUp(200f, 200f, cancelled = false)
        }
        assertEquals(3, count)
    }

    @Test fun docPolygonUsesTransformCoordinates() {
        val scale2x = ViewTransform(scrollX = 0f, scrollY = 0f, scale = 2f)
        val lookup = object : ElementLookup {
            override fun findElements(l: Float, t: Float, r: Float, b: Float, callback: (List<FoundElement>) -> Unit) = callback(emptyList())
            override fun findNearestElement(cx: Float, cy: Float, callback: (List<FoundElement>) -> Unit) = callback(emptyList())
        }
        // buf has no strokes — clear it so no nearest lookup is triggered
        buf.clear()
        var capturedPolygon: List<Pair<Float, Float>>? = null
        val handler = BindGestureHandler(buf, { scale2x }, lookup, { result -> capturedPolygon = result.docPolygon }, { _, _ -> })
        handler.onDown(100f, 100f) // screen → doc: (50, 50) at scale=2
        handler.onMove(200f, 200f) // → (100, 100)
        handler.onUp(200f, 200f, cancelled = false)
        assertNotNull(capturedPolygon)
        assertEquals(50f, capturedPolygon!![0].first, 0.01f)
        assertEquals(50f, capturedPolygon!![0].second, 0.01f)
        assertEquals(100f, capturedPolygon!![1].first, 0.01f)
    }

    @Test fun endpointHitRadiusUsesDocCoordinates() {
        // Element at (200, 100) in doc space. With identity transform, screen == doc.
        // onUp at (200, 100) should be within ENDPOINT_HIT_RADIUS_DOC of the element.
        val target = FoundElement(i = 7, tag = "H1", id = "s-7", section = "s-7", text = "title", cx = 200f, cy = 100f)
        var capturedL = 0f; var capturedR = 0f
        val handler = makeHandler(
            onComplete = {},
            endpointElements = { l, _, r, _ -> capturedL = l; capturedR = r; listOf(target) },
        )
        handler.onDown(0f, 0f)
        handler.onMove(200f, 0f)
        handler.onUp(200f, 100f, cancelled = false)
        val r = BindGestureHandler.ENDPOINT_HIT_RADIUS_DOC
        assertEquals(200f - r, capturedL, 0.01f)
        assertEquals(200f + r, capturedR, 0.01f)
    }
}
