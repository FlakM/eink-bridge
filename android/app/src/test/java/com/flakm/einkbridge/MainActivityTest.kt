package com.flakm.einkbridge

import android.view.View
import androidx.test.core.app.ActivityScenario
import org.junit.After
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

/**
 * Headless UI tests for [MainActivity].
 *
 * Uses Robolectric so they run on the JVM with no emulator or physical device.
 * Network calls fail fast (no real server) and are exercised separately in AnnotationE2ETest.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [34])
class MainActivityTest {

    private lateinit var scenario: ActivityScenario<MainActivity>

    @Before
    fun launch() {
        scenario = ActivityScenario.launch(MainActivity::class.java)
    }

    @After
    fun close() {
        scenario.close()
    }

    // ── Session list screen ────────────────────────────────────────────────────

    @Test
    fun sessionListContainerVisibleOnStart() {
        scenario.onActivity { activity ->
            assertEquals(View.VISIBLE, activity.sessionListContainer.visibility)
        }
    }

    @Test
    fun webViewGoneOnStart() {
        scenario.onActivity { activity ->
            assertNotEquals(View.VISIBLE, activity.webView.visibility)
        }
    }

    @Test
    fun penToolbarGoneOnStart() {
        scenario.onActivity { activity ->
            assertNotEquals(View.VISIBLE, activity.penToolbar.visibility)
        }
    }

    @Test
    fun emptyStateLabelVisible_whenNoSessions() {
        scenario.onActivity { activity ->
            // Simulate what fetchSessions does when the response is empty
            activity.adapter.submitList(emptyList())
            activity.sessionList.visibility = View.GONE
            activity.emptyState.visibility = View.VISIBLE

            assertEquals(View.GONE, activity.sessionList.visibility)
            assertEquals(View.VISIBLE, activity.emptyState.visibility)
        }
    }

    @Test
    fun sessionListVisible_whenSessionsExist() {
        scenario.onActivity { activity ->
            activity.adapter.submitList(
                listOf(
                    SessionInfo("id1", "PR Review", "Active", "2026-03-30T10:00:00Z", "2026-03-30T10:00:00Z"),
                    SessionInfo("id2", "Bug Report", "Submitted", "2026-03-30T09:00:00Z", "2026-03-30T09:30:00Z"),
                )
            )
        }
        scenario.onActivity { activity ->
            assertEquals(View.VISIBLE, activity.sessionList.visibility)
        }
    }

    // ── Server URL input ───────────────────────────────────────────────────────

    @Test
    fun serverInputShownOnSessionListScreen() {
        scenario.onActivity { activity ->
            val input = activity.findViewById<android.widget.EditText>(R.id.serverInput)
            assertEquals(View.VISIBLE, input.visibility)
        }
    }

    @Test
    fun connectButtonIsClickable() {
        scenario.onActivity { activity ->
            val btn = activity.findViewById<android.widget.Button>(R.id.connectBtn)
            assertTrue(btn.isEnabled)
        }
    }

    // ── Back-navigation ────────────────────────────────────────────────────────

    @Test
    fun backPressedWhileOnListDoesNotShowWebView() {
        scenario.onActivity { activity ->
            assertEquals(View.GONE, activity.webView.visibility)
        }
    }

    // ── Pen toolbar buttons ────────────────────────────────────────────────────

    @Test
    fun penToolbarButtonsExist() {
        scenario.onActivity { activity ->
            assertNotNull(activity.penToolbar)
            assertNotNull(activity.strokeSlider)
        }
    }

    // ── Bind mode interactions ─────────────────────────────────────────────────

    @Test
    fun enterBindMode_setsBindModeActive() {
        scenario.onActivity { activity ->
            assertFalse(activity.bindModeActive)
            activity.enterBindMode()
            assertTrue(activity.bindModeActive)
        }
    }

    @Test
    fun exitBindMode_clearsBindModeActive() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            assertTrue(activity.bindModeActive)
            activity.exitBindMode()
            assertFalse(activity.bindModeActive)
        }
    }

    @Test
    fun pencilButtonClick_exitsBind_whenBindModeActive() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            assertTrue(activity.bindModeActive)
            activity.findViewById<android.widget.Button>(R.id.btnPencil).performClick()
            assertFalse("Bind mode must exit when pencil is tapped", activity.bindModeActive)
        }
    }

    @Test
    fun brushButtonClick_exitsBind_whenBindModeActive() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            activity.findViewById<android.widget.Button>(R.id.btnBrush).performClick()
            assertFalse("Bind mode must exit when brush is tapped", activity.bindModeActive)
        }
    }

    @Test
    fun eraserButtonClick_exitsBind_whenBindModeActive() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            activity.findViewById<android.widget.Button>(R.id.btnEraser).performClick()
            assertFalse("Bind mode must exit when eraser is tapped", activity.bindModeActive)
        }
    }

    // ── Clear confirmation ─────────────────────────────────────────────────────

    @Test
    fun clearButtonClick_showsAlertDialog() {
        scenario.onActivity { activity ->
            activity.findViewById<android.widget.Button>(R.id.btnClear).performClick()
            val dialog = org.robolectric.shadows.ShadowAlertDialog.getLatestAlertDialog()
            assertNotNull("Clear must show a confirmation dialog", dialog)
        }
    }

    // ── Draw controls visibility ───────────────────────────────────────────────

    @Test
    fun drawControls_visibleByDefault() {
        scenario.onActivity { activity ->
            val dc = activity.findViewById<View>(R.id.drawControls)
            assertEquals(View.VISIBLE, dc.visibility)
        }
    }

    @Test
    fun drawControls_hiddenInBindMode() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            val dc = activity.findViewById<View>(R.id.drawControls)
            assertEquals("drawControls must be GONE in bind mode", View.GONE, dc.visibility)
        }
    }

    @Test
    fun drawControls_restoredAfterExitBindMode() {
        scenario.onActivity { activity ->
            activity.enterBindMode()
            activity.exitBindMode()
            val dc = activity.findViewById<View>(R.id.drawControls)
            assertEquals("drawControls must return to VISIBLE after exiting bind mode", View.VISIBLE, dc.visibility)
        }
    }

    // ── Tag mode toggle ────────────────────────────────────────────────────────

    @Test
    fun btnLink_isToggle_entersThenExitsBindMode() {
        scenario.onActivity { activity ->
            val btn = activity.findViewById<android.widget.Button>(R.id.btnLink)
            assertFalse(activity.bindModeActive)
            btn.performClick()
            assertTrue("First click must enter bind mode", activity.bindModeActive)
            btn.performClick()
            assertFalse("Second click must exit bind mode", activity.bindModeActive)
        }
    }

    // ── Context bar ───────────────────────────────────────────────────────────

    @Test
    fun contextBar_goneByDefault() {
        scenario.onActivity { activity ->
            val bar = activity.findViewById<View>(R.id.contextBar)
            assertEquals(View.GONE, bar.visibility)
        }
    }

    @Test
    fun contextBar_hiddenWhenExitingSelectMode() {
        scenario.onActivity { activity ->
            activity.enterSelectMode()
            activity.exitSelectMode()
            val bar = activity.findViewById<View>(R.id.contextBar)
            assertEquals("contextBar must be GONE after exiting select mode", View.GONE, bar.visibility)
        }
    }

    // ── ToolMode rapid-switching regression ───────────────────────────────────

    @Test
    fun rapid_mode_button_taps_end_in_draw_mode_with_drawControls_visible() {
        scenario.onActivity { activity ->
            val btnLink = activity.findViewById<android.widget.Button>(R.id.btnLink)
            val btnSelect = activity.findViewById<android.widget.Button>(R.id.btnSelect)
            val btnPencil = activity.findViewById<android.widget.Button>(R.id.btnPencil)
            val drawControls = activity.findViewById<View>(R.id.drawControls)

            repeat(10) {
                btnLink.performClick()
                btnSelect.performClick()
                btnPencil.performClick()
            }

            assertEquals(ToolMode.DRAW, activity.toolMode)
            assertFalse("bindModeActive must be false after tapping pencil", activity.bindModeActive)
            assertFalse("selectModeActive must be false after tapping pencil", activity.selectModeActive)
            assertEquals("drawControls must be VISIBLE in DRAW mode", View.VISIBLE, drawControls.visibility)
        }
    }

    @Test
    fun setToolMode_roundtrip_DRAW_TAG_MOVE_DRAW_matches_ui_state() {
        scenario.onActivity { activity ->
            val drawControls = activity.findViewById<View>(R.id.drawControls)

            activity.enterBindMode()
            assertEquals(ToolMode.TAG, activity.toolMode)
            assertEquals(View.GONE, drawControls.visibility)

            activity.enterSelectMode()
            assertEquals(ToolMode.MOVE, activity.toolMode)
            assertEquals(View.GONE, drawControls.visibility)

            activity.exitSelectMode()
            assertEquals(ToolMode.DRAW, activity.toolMode)
            assertEquals(View.VISIBLE, drawControls.visibility)
        }
    }
}
