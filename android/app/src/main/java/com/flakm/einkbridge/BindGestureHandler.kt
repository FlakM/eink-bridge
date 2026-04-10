package com.flakm.einkbridge

internal data class BindGestureResult(
    val strokeIndices: Set<Int>,
    val elements: List<FoundElement>,
    val docPolygon: List<Pair<Float, Float>>,
)

internal class BindGestureHandler(
    private val buf: StrokeBuffer,
    private val getTransform: () -> ViewTransform,
    private val elementLookup: ElementLookup,
    private val onGestureComplete: (BindGestureResult) -> Unit,
    private val onTap: (screenX: Float, screenY: Float) -> Unit,
) {
    private var bindPoints: MutableList<Pair<Float, Float>>? = null

    fun onDown(screenX: Float, screenY: Float) {
        bindPoints = mutableListOf(screenX to screenY)
    }

    fun onMove(screenX: Float, screenY: Float) {
        bindPoints?.add(screenX to screenY)
    }

    fun onUp(screenX: Float, screenY: Float, cancelled: Boolean) {
        val pts = bindPoints ?: emptyList()
        bindPoints = null
        val moved = pts.size > 1 && run {
            val dx = pts.last().first - pts.first().first
            val dy = pts.last().second - pts.first().second
            dx * dx + dy * dy > TAP_THRESHOLD_PX2
        }
        if (moved && !cancelled) complete(pts, screenX to screenY) else onTap(screenX, screenY)
    }

    fun currentPoints(): List<Pair<Float, Float>>? = bindPoints?.toList()

    private fun complete(pts: List<Pair<Float, Float>>, endpoint: Pair<Float, Float>) {
        val t = getTransform()
        val docPolygon = pts.map { t.screenToDocX(it.first) to t.screenToDocY(it.second) }

        val strokeIndices = mutableSetOf<Int>()
        buf.strokes.forEachIndexed { idx, stroke ->
            val (cx, cy) = strokeCentroid(stroke)
            val centroidInside = pointInPolygon(cx, cy, docPolygon)
            val anyPointInside = stroke.points.any { (px, py) -> pointInPolygon(px, py, docPolygon) }
            val centroidNearBoundary = distToPolygonBoundary(cx, cy, docPolygon) <= CENTROID_TOLERANCE_DOC
            if (centroidInside || anyPointInside || centroidNearBoundary) strokeIndices.add(idx)
        }

        val centroids = strokeIndices.mapNotNull { idx ->
            buf.strokes.getOrNull(idx)?.let { strokeCentroid(it) }
        }

        // If the gesture endpoint lands on an HTML element, use it directly.
        // This lets the user "draw a line" to the target by ending the lasso near it.
        val endDocX = t.screenToDocX(endpoint.first)
        val endDocY = t.screenToDocY(endpoint.second)
        val r = ENDPOINT_HIT_RADIUS_DOC
        elementLookup.findElements(endDocX - r, endDocY - r, endDocX + r, endDocY + r) { endElements ->
            if (endElements.isNotEmpty()) {
                onGestureComplete(BindGestureResult(strokeIndices, endElements, docPolygon))
            } else if (centroids.isEmpty()) {
                onGestureComplete(BindGestureResult(strokeIndices, emptyList(), docPolygon))
            } else {
                val cx = centroids.map { it.first }.average().toFloat()
                val cy = centroids.map { it.second }.average().toFloat()
                elementLookup.findNearestElement(cx, cy) { elements ->
                    onGestureComplete(BindGestureResult(strokeIndices, elements, docPolygon))
                }
            }
        }
    }

    companion object {
        const val TAP_THRESHOLD_PX2 = 30f * 30f
        const val ENDPOINT_HIT_RADIUS_DOC = 50f
        const val CENTROID_TOLERANCE_DOC = 30f
    }
}
