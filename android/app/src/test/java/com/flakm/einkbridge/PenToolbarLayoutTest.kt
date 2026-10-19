package com.flakm.einkbridge

import android.view.View
import androidx.test.core.app.ActivityScenario
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import kotlin.math.roundToInt

/**
 * The tablet is 992dp wide in portrait. The toolbar used to add up to more than that,
 * so DONE sat off the right edge and the layout looked finished in the code and broken
 * on the device. Every width here is measured through the real layout, not summed.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34], qualifiers = "w992dp-h1322dp-xhdpi")
class PenToolbarLayoutTest {
    private val budgetDp = 900f

    @Test
    fun inDrawModeTheToolbarFitsPortraitWithSlack() = assertToolbarFits(ToolMode.DRAW)

    @Test
    fun inMoveModeWithTheContextBarTheToolbarFitsPortraitWithSlack() = assertToolbarFits(ToolMode.MOVE)

    private fun assertToolbarFits(mode: ToolMode) {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                val dm = activity.resources.displayMetrics
                assertEquals(992, (dm.widthPixels / dm.density).roundToInt())
                val toolbar = activity.penToolbar
                toolbar.visibility = View.VISIBLE
                activity.setToolMode(mode)
                val contextBar = activity.findViewById<View>(R.id.contextBar)
                if (mode == ToolMode.MOVE) contextBar.visibility = View.VISIBLE
                assertEquals(
                    "draw controls are shown only in DRAW mode",
                    if (mode == ToolMode.DRAW) View.VISIBLE else View.GONE,
                    activity.findViewById<View>(R.id.drawControls).visibility,
                )

                val content = activity.findViewById<View>(android.R.id.content)
                content.measure(exactly(dm.widthPixels), exactly(dm.heightPixels))
                content.layout(0, 0, dm.widthPixels, dm.heightPixels)

                val done = activity.findViewById<View>(R.id.btnSubmit)
                val doneRight = leftInRoot(done, toolbar) + done.width + toolbar.paddingRight
                assertTrue("DONE ends at ${doneRight}px, the toolbar at ${toolbar.width}px", doneRight <= toolbar.width)
                assertTrue(
                    "the spacer has collapsed: the row is wider than the screen",
                    activity.findViewById<View>(R.id.toolbarSpacer).width > 0,
                )

                toolbar.measure(View.MeasureSpec.UNSPECIFIED, exactly(toolbar.height))
                val naturalDp = toolbar.measuredWidth / dm.density
                assertTrue("the toolbar wants ${naturalDp}dp in $mode; the budget is ${budgetDp}dp", naturalDp <= budgetDp)
            }
        }
    }

    @Test
    fun theColourDrawerOpensCentredOverItsButton() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            scenario.onActivity { activity ->
                val dm = activity.resources.displayMetrics
                activity.penToolbar.visibility = View.VISIBLE
                val content = activity.findViewById<View>(android.R.id.content)
                content.measure(exactly(dm.widthPixels), exactly(dm.heightPixels))
                content.layout(0, 0, dm.widthPixels, dm.heightPixels)

                val btnColor = activity.findViewById<View>(R.id.btnColor)
                btnColor.performClick()
                val drawer = activity.colorDrawer
                assertEquals(View.VISIBLE, drawer.visibility)
                val buttonCentre = leftInRoot(btnColor, content) + btnColor.width / 2
                val drawerCentre = drawer.translationX + drawer.measuredWidth / 2f
                assertEquals("the drawer is centred over the colour button", buttonCentre.toFloat(), drawerCentre, dm.density)

                btnColor.performClick()
                assertEquals("a second tap closes it", View.GONE, drawer.visibility)
            }
        }
    }

    private fun exactly(px: Int) = View.MeasureSpec.makeMeasureSpec(px, View.MeasureSpec.EXACTLY)

    private fun leftInRoot(view: View, root: View): Int {
        var x = 0
        var v: View? = view
        while (v != null && v !== root) {
            x += v.left
            v = v.parent as? View
        }
        return x
    }
}
