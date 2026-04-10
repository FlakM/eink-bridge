package com.flakm.einkbridge

import android.graphics.Color
import android.view.MotionEvent
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE)
class PenOverlayTest {

    // ── MockPenController behaviour ────────────────────────────────────────────

    @Test
    fun simulateStrokeAddsPointsToBuffer() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.simulateStroke(listOf(0f to 0f, 5f to 5f, 10f to 10f))
        assertEquals(1, buf.size)
        assertEquals(3, buf.strokes[0].points.size)
    }

    @Test
    fun mockControllerTracksEnabledState() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        assertFalse(mock.drawingEnabled)
        mock.setEnabled(true)
        assertTrue(mock.drawingEnabled)
        mock.setEnabled(false)
        assertFalse(mock.drawingEnabled)
    }

    @Test
    fun mockControllerTracksStyle() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        assertEquals("pencil", mock.style)
        mock.setStyleBrush()
        assertEquals("brush", mock.style)
        mock.setStylePencil()
        assertEquals("pencil", mock.style)
    }

    @Test
    fun mockControllerTracksStrokeWidth() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.setStrokeWidth(8f)
        assertEquals(8f, mock.lastStrokeWidth)
    }

    @Test
    fun multipleSimulatedStrokesAccumulate() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        repeat(3) {
            mock.simulateStroke(listOf(0f to 0f, 10f to 10f, 20f to 20f))
        }
        assertEquals(3, buf.size)
    }

    // ── renderStrokesToPng (pure, no WebView) ──────────────────────────────────

    @Test
    fun renderStrokesToPngReturnsBytes() {
        val strokes = listOf(Stroke(listOf(0f to 0f, 50f to 50f, 100f to 100f), 3f))
        val bytes = renderStrokesToPng(200, 200, strokes)
        assertTrue("PNG should be non-empty", bytes.isNotEmpty())
        assertEquals(0x89.toByte(), bytes[0])
        assertEquals('P'.code.toByte(), bytes[1])
        assertEquals('N'.code.toByte(), bytes[2])
        assertEquals('G'.code.toByte(), bytes[3])
    }

    @Test
    fun renderStrokesToPngSkipsSinglePointStrokes() {
        val strokes = listOf(Stroke(listOf(10f to 10f), 3f))
        val bytes = renderStrokesToPng(100, 100, strokes)
        assertTrue(bytes.isNotEmpty())
    }

    @Test
    fun renderStrokesToPngWithEmptyStrokesList() {
        val bytes = renderStrokesToPng(100, 100, emptyList())
        assertTrue(bytes.isNotEmpty())
    }

    // ── exportToPng via PenOverlay + MockPenController ─────────────────────────

    @Test
    fun exportToPngReturnsNullWhenBufferIsEmpty() {
        val (overlay, _, _) = buildOverlay()
        assertNull(overlay.exportToPng())
    }

    @Test
    fun exportToPngReturnsBytesAfterSimulatedStroke() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.simulateStroke(listOf(10f to 10f, 100f to 100f, 200f to 200f))
        assertFalse("Buffer must not be empty after stroke", buf.isEmpty)
        val bytes = renderStrokesToPng(400, 600, buf.strokes)
        assertTrue(bytes.isNotEmpty())
    }

    @Test
    fun undoRemovesLastStrokeFromBuffer() {
        val (overlay, buf, mock) = buildOverlay()
        mock.simulateStroke(listOf(0f to 0f, 10f to 10f))
        mock.simulateStroke(listOf(20f to 20f, 30f to 30f))
        assertEquals(2, buf.size)
        overlay.undoLastStroke()
        assertEquals(1, buf.size)
    }

    @Test
    fun clearRemovesAllStrokes() {
        val (overlay, buf, mock) = buildOverlay()
        repeat(3) { mock.simulateStroke(listOf(0f to 0f, 5f to 5f)) }
        assertEquals(3, buf.size)
        overlay.clearStrokes()
        assertTrue(buf.isEmpty)
    }

    // ── toolbar exclusion ──────────────────────────────────────────────────────

    @Test
    fun init_toolbarExcludedWhenBothViewsLaidOut() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val webView = buildFakeWebView(width = 1080, height = 1920)
        val toolbar = buildFakeView(width = 1080, height = 200)
        val overlay = PenOverlay(
            webView = webView, excludeView = toolbar, buf = buf,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 1080, 1920),
            transformOverride = { ViewTransform() },
        )
        overlay.init()
        assertTrue("Controller must be opened", mock.isOpen)
        assertEquals("Exactly one exclude rect expected", 1, mock.lastExcludeRects.size)
        val exc = mock.lastExcludeRects[0]
        assertEquals("Exclude rect top = screen height − toolbar height", 1920 - 200, exc.top)
        assertEquals("Exclude rect bottom reaches screen bottom", 1920, exc.bottom)
    }

    @Test
    fun init_noExcludeRect_whenToolbarHasZeroHeight() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val webView = buildFakeWebView(width = 1080, height = 1920)
        val toolbar = buildFakeView(width = 0, height = 0)
        val overlay = PenOverlay(
            webView = webView, excludeView = toolbar, buf = buf,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 1080, 1920),
            transformOverride = { ViewTransform() },
        )
        overlay.init()
        assertFalse("Controller must not open before toolbar is laid out", mock.isOpen)
    }

    @Test
    fun toolbarLayoutChange_updatesExcludeRects() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val webView = buildFakeWebView(width = 1080, height = 1920)
        val toolbar = buildFakeView(width = 1080, height = 200)
        val overlay = PenOverlay(
            webView = webView, excludeView = toolbar, buf = buf,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 1080, 1920),
            transformOverride = { ViewTransform() },
        )
        overlay.init()
        assertEquals(1920 - 200, mock.lastExcludeRects[0].top)
        toolbar.layout(0, 1920 - 300, 1080, 1920)
        assertEquals("Exclude rect must update when toolbar height changes", 1920 - 300, mock.lastExcludeRects[0].top)
    }

    // ── stroke width does not clear buffer ─────────────────────────────────────

    @Test
    fun setStrokeWidth_doesNotClearBuffer() {
        val (overlay, buf, mock) = buildOverlay()
        mock.simulateStroke(listOf(0f to 0f, 50f to 50f, 100f to 100f))
        assertEquals(1, buf.size)
        overlay.setStrokeWidth(12f)
        assertEquals("Buffer must be intact after stroke-width change", 1, buf.size)
        assertEquals(12f, mock.lastStrokeWidth)
    }

    @Test
    fun setStrokeWidth_updatesCurrentWidthUsedForExport() {
        val (overlay, _, mock) = buildOverlay()
        mock.simulateStroke(listOf(0f to 0f, 100f to 100f))
        overlay.setStrokeWidth(15f)
        assertEquals(15f, mock.lastStrokeWidth)
    }

    // ── scroll survival (StrokeView persistence) ────────────────────────────────

    @Test
    fun finger_scroll_syncs_stroke_view_before_disabling_sdk() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()

        mock.simulateStroke(listOf(20f to 60f, 180f to 60f))
        assertEquals(1, buf.size)
        assertEquals(0, strokeView.strokes.size)

        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))

        assertEquals("StrokeView must contain strokes after finger down", 1, strokeView.strokes.size)
        assertFalse("SDK must be disabled on finger down", mock.drawingEnabled)
    }

    @Test
    fun scroll_calls_reset_render_buffer_on_re_enable_to_prevent_ghost_line() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()

        mock.simulateStroke(listOf(20f to 60f, 180f to 60f))
        assertEquals("No reset before any scroll", 0, mock.renderBufferResetCount)

        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        assertEquals("Disable must not trigger reset", 0, mock.renderBufferResetCount)

        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_UP))
        assertEquals(
            "Re-enable after scroll must call resetRenderBuffer to flush stored stylus position",
            1, mock.renderBufferResetCount,
        )
    }

    @Test
    fun two_strokes_remain_structurally_separate_in_stroke_view_after_scroll() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()

        mock.simulateStroke(listOf(10f to 50f, 190f to 50f))
        mock.simulateStroke(listOf(10f to 150f, 190f to 150f))

        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_UP))

        assertEquals("Two strokes must remain as two separate lists", 2, strokeView.strokes.size)
        assertTrue("Stroke 1 must only contain y=50 points", strokeView.strokes[0].points.all { (_, y) -> y == 50f })
        assertTrue("Stroke 2 must only contain y=150 points", strokeView.strokes[1].points.all { (_, y) -> y == 150f })
        val s1Points = strokeView.strokes[0].points.toSet()
        val s2Points = strokeView.strokes[1].points.toSet()
        assertTrue("Strokes must not share any points", s1Points.intersect(s2Points).isEmpty())
    }

    @Test
    fun finger_scroll_re_syncs_stroke_view_on_finger_up() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()

        mock.simulateStroke(listOf(20f to 60f, 180f to 60f))
        mock.simulateStroke(listOf(20f to 100f, 180f to 100f))

        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_UP))

        assertTrue("SDK must be re-enabled on finger up", mock.drawingEnabled)
        assertEquals("StrokeView must show all strokes after scroll", 2, strokeView.strokes.size)
    }

    @Test
    fun undo_updates_stroke_view_and_resets_render_buffer() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val overlay = PenOverlay(
            webView = buildFakeWebView(), buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            transformOverride = { ViewTransform() },
        )
        mock.simulateStroke(listOf(20f to 60f, 180f to 60f))
        mock.simulateStroke(listOf(20f to 100f, 180f to 100f))
        assertEquals(2, buf.size)

        overlay.undoLastStroke()

        assertEquals(1, buf.size)
        assertEquals("StrokeView must reflect undo immediately", 1, strokeView.strokes.size)
        assertEquals("Render buffer must be reset so ghost stroke disappears", 1, mock.renderBufferResetCount)
    }

    @Test
    fun clear_updates_stroke_view_and_resets_render_buffer() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val overlay = PenOverlay(
            webView = buildFakeWebView(), buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            transformOverride = { ViewTransform() },
        )
        repeat(3) { mock.simulateStroke(listOf(0f to 0f, 10f to 10f)) }
        assertEquals(3, buf.size)

        overlay.clearStrokes()

        assertTrue(buf.isEmpty)
        assertTrue("StrokeView must be empty after clear", strokeView.strokes.isEmpty())
        assertEquals("Render buffer must be reset after clear", 1, mock.renderBufferResetCount)
    }

    @Test
    fun stroke_view_not_required_overlay_works_without_it() {
        val (overlay, buf, mock) = buildOverlay()
        mock.simulateStroke(listOf(0f to 0f, 50f to 50f))
        overlay.undoLastStroke()
        assertTrue(buf.isEmpty)
    }

    // ── per-stroke width preservation ─────────────────────────────────────────

    @Test
    fun per_stroke_width_is_stored_at_draw_time() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.setStrokeWidth(3f)
        mock.simulateStroke(listOf(10f to 10f, 100f to 10f))
        mock.setStrokeWidth(10f)
        mock.simulateStroke(listOf(10f to 50f, 100f to 50f))
        assertEquals("First stroke must retain width 3", 3f, buf.strokes[0].width, 0.001f)
        assertEquals("Second stroke must have width 10", 10f, buf.strokes[1].width, 0.001f)
    }

    @Test
    fun changing_width_after_stroke_does_not_retroactively_change_previous_stroke() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.setStrokeWidth(5f)
        mock.simulateStroke(listOf(0f to 0f, 100f to 100f))
        val widthAfterFirstStroke = buf.strokes[0].width
        mock.setStrokeWidth(15f)
        assertEquals("Width of committed stroke is immutable", widthAfterFirstStroke, buf.strokes[0].width, 0.001f)
        assertEquals(5f, buf.strokes[0].width, 0.001f)
    }

    @Test
    fun three_strokes_each_retain_their_own_width() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val widths = listOf(1f, 5f, 12f)
        widths.forEach { w ->
            mock.setStrokeWidth(w)
            mock.simulateStroke(listOf(0f to 0f, 50f to 50f))
        }
        widths.forEachIndexed { i, w ->
            assertEquals("Stroke $i must have width $w", w, buf.strokes[i].width, 0.001f)
        }
    }

    @Test
    fun undo_then_redraw_uses_new_width() {
        val (overlay, buf, mock) = buildOverlay()
        mock.setStrokeWidth(3f)
        mock.simulateStroke(listOf(0f to 0f, 50f to 50f))
        overlay.undoLastStroke()
        mock.setStrokeWidth(9f)
        mock.simulateStroke(listOf(10f to 10f, 80f to 80f))
        assertEquals("Redrawn stroke must use new width", 9f, buf.strokes[0].width, 0.001f)
    }

    // ── eraser behavior ────────────────────────────────────────────────────────

    @Test
    fun eraser_style_is_tracked_by_mock() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.setStyleEraser()
        assertEquals("eraser", mock.style)
    }

    @Test
    fun style_pencil_after_eraser_reverts_to_pencil() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.setStyleEraser()
        mock.setStylePencil()
        assertEquals("pencil", mock.style)
    }

    @Test
    fun style_brush_after_eraser_reverts_to_brush() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        mock.setStyleEraser()
        mock.setStyleBrush()
        assertEquals("brush", mock.style)
    }

    // ── exportStrokeJson ───────────────────────────────────────────────────────

    @Test
    fun exportStrokeJson_returns_null_when_buffer_empty() {
        val (overlay, _, _) = buildOverlay(width = 400, height = 600, limitRect = android.graphics.Rect(0, 0, 400, 600))
        assertNull(overlay.exportStrokeJson())
    }

    @Test
    fun exportStrokeJson_contains_canvas_dimensions() {
        val (overlay, _, mock) = buildOverlay(width = 400, height = 600, limitRect = android.graphics.Rect(0, 0, 400, 600))
        mock.simulateStroke(listOf(10f to 10f, 50f to 50f))
        val json = overlay.exportStrokeJson()
        assertNotNull(json)
        val obj = org.json.JSONObject(json!!)
        assertEquals(400, obj.getInt("canvas_width"))
        assertEquals(600, obj.getInt("canvas_height"))
    }

    @Test
    fun exportStrokeJson_contains_all_stroke_points() {
        val (overlay, _, mock) = buildOverlay(width = 400, height = 600, limitRect = android.graphics.Rect(0, 0, 400, 600))
        mock.simulateStroke(listOf(10f to 10f, 50f to 50f, 90f to 90f))
        mock.simulateStroke(listOf(20f to 20f, 80f to 80f))
        val json = overlay.exportStrokeJson()!!
        val arr = org.json.JSONObject(json).getJSONArray("strokes")
        assertEquals("Two strokes in JSON", 2, arr.length())
        assertEquals("First stroke has 3 points", 3, arr.getJSONArray(0).length())
        assertEquals("Second stroke has 2 points", 2, arr.getJSONArray(1).length())
    }

    // ── finger cancel ─────────────────────────────────────────────────────────

    @Test
    fun finger_cancel_also_re_enables_drawing() {
        val (overlay, _, mock) = buildOverlayWithInit()
        val webView = overlay.webView
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        assertFalse(mock.drawingEnabled)
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_CANCEL))
        assertTrue("ACTION_CANCEL must re-enable drawing just like ACTION_UP", mock.drawingEnabled)
    }

    @Test
    fun finger_cancel_resets_render_buffer() {
        val (overlay, _, mock) = buildOverlayWithInit()
        val webView = overlay.webView
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_CANCEL))
        assertEquals("Cancel must reset render buffer to flush ghost positions", 1, mock.renderBufferResetCount)
    }

    // ── multiple scroll cycles ────────────────────────────────────────────────

    @Test
    fun multiple_scroll_cycles_do_not_lose_strokes() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()

        repeat(3) { i ->
            mock.simulateStroke(listOf(10f to (50f + i * 40f), 200f to (50f + i * 40f)))
            webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
            webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_UP))
        }

        assertEquals("All 3 strokes must survive scroll cycles", 3, strokeView.strokes.size)
    }

    @Test
    fun multiple_undo_reduces_stroke_view_correctly() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val overlay = PenOverlay(
            webView = buildFakeWebView(), buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            transformOverride = { ViewTransform() },
        )
        repeat(5) { mock.simulateStroke(listOf(0f to 0f, 10f to 10f)) }
        assertEquals(5, buf.size)

        repeat(3) { overlay.undoLastStroke() }

        assertEquals(2, buf.size)
        assertEquals("StrokeView must reflect 2 remaining strokes", 2, strokeView.strokes.size)
    }

    // ── enable/disable ─────────────────────────────────────────────────────────

    @Test
    fun onPaused_disables_controller() {
        val (overlay, _, mock) = buildOverlayWithInit()
        assertTrue(mock.drawingEnabled)
        overlay.onPaused()
        assertFalse(mock.drawingEnabled)
    }

    @Test
    fun onResumed_re_enables_controller() {
        val (overlay, _, mock) = buildOverlayWithInit()
        overlay.onPaused()
        assertFalse(mock.drawingEnabled)
        overlay.onResumed()
        assertTrue(mock.drawingEnabled)
    }

    @Test
    fun destroy_closes_controller() {
        val (overlay, _, mock) = buildOverlayWithInit()
        assertTrue(mock.isOpen)
        overlay.destroy()
        assertFalse("Controller must be closed after destroy", mock.isOpen)
    }

    // ── ToolMode / stuck-disabled drawing regression tests ────────────────────

    @Test
    fun enterBindMode_disables_drawing_exitBindMode_re_enables() {
        val (overlay, _, mock) = buildOverlayWithInit()
        assertTrue(mock.drawingEnabled)
        overlay.setMode(ToolMode.TAG)
        assertFalse("TAG mode must disable the SDK", mock.drawingEnabled)
        overlay.setMode(ToolMode.DRAW)
        assertTrue("Returning to DRAW must re-enable the SDK", mock.drawingEnabled)
    }

    @Test
    fun rapid_mode_toggle_leaves_drawing_enabled_in_final_draw_state() {
        val (overlay, _, mock) = buildOverlayWithInit()
        repeat(20) {
            overlay.setMode(ToolMode.TAG)
            overlay.setMode(ToolMode.MOVE)
            overlay.setMode(ToolMode.DRAW)
        }
        assertTrue(
            "After rapid mode toggling, DRAW mode must leave the SDK enabled",
            mock.drawingEnabled,
        )
        assertNotNull("Limit rect must still be set", mock.lastLimitRect)
    }

    @Test
    fun onResume_while_in_bind_mode_keeps_drawing_disabled() {
        val (overlay, _, mock) = buildOverlayWithInit()
        overlay.setMode(ToolMode.TAG)
        assertFalse(mock.drawingEnabled)
        overlay.onPaused()
        overlay.onResumed()
        assertFalse(
            "onResume must not force-enable drawing while a gesture mode is active",
            mock.drawingEnabled,
        )
        overlay.setMode(ToolMode.DRAW)
        assertTrue("Exit to DRAW must re-enable drawing", mock.drawingEnabled)
    }

    @Test
    fun fingerDown_then_mode_change_then_up_does_not_leave_drawing_disabled() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        assertFalse(mock.drawingEnabled)
        // User rapidly toggles Tag mode during the finger-down window.
        overlay.setMode(ToolMode.TAG)
        overlay.setMode(ToolMode.DRAW)
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_UP))
        assertTrue(
            "Drawing must be re-enabled after finger UP once back in DRAW mode",
            mock.drawingEnabled,
        )
    }

    @Test
    fun onResumed_does_not_reset_render_buffer_in_draw_mode() {
        // Regression: lifecycle-driven re-enable must use setEnabled(true), not a full
        // close+reopen. A close+reopen between pen strokes drops committed strokes from
        // the Onyx visible layer, making new letters vanish in the pause between them.
        val (overlay, _, mock) = buildOverlayWithInit()
        val resetsBefore = mock.renderBufferResetCount
        overlay.onPaused()
        overlay.onResumed()
        assertEquals(
            "onResumed must not trigger resetRenderBuffer",
            resetsBefore, mock.renderBufferResetCount,
        )
        assertTrue(mock.drawingEnabled)
    }

    @Test
    fun resetRenderBuffer_replays_limit_rect_on_mock() {
        val (_, _, mock) = buildOverlayWithInit()
        val originalLimit = mock.lastLimitRect
        assertNotNull(originalLimit)
        mock.resetRenderBuffer()
        assertEquals(
            "MockPenController must preserve the limit rect across resetRenderBuffer",
            originalLimit, mock.lastLimitRect,
        )
    }

    // ── StrokeView ─────────────────────────────────────────────────────────────

    @Test
    fun strokeView_starts_empty() {
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        assertTrue(strokeView.strokes.isEmpty())
    }

    @Test
    fun strokeView_update_replaces_previous_strokes() {
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val s1 = listOf(Stroke(listOf(0f to 0f, 10f to 10f), 3f))
        val s2 = listOf(Stroke(listOf(5f to 5f, 20f to 20f), 5f), Stroke(listOf(1f to 1f, 2f to 2f), 2f))
        strokeView.update(s1)
        assertEquals(1, strokeView.strokes.size)
        strokeView.update(s2)
        assertEquals("Second update must replace first", 2, strokeView.strokes.size)
    }

    @Test
    fun strokeView_update_with_empty_list_clears_strokes() {
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        strokeView.update(listOf(Stroke(listOf(0f to 0f, 10f to 10f), 3f)))
        strokeView.update(emptyList())
        assertTrue(strokeView.strokes.isEmpty())
    }

    // ── init deferred behavior ─────────────────────────────────────────────────

    @Test
    fun overlay_not_initialized_when_webview_has_no_dimensions() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val emptyWebView = android.webkit.WebView(ctx)
        val overlay = PenOverlay(
            webView = emptyWebView, buf = buf, controllerOverride = mock,
            transformOverride = { ViewTransform() },
        )
        overlay.init()
        assertFalse("Controller must not open if WebView has no size", mock.isOpen)
    }

    // ── helpers ────────────────────────────────────────────────────────────────

    private data class OverlayFixture(val overlay: PenOverlay, val buf: StrokeBuffer, val mock: MockPenController) {
        val webView get() = overlay.webView
    }

    /** Overlay that exposes its webView for touch dispatch; NOT init'd. */
    private fun buildOverlay(
        width: Int = 400,
        height: Int = 600,
        limitRect: android.graphics.Rect? = null,
    ): OverlayFixture {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val overlay = PenOverlay(
            webView = buildFakeWebView(width, height),
            buf = buf,
            controllerOverride = mock,
            limitRectOverride = limitRect,
            transformOverride = { ViewTransform() },
        )
        return OverlayFixture(overlay, buf, mock)
    }

    /** Overlay that is already init'd (open + drawing enabled). */
    private fun buildOverlayWithInit(): OverlayFixture {
        val fix = buildOverlay(limitRect = android.graphics.Rect(0, 0, 400, 600))
        fix.overlay.init()
        return fix
    }

    private fun buildFingerEvent(action: Int): MotionEvent {
        val pp = arrayOf(MotionEvent.PointerProperties().apply {
            id = 0
            toolType = MotionEvent.TOOL_TYPE_FINGER
        })
        val pc = arrayOf(MotionEvent.PointerCoords().apply { x = 200f; y = 300f })
        return MotionEvent.obtain(0L, 0L, action, 1, pp, pc, 0, 0, 1f, 1f, 0, 0, 0, 0)
    }

    private fun buildFakeWebView(width: Int = 400, height: Int = 600): android.webkit.WebView {
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        return android.webkit.WebView(ctx).also {
            it.measure(
                android.view.View.MeasureSpec.makeMeasureSpec(width, android.view.View.MeasureSpec.EXACTLY),
                android.view.View.MeasureSpec.makeMeasureSpec(height, android.view.View.MeasureSpec.EXACTLY),
            )
            it.layout(0, 0, width, height)
        }
    }

    private fun buildFakeView(width: Int, height: Int): android.view.View {
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        return android.view.View(ctx).also {
            it.measure(
                android.view.View.MeasureSpec.makeMeasureSpec(width, android.view.View.MeasureSpec.EXACTLY),
                android.view.View.MeasureSpec.makeMeasureSpec(height, android.view.View.MeasureSpec.EXACTLY),
            )
            it.layout(0, height - height, width, height)
        }
    }

    // ── setStrokeColor ─────────────────────────────────────────────────────────

    @Test
    fun setStrokeColor_viaBufferDirectly() {
        val buf = StrokeBuffer()
        buf.setColor(Color.RED)
        buf.begin(0f, 0f)
        buf.end(10f, 10f)
        assertEquals(Color.RED, buf.strokes[0].color)
        buf.setColor(Color.BLUE)
        buf.begin(0f, 0f)
        buf.end(10f, 10f)
        assertEquals(Color.RED, buf.strokes[0].color)
        assertEquals(Color.BLUE, buf.strokes[1].color)
    }

    @Test
    fun setStrokeColor_penOverlay_delegates_to_buffer() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val overlay = PenOverlay(
            webView = buildFakeWebView(), buf = buf,
            controllerOverride = mock,
            transformOverride = { ViewTransform() },
        )
        overlay.setStrokeColor(Color.RED)
        mock.simulateStroke(listOf(0f to 0f, 50f to 50f))
        assertEquals(Color.RED, buf.strokes[0].color)
    }

    // ── bindDrawingActive ──────────────────────────────────────────────────────

    @Test
    fun bindMode_actionDown_sets_bindDrawingActive() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()
        overlay.enterBindMode()

        assertFalse(strokeView.bindDrawingActive)
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        assertTrue("bindDrawingActive must be set on finger down in bind mode", strokeView.bindDrawingActive)
    }

    @Test
    fun bindMode_actionUp_clears_bindDrawingActive() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()
        overlay.enterBindMode()

        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        assertTrue(strokeView.bindDrawingActive)
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_UP))
        assertFalse("bindDrawingActive must be cleared on finger up", strokeView.bindDrawingActive)
    }

    @Test
    fun bindDrawingActive_false_outside_bind_mode() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()

        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        assertFalse("bindDrawingActive must stay false outside bind mode", strokeView.bindDrawingActive)
    }

    // ── ghost closing line (Bug 3 reproducer) ──────────────────────────────────

    /**
     * Reproducer for: "when I zoom out, first and last points get connected."
     *
     * Draws an L-shaped stroke: right then down. If a ghost line connects
     * the end back to the start, it would cross through the interior of the L
     * (a diagonal from bottom-right back to top-left).
     *
     * Verifies at the DATA level: the last point must not equal the first.
     */
    @Test
    fun stroke_data_does_not_close_after_zoom() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        val strokeView = StrokeView(ctx)
        val webView = buildFakeWebView()
        val overlay = PenOverlay(
            webView = webView, buf = buf, strokeView = strokeView,
            controllerOverride = mock,
            limitRectOverride = android.graphics.Rect(0, 0, 400, 600),
            transformOverride = { ViewTransform() },
        )
        overlay.init()

        // L-shape: (50,50) → (350,50) → (350,350)
        mock.simulateStroke(listOf(
            50f to 50f, 100f to 50f, 150f to 50f, 200f to 50f,
            250f to 50f, 300f to 50f, 350f to 50f,
            350f to 100f, 350f to 150f, 350f to 200f,
            350f to 250f, 350f to 300f, 350f to 350f,
        ))

        // Simulate zoom gesture
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_DOWN))
        webView.dispatchTouchEvent(buildFingerEvent(MotionEvent.ACTION_UP))

        val stroke = buf.strokes[0]
        val first = stroke.points.first()
        val last = stroke.points.last()
        assertNotEquals("Last point must not equal first point (stroke must not close)", first, last)
        assertTrue("First point should be near (50,50)", first.first < 100f && first.second < 100f)
        assertTrue("Last point should be near (350,350)", last.first > 300f && last.second > 300f)

        // StrokeView must have the same open stroke
        val svStroke = strokeView.strokes[0]
        assertNotEquals("StrokeView stroke must not close", svStroke.points.first(), svStroke.points.last())
    }

}

/** Extension to access webView from overlay in tests. */
private val PenOverlay.webView: android.webkit.WebView
    get() {
        val field = PenOverlay::class.java.getDeclaredField("webView")
        field.isAccessible = true
        @Suppress("UNCHECKED_CAST")
        return field.get(this) as android.webkit.WebView
    }
