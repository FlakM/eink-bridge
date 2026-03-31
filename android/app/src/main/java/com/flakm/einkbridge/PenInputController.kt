package com.flakm.einkbridge

import android.graphics.Rect
import android.view.View

/**
 * Abstraction over the Onyx raw-drawing SDK, enabling mock injection in tests.
 *
 * [OnyxPenController] is the real implementation; [MockPenController] lives in the test source set.
 */
interface PenInputController {
    /** Called once the host view is laid out and ready. */
    fun open(view: View, limitRect: Rect, excludeRects: List<Rect>)
    /** Update the draw zone without tearing down the session (e.g. toolbar resize). */
    fun updateLimitRect(limitRect: Rect, excludeRects: List<Rect>)
    fun setEnabled(enabled: Boolean)
    /**
     * Request a stroke-width change.
     * Implementations may defer the actual SDK call to the next stroke begin to avoid
     * clearing the hardware rendering buffer mid-session.
     */
    fun setStrokeWidth(width: Float)
    fun setStylePencil()
    fun setStyleBrush()
    fun setStyleEraser()
    /**
     * Clears and resets the Onyx hardware render layer.
     * Call after undo or clear so the hardware layer no longer shows ghost strokes.
     * [StrokeView] then provides the correct visual via its own Android canvas.
     */
    fun resetRenderBuffer()
    fun close()
}
