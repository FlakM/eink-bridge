package com.flakm.einkbridge

import org.junit.Assert.*
import org.junit.Test

/**
 * Tests for [rawDrawingAction] — the pure function that decides whether to
 * enable/disable Onyx raw drawing based on finger vs pen input.
 *
 * Constants match MotionEvent values:
 *   TOOL_TYPE_STYLUS = 2,  ACTION_DOWN = 0,  ACTION_UP = 1,  ACTION_CANCEL = 3
 */
class TouchRoutingTest {
    private val finger = { _: Int -> 1 }  // TOOL_TYPE_FINGER
    private val stylus = { _: Int -> 2 }  // TOOL_TYPE_STYLUS
    private val eraser = { _: Int -> 4 }  // TOOL_TYPE_ERASER

    @Test fun fingerDownDisablesRawDrawing() {
        assertEquals(false, rawDrawingAction(1, finger, actionDown))
    }

    @Test fun fingerUpEnablesRawDrawing() {
        assertEquals(true, rawDrawingAction(1, finger, actionUp))
    }

    @Test fun fingerCancelEnablesRawDrawing() {
        assertEquals(true, rawDrawingAction(1, finger, actionCancel))
    }

    @Test fun fingerMoveIsNoop() {
        assertNull(rawDrawingAction(1, finger, 2)) // ACTION_MOVE
    }

    @Test fun stylusDownIsNoop() {
        assertNull(rawDrawingAction(1, stylus, actionDown))
    }

    @Test fun stylusUpIsNoop() {
        assertNull(rawDrawingAction(1, stylus, actionUp))
    }

    @Test fun mixedFingersAndStylusIsNoop() {
        // Pointer 0 is finger, pointer 1 is stylus — pen wins, no action
        val mixed = { i: Int -> if (i == 0) 1 else 2 }
        assertNull(rawDrawingAction(2, mixed, actionDown))
    }

    @Test fun multipleFingerDownDisablesRawDrawing() {
        val twoFingers = { _: Int -> 1 }
        assertEquals(false, rawDrawingAction(2, twoFingers, actionDown))
    }

    @Test fun stylusCancelIsNoop() {
        assertNull(rawDrawingAction(1, stylus, actionCancel))
    }

    @Test fun eraserDownIsNoop() {
        assertNull(rawDrawingAction(1, eraser, actionDown))
    }

    @Test fun eraserUpIsNoop() {
        assertNull(rawDrawingAction(1, eraser, actionUp))
    }

    @Test fun stylusMoveIsNoop() {
        assertNull(rawDrawingAction(1, stylus, 2)) // ACTION_MOVE
    }

    @Test fun unknownActionWithFingerIsNoop() {
        assertNull(rawDrawingAction(1, finger, 99))
    }

    @Test fun zeroPointerCountFingerDownIsNoop() {
        // If pointerCount is 0, no tools to check → no stylus → should respond to finger action
        // rawDrawingAction checks (0 until 0) — never runs → hasStylus = false → actionDown = false
        assertEquals(false, rawDrawingAction(0, finger, actionDown))
    }

    @Test fun twoFingerUpEnablesRawDrawing() {
        assertEquals(true, rawDrawingAction(2, { _: Int -> 1 }, actionUp))
    }

    companion object {
        private const val actionDown = 0
        private const val actionUp = 1
        private const val actionCancel = 3
    }
}
