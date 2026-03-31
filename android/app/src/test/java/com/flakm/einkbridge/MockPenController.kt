package com.flakm.einkbridge

import android.graphics.Rect
import android.view.View

/**
 * Test double for [PenInputController].
 *
 * Callers use [simulateStroke] to inject fake stylus paths directly into a [StrokeBuffer],
 * bypassing the Onyx SDK entirely.
 */
internal class MockPenController(private val buf: StrokeBuffer) : PenInputController {
    var isOpen = false
    var drawingEnabled = false
    var lastStrokeWidth = 3.0f
    var style = "pencil"
    var lastLimitRect: Rect? = null
    var lastExcludeRects: List<Rect> = emptyList()
    var renderBufferResetCount = 0

    override fun open(view: View, limitRect: Rect, excludeRects: List<Rect>) {
        isOpen = true
        drawingEnabled = true
        lastLimitRect = limitRect
        lastExcludeRects = excludeRects
    }

    override fun updateLimitRect(limitRect: Rect, excludeRects: List<Rect>) {
        lastLimitRect = limitRect
        lastExcludeRects = excludeRects
    }

    override fun setEnabled(enabled: Boolean) { drawingEnabled = enabled }
    override fun setStrokeWidth(width: Float) { lastStrokeWidth = width }
    override fun setStylePencil() { style = "pencil" }
    override fun setStyleBrush() { style = "brush" }
    override fun setStyleEraser() { style = "eraser" }
    override fun resetRenderBuffer() { drawingEnabled = true; renderBufferResetCount++ }
    override fun close() { isOpen = false; drawingEnabled = false }

    /**
     * Injects a stroke directly into the buffer — simulates the full begin/move/end
     * sequence that the Onyx driver would produce for a stylus gesture.
     */
    fun simulateStroke(points: List<Pair<Float, Float>>) {
        require(points.size >= 2) { "stroke needs at least 2 points" }
        buf.begin(points.first().first, points.first().second, lastStrokeWidth)
        points.subList(1, points.size - 1).forEach { (x, y) -> buf.addPoint(x, y) }
        buf.end(points.last().first, points.last().second)
    }
}
