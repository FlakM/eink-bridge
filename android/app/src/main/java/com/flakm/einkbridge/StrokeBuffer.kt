package com.flakm.einkbridge

internal data class Stroke(val points: List<Pair<Float, Float>>, val width: Float)

/**
 * WebView transform state used to convert between screen and document coordinates.
 * Screen → Document: docX = (screenX + scrollX) / scale
 * Document → Screen: screenX = docX * scale - scrollX
 */
internal data class ViewTransform(
    val scrollX: Float = 0f,
    val scrollY: Float = 0f,
    val scale: Float = 1f,
) {
    fun screenToDocX(sx: Float) = (sx + scrollX) / scale
    fun screenToDocY(sy: Float) = (sy + scrollY) / scale
    fun docToScreenX(dx: Float) = dx * scale - scrollX
    fun docToScreenY(dy: Float) = dy * scale - scrollY
    fun docToScreenWidth(w: Float) = w * scale
}

/**
 * Pure-Kotlin stroke accumulator — no Android dependencies, fully unit-testable.
 *
 * All stored coordinates are in **document space**. Callers pass a [ViewTransform]
 * to [begin]/[addPoint]/[end] so screen-space SDK input is converted on the fly.
 */
internal class StrokeBuffer {
    private val _strokes = mutableListOf<Stroke>()
    private var currentPoints = mutableListOf<Pair<Float, Float>>()
    private var currentWidth = 3f
    private var currentTransform = ViewTransform()

    val strokes: List<Stroke> get() = _strokes.toList()
    val isEmpty: Boolean get() = _strokes.isEmpty() && currentPoints.size < 2
    val size: Int get() = _strokes.size

    /** Returns committed strokes plus the in-progress stroke (if any). */
    fun allStrokes(): List<Stroke> {
        if (currentPoints.size < 2) return _strokes.toList()
        return _strokes + Stroke(currentPoints.toList(), currentWidth)
    }

    fun begin(x: Float, y: Float, width: Float = 3f, transform: ViewTransform = ViewTransform()) {
        currentTransform = transform
        val docW = width / transform.scale
        currentWidth = docW
        val docPt = transform.screenToDocX(x) to transform.screenToDocY(y)
        currentPoints = mutableListOf(docPt)
    }

    fun addPoint(x: Float, y: Float) {
        currentPoints.add(currentTransform.screenToDocX(x) to currentTransform.screenToDocY(y))
    }

    fun end(x: Float, y: Float) {
        val docPt = currentTransform.screenToDocX(x) to currentTransform.screenToDocY(y)
        currentPoints.add(docPt)
        commit()
    }

    fun commit() {
        if (currentPoints.size > 1) {
            _strokes.add(Stroke(currentPoints.toList(), currentWidth))
        }
        currentPoints = mutableListOf()
    }

    fun undo() {
        if (_strokes.isNotEmpty()) _strokes.removeAt(_strokes.lastIndex)
    }

    fun clear() {
        _strokes.clear()
        currentPoints.clear()
    }

    /**
     * Removes every committed stroke that comes within [radius] screen-pixels of any point in [path].
     * [path] is in screen coordinates; stored strokes are in document coordinates.
     */
    fun erase(path: List<Pair<Float, Float>>, radius: Float, transform: ViewTransform = ViewTransform()): Boolean {
        if (path.isEmpty()) return false
        val docPath = path.map { (x, y) -> transform.screenToDocX(x) to transform.screenToDocY(y) }
        val docRadius = radius / transform.scale
        val r2 = docRadius * docRadius
        val before = _strokes.size
        _strokes.removeAll { stroke ->
            stroke.points.any { (sx, sy) ->
                docPath.any { (ex, ey) ->
                    (ex - sx) * (ex - sx) + (ey - sy) * (ey - sy) <= r2
                }
            }
        }
        return _strokes.size < before
    }
}
