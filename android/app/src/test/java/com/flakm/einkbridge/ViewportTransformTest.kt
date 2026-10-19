package com.flakm.einkbridge

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * Numbers taken from the tablet itself, with the same session page measured in both
 * orientations (see the DevTools probe): the visual viewport keeps a CSS width of 992 and
 * a pan of 704 while its scale goes from 1 to 1.333, and the view goes from 1860 px wide
 * in portrait to 2480 in landscape.
 */
class ViewportTransformTest {
    private val portrait = ViewportReport(
        scale = 1f, widthCss = 992f, heightCss = 1289.6f, pageLeftCss = 704f, pageTopCss = 0f,
    )
    private val landscape = ViewportReport(
        scale = 1.333f, widthCss = 992f, heightCss = 719.2f, pageLeftCss = 704f, pageTopCss = 0f,
    )

    @Test
    fun theScaleIsViewPixelsPerCssPixelNotTheZoomFactor() {
        // `visualViewport.scale` is the zoom relative to the layout, and using it directly
        // is what put the ink a third of a paragraph away after a rotation.
        assertEquals(1.875f, viewTransformFor(1860f, portrait)!!.scale, 0.001f)
        assertEquals(2.5f, viewTransformFor(2480f, landscape)!!.scale, 0.001f)
    }

    @Test
    fun theContentsLeftEdgeIsTheViewsLeftEdgeInEveryOrientation() {
        // The invariant. The page centres a 992 px column at 704 in a 2400 px body and pans
        // the visual viewport there, so document x=704 must land on view x=0 -- in both.
        for ((width, report) in listOf(1860f to portrait, 2480f to landscape)) {
            val transform = viewTransformFor(width, report)!!
            assertEquals("content left in ${report.widthCss}px viewport", 0f, transform.docToScreenX(704f), 0.01f)
            assertEquals("content top", 0f, transform.docToScreenY(0f), 0.01f)
        }
    }

    @Test
    fun aParagraphKeepsItsPlaceRelativeToItsTextOnRotation() {
        // The paragraph sits 214.3 css px below the content top and 56 px in from its left.
        // Those offsets must scale with the view in both orientations, never jump.
        val portraitTransform = viewTransformFor(1860f, portrait)!!
        val landscapeTransform = viewTransformFor(2480f, landscape)!!
        assertEquals(214.3f * 1.875f, portraitTransform.docToScreenY(214.3f), 0.05f)
        assertEquals(214.3f * 2.5f, landscapeTransform.docToScreenY(214.3f), 0.05f)
        // And the ratio between the two is the ratio of the view widths, not a constant.
        assertEquals(
            2480f / 1860f,
            landscapeTransform.docToScreenY(214.3f) / portraitTransform.docToScreenY(214.3f),
            0.001f,
        )
    }

    @Test
    fun withoutAReportThereIsNoTransformToTrust() {
        assertNull(viewTransformFor(1860f, null))
        assertNull("a zero width would divide by zero", viewTransformFor(1860f, ViewportReport()))
        assertNull(viewTransformFor(0f, portrait))
    }
}
