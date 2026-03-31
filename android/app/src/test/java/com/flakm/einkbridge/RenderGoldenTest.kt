package com.flakm.einkbridge

import android.graphics.BitmapFactory
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import java.io.File

/**
 * Golden-image regression tests for [renderStrokesToPng].
 *
 * Each scenario draws a deterministic shape, renders it to PNG, and compares pixel-by-pixel
 * against a stored golden file in src/test/snapshots/png/.
 *
 * To regenerate goldens after an intentional change:
 *   just golden-android
 * which runs:
 *   UPDATE_GOLDENS=1 ./gradlew testDebugUnitTest --tests "*.RenderGoldenTest*"
 */
@RunWith(RobolectricTestRunner::class)
@Config(manifest = Config.NONE)
class RenderGoldenTest {

    private val goldenDir = File("src/test/snapshots/png")

    // ── Basic shapes ────────────────────────────────────────────────────────────

    @Test
    fun l_shape_3px() {
        val buf = draw {
            stroke(50f, 50f,  50f, 100f,  50f, 150f,  50f, 200f)
            stroke(50f, 200f, 100f, 200f, 150f, 200f, 200f, 200f)
        }
        assertMatchesGolden("l_shape_3px", renderStrokesToPng(250, 250, buf.strokes))
    }

    @Test
    fun cross_5px() {
        val buf = draw {
            width(5f)
            stroke(100f, 20f,  100f, 100f, 100f, 180f)
            stroke(20f,  100f, 100f, 100f, 180f, 100f)
        }
        assertMatchesGolden("cross_5px", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun square_outline_4px() {
        val buf = draw {
            width(4f)
            stroke(20f,  20f,  180f, 20f)
            stroke(180f, 20f,  180f, 180f)
            stroke(180f, 180f, 20f,  180f)
            stroke(20f,  180f, 20f,  20f)
        }
        assertMatchesGolden("square_outline_4px", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun triangle_3px() {
        val buf = draw {
            stroke(100f, 20f,  20f,  180f)
            stroke(20f,  180f, 180f, 180f)
            stroke(180f, 180f, 100f, 20f)
        }
        assertMatchesGolden("triangle_3px", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun diagonal_thin_1px() {
        val buf = draw {
            width(1f)
            stroke(10f, 10f, 190f, 190f)
        }
        assertMatchesGolden("diagonal_thin_1px", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun diagonal_thick_12px() {
        val buf = draw {
            width(12f)
            stroke(10f, 10f, 190f, 190f)
        }
        assertMatchesGolden("diagonal_thick_12px", renderStrokesToPng(200, 200, buf.strokes))
    }

    // ── Width variations across renders ─────────────────────────────────────────

    @Test
    fun three_horizontal_lines_rendered_at_2px() {
        val buf = draw {
            width(2f)
            stroke(20f, 60f,  180f, 60f)
            stroke(20f, 100f, 180f, 100f)
            stroke(20f, 140f, 180f, 140f)
        }
        assertMatchesGolden("three_lines_2px", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun three_horizontal_lines_rendered_at_8px() {
        val buf = draw {
            width(8f)
            stroke(20f, 60f,  180f, 60f)
            stroke(20f, 100f, 180f, 100f)
            stroke(20f, 140f, 180f, 140f)
        }
        assertMatchesGolden("three_lines_8px", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun width_change_does_not_alter_render_of_previous_strokes() {
        // First stroke at 3f (default), second stroke at 10f
        val buf = draw { stroke(20f, 50f, 180f, 50f) }
        val mock = MockPenController(buf)
        mock.setStrokeWidth(10f)
        mock.simulateStroke(listOf(20f to 150f, 180f to 150f))
        assertEquals("Both strokes must survive the width change", 2, buf.strokes.size)
        assertEquals("First stroke must retain width 3", 3f, buf.strokes[0].width, 0.001f)
        assertEquals("Second stroke must have width 10", 10f, buf.strokes[1].width, 0.001f)
        assertMatchesGolden("width_change_two_strokes_at_10px", renderStrokesToPng(200, 200, buf.strokes))
    }

    // ── Undo and clear ──────────────────────────────────────────────────────────

    @Test
    fun undo_removes_last_stroke() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val overlay = PenOverlay(webView = fakeWebView(), buf = buf, controllerOverride = mock)
        mock.simulateStroke(listOf(20f to 60f,  180f to 60f))
        mock.simulateStroke(listOf(20f to 100f, 180f to 100f))
        mock.simulateStroke(listOf(20f to 140f, 180f to 140f))
        overlay.undoLastStroke()
        assertEquals(2, buf.size)
        assertMatchesGolden("after_undo_2_of_3_lines", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun clear_leaves_blank_canvas() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val overlay = PenOverlay(webView = fakeWebView(), buf = buf, controllerOverride = mock)
        repeat(3) { mock.simulateStroke(listOf(20f to 50f, 180f to 50f)) }
        overlay.clearStrokes()
        assertTrue(buf.isEmpty)
        assertMatchesGolden("blank_canvas_after_clear", renderStrokesToPng(200, 200, emptyList()))
    }

    @Test
    fun undo_all_strokes_one_by_one() {
        val buf = StrokeBuffer()
        val mock = MockPenController(buf)
        val overlay = PenOverlay(webView = fakeWebView(), buf = buf, controllerOverride = mock)
        repeat(4) { i -> mock.simulateStroke(listOf(20f to (40f + i * 40f), 180f to (40f + i * 40f))) }
        assertEquals(4, buf.size)
        repeat(4) { overlay.undoLastStroke() }
        assertTrue("Buffer must be empty after undoing all strokes", buf.isEmpty)
        assertMatchesGolden("blank_canvas_after_full_undo", renderStrokesToPng(200, 200, buf.strokes))
    }

    // ── Complex compound shapes ─────────────────────────────────────────────────

    @Test
    fun lgtm_checkmark_shape() {
        val buf = draw {
            width(5f)
            stroke(30f, 100f, 70f, 150f)
            stroke(70f, 150f, 160f, 40f)
        }
        assertMatchesGolden("lgtm_checkmark", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun annotated_underline_with_margin_note() {
        val buf = draw {
            stroke(20f, 150f, 200f, 150f)
            stroke(210f, 130f, 210f, 170f)
        }
        assertMatchesGolden("underline_with_margin_tick", renderStrokesToPng(240, 200, buf.strokes))
    }

    @Test
    fun question_mark_shape() {
        val buf = draw {
            width(4f)
            stroke(80f, 40f, 120f, 40f, 140f, 70f, 120f, 100f, 100f, 110f)
            stroke(100f, 120f, 100f, 150f)
            stroke(98f, 165f, 102f, 165f)
        }
        assertMatchesGolden("question_mark", renderStrokesToPng(200, 200, buf.strokes))
    }

    // ── NEW tests with per-stroke widths ────────────────────────────────────────

    @Test
    fun per_stroke_different_widths_render_correctly() {
        val buf = draw {
            width(2f)
            stroke(20f, 60f, 180f, 60f)   // thin stroke
            width(10f)
            stroke(20f, 120f, 180f, 120f) // thick stroke
        }
        assertMatchesGolden("per_stroke_thin_then_thick", renderStrokesToPng(200, 180, buf.strokes))
    }

    @Test
    fun single_stroke_thick_15px() {
        val buf = draw {
            width(15f)
            stroke(20f, 100f, 180f, 100f)
        }
        assertMatchesGolden("single_stroke_15px", renderStrokesToPng(200, 200, buf.strokes))
    }

    @Test
    fun overlapping_strokes_cross() {
        val buf = draw {
            width(3f)
            stroke(20f, 100f, 180f, 100f) // horizontal
            stroke(100f, 20f, 100f, 180f) // vertical, crosses horizontal
        }
        assertMatchesGolden("overlapping_cross", renderStrokesToPng(200, 200, buf.strokes))
    }

    // ── Infrastructure ──────────────────────────────────────────────────────────

    /** Tiny DSL for building a [StrokeBuffer] from flat coordinate lists. */
    private fun draw(block: DrawContext.() -> Unit): StrokeBuffer {
        val buf = StrokeBuffer()
        DrawContext(buf).block()
        return buf
    }

    private inner class DrawContext(private val buf: StrokeBuffer) {
        private val mock = MockPenController(buf)

        fun width(w: Float) { mock.setStrokeWidth(w) }

        /** Each pair of floats is one (x, y) point. */
        fun stroke(vararg coords: Float) {
            require(coords.size >= 4 && coords.size % 2 == 0)
            val points = coords.toList().chunked(2).map { it[0] to it[1] }
            mock.simulateStroke(points)
        }
    }

    private fun fakeWebView(): android.webkit.WebView {
        val ctx = androidx.test.core.app.ApplicationProvider.getApplicationContext<android.app.Application>()
        return android.webkit.WebView(ctx)
    }

    private fun assertMatchesGolden(name: String, bytes: ByteArray) {
        val file = File(goldenDir, "$name.png")
        if (System.getenv("UPDATE_GOLDENS") == "1") {
            goldenDir.mkdirs()
            file.writeBytes(bytes)
            println("Golden updated: $name")
            return
        }
        assertTrue(
            "Golden missing — run: UPDATE_GOLDENS=1 ./gradlew testDebugUnitTest --tests '*.RenderGoldenTest*'\n  Expected: ${file.absolutePath}",
            file.exists(),
        )
        val actual = BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
        val expected = file.readBytes().let { BitmapFactory.decodeByteArray(it, 0, it.size) }
        assertEquals("$name: width", expected.width, actual.width)
        assertEquals("$name: height", expected.height, actual.height)
        var diffPx = 0
        for (y in 0 until expected.height) for (x in 0 until expected.width)
            if (actual.getPixel(x, y) != expected.getPixel(x, y)) diffPx++
        assertEquals("$name: $diffPx pixels differ from golden", 0, diffPx)
    }
}
